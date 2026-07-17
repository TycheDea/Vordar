use super::store::{CpuSkin, MeshStore, MESH_UPLOADS_PER_FRAME};
use crate::culling::{classify, Frustum, Visibility};
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

/// Distance beyond which a rig poses at half rate (`pose_with_lod`). Matches
/// the server's replication AOI radius (server/vordar-server/src/net/mod.rs
/// AOI_RADIUS) — past that edge a rig is already replication fringe, so
/// halving its pose rate costs no fidelity even at max zoom.
pub(crate) const LOD_POSE_DISTANCE: f32 = 40.0;

/// Cached pose for a far rig that skipped a `pose_player_into` call: the
/// palette/globals it last computed, the `dt` banked since then so the next
/// pose stays wall-clock exact, and the frame it was last touched so
/// `MeshRenderSyncSystem` can evict entries for despawned rigs.
pub(crate) struct LodEntry {
    palette:    Vec<Mat4>,
    globals:    Vec<Mat4>,
    pending_dt: f32,
    last_seen:  u64,
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

/// Distance-LOD wrapper over `pose_player_into`. `!far` drops any stale cache
/// entry and poses every call, same as before LOD existed. `far` poses only
/// on first sight or when `(frame + entity.id()) % 2 == 0` — half the calls,
/// staggered across entities by id so far rigs don't all pose on the same
/// frame — replaying the cached palette/globals on skipped frames and
/// banking `delta` into `pending_dt` so the next pose applies the full
/// elapsed time instead of just its own frame's slice.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pose_with_lod(
    lod:     &mut HashMap<Entity, LodEntry>,
    entity:  Entity,
    frame:   u64,
    far:     bool,
    player:  &mut AnimationPlayer,
    skin:    &CpuSkin,
    delta:   f32,
    scratch: &mut PoseScratch,
) {
    if !far {
        lod.remove(&entity);
        pose_player_into(player, skin, delta, scratch);
        return;
    }

    let is_new = !lod.contains_key(&entity);
    let entry = lod.entry(entity).or_insert_with(|| LodEntry {
        palette:    Vec::new(),
        globals:    Vec::new(),
        pending_dt: 0.0,
        last_seen:  frame,
    });

    if is_new || (frame + entity.id() as u64).is_multiple_of(2) {
        pose_player_into(player, skin, delta + entry.pending_dt, scratch);
        entry.pending_dt = 0.0;
        entry.palette.clear();
        entry.palette.extend_from_slice(&scratch.palette);
        entry.globals.clear();
        entry.globals.extend_from_slice(&scratch.globals);
    } else {
        entry.pending_dt += delta;
        scratch.palette.clear();
        scratch.palette.extend_from_slice(&entry.palette);
        scratch.globals.clear();
        scratch.globals.extend_from_slice(&entry.globals);
    }
    entry.last_seen = frame;
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
/// `ranges` are (mesh index, first instance, instance count) into `instances`,
/// covering the camera-frustum-visible instances; `shadow_ranges` covers the
/// sun-volume-visible ones, which is a different subset of the same buffer.
#[derive(Default)]
pub struct MeshDrawList {
    pub(crate) instances:     Vec<MeshInstance>,
    pub(crate) ranges:        Vec<(usize, u32, u32)>,
    pub(crate) shadow_ranges: Vec<(usize, u32, u32)>,
}

/// Sorts `items` by `(mesh_idx, visibility)` — the `Visibility` `Ord` puts
/// `Both` before `CamOnly` before `ShadowOnly` — then packs each mesh's group
/// into one contiguous run of `instances` and emits: a camera range over the
/// `Both`+`CamOnly` prefix, a shadow range over the `Both` prefix, and a
/// shadow range over the `ShadowOnly` suffix, each skipped when empty. Both
/// range lists stay ascending by `first`. Drains `items` for reuse next frame.
pub(crate) fn pack_visible<T: Copy>(
    items:         &mut Vec<(usize, Visibility, T)>,
    instances:     &mut Vec<T>,
    ranges:        &mut Vec<(usize, u32, u32)>,
    shadow_ranges: &mut Vec<(usize, u32, u32)>,
) {
    items.sort_by_key(|(idx, vis, _)| (*idx, *vis));

    let mut i = 0;
    while i < items.len() {
        let mesh_idx = items[i].0;
        let mut j = i;
        let mut both = 0u32;
        let mut cam_only = 0u32;
        let mut shadow_only = 0u32;
        let first = instances.len() as u32;
        while j < items.len() && items[j].0 == mesh_idx {
            match items[j].1 {
                Visibility::Both => both += 1,
                Visibility::CamOnly => cam_only += 1,
                Visibility::ShadowOnly => shadow_only += 1,
            }
            instances.push(items[j].2);
            j += 1;
        }
        if both + cam_only > 0 {
            ranges.push((mesh_idx, first, both + cam_only));
        }
        if both > 0 {
            shadow_ranges.push((mesh_idx, first, both));
        }
        if shadow_only > 0 {
            shadow_ranges.push((mesh_idx, first + both + cam_only, shadow_only));
        }
        i = j;
    }
    items.clear();
}

/// The skinned counterpart: each instance additionally names a `joint_base`
/// offset into the flat `joints` palette (one contiguous block per instance).
#[derive(Default)]
pub struct SkinnedDrawList {
    pub(crate) instances:     Vec<SkinnedMeshInstance>,
    pub(crate) joints:        Vec<[[f32; 4]; 4]>,
    pub(crate) ranges:        Vec<(usize, u32, u32)>,
    pub(crate) shadow_ranges: Vec<(usize, u32, u32)>,
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
    items:         Vec<(usize, Visibility, MeshInstance)>,
    skinned_items: Vec<(usize, Visibility, SkinnedMeshInstance)>,
    pose_scratch:  PoseScratch,
    /// Distance-LOD pose cache, keyed by entity; entries not touched this
    /// frame are evicted after the entity loop.
    lod:   HashMap<Entity, LodEntry>,
    frame: u64,
    /// Throttles the 80%-of-cap warning to ~once per 5 s.
    warn_accum: f32,
}

impl Default for MeshRenderSyncSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl MeshRenderSyncSystem {
    pub fn new() -> Self {
        Self {
            items:         Vec::new(),
            skinned_items: Vec::new(),
            pose_scratch:  PoseScratch::default(),
            lod:           HashMap::new(),
            frame:         0,
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
        self.frame += 1;
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
        list.shadow_ranges.clear();
        skinned.instances.clear();
        skinned.joints.clear();
        skinned.ranges.clear();
        skinned.shadow_ranges.clear();
        self.items.clear();
        self.skinned_items.clear();

        // Skinned entities lacking a player: attach a default after the query.
        let mut needs_player: Vec<Entity> = Vec::new();
        // Total static-mesh entities considered this frame (drawn or culled) — the dev-overlay denominator.
        let mut total_statics: u32 = 0;

        {
            let state = resources.get::<RendererState>().expect("checked above");
            store.integrate(&state.device, &state.queue, &state.material_bgl, &state.mipgen, MESH_UPLOADS_PER_FRAME);
            let cam = Frustum::from_view_proj(state.camera.build_view_projection_matrix());
            let sun = Frustum::from_view_proj(crate::shadow::fit_light_vp(state.camera.target, state.light_dir));
            let camera_target = state.camera.target;
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
                    None => {
                        total_statics += 1;
                        let world_aabb = store.meshes[idx].local_aabb.transformed(&model);
                        let Some(vis) = classify(&world_aabb, &cam, &sun) else { continue };
                        self.items.push((idx, vis, MeshInstance {
                            model: model.to_cols_array_2d(),
                            tint,
                        }));
                    }
                    // Skinned mesh — needs an AnimationPlayer to pose.
                    Some(cpu_skin) => {
                        let world = store.meshes[idx].local_aabb.transformed(&model);
                        let Some(vis) = classify(&world, &cam, &sun) else { continue };
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
                        let far = transform.position.distance_squared(camera_target)
                            > LOD_POSE_DISTANCE * LOD_POSE_DISTANCE;
                        pose_with_lod(&mut self.lod, entity, self.frame, far, player, cpu_skin, delta, &mut self.pose_scratch);
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
                        self.skinned_items.push((idx, vis, SkinnedMeshInstance {
                            model: model.to_cols_array_2d(),
                            tint,
                            joint_base,
                            _pad: [0; 3],
                        }));
                    }
                }
            }
            self.lod.retain(|_, e| e.last_seen == self.frame);
        }

        for entity in needs_player {
            let _ = world.insert_one(entity, AnimationPlayer::default());
        }

        // Group by mesh so each mesh draws as one instanced call, split into
        // the camera-visible range and the shadow-visible range.
        pack_visible(&mut self.items, &mut list.instances, &mut list.ranges, &mut list.shadow_ranges);
        pack_visible(&mut self.skinned_items, &mut skinned.instances, &mut skinned.ranges, &mut skinned.shadow_ranges);

        // Cap guardrails: meter in the dev overlay, throttled warning past
        // 80% — the seam that flags the future enemy influx early.
        let skinned_count = skinned.instances.len();
        if let Some(stats) = resources.get_mut::<engine_app::dev_stats::DevStats>() {
            stats.set("skinned", format!("{skinned_count}/{MAX_SKINNED_INSTANCES}"));
            stats.set("streaming", format!("{} pending", store.pending_count()));
            stats.set("tex mem (assets)", format!("{} MB", store.texture_memory_bytes() / (1024 * 1024)));
            let camera_visible: u32 = list.ranges.iter().map(|&(_, _, count)| count).sum();
            stats.set("statics drawn", format!("{camera_visible}/{total_statics}"));
            let skinned_camera_visible: u32 = skinned.ranges.iter().map(|&(_, _, count)| count).sum();
            stats.set("skinned drawn", format!("{skinned_camera_visible}/{skinned_count}"));
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

    /// A far rig (`far = true`) poses only every other frame: on the frames
    /// it skips, `pose_with_lod` must replay the exact previous palette
    /// (bit-equal) and leave `player.time` untouched, while still banking
    /// the skipped `dt` so the next pose lands on wall-clock-exact time.
    /// The entity's first-ever call always poses (no cache yet), so the
    /// four tracked frames below start from a zero-`dt` seed call that
    /// creates the cache entry without advancing `player.time` — putting
    /// all four tracked frames under pure frame-parity.
    #[test]
    fn pose_with_lod_skips_far_rigs_every_other_frame_and_banks_dt() {
        let skin = stub_skin(4);
        let mut world = hecs::World::new();
        let far_entity = world.spawn(());
        let mut player = AnimationPlayer { clip: "clip_a".into(), ..Default::default() };
        let mut lod: HashMap<Entity, LodEntry> = HashMap::new();
        let mut scratch = PoseScratch::default();

        pose_with_lod(&mut lod, far_entity, 0, true, &mut player, &skin, 0.0, &mut scratch);
        assert_eq!(player.time, 0.0, "seed call must not advance time");
        let mut last_posed_palette = scratch.palette.clone();

        let mut poses = 0;
        for frame in 1..=4u64 {
            let time_before = player.time;
            pose_with_lod(&mut lod, far_entity, frame, true, &mut player, &skin, 0.016, &mut scratch);
            if player.time != time_before {
                poses += 1;
                last_posed_palette = scratch.palette.clone();
            } else {
                assert_eq!(player.time, time_before, "skipped frame {frame} must not advance player.time");
                assert_eq!(
                    scratch.palette, last_posed_palette,
                    "skipped frame {frame} must replay the previous posed palette bit-for-bit"
                );
            }
        }

        assert_eq!(poses, 2, "far rig must pose exactly twice across the 4 tracked frames");
        assert!(
            (player.time - 4.0 * 0.016).abs() < 1e-6,
            "banked dt from skipped frames must fully replay by frame 4, got {}",
            player.time
        );
    }

    /// A near rig (`far = false`) poses every call and carries no LOD cache
    /// entry at all — the LOD path must be a no-op for in-range rigs.
    #[test]
    fn pose_with_lod_poses_near_rig_every_frame_with_no_cache_entry() {
        let skin = stub_skin(4);
        let mut world = hecs::World::new();
        let near_entity = world.spawn(());
        let mut player = AnimationPlayer { clip: "clip_a".into(), ..Default::default() };
        let mut lod: HashMap<Entity, LodEntry> = HashMap::new();
        let mut scratch = PoseScratch::default();

        for frame in 1..=4u64 {
            let time_before = player.time;
            pose_with_lod(&mut lod, near_entity, frame, false, &mut player, &skin, 0.016, &mut scratch);
            assert_ne!(player.time, time_before, "near rig must pose every frame");
        }

        assert!((player.time - 4.0 * 0.016).abs() < 1e-6);
        assert!(!lod.contains_key(&near_entity), "near rig must leave no lod cache entry");
    }

    /// `MeshRenderSyncSystem::run` evicts an entity's LOD entry once a frame
    /// passes without it being touched (e.g. it despawned) — the same
    /// `last_seen == frame` retain predicate the system runs after its
    /// entity loop.
    #[test]
    fn lod_retain_evicts_rigs_not_seen_this_frame() {
        let mut world = hecs::World::new();
        let entity = world.spawn(());
        let mut lod: HashMap<Entity, LodEntry> = HashMap::new();
        lod.insert(entity, LodEntry { palette: Vec::new(), globals: Vec::new(), pending_dt: 0.0, last_seen: 5 });

        let frame = 6u64; // entity absent from this frame's entity loop
        lod.retain(|_, e| e.last_seen == frame);

        assert!(!lod.contains_key(&entity), "a rig absent this frame must be evicted from the lod cache");
    }

    /// Two meshes: mesh 0 gets one instance of every `Visibility` variant,
    /// mesh 1 gets only a `ShadowOnly` instance. Sentinel `u32` payloads let
    /// the assertions read off the exact packed order and range math.
    #[test]
    fn pack_visible_orders_by_mesh_then_visibility_and_splits_camera_shadow_ranges() {
        let mut items: Vec<(usize, Visibility, u32)> = vec![
            (0, Visibility::ShadowOnly, 102),
            (0, Visibility::Both, 100),
            (0, Visibility::CamOnly, 101),
            (1, Visibility::ShadowOnly, 200),
        ];
        let mut instances: Vec<u32> = Vec::new();
        let mut ranges = Vec::new();
        let mut shadow_ranges = Vec::new();

        pack_visible(&mut items, &mut instances, &mut ranges, &mut shadow_ranges);

        assert!(items.is_empty(), "items must be drained for reuse next frame");
        assert_eq!(instances, vec![100, 101, 102, 200], "Both, then CamOnly, then ShadowOnly, per mesh group");
        assert_eq!(ranges, vec![(0, 0, 2)], "camera range covers only the Both+CamOnly prefix of mesh 0");
        assert_eq!(
            shadow_ranges,
            vec![(0, 0, 1), (0, 2, 1), (1, 3, 1)],
            "shadow ranges: mesh 0's Both prefix, mesh 0's ShadowOnly suffix, then mesh 1's ShadowOnly-only run"
        );
        assert!(
            !ranges.iter().any(|&(idx, _, _)| idx == 1),
            "mesh 1 has no camera-visible instances, so it must be absent from the camera ranges"
        );
    }

    /// The skinned counterpart of `pack_visible_orders_by_mesh_then_visibility_...`:
    /// joints are appended to the palette at pose time, before packing, so
    /// `joint_base` must ride along with its instance through the
    /// Both/CamOnly/ShadowOnly reorder — if packing and joint upload ever
    /// desynced, this is the corruption that would put one rig's pose on
    /// another's mesh.
    #[test]
    fn pack_visible_preserves_joint_base_across_reordering() {
        fn skinned(joint_base: u32) -> SkinnedMeshInstance {
            SkinnedMeshInstance { model: Mat4::IDENTITY.to_cols_array_2d(), tint: [1.0; 4], joint_base, _pad: [0; 3] }
        }
        let mut items: Vec<(usize, Visibility, SkinnedMeshInstance)> = vec![
            (0, Visibility::ShadowOnly, skinned(30)),
            (0, Visibility::Both, skinned(10)),
            (0, Visibility::CamOnly, skinned(20)),
        ];
        let mut instances = Vec::new();
        let mut ranges = Vec::new();
        let mut shadow_ranges = Vec::new();

        pack_visible(&mut items, &mut instances, &mut ranges, &mut shadow_ranges);

        let packed_bases: Vec<u32> = instances.iter().map(|i| i.joint_base).collect();
        assert_eq!(
            packed_bases,
            vec![10, 20, 30],
            "joint_base must travel with its instance through the Both/CamOnly/ShadowOnly reorder"
        );
        assert_eq!(ranges, vec![(0, 0, 2)], "camera range covers the Both+CamOnly prefix");
        assert_eq!(
            shadow_ranges,
            vec![(0, 0, 1), (0, 2, 1)],
            "shadow ranges: the Both prefix, then the ShadowOnly suffix"
        );
    }

    /// A real perspective camera and its fitted shadow volume disagree on a
    /// point 30 units behind the eye but still close to the target: culled
    /// from the camera frustum, still inside the shadow volume's fitted
    /// ortho box. A point far off both axes lands in neither.
    #[test]
    fn classify_combines_real_camera_and_shadow_frustum() {
        let camera = crate::camera::Camera::new(16.0 / 9.0);
        let cam_frustum = Frustum::from_view_proj(camera.build_view_projection_matrix());
        let sun_frustum = Frustum::from_view_proj(crate::shadow::fit_light_vp(
            camera.target,
            glam::Vec3::new(-1.0, 2.0, -1.0),
        ));

        let eye_dir = (camera.eye() - camera.target).normalize();
        let behind_eye = camera.eye() + eye_dir * 30.0;
        let near_target = crate::culling::Aabb {
            min: behind_eye - glam::Vec3::splat(0.5),
            max: behind_eye + glam::Vec3::splat(0.5),
        };
        assert_eq!(
            classify(&near_target, &cam_frustum, &sun_frustum),
            Some(Visibility::ShadowOnly),
            "behind the eye but within the shadow volume's fitted box must classify ShadowOnly"
        );

        let far_away = camera.target + glam::Vec3::new(300.0, 0.0, 0.0);
        let outside = crate::culling::Aabb {
            min: far_away - glam::Vec3::splat(0.5),
            max: far_away + glam::Vec3::splat(0.5),
        };
        assert_eq!(
            classify(&outside, &cam_frustum, &sun_frustum),
            None,
            "far outside both volumes must classify None"
        );
    }
}
