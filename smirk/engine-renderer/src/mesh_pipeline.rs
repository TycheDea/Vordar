use std::mem::size_of;
use wgpu::VertexFormat::{Float32x2, Float32x3, Float32x4};
use wgpu::{
    BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType,
    Device, RenderPipeline, SamplerBindingType, ShaderStages, TextureFormat,
    TextureSampleType, TextureViewDimension, VertexAttribute, VertexBufferLayout,
    VertexStepMode,
};

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
    pub(crate) emissive:   [f32; 4], // emissiveFactor × KHR emissive_strength; w unused
    pub(crate) mr:         [f32; 4], // x = metallicFactor, y = roughnessFactor
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

pub(crate) fn create_mesh_pipeline(
    device:                     &Device,
    surface_format:             TextureFormat,
    camera_bind_group_layout:   &BindGroupLayout,
    material_bind_group_layout: &BindGroupLayout,
) -> RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("mesh_shader.wgsl"));

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:              Some("Mesh Pipeline Layout"),
        bind_group_layouts: &[
            Some(camera_bind_group_layout),
            Some(material_bind_group_layout),
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

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("Mesh Pipeline"),
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
            depth_write_enabled: Some(true),
            depth_compare:       Some(wgpu::CompareFunction::Less),
            stencil:             Default::default(),
            bias:                Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module:      &shader,
            entry_point: Some("frag_main"),
            targets:     &[Some(wgpu::ColorTargetState {
                format:     surface_format,
                blend:      Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache:          None,
    })
}
