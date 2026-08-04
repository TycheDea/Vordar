// Shadow mapping: three concentric texel-snapped cascades for the sun.
// The camera is a bounded orbit over compact zones; the near cascades
// tighten around the focus for crisp contact shadows while the outer
// cascade spans the max shadow distance. Receivers PCF-filter in the
// geometry shaders via the shared camera bind group (bindings 2–4 — the
// skinned pipeline already uses the default max of 4 bind groups, so
// shadows can't have their own).

use glam::{Mat4, Vec3};
use wgpu::{Device, TextureFormat};

pub(crate) const SHADOW_SIZE: u32 = 2048;
const DEPTH_RANGE: f32 = 400.0;

/// Shadow map layer count: three concentric cascades. The array plumbing
/// (texture layers, receiver light-VP array, cast dynamic-offset buffer) is
/// sized off this constant.
pub(crate) const CASCADE_COUNT: u32 = 3;

/// Per-cascade half-extent, outermost last. The outer value is the max
/// shadow distance: it must reach the visible ground corners at the
/// client's max orbit radius (CameraConfig 100 → ~120–165 m depending on
/// projection mode and pitch). Pitch is free, so no extent covers every
/// perspective view — the receiver fades to unshadowed at the outer edge
/// (shadow_sample.wgsl) instead of popping. Inner values tighten around
/// the focus for denser texels.
const CASCADE_HALF_EXTENTS: [f32; CASCADE_COUNT as usize] = [24.0, 60.0, 160.0];

/// Stride between cascades in the cast uniform buffer. Must be a multiple of
/// `Limits::min_uniform_buffer_offset_alignment` (256 covers every wgpu
/// backend's default) so each cascade's slice is selectable by dynamic offset.
const CAST_STRIDE: wgpu::BufferAddress = 256;

/// Creates the shadow depth texture (`CASCADE_COUNT` array layers) plus a
/// per-layer view for each cascade's render-pass depth attachment and one
/// `D2Array` view over the whole texture for the receiver's sampled binding.
pub(crate) fn create_shadow_texture(
    device: &Device,
) -> (wgpu::Texture, Vec<wgpu::TextureView>, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label:           Some("Shadow Map"),
        size:            wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: CASCADE_COUNT },
        mip_level_count: 1,
        sample_count:    1,
        dimension:       wgpu::TextureDimension::D2,
        format:          TextureFormat::Depth32Float,
        usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats:    &[],
    });
    let cascade_views = (0..CASCADE_COUNT)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label:             Some("Shadow Map Cascade View"),
                dimension:         Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label:     Some("Shadow Map Array View"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (texture, cascade_views, array_view)
}

/// Comparison sampler for PCF (hardware 2×2 per tap).
pub(crate) fn create_shadow_sampler(device: &Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label:          Some("Shadow Comparison Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter:     wgpu::FilterMode::Linear,
        min_filter:     wgpu::FilterMode::Linear,
        compare:        Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    })
}

/// The sun's view-projection fitted around `target` at `half_extent`,
/// texel-snapped so an orbiting/panning camera never makes shadow edges
/// shimmer. Pure — unit tested. `light_dir` points TOWARD the light.
fn fit_vp(target: Vec3, light_dir: Vec3, half_extent: f32) -> Mat4 {
    let dir = light_dir.normalize_or_zero();
    let dir = if dir == Vec3::ZERO { Vec3::Y } else { dir };
    let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };

    // Build the light view around the origin first to get stable axes.
    let view = Mat4::look_at_rh(dir * (DEPTH_RANGE * 0.5), Vec3::ZERO, up);

    // Snap the target to this cascade's own texel grid in light space.
    let texel = (half_extent * 2.0) / SHADOW_SIZE as f32;
    let t_light = view.transform_point3(target);
    let snapped = Vec3::new(
        (t_light.x / texel).floor() * texel,
        (t_light.y / texel).floor() * texel,
        t_light.z,
    );

    let proj = Mat4::orthographic_rh(
        snapped.x - half_extent,
        snapped.x + half_extent,
        snapped.y - half_extent,
        snapped.y + half_extent,
        0.0,
        DEPTH_RANGE,
    );
    proj * view
}

/// The sun's view-projection at the outer cascade's half-extent — the
/// conservative bound used for sun-frustum culling (mesh/sync.rs): anything
/// outside it is outside every cascade, so per-cascade culling is unneeded.
pub(crate) fn fit_light_vp(target: Vec3, light_dir: Vec3) -> Mat4 {
    fit_vp(target, light_dir, CASCADE_HALF_EXTENTS[CASCADE_COUNT as usize - 1])
}

/// Per-cascade fitted light view-projections, each independently texel-
/// snapped at its own half-extent (`CASCADE_HALF_EXTENTS`).
pub(crate) fn fit_cascades(target: Vec3, light_dir: Vec3) -> [Mat4; CASCADE_COUNT as usize] {
    std::array::from_fn(|i| fit_vp(target, light_dir, CASCADE_HALF_EXTENTS[i]))
}

/// Creates the dynamic-offset cast uniform buffer: `CASCADE_COUNT` slots of
/// `CAST_STRIDE` bytes, one 64-byte light-VP mat4 per cascade, so a shadow
/// draw selects its cascade via `set_bind_group`'s dynamic offset.
pub(crate) fn create_cast_buffer(device: &Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Shadow Cast Uniform"),
        size:               CAST_STRIDE * CASCADE_COUNT as u64,
        usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Dynamic offset (bytes) of `cascade`'s slot in the cast uniform buffer.
pub(crate) fn cast_offset(cascade: u32) -> wgpu::DynamicOffset {
    cascade as wgpu::DynamicOffset * CAST_STRIDE as wgpu::DynamicOffset
}

/// Writes `cascades` into both the tight receiver uniform (`array<mat4x4,
/// CASCADE_COUNT>`, sampled by `shadow_sample.wgsl`) and the 256-stride cast
/// buffer (dynamic-offset per cascade, read by `shadow.wgsl`).
pub(crate) fn write_cascade_uniforms(
    queue:           &wgpu::Queue,
    light_vp_buffer: &wgpu::Buffer,
    cast_buffer:     &wgpu::Buffer,
    cascades:        &[Mat4; CASCADE_COUNT as usize],
) {
    let tight: Vec<f32> = cascades.iter().flat_map(|m| m.to_cols_array()).collect();
    queue.write_buffer(light_vp_buffer, 0, bytemuck::cast_slice(&tight));
    for (i, m) in cascades.iter().enumerate() {
        queue.write_buffer(cast_buffer, cast_offset(i as u32) as u64, bytemuck::cast_slice(&m.to_cols_array()));
    }
}

/// Depth-only pipeline variants of the three geometry pipelines, plus a
/// fragment-discard variant of mesh/skinned for glTF MASK primitives (see
/// `mesh_masked`/`skinned_masked`) so their cutout regions neither cast a
/// solid-quad shadow nor write full-quad depth.
pub(crate) struct ShadowPipelines {
    pub(crate) sdf:            wgpu::RenderPipeline,
    pub(crate) mesh:           wgpu::RenderPipeline,
    pub(crate) skinned:        wgpu::RenderPipeline,
    pub(crate) mesh_masked:    wgpu::RenderPipeline,
    pub(crate) skinned_masked: wgpu::RenderPipeline,
    pub(crate) bgl:            wgpu::BindGroupLayout, // light_vp for the vertex stage
}

impl ShadowPipelines {
    pub(crate) fn new(device: &Device, joint_bgl: &wgpu::BindGroupLayout, material_bgl: &wgpu::BindGroupLayout) -> Self {
        use crate::instance::SdfInstance;
        use crate::mesh_pipeline::{MeshVertex, MESH_INSTANCE_SIZE};
        use crate::sdf_pipeline::Vertex;
        use crate::skinned_pipeline::{SKINNED_INSTANCE_SIZE, SKINNED_VERTEX_SIZE};
        use std::mem::size_of;
        use wgpu::VertexFormat::{Float32x2, Float32x3, Float32x4, Uint16x4, Uint32};
        use wgpu::{VertexAttribute, VertexBufferLayout, VertexStepMode};

        let shader = device.create_shader_module(wgpu::include_wgsl!("shadow.wgsl"));

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("Shadow Cast BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size:   wgpu::BufferSize::new(64),
                },
                count: None,
            }],
        });

        let layout_static = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Shadow Static Layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size:     0,
        });
        let layout_skinned = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Shadow Skinned Layout"),
            bind_group_layouts: &[Some(&bgl), Some(joint_bgl)],
            immediate_size:     0,
        });
        let layout_static_masked = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Shadow Static Masked Layout"),
            bind_group_layouts: &[Some(&bgl), Some(material_bgl)],
            immediate_size:     0,
        });
        let layout_skinned_masked = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Shadow Skinned Masked Layout"),
            bind_group_layouts: &[Some(&bgl), Some(joint_bgl), Some(material_bgl)],
            immediate_size:     0,
        });

        let make = |label: &str,
                    layout: &wgpu::PipelineLayout,
                    entry: &str,
                    buffers: &[VertexBufferLayout]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:  Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module:      &shader,
                    entry_point: Some(entry),
                    buffers,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format:              TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare:       Some(wgpu::CompareFunction::Less),
                    stencil:             Default::default(),
                    // Slope-scaled bias against acne; tuned with the PCF radius.
                    // Shared across all cascades (same pipeline for every layer).
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
                }),
                multisample:    Default::default(),
                fragment:       None, // depth-only
                multiview_mask: None,
                cache:          None,
            })
        };

        let make_masked = |label: &str,
                            layout: &wgpu::PipelineLayout,
                            vs_entry: &str,
                            fs_entry: &str,
                            buffers: &[VertexBufferLayout]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:  Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module:      &shader,
                    entry_point: Some(vs_entry),
                    buffers,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format:              TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare:       Some(wgpu::CompareFunction::Less),
                    stencil:             Default::default(),
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module:      &shader,
                    entry_point: Some(fs_entry),
                    targets:     &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache:          None,
            })
        };

        // Vertex/instance layouts mirror the main pipelines; the shader reads
        // a subset of attributes (position, model rows, skin data).
        let sdf_vertex = [
            VertexAttribute { offset: 0, shader_location: 0, format: Float32x3 },
        ];
        let sdf_instance = [
            VertexAttribute { offset:  0, shader_location: 3, format: Float32x4 },
            VertexAttribute { offset: 16, shader_location: 4, format: Float32x4 },
            VertexAttribute { offset: 32, shader_location: 5, format: Float32x4 },
            VertexAttribute { offset: 48, shader_location: 6, format: Float32x4 },
        ];
        let mesh_vertex = [
            VertexAttribute { offset: 0, shader_location: 0, format: Float32x3 },
        ];
        let mesh_instance = [
            VertexAttribute { offset:  0, shader_location: 4, format: Float32x4 },
            VertexAttribute { offset: 16, shader_location: 5, format: Float32x4 },
            VertexAttribute { offset: 32, shader_location: 6, format: Float32x4 },
            VertexAttribute { offset: 48, shader_location: 7, format: Float32x4 },
        ];
        let skinned_vertex = [
            VertexAttribute { offset:  0, shader_location: 0, format: Float32x3 },
            VertexAttribute { offset: 48, shader_location: 4, format: Uint16x4 },
            VertexAttribute { offset: 56, shader_location: 5, format: Float32x4 },
        ];
        let skinned_instance = [
            VertexAttribute { offset:  0, shader_location: 6, format: Float32x4 },
            VertexAttribute { offset: 16, shader_location: 7, format: Float32x4 },
            VertexAttribute { offset: 32, shader_location: 8, format: Float32x4 },
            VertexAttribute { offset: 48, shader_location: 9, format: Float32x4 },
            VertexAttribute { offset: 80, shader_location: 11, format: Uint32 },
        ];
        // Masked variants add the UV attribute (location 1) the mask fragment
        // stage samples; joints/weights/model/tint locations are unchanged.
        let mesh_vertex_masked = [
            VertexAttribute { offset: 0,  shader_location: 0, format: Float32x3 },
            VertexAttribute { offset: 24, shader_location: 1, format: Float32x2 },
        ];
        let skinned_vertex_masked = [
            VertexAttribute { offset:  0, shader_location: 0, format: Float32x3 },
            VertexAttribute { offset: 24, shader_location: 1, format: Float32x2 },
            VertexAttribute { offset: 48, shader_location: 4, format: Uint16x4 },
            VertexAttribute { offset: 56, shader_location: 5, format: Float32x4 },
        ];

        let sdf = make(
            "Shadow SDF Pipeline",
            &layout_static,
            "sdf_vtx",
            &[
                VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode:    VertexStepMode::Vertex,
                    attributes:   &sdf_vertex,
                },
                VertexBufferLayout {
                    array_stride: size_of::<SdfInstance>() as u64,
                    step_mode:    VertexStepMode::Instance,
                    attributes:   &sdf_instance,
                },
            ],
        );
        let mesh = make(
            "Shadow Mesh Pipeline",
            &layout_static,
            "mesh_vtx",
            &[
                VertexBufferLayout {
                    array_stride: size_of::<MeshVertex>() as u64,
                    step_mode:    VertexStepMode::Vertex,
                    attributes:   &mesh_vertex,
                },
                VertexBufferLayout {
                    array_stride: MESH_INSTANCE_SIZE as u64,
                    step_mode:    VertexStepMode::Instance,
                    attributes:   &mesh_instance,
                },
            ],
        );
        let skinned = make(
            "Shadow Skinned Pipeline",
            &layout_skinned,
            "skinned_vtx",
            &[
                VertexBufferLayout {
                    array_stride: SKINNED_VERTEX_SIZE as u64,
                    step_mode:    VertexStepMode::Vertex,
                    attributes:   &skinned_vertex,
                },
                VertexBufferLayout {
                    array_stride: SKINNED_INSTANCE_SIZE as u64,
                    step_mode:    VertexStepMode::Instance,
                    attributes:   &skinned_instance,
                },
            ],
        );

        let mesh_masked = make_masked(
            "Shadow Mesh Masked Pipeline",
            &layout_static_masked,
            "mesh_vtx_masked",
            "mesh_frag_masked",
            &[
                VertexBufferLayout {
                    array_stride: size_of::<MeshVertex>() as u64,
                    step_mode:    VertexStepMode::Vertex,
                    attributes:   &mesh_vertex_masked,
                },
                VertexBufferLayout {
                    array_stride: MESH_INSTANCE_SIZE as u64,
                    step_mode:    VertexStepMode::Instance,
                    attributes:   &mesh_instance,
                },
            ],
        );
        let skinned_masked = make_masked(
            "Shadow Skinned Masked Pipeline",
            &layout_skinned_masked,
            "skinned_vtx_masked",
            "skinned_frag_masked",
            &[
                VertexBufferLayout {
                    array_stride: SKINNED_VERTEX_SIZE as u64,
                    step_mode:    VertexStepMode::Vertex,
                    attributes:   &skinned_vertex_masked,
                },
                VertexBufferLayout {
                    array_stride: SKINNED_INSTANCE_SIZE as u64,
                    step_mode:    VertexStepMode::Instance,
                    attributes:   &skinned_instance,
                },
            ],
        );

        Self { sdf, mesh, skinned, mesh_masked, skinned_masked, bgl }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_volume_contains_points_near_target() {
        let target = Vec3::new(100.0, 0.0, -40.0);
        let vp = fit_light_vp(target, Vec3::new(-1.0, 2.0, -1.0));
        for offset in [
            Vec3::ZERO,
            Vec3::new(50.0, 0.0, 0.0),
            Vec3::new(-50.0, 10.0, 30.0),
            Vec3::new(0.0, 5.0, -50.0),
        ] {
            let p = vp.project_point3(target + offset);
            assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0, "{offset:?} → {p:?} outside NDC xy");
            assert!((0.0..=1.0).contains(&p.z), "{offset:?} → z {p:?} outside [0,1]");
        }
    }

    #[test]
    fn texel_snap_quantizes_translation() {
        // Sub-texel target movement must not change the matrix at all —
        // that is what kills edge shimmer while the camera pans.
        let dir = Vec3::new(-1.0, 2.0, -1.0);
        let texel = (CASCADE_HALF_EXTENTS[2] * 2.0) / SHADOW_SIZE as f32;
        let a = fit_light_vp(Vec3::ZERO, dir);
        let b = fit_light_vp(Vec3::new(texel * 0.2, 0.0, texel * 0.2), dir);
        assert_eq!(a.to_cols_array(), b.to_cols_array(), "sub-texel pan must snap identically");

        // A large move does change it.
        let c = fit_light_vp(Vec3::new(10.0, 0.0, 0.0), dir);
        assert_ne!(a.to_cols_array(), c.to_cols_array());
    }

    #[test]
    fn near_cascade_has_denser_texels_than_far_cascade() {
        // Same world-space offset must map to a larger NDC delta in the near
        // cascade than in the far one — i.e. the near cascade covers fewer
        // world units per texel (denser).
        let target = Vec3::ZERO;
        let dir = Vec3::new(-1.0, 2.0, -1.0);
        let cascades = fit_cascades(target, dir);
        let offset = Vec3::new(15.0, 0.0, 0.0);
        let p_near = cascades[0].project_point3(target + offset);
        let p_far = cascades[CASCADE_COUNT as usize - 1].project_point3(target + offset);
        let near_mag = p_near.x.hypot(p_near.y);
        let far_mag = p_far.x.hypot(p_far.y);
        assert!(
            near_mag > far_mag,
            "near cascade must be denser: near_ndc={near_mag} far_ndc={far_mag}"
        );
    }

    #[test]
    fn every_cascade_texel_snaps_independently() {
        let dir = Vec3::new(-1.0, 2.0, -1.0);
        let base = fit_cascades(Vec3::ZERO, dir);
        for (i, half_extent) in CASCADE_HALF_EXTENTS.iter().enumerate() {
            let texel = (half_extent * 2.0) / SHADOW_SIZE as f32;
            let sub_texel_moved = fit_cascades(Vec3::new(texel * 0.2, 0.0, texel * 0.2), dir);
            assert_eq!(
                base[i].to_cols_array(),
                sub_texel_moved[i].to_cols_array(),
                "cascade {i} must snap sub-texel target moves identically"
            );

            let large_moved = fit_cascades(Vec3::new(*half_extent * 0.5, 0.0, 0.0), dir);
            assert_ne!(
                base[i].to_cols_array(),
                large_moved[i].to_cols_array(),
                "cascade {i} must change for a large target move"
            );
        }
    }

    #[test]
    fn containment_selects_tightest_cascade() {
        let target = Vec3::ZERO;
        let dir = Vec3::new(-1.0, 2.0, -1.0);
        let cascades = fit_cascades(target, dir);

        let near = cascades[0].project_point3(target + Vec3::new(20.0, 0.0, 0.0));
        assert!(near.x.abs() <= 1.0 && near.y.abs() <= 1.0, "20u point must be inside cascade 0: {near:?}");

        // 150 u is the visible-ground-corner bound at max orbit radius 100 —
        // the outer cascade must contain it or far shadows vanish at max zoom.
        let far_point = target + Vec3::new(150.0, 0.0, 0.0);
        let far_in_outer = cascades[CASCADE_COUNT as usize - 1].project_point3(far_point);
        assert!(
            far_in_outer.x.abs() <= 1.0 && far_in_outer.y.abs() <= 1.0,
            "150u point must be inside the outer cascade: {far_in_outer:?}"
        );
        let far_in_near = cascades[0].project_point3(far_point);
        assert!(
            far_in_near.x.abs() > 1.0 || far_in_near.y.abs() > 1.0,
            "150u point must be outside cascade 0: {far_in_near:?}"
        );
    }
}
