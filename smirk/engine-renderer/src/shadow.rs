// Shadow mapping: one fitted orthographic cascade for the sun.
// The camera is a bounded orbit (radius 16–55) over compact zones, so a
// single 2048² map fitted around the camera target holds up; CSM stays
// future work. Receivers PCF-filter in the geometry shaders via the shared
// camera bind group (bindings 2–4 — the skinned pipeline already uses the
// default max of 4 bind groups, so shadows can't have their own).

use glam::{Mat4, Vec3};
use wgpu::{Device, TextureFormat};

pub(crate) const SHADOW_SIZE: u32 = 2048;
/// Half-extent of the fitted ortho volume: covers the max orbit radius (55)
/// plus grounded-entity margin. Fixed so the texel size is stable and the
/// origin can snap to whole texels (no orbit shimmer).
const HALF_EXTENT: f32 = 80.0;
const DEPTH_RANGE: f32 = 400.0;

pub(crate) fn create_shadow_texture(device: &Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label:           Some("Shadow Map"),
        size:            wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count:    1,
        dimension:       wgpu::TextureDimension::D2,
        format:          TextureFormat::Depth32Float,
        usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats:    &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
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

/// The sun's view-projection fitted around `target`, texel-snapped so an
/// orbiting/panning camera never makes shadow edges shimmer. Pure — unit
/// tested. `light_dir` points TOWARD the light.
pub(crate) fn fit_light_vp(target: Vec3, light_dir: Vec3) -> Mat4 {
    let dir = light_dir.normalize_or_zero();
    let dir = if dir == Vec3::ZERO { Vec3::Y } else { dir };
    let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };

    // Build the light view around the origin first to get stable axes.
    let view = Mat4::look_at_rh(dir * (DEPTH_RANGE * 0.5), Vec3::ZERO, up);

    // Snap the target to the shadow texel grid in light space.
    let texel = (HALF_EXTENT * 2.0) / SHADOW_SIZE as f32;
    let t_light = view.transform_point3(target);
    let snapped = Vec3::new(
        (t_light.x / texel).floor() * texel,
        (t_light.y / texel).floor() * texel,
        t_light.z,
    );

    let proj = Mat4::orthographic_rh(
        snapped.x - HALF_EXTENT,
        snapped.x + HALF_EXTENT,
        snapped.y - HALF_EXTENT,
        snapped.y + HALF_EXTENT,
        0.0,
        DEPTH_RANGE,
    );
    proj * view
}

/// Depth-only pipeline variants of the three geometry pipelines.
pub(crate) struct ShadowPipelines {
    pub(crate) sdf:     wgpu::RenderPipeline,
    pub(crate) mesh:    wgpu::RenderPipeline,
    pub(crate) skinned: wgpu::RenderPipeline,
    pub(crate) bgl:     wgpu::BindGroupLayout, // light_vp for the vertex stage
}

impl ShadowPipelines {
    pub(crate) fn new(device: &Device, joint_bgl: &wgpu::BindGroupLayout) -> Self {
        use crate::instance::SdfInstance;
        use crate::mesh_pipeline::{MeshVertex, MESH_INSTANCE_SIZE};
        use crate::sdf_pipeline::Vertex;
        use crate::skinned_pipeline::{SKINNED_INSTANCE_SIZE, SKINNED_VERTEX_SIZE};
        use std::mem::size_of;
        use wgpu::VertexFormat::{Float32x3, Float32x4, Uint16x4, Uint32};
        use wgpu::{VertexAttribute, VertexBufferLayout, VertexStepMode};

        let shader = device.create_shader_module(wgpu::include_wgsl!("shadow.wgsl"));

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("Shadow Cast BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
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
                    bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
                }),
                multisample:    Default::default(),
                fragment:       None, // depth-only
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

        Self { sdf, mesh, skinned, bgl }
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
        let texel = (HALF_EXTENT * 2.0) / SHADOW_SIZE as f32;
        let a = fit_light_vp(Vec3::ZERO, dir);
        let b = fit_light_vp(Vec3::new(texel * 0.2, 0.0, texel * 0.2), dir);
        assert_eq!(a.to_cols_array(), b.to_cols_array(), "sub-texel pan must snap identically");

        // A large move does change it.
        let c = fit_light_vp(Vec3::new(10.0, 0.0, 0.0), dir);
        assert_ne!(a.to_cols_array(), c.to_cols_array());
    }
}
