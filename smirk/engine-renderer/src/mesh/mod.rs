// glTF mesh rendering — the "real models" path next to the primitive pool.
//
// Two stages, split so the parse is testable without a GPU device:
//   CPU: load_gltf_data(path) -> MeshData   (gltf::import, node transforms
//        baked into vertices, per-primitive base-color material)
//   GPU: MeshStore::get_or_load uploads MeshData into vertex/index buffers
//        and a bind group per primitive (reuses the texture BGL).
//
// Unlike the SdfInstance pool there is no slot bookkeeping: the draw list is
// rebuilt from live entities every frame by MeshRenderSyncSystem, so despawn
// needs no hook and instancing falls out of grouping by mesh index.

mod gltf_import;
#[cfg(test)]
mod test_glb;

use crate::anim::{AnimationClip, Skeleton};
use crate::mesh_pipeline::{MaterialUniform, MeshInstance};
use crate::mipgen::MipGenerator;
use crate::skinned_pipeline::{SkinnedMeshInstance, SkinnedVertex, MAX_JOINT_MATRICES, MAX_SKINNED_INSTANCES};
use crate::texture::{self, ColorTexture};
use crate::RendererState;
use engine_app::scheduler::{InterpolationAlpha, System};
use engine_core::components::{AnimationPlayer, PreviousTransform, RenderMesh, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use glam::Mat4;
use hecs::Entity;
use std::collections::HashMap;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{BindGroup, BindGroupLayout, Buffer, BufferUsages, Device, Queue};

pub use gltf_import::{load_gltf_data, load_image_rgba, ImageData, MaterialData, MeshData, PrimitiveData, VertexSkin};

// ── GPU stage ─────────────────────────────────────────────────────────────────

pub(crate) struct GpuPrimitive {
    pub(crate) vertex_buffer: Buffer,
    pub(crate) index_buffer:  Buffer,
    pub(crate) index_count:   u32,
    // Textures + factor uniform kept alive alongside their bind group.
    pub(crate) _textures:          Vec<ColorTexture>,
    pub(crate) _material_buffer:   Buffer,
    pub(crate) material_bind_group: BindGroup,
}

/// CPU-side animation data kept next to a skinned GpuMesh so sampling needs no
/// GPU access. Present iff the mesh is skinned.
pub(crate) struct CpuSkin {
    pub(crate) skeleton: Skeleton,
    pub(crate) clips:    Vec<AnimationClip>,
}

pub(crate) struct GpuMesh {
    pub(crate) primitives: Vec<GpuPrimitive>,
    /// Some => primitives' vertex buffers hold `SkinnedVertex` and the mesh
    /// draws with the skinned pipeline; None => static (Phase-A) path.
    pub(crate) skin: Option<CpuSkin>,
}

/// One material texture slot: the image (sRGB or linear, mipped) when the
/// asset has one, else a 1×1 neutral default so the bind group is complete.
fn slot_texture(
    device:  &Device,
    queue:   &Queue,
    mipgen:  &MipGenerator,
    image:   &Option<ImageData>,
    srgb:    bool,
    neutral: [u8; 4],
) -> ColorTexture {
    match image {
        Some(img) => texture::create_rgba_texture_mipped(
            device, queue, mipgen, img.width, img.height, &img.pixels, srgb,
        ),
        None => texture::create_rgba_texture(device, queue, 1, 1, &neutral, false),
    }
}

pub(crate) fn upload_mesh(
    device: &Device,
    queue:  &Queue,
    layout: &BindGroupLayout,
    mipgen: &MipGenerator,
    data:   MeshData,
) -> GpuMesh {
    let skinned = data.skeleton.is_some();
    let primitives = data.primitives.iter().map(|p| {
        // Skinned meshes upload SkinnedVertex (adds joints/weights); static
        // meshes upload MeshVertex directly.
        let vertex_buffer = if skinned {
            let verts: Vec<SkinnedVertex> = p.vertices.iter().enumerate().map(|(i, v)| {
                let sk = p.skin.as_ref().map(|s| s[i]).unwrap_or(VertexSkin {
                    joints:  [0, 0, 0, 0],
                    weights: [1.0, 0.0, 0.0, 0.0],
                });
                SkinnedVertex {
                    position: v.position,
                    normal:   v.normal,
                    uv:       v.uv,
                    tangent:  v.tangent,
                    joints:   sk.joints,
                    weights:  sk.weights,
                }
            }).collect();
            device.create_buffer_init(&BufferInitDescriptor {
                label:    Some("Skinned Vertex Buffer"),
                contents: bytemuck::cast_slice(&verts),
                usage:    BufferUsages::VERTEX,
            })
        } else {
            device.create_buffer_init(&BufferInitDescriptor {
                label:    Some("Mesh Vertex Buffer"),
                contents: bytemuck::cast_slice(&p.vertices),
                usage:    BufferUsages::VERTEX,
            })
        };
        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label:    Some("Mesh Index Buffer"),
            contents: bytemuck::cast_slice(&p.indices),
            usage:    BufferUsages::INDEX,
        });

        // The five material textures (VQ-A2/C2): sRGB for color-like slots,
        // linear for data-like slots; 1×1 neutral defaults where absent.
        let m = &p.material;
        let albedo   = slot_texture(device, queue, mipgen, &m.base_color_image, true, [255; 4]);
        let normal   = slot_texture(device, queue, mipgen, &m.normal_image, false, [128, 128, 255, 255]);
        let mr       = slot_texture(device, queue, mipgen, &m.metallic_roughness_image, false, [255; 4]);
        let emissive = slot_texture(device, queue, mipgen, &m.emissive_image, true, [255; 4]);
        let ao       = slot_texture(device, queue, mipgen, &m.occlusion_image, false, [255; 4]);

        let uniform = MaterialUniform {
            base_color: m.base_color_factor,
            emissive: [
                m.emissive_factor[0] * m.emissive_strength,
                m.emissive_factor[1] * m.emissive_strength,
                m.emissive_factor[2] * m.emissive_strength,
                0.0,
            ],
            mr: [m.metallic_factor, m.roughness_factor, m.alpha_cutoff, 0.0],
        };
        let material_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label:    Some("Material Uniform"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage:    BufferUsages::UNIFORM,
        });

        let material_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("Material Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&albedo.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&albedo.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&normal.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&mr.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&emissive.view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&ao.view) },
                wgpu::BindGroupEntry { binding: 6, resource: material_buffer.as_entire_binding() },
            ],
        });

        GpuPrimitive {
            vertex_buffer,
            index_buffer,
            index_count: p.indices.len() as u32,
            _textures: vec![albedo, normal, mr, emissive, ao],
            _material_buffer: material_buffer,
            material_bind_group,
        }
    }).collect();

    let skin = data.skeleton.map(|skeleton| CpuSkin { skeleton, clips: data.clips });
    GpuMesh { primitives, skin }
}

/// Loaded meshes keyed by asset path. Failed loads are cached as None so a
/// bad path logs once, not every frame.
#[derive(Default)]
pub struct MeshStore {
    by_path:           HashMap<String, Option<usize>>,
    pub(crate) meshes: Vec<GpuMesh>,
}

impl MeshStore {
    /// Upload procedurally-built mesh data under a synthetic key (e.g.
    /// "zone-ground:start"). Re-registering a key uploads fresh data — zone
    /// rebuilds replace their ground.
    pub(crate) fn register(
        &mut self,
        device: &Device,
        queue:  &Queue,
        layout: &BindGroupLayout,
        mipgen: &MipGenerator,
        key:    &str,
        data:   MeshData,
    ) -> usize {
        let idx = self.meshes.len();
        self.meshes.push(upload_mesh(device, queue, layout, mipgen, data));
        self.by_path.insert(key.to_owned(), Some(idx));
        idx
    }

    pub(crate) fn get_or_load(
        &mut self,
        device: &Device,
        queue:  &Queue,
        layout: &BindGroupLayout,
        mipgen: &MipGenerator,
        path:   &str,
    ) -> Option<usize> {
        if let Some(&cached) = self.by_path.get(path) {
            return cached;
        }
        let result = match load_gltf_data(path) {
            Ok(data) => {
                let idx = self.meshes.len();
                self.meshes.push(upload_mesh(device, queue, layout, mipgen, data));
                Some(idx)
            }
            Err(e) => {
                log::error!("mesh load failed: {e}");
                None
            }
        };
        self.by_path.insert(path.to_owned(), result);
        result
    }
}

// ── Animation advance + skinning ────────────────────────────────────────────

/// Advance an `AnimationPlayer` by `dt`, sample its current pose (crossfading
/// out of `prev` if a blend is in progress), and return the joint palette plus
/// the bones' armature-space globals (pre-inverse-bind — what attachment
/// sockets need). Pure orchestration over `anim`'s sampling math — no GPU access.
pub(crate) fn pose_player(player: &mut AnimationPlayer, skin: &CpuSkin, dt: f32) -> (Vec<Mat4>, Vec<Mat4>) {
    let n = skin.skeleton.joint_count();
    let clip_by_name = |name: &str| skin.clips.iter().find(|c| c.name == name);
    let Some(cur_clip) = clip_by_name(&player.clip).or_else(|| skin.clips.first()) else {
        return (vec![Mat4::IDENTITY; n], vec![Mat4::IDENTITY; n]); // no clips: rest/bind pose
    };

    player.time += dt * player.speed;
    if player.looping && cur_clip.duration > 0.0 {
        player.time = player.time.rem_euclid(cur_clip.duration);
    } else {
        player.time = player.time.clamp(0.0, cur_clip.duration); // hold last frame
    }
    let cur_pose = crate::anim::sample_pose(&skin.skeleton, cur_clip, player.time);

    let pose = if player.prev.is_some() {
        player.blend_t += dt;
        let w = (player.blend_t / player.blend_dur).clamp(0.0, 1.0);
        let (pname, ptime) = {
            let p = player.prev.as_ref().unwrap();
            (p.clip.clone(), p.time)
        };
        let blended = match clip_by_name(&pname).or_else(|| skin.clips.first()) {
            Some(pc) => {
                let prev_pose = crate::anim::sample_pose(&skin.skeleton, pc, ptime);
                crate::anim::blend_poses(&prev_pose, &cur_pose, w)
            }
            None => cur_pose,
        };
        if w >= 1.0 {
            player.prev = None;
        }
        blended
    } else {
        cur_pose
    };

    let globals = crate::anim::global_transforms(&skin.skeleton, &pose);
    let palette = globals
        .iter()
        .zip(&skin.skeleton.joints)
        .map(|(g, j)| *g * j.inverse_bind)
        .collect();
    (palette, globals)
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
    // TEMP (anim feel-check): throttles a ~1 Hz log of each skinned player's clip.
    log_accum: f32,
    /// Throttles the 80%-of-cap warning (VQ-F2) to ~once per 5 s.
    warn_accum: f32,
}

impl MeshRenderSyncSystem {
    pub fn new() -> Self {
        Self { items: Vec::new(), skinned_items: Vec::new(), log_accum: 0.0, warn_accum: 0.0 }
    }
}

impl System for MeshRenderSyncSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        // Headless / pre-window: renderer resources absent, nothing to sync.
        if resources.get::<RendererState>().is_none() {
            return;
        }
        let alpha = resources.get::<InterpolationAlpha>().map(|a| a.0).unwrap_or(1.0);

        // TEMP (anim feel-check): once ~a second, log each skinned player's clip
        // so a headless dev can confirm the live pose is advancing. Remove once
        // the character animates on screen.
        self.log_accum += delta;
        let should_log = self.log_accum >= 1.0;
        if should_log {
            self.log_accum = 0.0;
        }

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
            for (entity, transform, prev, mesh, player) in world
                .query::<(Entity, &Transform, Option<&PreviousTransform>, &RenderMesh, Option<&mut AnimationPlayer>)>()
                .iter()
            {
                let Some(idx) = store.get_or_load(
                    &state.device, &state.queue, &state.material_bgl, &state.mipgen, &mesh.asset,
                )
                else { continue };
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
                    // Static mesh — Phase-A path.
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
                        let (mats, globals) = pose_player(player, cpu_skin, delta);
                        if should_log {
                            log::info!(
                                "skinned anim: clip={:?} time={:.2} blend={:.2} joints={}",
                                player.clip, player.time, player.blend_t, mats.len()
                            );
                        }
                        skinned.joints.extend(mats.iter().map(|m| m.to_cols_array_2d()));
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
                                    entry.insert(bone.clone(), model * globals[j]);
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

        // Cap guardrails (VQ-F2): meter in the dev overlay, throttled warning
        // past 80% — the seam that flags the future enemy influx early.
        let skinned_count = skinned.instances.len();
        if let Some(stats) = resources.get_mut::<engine_app::dev_stats::DevStats>() {
            stats.set("skinned", format!("{skinned_count}/{MAX_SKINNED_INSTANCES}"));
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
