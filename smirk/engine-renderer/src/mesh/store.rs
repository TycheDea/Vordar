use super::gltf_import::{ImageData, MeshData, VertexSkin};
use crate::anim::{AnimationClip, Skeleton};
use crate::mesh_pipeline::MaterialUniform;
use crate::mipgen::MipGenerator;
use crate::skinned_pipeline::SkinnedVertex;
use crate::texture::{self, ColorTexture};
use std::collections::HashMap;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{BindGroup, BindGroupLayout, Buffer, BufferUsages, Device, Queue};

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
        let result = match super::gltf_import::load_gltf_data(path) {
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
