use super::store::{CpuSkin, MeshStore, MESH_UPLOADS_PER_FRAME};
use crate::mesh_pipeline::MeshInstance;
use crate::RendererState;
use crate::skinned_pipeline::{SkinnedMeshInstance, MAX_JOINT_MATRICES, MAX_SKINNED_INSTANCES};
use crate::anim::LocalTransform;
use engine_app::scheduler::{InterpolationAlpha, System};
use engine_core::components::{AnimationPlayer, PreviousTransform, RenderMesh, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use glam::Mat4;
use hecs::Entity;
use std::collections::HashMap;

/// Scratch buffers behind `pose_player_into`, owned by `MeshRenderSyncSystem`
/// and reused across skinned instances and frames so posing settles into zero
/// heap allocations once warmed up.
#[derive(Default)]
pub(crate) struct PoseScratch {
    cur_pose:  Vec<LocalTransform>,
    prev_pose: Vec<LocalTransform>,
    done:      Vec<bool>,
    pub(crate) globals: Vec<Mat4>,
    pub(crate) palette: Vec<Mat4>,
}

/// Advance an `AnimationPlayer` by `dt`, sample its current pose (crossfading
/// out of `prev` if a blend is in progress), and write the joint palette plus
/// the bones' armature-space globals (pre-inverse-bind — what attachment
/// sockets need) into `scratch.palette` / `scratch.globals`. Pure
/// orchestration over `anim`'s sampling math — no GPU access. `scratch`'s
/// buffers are overwritten by the next call, so read them before calling
/// again.
pub(crate) fn pose_player_into(player: &mut AnimationPlayer, skin: &CpuSkin, dt: f32, scratch: &mut PoseScratch) {
    let n = skin.skeleton.joint_count();
    let clip_by_name = |name: &str| skin.clips.iter().find(|c| c.name == name);
    let Some(cur_clip) = clip_by_name(&player.clip).or_else(|| skin.clips.first()) else {
        // no clips: rest/bind pose
        scratch.globals.clear();
        scratch.globals.resize(n, Mat4::IDENTITY);
        scratch.palette.clear();
        scratch.palette.resize(n, Mat4::IDENTITY);
        return;
    };

    player.time += dt * player.speed;
    if player.looping && cur_clip.duration > 0.0 {
        player.time = player.time.rem_euclid(cur_clip.duration);
    } else {
        player.time = player.time.clamp(0.0, cur_clip.duration); // hold last frame
    }
    crate::anim::sample_pose_into(&skin.skeleton, cur_clip, player.time, &mut scratch.cur_pose);

    let pose: &[LocalTransform] = if player.prev.is_some() {
        player.blend_t += dt;
        let w = (player.blend_t / player.blend_dur).clamp(0.0, 1.0);
        let ptime = player.prev.as_ref().unwrap().time;
        // Look up the outgoing clip by reference into `player.prev` — the
        // borrow ends with this call, so the name never needs cloning.
        let prev_clip = clip_by_name(&player.prev.as_ref().unwrap().clip).or_else(|| skin.clips.first());
        if w >= 1.0 {
            player.prev = None;
        }
        match prev_clip {
            Some(pc) => {
                crate::anim::sample_pose_into(&skin.skeleton, pc, ptime, &mut scratch.prev_pose);
                scratch.cur_pose = crate::anim::blend_poses(&scratch.prev_pose, &scratch.cur_pose, w);
                &scratch.cur_pose
            }
            None => &scratch.cur_pose,
        }
    } else {
        &scratch.cur_pose
    };

    crate::anim::global_transforms_into(&skin.skeleton, pose, &mut scratch.globals, &mut scratch.done);
    crate::anim::palette_into(&scratch.globals, &skin.skeleton.joints, &mut scratch.palette);
}

/// Allocating wrapper over `pose_player_into`, kept so the unit test below
/// gets an owned palette/globals pair without holding a `PoseScratch` itself.
#[cfg(test)]
pub(crate) fn pose_player(player: &mut AnimationPlayer, skin: &CpuSkin, dt: f32) -> (Vec<Mat4>, Vec<Mat4>) {
    let mut scratch = PoseScratch::default();
    pose_player_into(player, skin, dt, &mut scratch);
    (scratch.palette, scratch.globals)
}

// ── Per-frame draw lists ────────────────────────────────────────────────────

/// Built by MeshRenderSyncSystem each display frame, consumed by RenderSystem.
/// `ranges` are (mesh index, first instance, instance count) into `instances`.
#[derive(Default)]
pub struct MeshDrawList {
    pub(crate) instances: Vec<MeshInstance>,
    pub(crate) ranges:    Vec<(usize, u32, u32)>,
}

/// The skinned counterpart: each instance additionally names a `joint_base`
/// offset into the flat `joints` palette (one contiguous block per instance).
#[derive(Default)]
pub struct SkinnedDrawList {
    pub(crate) instances: Vec<SkinnedMeshInstance>,
    pub(crate) joints:    Vec<[[f32; 4]; 4]>,
    pub(crate) ranges:    Vec<(usize, u32, u32)>,
}

/// Bone names published as attachment sockets each frame. Game code narrows or
/// widens this to the bones it actually reads.
pub struct SocketConfig {
    pub bones: Vec<String>,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self { bones: vec!["handslot.r".into(), "handslot.l".into(), "head".into()] }
    }
}

/// World-space bone transforms (`model · global[joint]`) for every skinned
/// entity posed this display frame, keyed by the bone names in `SocketConfig`.
/// Rebuilt by `MeshRenderSyncSystem`; consumers treat a missing entry as "no
/// socket this frame" and fall back to an entity-relative offset.
#[derive(Default)]
pub struct SocketTransforms(pub HashMap<Entity, HashMap<String, Mat4>>);

/// Collects every (Transform, RenderMesh) entity into the draw lists, loading
/// meshes on first sight. Static meshes go to MeshDrawList; skinned meshes are
/// posed (advancing their AnimationPlayer) and go to SkinnedDrawList. Position
/// is lerped against PreviousTransform exactly like RenderSyncSystem.
pub struct MeshRenderSyncSystem {
    // Scratch, reused across frames.
    items:         Vec<(usize, MeshInstance)>,
    skinned_items: Vec<(usize, SkinnedMeshInstance)>,
    pose_scratch:  PoseScratch,
    /// Throttles the 80%-of-cap warning to ~once per 5 s.
    warn_accum: f32,
}

impl MeshRenderSyncSystem {
    pub fn new() -> Self {
        Self {
            items:         Vec::new(),
            skinned_items: Vec::new(),
            pose_scratch:  PoseScratch::default(),
            warn_accum:    0.0,
        }
    }
}

impl System for MeshRenderSyncSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        // Headless / pre-window: renderer resources absent, nothing to sync.
        if resources.get::<RendererState>().is_none() {
            return;
        }
        let alpha = resources.get::<InterpolationAlpha>().map(|a| a.0).unwrap_or(1.0);

        // Take the stores out so they can borrow device/queue from RendererState
        // (Resources allows one borrow at a time; owned meanwhile).
        let Some(mut store) = resources.get_mut::<MeshStore>().map(std::mem::take) else { return };
        let mut list = resources.get_mut::<MeshDrawList>().map(std::mem::take).unwrap_or_default();
        let mut skinned = resources.get_mut::<SkinnedDrawList>().map(std::mem::take).unwrap_or_default();
        let socket_bones: Vec<String> = resources
            .get::<SocketConfig>()
            .map(|c| c.bones.clone())
            .unwrap_or_default();
        let mut sockets = resources.get_mut::<SocketTransforms>().map(std::mem::take).unwrap_or_default();
        sockets.0.clear();
        list.instances.clear();
        list.ranges.clear();
        skinned.instances.clear();
        skinned.joints.clear();
        skinned.ranges.clear();
        self.items.clear();
        self.skinned_items.clear();

        // Skinned entities lacking a player: attach a default after the query.
        let mut needs_player: Vec<Entity> = Vec::new();

        {
            let state = resources.get::<RendererState>().expect("checked above");
            store.integrate(&state.device, &state.queue, &state.material_bgl, &state.mipgen, MESH_UPLOADS_PER_FRAME);
            for (entity, transform, prev, mesh, player) in world
                .query::<(Entity, &Transform, Option<&PreviousTransform>, &RenderMesh, Option<&mut AnimationPlayer>)>()
                .iter()
            {
                let Some(idx) = store.get_or_request(&mesh.asset) else { continue };
                let render_pos = match prev {
                    Some(p) => p.position.lerp(transform.position, alpha),
                    None    => transform.position,
                };
                let model = Transform {
                    position: render_pos,
                    rotation: transform.rotation,
                    scale:    transform.scale,
                }.to_model_matrix();
                let tint = [mesh.tint.x, mesh.tint.y, mesh.tint.z, 1.0];

                match store.meshes[idx].skin.as_ref() {
                    // Static mesh — no skeleton to pose.
                    None => self.items.push((idx, MeshInstance {
                        model: model.to_cols_array_2d(),
                        tint,
                    })),
                    // Skinned mesh — needs an AnimationPlayer to pose.
                    Some(cpu_skin) => {
                        let Some(player) = player else {
                            needs_player.push(entity); // render next frame, once attached
                            continue;
                        };
                        let n = cpu_skin.skeleton.joint_count();
                        // Respect the palette/instance caps.
                        if self.skinned_items.len() >= MAX_SKINNED_INSTANCES
                            || skinned.joints.len() + n > MAX_JOINT_MATRICES
                        {
                            continue;
                        }
                        let joint_base = skinned.joints.len() as u32;
                        pose_player_into(player, cpu_skin, delta, &mut self.pose_scratch);
                        skinned.joints.extend(self.pose_scratch.palette.iter().map(|m| m.to_cols_array_2d()));
                        // Publish the configured attachment sockets for this entity.
                        if !socket_bones.is_empty() {
                            let entry = sockets.0.entry(entity).or_default();
                            for bone in &socket_bones {
                                if let Some(j) = cpu_skin
                                    .skeleton
                                    .joints
                                    .iter()
                                    .position(|jt| jt.name == *bone)
                                {
                                    entry.insert(bone.clone(), model * self.pose_scratch.globals[j]);
                                }
                            }
                        }
                        self.skinned_items.push((idx, SkinnedMeshInstance {
                            model: model.to_cols_array_2d(),
                            tint,
                            joint_base,
                            _pad: [0; 3],
                        }));
                    }
                }
            }
        }

        for entity in needs_player {
            let _ = world.insert_one(entity, AnimationPlayer::default());
        }

        // Group by mesh so each mesh draws as one instanced call.
        self.items.sort_by_key(|(idx, _)| *idx);
        for (idx, inst) in self.items.drain(..) {
            let first = list.instances.len() as u32;
            match list.ranges.last_mut() {
                Some((last_idx, _, count)) if *last_idx == idx => *count += 1,
                _ => list.ranges.push((idx, first, 1)),
            }
            list.instances.push(inst);
        }
        self.skinned_items.sort_by_key(|(idx, _)| *idx);
        for (idx, inst) in self.skinned_items.drain(..) {
            let first = skinned.instances.len() as u32;
            match skinned.ranges.last_mut() {
                Some((last_idx, _, count)) if *last_idx == idx => *count += 1,
                _ => skinned.ranges.push((idx, first, 1)),
            }
            skinned.instances.push(inst);
        }

        // Cap guardrails: meter in the dev overlay, throttled warning past
        // 80% — the seam that flags the future enemy influx early.
        let skinned_count = skinned.instances.len();
        if let Some(stats) = resources.get_mut::<engine_app::dev_stats::DevStats>() {
            stats.set("skinned", format!("{skinned_count}/{MAX_SKINNED_INSTANCES}"));
            stats.set("streaming", format!("{} pending", store.pending_count()));
            stats.set("tex mem (assets)", format!("{} MB", store.texture_memory_bytes() / (1024 * 1024)));
        }
        self.warn_accum += delta;
        if skinned_count * 10 > MAX_SKINNED_INSTANCES * 8 && self.warn_accum >= 5.0 {
            self.warn_accum = 0.0;
            log::warn!(
                "skinned instances at {skinned_count}/{MAX_SKINNED_INSTANCES} (>80% of the engine cap)"
            );
        }

        resources.insert(store);
        resources.insert(list);
        resources.insert(skinned);
        resources.insert(sockets);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Validates that pose_player advances the skeleton as time progresses.
    /// Catches a "character renders but doesn't animate" bug.
    #[test]
    fn human_locomotion_clips_actually_animate() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = super::super::gltf_import::load_gltf_data(path).unwrap();
        let skin = CpuSkin { skeleton: data.skeleton.unwrap(), clips: data.clips };
        for clip in ["idle", "walk", "run"] {
            let mut player = AnimationPlayer { clip: clip.into(), ..Default::default() };
            let (a, _) = pose_player(&mut player, &skin, 0.0);
            let (b, _) = pose_player(&mut player, &skin, 0.25);
            let moved = a.iter().zip(&b).any(|(x, y)| !x.abs_diff_eq(*y, 1e-4));
            assert!(moved, "clip {clip} must move the skeleton as time advances");
        }
    }

    fn stub_skin(joint_count: usize) -> CpuSkin {
        let joints = (0..joint_count)
            .map(|i| crate::anim::Joint {
                parent:       if i == 0 { None } else { Some(i - 1) },
                inverse_bind: Mat4::IDENTITY,
                rest:         crate::anim::LocalTransform::IDENTITY,
                name:         format!("bone{i}"),
            })
            .collect();
        let skeleton = crate::anim::Skeleton { joints, root: Mat4::IDENTITY };
        let clip = crate::anim::AnimationClip {
            name:     "clip_a".into(),
            duration: 1.0,
            tracks:   vec![crate::anim::JointTracks::default(); joint_count],
        };
        CpuSkin { skeleton, clips: vec![clip] }
    }

    /// The whole point of `pose_player_into`: once its `PoseScratch` is
    /// warmed by a first call, posing the same rig again must not grow any
    /// of its buffers — the steady-state per-frame allocation count is zero.
    #[test]
    fn pose_player_into_stops_growing_scratch_buffers_after_warmup() {
        let skin = stub_skin(64);
        let mut player = AnimationPlayer { clip: "clip_a".into(), ..Default::default() };
        let mut scratch = PoseScratch::default();

        pose_player_into(&mut player, &skin, 0.016, &mut scratch); // warm-up call
        let cap_globals = scratch.globals.capacity();
        let cap_palette = scratch.palette.capacity();
        let cap_pose = scratch.cur_pose.capacity();
        assert!(cap_globals >= 64 && cap_palette >= 64 && cap_pose >= 64);

        for _ in 0..5 {
            pose_player_into(&mut player, &skin, 0.016, &mut scratch);
            assert_eq!(scratch.globals.capacity(), cap_globals, "globals buffer grew after warm-up");
            assert_eq!(scratch.palette.capacity(), cap_palette, "palette buffer grew after warm-up");
            assert_eq!(scratch.cur_pose.capacity(), cap_pose, "pose buffer grew after warm-up");
        }
    }
}
