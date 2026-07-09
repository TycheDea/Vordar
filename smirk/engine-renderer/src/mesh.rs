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

use crate::anim::{AnimationClip, Interp, Joint, JointTracks, LocalTransform, Skeleton, Track};
use crate::mesh_pipeline::{MaterialUniform, MeshInstance, MeshVertex};
use crate::mipgen::MipGenerator;
use crate::skinned_pipeline::{SkinnedMeshInstance, SkinnedVertex, MAX_JOINT_MATRICES, MAX_SKINNED_INSTANCES};
use crate::tangent::generate_tangents;
use crate::texture::{self, ColorTexture};
use crate::RendererState;
use engine_app::scheduler::{InterpolationAlpha, System};
use engine_core::components::{AnimationPlayer, PreviousTransform, RenderMesh, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use glam::{Mat3, Mat4, Quat, Vec3};
use hecs::Entity;
use std::collections::HashMap;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{BindGroup, BindGroupLayout, Buffer, BufferUsages, Device, Queue};

// ── CPU stage ─────────────────────────────────────────────────────────────────

/// RGBA8 pixels, sRGB-encoded (glTF base-color convention).
pub struct ImageData {
    pub width:  u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Per-vertex skin binding, parallel to `PrimitiveData::vertices`. Kept
/// separate from `Vertex` so static meshes carry no skinning overhead and the
/// static GPU path is untouched.
#[derive(Clone, Copy)]
pub struct VertexSkin {
    pub joints:  [u16; 4],
    pub weights: [f32; 4],
}

/// The full glTF metallic-roughness material of one primitive (VQ-A2/C2/C4).
/// Missing maps stay None and bind 1×1 neutral defaults at upload; factors
/// multiply per the glTF spec.
pub struct MaterialData {
    pub base_color_factor: [f32; 4],
    pub metallic_factor:   f32,
    pub roughness_factor:  f32,
    /// Fragment-discard threshold from glTF alphaMode: 0.0 for OPAQUE (never
    /// discards), the cutoff for MASK. BLEND is approximated as MASK at 0.5 —
    /// there is no sorted transparency pass.
    pub alpha_cutoff:      f32,
    pub emissive_factor:   [f32; 3],
    /// KHR_materials_emissive_strength (1.0 when absent) — HDR emissive for
    /// bloom (VQ-C3).
    pub emissive_strength: f32,
    pub base_color_image:         Option<ImageData>, // sRGB
    pub normal_image:             Option<ImageData>, // linear
    pub metallic_roughness_image: Option<ImageData>, // linear (g=rough, b=metal)
    pub emissive_image:           Option<ImageData>, // sRGB
    pub occlusion_image:          Option<ImageData>, // linear (r)
}

impl Default for MaterialData {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0; 4],
            metallic_factor:   1.0,
            roughness_factor:  1.0,
            alpha_cutoff:      0.0,
            emissive_factor:   [0.0; 3],
            emissive_strength: 1.0,
            base_color_image:         None,
            normal_image:             None,
            metallic_roughness_image: None,
            emissive_image:           None,
            occlusion_image:          None,
        }
    }
}

pub struct PrimitiveData {
    pub vertices: Vec<MeshVertex>,
    pub indices:  Vec<u32>,
    pub material: MaterialData,
    /// Present iff this primitive is skinned (node had a skin). When Some,
    /// `vertices` are in mesh-local space (node transform NOT baked) — the
    /// joint palette places them.
    pub skin: Option<Vec<VertexSkin>>,
}

pub struct MeshData {
    pub primitives: Vec<PrimitiveData>,
    /// The mesh's skeleton, if any primitive is skinned. One skin per file
    /// (our character assets); a second skin is ignored with a warning.
    pub skeleton: Option<Skeleton>,
    /// Named animation clips, indexed-by-joint tracks. Empty for static meshes.
    pub clips: Vec<AnimationClip>,
}

/// Parse a .glb/.gltf file into CPU-side mesh data. Node hierarchy transforms
/// are baked into the vertices (static meshes only — skinning comes later),
/// so one MeshData draws with a single model matrix per instance.
pub fn load_gltf_data(path: &str) -> Result<MeshData, String> {
    let (doc, buffers, images) =
        gltf::import(path).map_err(|e| format!("{path}: {e}"))?;
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or_else(|| format!("{path}: no scene"))?;

    let mut primitives = Vec::new();
    for node in scene.nodes() {
        visit_node(&node, Mat4::IDENTITY, &buffers, &images, path, &mut primitives);
    }
    if primitives.is_empty() {
        return Err(format!("{path}: no triangle primitives in scene"));
    }

    // Skeleton + animations (absent for static meshes → None / empty).
    let (skeleton, clips) = match extract_skeleton(&doc, &buffers, path) {
        Some((skel, node_to_joint)) => {
            let clips = extract_clips(&doc, &buffers, &node_to_joint, skel.joint_count());
            (Some(skel), clips)
        }
        None => (None, Vec::new()),
    };

    Ok(MeshData { primitives, skeleton, clips })
}

/// Build the skeleton from the file's first skin. Returns the skeleton plus a
/// `glTF node index → joint index` map used to route animation channels.
/// `None` for a static (skin-less) file.
fn extract_skeleton(
    doc:     &gltf::Document,
    buffers: &[gltf::buffer::Data],
    path:    &str,
) -> Option<(Skeleton, HashMap<usize, usize>)> {
    let skin = doc.skins().next()?;
    if doc.skins().count() > 1 {
        log::warn!("{path}: multiple skins — using the first");
    }

    let joint_nodes: Vec<gltf::Node> = skin.joints().collect();
    let node_to_joint: HashMap<usize, usize> = joint_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.index(), i))
        .collect();

    let reader = skin.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
    let ibms: Vec<Mat4> = reader
        .read_inverse_bind_matrices()
        .map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect())
        .unwrap_or_else(|| vec![Mat4::IDENTITY; joint_nodes.len()]);

    // child node index → parent node index, over every node in the document.
    let mut parent_of: HashMap<usize, usize> = HashMap::new();
    for node in doc.nodes() {
        for child in node.children() {
            parent_of.insert(child.index(), node.index());
        }
    }

    let joints: Vec<Joint> = joint_nodes
        .iter()
        .enumerate()
        .map(|(i, jn)| {
            // Parent = nearest ancestor that is itself a joint (skips a
            // non-joint "Armature" root above the bones).
            let mut cur = parent_of.get(&jn.index()).copied();
            let parent = loop {
                match cur {
                    Some(pn) => match node_to_joint.get(&pn) {
                        Some(&pj) => break Some(pj),
                        None => cur = parent_of.get(&pn).copied(),
                    },
                    None => break None,
                }
            };
            let (t, r, s) = jn.transform().decomposed();
            Joint {
                parent,
                inverse_bind: ibms.get(i).copied().unwrap_or(Mat4::IDENTITY),
                rest: LocalTransform {
                    translation: Vec3::from(t),
                    rotation:    Quat::from_array(r),
                    scale:       Vec3::from(s),
                },
                name: jn.name().unwrap_or_default().to_string(),
            }
        })
        .collect();

    // Root offset: the world transform of the non-joint nodes the bones hang
    // under (an exporter's "Armature"/"Rig" node, which may carry a scale or a
    // ground offset). Fold it into every root joint's global so an armature
    // authored at 2 m or grounded to the floor loads correctly. Taken from the
    // first root joint's ancestor chain (a single armature is the norm).
    let all_nodes: Vec<gltf::Node> = doc.nodes().collect();
    let root = joints
        .iter()
        .zip(joint_nodes.iter())
        .find(|(j, _)| j.parent.is_none())
        .map(|(_, top)| {
            let mut chain = Vec::new(); // immediate parent → … → scene root
            let mut cur = parent_of.get(&top.index()).copied();
            while let Some(n) = cur {
                chain.push(n);
                cur = parent_of.get(&n).copied();
            }
            chain.iter().rev().fold(Mat4::IDENTITY, |acc, &n| {
                acc * Mat4::from_cols_array_2d(&all_nodes[n].transform().matrix())
            })
        })
        .unwrap_or(Mat4::IDENTITY);

    Some((Skeleton { joints, root }, node_to_joint))
}

/// Read every animation into per-joint keyframe tracks. Channels targeting
/// non-joint nodes are ignored (we only skin the skeleton).
fn extract_clips(
    doc:           &gltf::Document,
    buffers:       &[gltf::buffer::Data],
    node_to_joint: &HashMap<usize, usize>,
    joint_count:   usize,
) -> Vec<AnimationClip> {
    use gltf::animation::util::ReadOutputs;
    use gltf::animation::Interpolation;

    doc.animations()
        .enumerate()
        .map(|(ai, anim)| {
            let mut tracks = vec![JointTracks::default(); joint_count];
            let mut duration = 0.0f32;

            for channel in anim.channels() {
                let node_idx = channel.target().node().index();
                let Some(&jx) = node_to_joint.get(&node_idx) else { continue };

                let raw_interp = channel.sampler().interpolation();
                let interp = match raw_interp {
                    Interpolation::Step => Interp::Step,
                    Interpolation::Linear | Interpolation::CubicSpline => Interp::Linear,
                };
                let is_cubic = raw_interp == Interpolation::CubicSpline;

                let reader = channel.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
                let times: Vec<f32> = match reader.read_inputs() {
                    Some(i) => i.collect(),
                    None => continue,
                };
                if let Some(&last) = times.last() {
                    duration = duration.max(last);
                }

                match reader.read_outputs() {
                    Some(ReadOutputs::Translations(it)) => {
                        let vals = keyframe_values(it.map(Vec3::from).collect(), times.len(), is_cubic);
                        tracks[jx].translation = Some(Track { times, values: vals, interp });
                    }
                    Some(ReadOutputs::Rotations(it)) => {
                        let vals = keyframe_values(
                            it.into_f32().map(Quat::from_array).collect(),
                            times.len(),
                            is_cubic,
                        );
                        tracks[jx].rotation = Some(Track { times, values: vals, interp });
                    }
                    Some(ReadOutputs::Scales(it)) => {
                        let vals = keyframe_values(it.map(Vec3::from).collect(), times.len(), is_cubic);
                        tracks[jx].scale = Some(Track { times, values: vals, interp });
                    }
                    _ => {} // morph-target weights: not skinned
                }
            }

            AnimationClip {
                name: anim.name().map(str::to_owned).unwrap_or_else(|| format!("anim{ai}")),
                duration,
                tracks,
            }
        })
        .collect()
}

/// CUBICSPLINE outputs store (in-tangent, value, out-tangent) per keyframe.
/// Downsampled to Linear, only the middle value survives.
fn keyframe_values<T: Copy>(vals: Vec<T>, times_len: usize, is_cubic: bool) -> Vec<T> {
    if is_cubic && vals.len() == 3 * times_len {
        (0..times_len).map(|i| vals[3 * i + 1]).collect()
    } else {
        vals
    }
}

fn visit_node(
    node:       &gltf::Node,
    parent:     Mat4,
    buffers:    &[gltf::buffer::Data],
    images:     &[gltf::image::Data],
    path:       &str,
    out:        &mut Vec<PrimitiveData>,
) {
    let global = parent * Mat4::from_cols_array_2d(&node.transform().matrix());

    if let Some(mesh) = node.mesh() {
        // Skinned primitives stay in mesh-local space (identity vertex
        // transform) — the joint palette does the placement. Static ones bake
        // the node hierarchy into their vertices as before.
        let skinned = node.skin().is_some();
        let vtx_xform  = if skinned { Mat4::IDENTITY } else { global };
        let normal_mat = if skinned { Mat3::IDENTITY } else { Mat3::from_mat4(global).inverse().transpose() };

        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                log::warn!("{path}: skipping non-triangle primitive ({:?})", prim.mode());
                continue;
            }
            let reader = prim.reader(|b| buffers.get(b.index()).map(|d| &d.0[..]));
            let Some(positions) = reader.read_positions() else {
                log::warn!("{path}: primitive without positions skipped");
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|n| n.collect())
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
            let uvs: Vec<[f32; 2]> = reader
                .read_tex_coords(0)
                .map(|t| t.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

            let indices: Vec<u32> = reader
                .read_indices()
                .map(|i| i.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());

            // Tangents: from the asset when present, otherwise generated in
            // source space (VQ-C4). Generation runs pre-transform; the
            // normal-matrix rotation below carries them to world space.
            let tangents: Vec<[f32; 4]> = reader
                .read_tangents()
                .map(|t| t.collect())
                .unwrap_or_else(|| generate_tangents(&positions, &normals, &uvs, &indices));

            let vertices: Vec<MeshVertex> = positions
                .iter()
                .zip(normals.iter())
                .zip(uvs.iter())
                .zip(tangents.iter())
                .map(|(((p, n), uv), t)| MeshVertex {
                    position: vtx_xform.transform_point3((*p).into()).to_array(),
                    normal:   (normal_mat * glam::Vec3::from(*n))
                        .normalize_or_zero()
                        .to_array(),
                    uv: *uv,
                    tangent: {
                        let txyz = (normal_mat * glam::Vec3::new(t[0], t[1], t[2]))
                            .normalize_or_zero();
                        [txyz.x, txyz.y, txyz.z, t[3]]
                    },
                })
                .collect();

            // Skin bindings (joints + weights), only for skinned primitives.
            let skin = if skinned {
                let joints = reader.read_joints(0).map(|j| j.into_u16().collect::<Vec<[u16; 4]>>());
                let weights = reader.read_weights(0).map(|w| w.into_f32().collect::<Vec<[f32; 4]>>());
                match (joints, weights) {
                    (Some(j), Some(w)) => Some(
                        j.iter()
                            .zip(w.iter())
                            .map(|(&joints, &weights)| VertexSkin { joints, weights })
                            .collect(),
                    ),
                    _ => {
                        log::warn!("{path}: skinned primitive missing JOINTS_0/WEIGHTS_0");
                        None
                    }
                }
            } else {
                None
            };

            let material = read_material(&prim.material(), images, path);

            out.push(PrimitiveData { vertices, indices, material, skin });
        }
    }

    for child in node.children() {
        visit_node(&child, global, buffers, images, path, out);
    }
}

/// Read the whole glTF metallic-roughness material of a primitive (VQ-A2):
/// every texture slot plus the scalar/vector factors.
fn read_material(
    mat:    &gltf::Material,
    images: &[gltf::image::Data],
    path:   &str,
) -> MaterialData {
    let fetch = |index: usize, slot: &str| -> Option<ImageData> {
        let img = images.get(index)?;
        let converted = to_rgba8(img);
        if converted.is_none() {
            log::warn!("{path}: unsupported {slot} image format {:?}", img.format);
        }
        converted
    };

    let pbr = mat.pbr_metallic_roughness();
    MaterialData {
        base_color_factor: pbr.base_color_factor(),
        metallic_factor:   pbr.metallic_factor(),
        roughness_factor:  pbr.roughness_factor(),
        alpha_cutoff: match mat.alpha_mode() {
            gltf::material::AlphaMode::Opaque => 0.0,
            gltf::material::AlphaMode::Mask => mat.alpha_cutoff().unwrap_or(0.5),
            gltf::material::AlphaMode::Blend => 0.5,
        },
        emissive_factor:   mat.emissive_factor(),
        emissive_strength: mat.emissive_strength().unwrap_or(1.0),
        base_color_image: pbr
            .base_color_texture()
            .and_then(|i| fetch(i.texture().source().index(), "base-color")),
        normal_image: mat
            .normal_texture()
            .and_then(|i| fetch(i.texture().source().index(), "normal")),
        metallic_roughness_image: pbr
            .metallic_roughness_texture()
            .and_then(|i| fetch(i.texture().source().index(), "metallic-roughness")),
        emissive_image: mat
            .emissive_texture()
            .and_then(|i| fetch(i.texture().source().index(), "emissive")),
        occlusion_image: mat
            .occlusion_texture()
            .and_then(|i| fetch(i.texture().source().index(), "occlusion")),
    }
}

/// Convert a decoded glTF image to tightly-packed RGBA8. 16-bit and float
/// formats are not supported (None).
fn to_rgba8(img: &gltf::image::Data) -> Option<ImageData> {
    use gltf::image::Format;
    let pixels = match img.format {
        Format::R8G8B8A8 => img.pixels.clone(),
        Format::R8G8B8 => img.pixels.chunks_exact(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        Format::R8 => img.pixels.iter()
            .flat_map(|&g| [g, g, g, 255])
            .collect(),
        Format::R8G8 => img.pixels.chunks_exact(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        _ => return None,
    };
    Some(ImageData { width: img.width, height: img.height, pixels })
}

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

/// Decode a PNG/JPG from disk into tightly-packed RGBA8 — the seam for
/// building procedural `MaterialData` (ground texture sets) outside glTF.
pub fn load_image_rgba(path: &str) -> Result<ImageData, String> {
    let img = image::open(path).map_err(|e| format!("{path}: {e}"))?.into_rgba8();
    Ok(ImageData {
        width:  img.width(),
        height: img.height(),
        pixels: img.into_raw(),
    })
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
fn pose_player(player: &mut AnimationPlayer, skin: &CpuSkin, dt: f32) -> (Vec<Mat4>, Vec<Mat4>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal single-triangle GLB by hand: one node (translated by
    /// (1,2,3)) with positions/normals/uvs/u16 indices and a solid
    /// baseColorFactor material. Exercises the whole CPU stage without
    /// depending on asset files or a GPU.
    fn write_test_glb(path: &std::path::Path) {
        let mut bin: Vec<u8> = Vec::new();
        let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let normals:   [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
        let uvs:       [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let indices:   [u16; 3]      = [0, 1, 2];
        for v in positions.iter().flatten() { bin.extend_from_slice(&v.to_le_bytes()); }
        for v in normals.iter().flatten()   { bin.extend_from_slice(&v.to_le_bytes()); }
        for v in uvs.iter().flatten()       { bin.extend_from_slice(&v.to_le_bytes()); }
        for i in indices                    { bin.extend_from_slice(&i.to_le_bytes()); }

        let json = format!(r#"{{
            "asset": {{"version": "2.0"}},
            "scene": 0,
            "scenes": [{{"nodes": [0]}}],
            "nodes": [{{"mesh": 0, "translation": [1, 2, 3]}}],
            "meshes": [{{"primitives": [{{
                "attributes": {{"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}},
                "indices": 3, "material": 0
            }}]}}],
            "materials": [{{"pbrMetallicRoughness": {{"baseColorFactor": [0.2, 0.4, 0.8, 1.0]}},
                            "alphaMode": "MASK", "alphaCutoff": 0.35}}],
            "buffers": [{{"byteLength": {bin_len}}}],
            "bufferViews": [
                {{"buffer": 0, "byteOffset": 0,  "byteLength": 36}},
                {{"buffer": 0, "byteOffset": 36, "byteLength": 36}},
                {{"buffer": 0, "byteOffset": 72, "byteLength": 24}},
                {{"buffer": 0, "byteOffset": 96, "byteLength": 6}}
            ],
            "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                  "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}},
                {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}},
                {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"}},
                {{"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}}
            ]
        }}"#, bin_len = bin.len());

        let mut json_bytes = json.into_bytes();
        while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }
        while bin.len() % 4 != 0 { bin.push(0); }

        let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(&0x46546C67u32.to_le_bytes()); // magic "glTF"
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // "JSON"
        glb.extend_from_slice(&json_bytes);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x004E4942u32.to_le_bytes()); // "BIN\0"
        glb.extend_from_slice(&bin);
        std::fs::write(path, glb).unwrap();
    }

    #[test]
    fn loads_triangle_glb_with_baked_node_transform() {
        let path = std::env::temp_dir().join("vordar_mesh_test_triangle.glb");
        write_test_glb(&path);

        let data = load_gltf_data(path.to_str().unwrap()).unwrap();
        assert_eq!(data.primitives.len(), 1);
        let p = &data.primitives[0];
        assert_eq!(p.vertices.len(), 3);
        assert_eq!(p.indices, vec![0, 1, 2]);
        // Node translation (1,2,3) must be baked into positions.
        assert_eq!(p.vertices[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(p.vertices[1].position, [2.0, 2.0, 3.0]);
        // Normals survive (pure translation — unchanged).
        assert_eq!(p.vertices[0].normal, [0.0, 0.0, 1.0]);
        assert_eq!(p.vertices[2].uv, [0.0, 1.0]);
        // Solid-color material, no texture.
        assert_eq!(p.material.base_color_factor, [0.2, 0.4, 0.8, 1.0]);
        assert!(p.material.base_color_image.is_none());
        // alphaMode MASK carries its cutoff (OPAQUE would read 0.0).
        assert_eq!(p.material.alpha_cutoff, 0.35);
        // No TANGENT accessor in the file — generated from UVs. This
        // triangle's UVs map u to +X, so the tangent points along +X.
        assert_eq!(p.vertices[0].tangent, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(load_gltf_data("does/not/exist.glb").is_err());
    }

    /// Hand-build a skinned + animated GLB: three vertices stacked on +Y, a
    /// two-joint chain (root at origin, child at +1 Y), and a clip that rotates
    /// the root 90° about Z over one second. Proves the whole skinning CPU
    /// stage — skin hierarchy, inverse binds, animation channels, and the
    /// bake-branch (skinned vertices stay in mesh-local space) — without a GPU.
    fn write_skinned_glb(path: &std::path::Path) {
        // Pad to 4 bytes, append, return (offset, len).
        fn push(bin: &mut Vec<u8>, data: &[u8]) -> (usize, usize) {
            while bin.len() % 4 != 0 { bin.push(0); }
            let off = bin.len();
            bin.extend_from_slice(data);
            (off, data.len())
        }
        fn f32s(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }
        fn u16s(v: &[u16]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

        let mut bin = Vec::new();
        let (pos_off, pos_len) = push(&mut bin, &f32s(&[0.0, 0.0, 0.0,  0.0, 1.0, 0.0,  0.0, 2.0, 0.0]));
        let (joi_off, joi_len) = push(&mut bin, &u16s(&[0, 0, 0, 0,  1, 0, 0, 0,  1, 0, 0, 0]));
        let (wei_off, wei_len) = push(&mut bin, &f32s(&[1.0, 0.0, 0.0, 0.0,  1.0, 0.0, 0.0, 0.0,  1.0, 0.0, 0.0, 0.0]));
        let (idx_off, idx_len) = push(&mut bin, &u16s(&[0, 1, 2]));
        // Inverse binds (column-major): joint0 = I, joint1 = translate(0,-1,0).
        let ibm = f32s(&[
            1.0, 0.0, 0.0, 0.0,  0.0, 1.0, 0.0, 0.0,  0.0, 0.0, 1.0, 0.0,  0.0,  0.0, 0.0, 1.0,
            1.0, 0.0, 0.0, 0.0,  0.0, 1.0, 0.0, 0.0,  0.0, 0.0, 1.0, 0.0,  0.0, -1.0, 0.0, 1.0,
        ]);
        let (ibm_off, ibm_len) = push(&mut bin, &ibm);
        let (ti_off, ti_len) = push(&mut bin, &f32s(&[0.0, 1.0]));
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let (ro_off, ro_len) = push(&mut bin, &f32s(&[0.0, 0.0, 0.0, 1.0,  0.0, 0.0, s, s]));

        let json = format!(r#"{{
            "asset": {{"version": "2.0"}},
            "scene": 0,
            "scenes": [{{"nodes": [0, 1]}}],
            "nodes": [
                {{"mesh": 0, "skin": 0}},
                {{"translation": [0, 0, 0], "children": [2]}},
                {{"translation": [0, 1, 0]}}
            ],
            "skins": [{{"joints": [1, 2], "inverseBindMatrices": 4}}],
            "meshes": [{{"primitives": [{{
                "attributes": {{"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}},
                "indices": 3
            }}]}}],
            "animations": [{{
                "name": "Spin",
                "channels": [{{"sampler": 0, "target": {{"node": 1, "path": "rotation"}}}}],
                "samplers": [{{"input": 5, "output": 6, "interpolation": "LINEAR"}}]
            }}],
            "buffers": [{{"byteLength": {bin_len}}}],
            "bufferViews": [
                {{"buffer": 0, "byteOffset": {pos_off}, "byteLength": {pos_len}}},
                {{"buffer": 0, "byteOffset": {joi_off}, "byteLength": {joi_len}}},
                {{"buffer": 0, "byteOffset": {wei_off}, "byteLength": {wei_len}}},
                {{"buffer": 0, "byteOffset": {idx_off}, "byteLength": {idx_len}}},
                {{"buffer": 0, "byteOffset": {ibm_off}, "byteLength": {ibm_len}}},
                {{"buffer": 0, "byteOffset": {ti_off}, "byteLength": {ti_len}}},
                {{"buffer": 0, "byteOffset": {ro_off}, "byteLength": {ro_len}}}
            ],
            "accessors": [
                {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                  "min": [0.0, 0.0, 0.0], "max": [0.0, 2.0, 0.0]}},
                {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "VEC4"}},
                {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"}},
                {{"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}},
                {{"bufferView": 4, "componentType": 5126, "count": 2, "type": "MAT4"}},
                {{"bufferView": 5, "componentType": 5126, "count": 2, "type": "SCALAR",
                  "min": [0.0], "max": [1.0]}},
                {{"bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC4"}}
            ]
        }}"#, bin_len = bin.len());

        let mut json_bytes = json.into_bytes();
        while json_bytes.len() % 4 != 0 { json_bytes.push(b' '); }
        while bin.len() % 4 != 0 { bin.push(0); }

        let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend_from_slice(&0x46546C67u32.to_le_bytes());
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        glb.extend_from_slice(&json_bytes);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(&0x004E4942u32.to_le_bytes());
        glb.extend_from_slice(&bin);
        std::fs::write(path, glb).unwrap();
    }

    #[test]
    fn loads_skinned_animated_glb() {
        use crate::anim::{joint_matrices, sample_pose};

        let path = std::env::temp_dir().join("vordar_mesh_test_skinned.glb");
        write_skinned_glb(&path);
        let data = load_gltf_data(path.to_str().unwrap()).unwrap();

        // Skeleton: two joints, child parented to root.
        let skel = data.skeleton.as_ref().expect("skinned mesh has a skeleton");
        assert_eq!(skel.joint_count(), 2);
        assert_eq!(skel.joints[0].parent, None);
        assert_eq!(skel.joints[1].parent, Some(0));
        // Child inverse bind is translate(0,-1,0).
        assert!(skel.joints[1].inverse_bind
            .abs_diff_eq(Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0)), 1e-5));

        // Skin bindings present; vertices are NOT baked (skinned → mesh-local).
        let p = &data.primitives[0];
        let skin = p.skin.as_ref().expect("skinned primitive carries joints/weights");
        assert_eq!(skin.len(), 3);
        assert_eq!(skin[0].joints[0], 0);
        assert_eq!(skin[1].joints[0], 1);
        assert_eq!(p.vertices[1].position, [0.0, 1.0, 0.0], "skinned verts stay in local space");

        // Clip: one animation, ~1s, with a rotation track on the root joint.
        assert_eq!(data.clips.len(), 1);
        let clip = &data.clips[0];
        assert_eq!(clip.name, "Spin");
        assert!((clip.duration - 1.0).abs() < 1e-5);
        assert!(clip.tracks[0].rotation.is_some(), "root joint is animated");

        // End of clip: root rotated 90° about Z. A bind-space point at the
        // child (0,1,0), skinned by the child joint matrix, swings to (-1,0,0).
        let pose = sample_pose(skel, clip, clip.duration);
        let mats = joint_matrices(skel, &pose);
        let skinned = mats[1].transform_point3(Vec3::new(0.0, 1.0, 0.0));
        assert!(skinned.abs_diff_eq(Vec3::new(-1.0, 0.0, 0.0), 1e-4), "got {skinned}");
    }

    /// Exercises real-asset paths the synthetic GLB can't: embedded PNG
    /// decode and image-format conversion. Skips when the content asset is
    /// absent (it lives in the game repo, not the engine).
    #[test]
    fn loads_real_textured_asset_if_present() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/avocado.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        assert!(!data.primitives.is_empty());
        let p = &data.primitives[0];
        assert!(p.vertices.len() > 100, "real mesh, not a placeholder");
        let img = p.material.base_color_image.as_ref().expect("avocado has a base-color texture");
        assert_eq!(img.pixels.len() as u32, img.width * img.height * 4, "tightly packed RGBA8");
    }

    /// Real rigged+animated asset (the Fox dev probe): proves skin + multi-clip
    /// extraction on production data. Skips if absent.
    #[test]
    fn loads_skinned_fox_asset_if_present() {
        use crate::anim::joint_matrices;

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/fox.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().expect("fox is skinned");
        assert!(skel.joint_count() > 10, "real skeleton");
        assert!(data.primitives[0].skin.is_some(), "vertices carry skin bindings");
        // Named locomotion clips the controller maps to idle/walk/run.
        let names: Vec<&str> = data.clips.iter().map(|c| c.name.as_str()).collect();
        for want in ["Survey", "Walk", "Run"] {
            assert!(names.contains(&want), "expected clip {want:?}, got {names:?}");
        }
        // Every clip poses to a full, finite joint palette.
        for clip in &data.clips {
            let pose = crate::anim::sample_pose(skel, clip, clip.duration * 0.5);
            let mats = joint_matrices(skel, &pose);
            assert_eq!(mats.len(), skel.joint_count());
            assert!(mats.iter().all(|m| m.is_finite()), "clip {} produced NaN", clip.name);
        }
    }

    /// Real game character (the KayKit human race): a Phase-C skinned humanoid,
    /// preprocessed so its armature carries a scale + ground offset. Proves the
    /// clip mapping the locomotion controller expects, and that the root offset
    /// baked onto the armature is picked up (feet grounded, character scaled).
    #[test]
    fn loads_human_character_asset_if_present() {
        use crate::anim::joint_matrices;

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().expect("human is skinned");
        assert!(skel.joint_count() > 20, "full humanoid skeleton");
        // Every primitive is skinned (body + face meshes).
        assert!(data.primitives.len() >= 2, "body plus face primitives");
        assert!(
            data.primitives.iter().all(|p| p.skin.is_some()),
            "all primitives carry skin bindings"
        );
        // Locomotion, the per-ability attack clips, hit react, and death
        // (the Mixamo clip library merged by mixamo_to_glb.py).
        let names: Vec<&str> = data.clips.iter().map(|c| c.name.as_str()).collect();
        for want in [
            "idle",
            "walk",
            "run",
            "attack_slash",
            "attack_heavy",
            "attack_cast",
            "hit",
            "death",
            "leap",
            "dodge",
        ] {
            assert!(names.contains(&want), "expected clip {want:?}, got {names:?}");
        }
        // The armature offset was captured: a non-identity root that scales down
        // (< 1) and drops the character so its feet sit on the floor (y < 0).
        assert!(!skel.root.abs_diff_eq(Mat4::IDENTITY, 1e-4), "armature offset captured");
        assert!(skel.root.w_axis.y < 0.0, "grounded below origin, got {}", skel.root.w_axis.y);
        assert!(skel.root.x_axis.x < 1.0 && skel.root.x_axis.x > 0.0, "scaled down");
        // Every clip poses to a full, finite palette.
        for clip in &data.clips {
            let pose = crate::anim::sample_pose(skel, clip, clip.duration * 0.5);
            let mats = joint_matrices(skel, &pose);
            assert!(mats.iter().all(|m| m.is_finite()), "clip {} produced NaN", clip.name);
        }
    }

    /// DIAGNOSTIC: does advancing an AnimationPlayer on the real human's walk
    /// clip actually move the skeleton? Catches a dead animation path (clips
    /// present but not driving joints, or time not advancing) — the CPU half of
    /// a "character renders but doesn't animate" bug.
    #[test]
    fn human_locomotion_clips_actually_animate() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skin = CpuSkin { skeleton: data.skeleton.unwrap(), clips: data.clips };
        for clip in ["idle", "walk", "run"] {
            let mut player = AnimationPlayer { clip: clip.into(), ..Default::default() };
            let (a, _) = pose_player(&mut player, &skin, 0.0);
            let (b, _) = pose_player(&mut player, &skin, 0.25);
            let moved = a.iter().zip(&b).any(|(x, y)| !x.abs_diff_eq(*y, 1e-4));
            assert!(moved, "clip {clip} must move the skeleton as time advances");
        }
    }

    /// DIAGNOSTIC (the "half under the field" report): with the armature's
    /// baked ground offset applied, no clip may pose any joint meaningfully
    /// below the floor plane (floor top = −0.5, joints sit above the sole).
    /// Catches clips whose ground reference disagrees with the bind pose —
    /// prime suspect for a character rendering half-sunk.
    #[test]
    fn human_clips_stay_above_the_floor() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        const FLOOR: f32 = -0.5;
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().unwrap();
        for clip in &data.clips {
            for i in 0..=4 {
                let t = clip.duration * i as f32 / 4.0;
                let pose = crate::anim::sample_pose(skel, clip, t);
                let globals = crate::anim::global_transforms(skel, &pose);
                let min_y = globals
                    .iter()
                    .map(|g| g.to_scale_rotation_translation().2.y)
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    min_y > FLOOR - 0.35,
                    "clip {:?} at t={t:.2}s poses a joint at y={min_y:.2} — well below the floor",
                    clip.name
                );
            }
        }
    }

    /// DIAGNOSTIC: CPU-skin the actual mesh vertices through the same palette
    /// the GPU gets (root · global · inverse_bind) — the grounded-joints probe
    /// above can't see an inverse-bind inconsistency, this can. The idle pose
    /// must put the soles on the floor (−0.5) and the crown near +1.24.
    #[test]
    fn human_skinned_vertices_stand_on_the_floor() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().unwrap();
        let clip = data.clips.iter().find(|c| c.name == "idle").unwrap();
        let pose = crate::anim::sample_pose(skel, clip, 0.0);
        let palette = crate::anim::joint_matrices(skel, &pose);

        let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
        for p in &data.primitives {
            let skin = p.skin.as_ref().expect("human primitives are skinned");
            for (v, s) in p.vertices.iter().zip(skin) {
                let pos = glam::Vec3::from_array(v.position);
                let mut out = glam::Vec3::ZERO;
                for (j, w) in s.joints.iter().zip(s.weights) {
                    if w > 0.0 {
                        out += palette[*j as usize].transform_point3(pos) * w;
                    }
                }
                min_y = min_y.min(out.y);
                max_y = max_y.max(out.y);
            }
        }
        println!("idle t=0 skinned vertices: y {min_y:.3} .. {max_y:.3}");
        assert!(
            (min_y - (-0.5)).abs() < 0.1,
            "soles must touch the floor: min_y = {min_y:.3}"
        );
        assert!(
            max_y > 0.9 && max_y < 1.5,
            "crown around head height: max_y = {max_y:.3}"
        );
    }

    /// The weapon-hand socket: the human skeleton keeps its bone names, the
    /// hand bone's global transform is finite, and it travels during the swing
    /// clip — the position a cast burst spawns from.
    #[test]
    fn human_hand_socket_exists_and_moves_during_swing() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
        if !std::path::Path::new(path).exists() {
            return;
        }
        let data = load_gltf_data(path).unwrap();
        let skel = data.skeleton.as_ref().unwrap();
        let j = skel
            .joints
            .iter()
            .position(|jt| jt.name == "handslot.r")
            .expect("human skeleton keeps its bone names");
        let clip = data
            .clips
            .iter()
            .find(|c| c.name == "attack_slash")
            .unwrap();
        let at = |t: f32| {
            let pose = crate::anim::sample_pose(skel, clip, t);
            let g = crate::anim::global_transforms(skel, &pose);
            g[j].to_scale_rotation_translation().2
        };
        let (a, b) = (at(0.0), at(clip.duration * 0.5));
        assert!(a.is_finite() && b.is_finite(), "socket transforms must be finite");
        assert!(
            a.distance(b) > 0.05,
            "hand socket must travel during the swing: {a} -> {b}"
        );
    }
}
