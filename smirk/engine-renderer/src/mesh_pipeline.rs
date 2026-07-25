// Static glTF mesh pipeline: `MeshVertex` (position/normal/uv/tangent), the
// per-instance `MeshInstance`, and the material bind group `MaterialUniform`
// describes (base color factor + texture). Parallel to skinned_pipeline.rs
// minus the joint palette.

use std::mem::size_of;
use wgpu::VertexFormat::{Float32x2, Float32x3, Float32x4};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, Device, RenderPipeline,
    SamplerBindingType, ShaderStages, TextureFormat, TextureSampleType, TextureViewDimension,
    VertexAttribute, VertexBufferLayout, VertexStepMode,
};
use crate::texture::ColorTexture;

/// GPU vertex for static glTF meshes: the primitive-pass `Vertex` plus a
/// vec4 tangent (xyz + handedness w, glTF convention) for normal mapping.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3], //  0
    pub normal:   [f32; 3], // 12
    pub uv:       [f32; 2], // 24
    pub tangent:  [f32; 4], // 32
}                           // 48 bytes

/// Per-instance GPU data for the mesh pass.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MeshInstance {
    pub(crate) model: [[f32; 4]; 4], // offset  0 — 64 bytes
    pub(crate) tint:  [f32; 4],      // offset 64 — 16 bytes
}                                    // total: 80 bytes

pub(crate) const MESH_INSTANCE_SIZE: usize = size_of::<MeshInstance>(); // 80

/// Per-primitive PBR factors (glTF material), bound at group 1 binding 6.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MaterialUniform {
    pub(crate) base_color: [f32; 4], // baseColorFactor
    pub(crate) emissive:   [f32; 4], // emissiveFactor × KHR emissive_strength; w = 1.0 opts into the world-space detail overlay (mesh_shader.wgsl)
    pub(crate) mr:         [f32; 4], // x = metallicFactor, y = roughnessFactor, z = mask cutoff (0 = opaque), w = 1 for BLEND
}

fn texture_entry(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Texture {
            multisampled:   false,
            view_dimension: TextureViewDimension::D2,
            sample_type:    TextureSampleType::Float { filterable: true },
        },
        count: None,
    }
}

/// Bind group layout for the full PBR material (group 1 of the mesh and
/// skinned pipelines): albedo/normal/metallic-roughness/emissive/AO textures,
/// one shared sampler, and the factor uniform. Missing maps bind 1×1 neutral
/// defaults so untextured content keeps working.
pub(crate) fn create_material_bind_group_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label:   Some("Material BGL"),
        entries: &[
            texture_entry(0), // albedo (sRGB)
            BindGroupLayoutEntry {
                binding:    1,
                visibility: ShaderStages::FRAGMENT,
                ty:         BindingType::Sampler(SamplerBindingType::Filtering),
                count:      None,
            },
            texture_entry(2), // normal (linear)
            texture_entry(3), // metallic-roughness (linear; g=rough, b=metal)
            texture_entry(4), // emissive (sRGB)
            texture_entry(5), // occlusion (linear; r)
            BindGroupLayoutEntry {
                binding:    6,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   None,
                },
                count: None,
            },
        ],
    })
}

/// Bind group layout for the world-space detail overlay (group 3 of the
/// static mesh pipeline only — the skinned pipeline is already at
/// `wgpu::Limits::default().max_bind_groups` with no room for a 4th group,
/// and characters don't opt into this layer). Two shared maps (detail
/// albedo, detail normal) plus one sampler, bound once per pass — not per
/// primitive, unlike the material BGL — since every opted-in material reads
/// the same global tile.
pub(crate) fn create_detail_bind_group_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label:   Some("Detail BGL"),
        entries: &[
            texture_entry(0), // detail albedo (sRGB, BC7)
            texture_entry(1), // detail normal (linear, tangent-space, z reconstructed from xy — BC5)
            BindGroupLayoutEntry {
                binding:    2,
                visibility: ShaderStages::FRAGMENT,
                ty:         BindingType::Sampler(SamplerBindingType::Filtering),
                count:      None,
            },
        ],
    })
}

/// Bind group for `create_detail_bind_group_layout`'s layout — the shared
/// sampler comes from `albedo` (both slots' samplers are built by
/// `texture::make_sampler`, so either would do).
pub(crate) fn create_detail_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    albedo: &ColorTexture,
    normal: &ColorTexture,
) -> BindGroup {
    device.create_bind_group(&BindGroupDescriptor {
        label:   Some("Detail Bind Group"),
        layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: BindingResource::TextureView(&albedo.view) },
            BindGroupEntry { binding: 1, resource: BindingResource::TextureView(&normal.view) },
            BindGroupEntry { binding: 2, resource: BindingResource::Sampler(&albedo.sampler) },
        ],
    })
}

pub(crate) fn create_mesh_pipeline(
    device:                     &Device,
    surface_format:             TextureFormat,
    camera_bind_group_layout:   &BindGroupLayout,
    material_bind_group_layout: &BindGroupLayout,
    env_bind_group_layout:      &BindGroupLayout,
    detail_bind_group_layout:   &BindGroupLayout,
    transparent:                bool,
) -> RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label:  Some("mesh_shader.wgsl"),
        source: wgpu::ShaderSource::Wgsl(include_str!(concat!(env!("OUT_DIR"), "/mesh_shader.wgsl")).into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:              Some("Mesh Pipeline Layout"),
        bind_group_layouts: &[
            Some(camera_bind_group_layout),
            Some(material_bind_group_layout),
            Some(env_bind_group_layout),
            Some(detail_bind_group_layout),
        ],
        immediate_size: 0,
    });

    let vertex_attributes = [
        VertexAttribute { offset:  0, shader_location: 0, format: Float32x3 }, // position
        VertexAttribute { offset: 12, shader_location: 1, format: Float32x3 }, // normal
        VertexAttribute { offset: 24, shader_location: 2, format: Float32x2 }, // uv
        VertexAttribute { offset: 32, shader_location: 3, format: Float32x4 }, // tangent
    ];
    let vertex_buffer_layout = VertexBufferLayout {
        array_stride: size_of::<MeshVertex>() as u64, // 48 bytes
        step_mode:    VertexStepMode::Vertex,
        attributes:   &vertex_attributes,
    };

    let instance_attributes = [
        // model matrix — 4 rows of Float32x4, shader_locations 4..=7
        VertexAttribute { offset:  0, shader_location: 4, format: Float32x4 },
        VertexAttribute { offset: 16, shader_location: 5, format: Float32x4 },
        VertexAttribute { offset: 32, shader_location: 6, format: Float32x4 },
        VertexAttribute { offset: 48, shader_location: 7, format: Float32x4 },
        // tint — vec4 at byte 64
        VertexAttribute { offset: 64, shader_location: 8, format: Float32x4 },
    ];
    let instance_buffer_layout = VertexBufferLayout {
        array_stride: MESH_INSTANCE_SIZE as u64,
        step_mode:    VertexStepMode::Instance,
        attributes:   &instance_attributes,
    };

    // Shader outputs premultiplied rgb for BLEND fragments (particle_pipeline.rs's premultiplied BlendState).
    let premultiplied = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation:  wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation:  wgpu::BlendOperation::Add,
        },
    };
    let (label, depth_write_enabled, alpha_to_coverage_enabled, blend) = if transparent {
        ("Mesh Pipeline (transparent)", false, false, premultiplied)
    } else {
        ("Mesh Pipeline", true, true, wgpu::BlendState::REPLACE)
    };

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module:      &shader,
            entry_point: Some("vtx_main"),
            buffers:     &[vertex_buffer_layout, instance_buffer_layout],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format:              TextureFormat::Depth32Float,
            depth_write_enabled: Some(depth_write_enabled),
            depth_compare:       Some(wgpu::CompareFunction::Less),
            stencil:             Default::default(),
            bias:                Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: crate::post::SCENE_SAMPLES,
            alpha_to_coverage_enabled,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module:      &shader,
            entry_point: Some("frag_main"),
            targets:     &[Some(wgpu::ColorTargetState {
                format:     surface_format,
                blend:      Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache:          None,
    })
}
