// Skinned mesh pipeline — parallel to mesh_pipeline.rs but with a wider vertex
// (joints + weights), a per-instance `joint_base`, and a third bind group: the
// joint-palette storage buffer read in the vertex shader.

use std::mem::size_of;
use wgpu::VertexFormat::{Float32x2, Float32x3, Float32x4, Uint16x4, Uint32};
use wgpu::{
    BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType,
    BufferBindingType, Device, RenderPipeline, ShaderStages, TextureFormat,
    VertexAttribute, VertexBufferLayout, VertexStepMode,
};

/// GPU vertex for skinned meshes. `joints` are u16×4 (delivered to the shader
/// as `vec4<u32>` by the Uint16x4 format); `weights` sum to 1. `tangent` is
/// xyz + handedness w (glTF convention) for normal mapping.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SkinnedVertex {
    pub(crate) position: [f32; 3], // 0
    pub(crate) normal:   [f32; 3], // 12
    pub(crate) uv:       [f32; 2], // 24
    pub(crate) tangent:  [f32; 4], // 32
    pub(crate) joints:   [u16; 4], // 48
    pub(crate) weights:  [f32; 4], // 56
}                                  // 72 bytes

/// Per-instance data for the skinned pass: transform, tint, and the base
/// offset of this instance's joint block in the shared joint storage buffer.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct SkinnedMeshInstance {
    pub(crate) model:      [[f32; 4]; 4], // 0  — 64 bytes
    pub(crate) tint:       [f32; 4],      // 64 — 16 bytes
    pub(crate) joint_base: u32,           // 80
    pub(crate) _pad:       [u32; 3],      // 84 — pad to 96
}

pub(crate) const SKINNED_VERTEX_SIZE:   usize = size_of::<SkinnedVertex>();       // 72
pub(crate) const SKINNED_INSTANCE_SIZE: usize = size_of::<SkinnedMeshInstance>(); // 96

/// Skinned draw caps. The joint palette holds up to
/// `MAX_SKINNED_INSTANCES × 64` matrices (64 joints is comfortable headroom
/// for humanoid rigs). Sized so a full instance set can't overflow it.
pub(crate) const MAX_SKINNED_INSTANCES: usize = 256;
pub(crate) const MAX_JOINT_MATRICES:    usize = MAX_SKINNED_INSTANCES * 64; // 16_384

/// Bind group layout for the joint palette (group 2): a read-only storage
/// buffer of mat4x4 visible to the vertex stage.
pub(crate) fn create_joint_bind_group_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label:   Some("Joint Palette BGL"),
        entries: &[BindGroupLayoutEntry {
            binding:    0,
            visibility: ShaderStages::VERTEX,
            ty:         BindingType::Buffer {
                ty:                 BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size:   None,
            },
            count: None,
        }],
    })
}

pub(crate) fn create_skinned_pipeline(
    device:         &Device,
    surface_format: TextureFormat,
    camera_bgl:     &BindGroupLayout,
    material_bgl:   &BindGroupLayout,
    joint_bgl:      &BindGroupLayout,
    env_bgl:        &BindGroupLayout,
) -> RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("skinned_mesh_shader.wgsl"));

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:              Some("Skinned Pipeline Layout"),
        bind_group_layouts: &[Some(camera_bgl), Some(material_bgl), Some(joint_bgl), Some(env_bgl)],
        immediate_size:     0,
    });

    let vertex_attributes = [
        VertexAttribute { offset:  0, shader_location: 0, format: Float32x3 }, // position
        VertexAttribute { offset: 12, shader_location: 1, format: Float32x3 }, // normal
        VertexAttribute { offset: 24, shader_location: 2, format: Float32x2 }, // uv
        VertexAttribute { offset: 32, shader_location: 3, format: Float32x4 }, // tangent
        VertexAttribute { offset: 48, shader_location: 4, format: Uint16x4 },  // joints
        VertexAttribute { offset: 56, shader_location: 5, format: Float32x4 }, // weights
    ];
    let vertex_buffer_layout = VertexBufferLayout {
        array_stride: SKINNED_VERTEX_SIZE as u64,
        step_mode:    VertexStepMode::Vertex,
        attributes:   &vertex_attributes,
    };

    let instance_attributes = [
        VertexAttribute { offset:  0, shader_location: 6, format: Float32x4 }, // model row 0
        VertexAttribute { offset: 16, shader_location: 7, format: Float32x4 },
        VertexAttribute { offset: 32, shader_location: 8, format: Float32x4 },
        VertexAttribute { offset: 48, shader_location: 9, format: Float32x4 },
        VertexAttribute { offset: 64, shader_location: 10, format: Float32x4 }, // tint
        VertexAttribute { offset: 80, shader_location: 11, format: Uint32 },    // joint_base
    ];
    let instance_buffer_layout = VertexBufferLayout {
        array_stride: SKINNED_INSTANCE_SIZE as u64,
        step_mode:    VertexStepMode::Instance,
        attributes:   &instance_attributes,
    };

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("Skinned Pipeline"),
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
        multisample: wgpu::MultisampleState { count: crate::post::SCENE_SAMPLES, ..Default::default() },
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
