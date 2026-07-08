use crate::pipeline::Vertex;
use std::mem::size_of;
use wgpu::VertexFormat::{Float32x2, Float32x3, Float32x4};
use wgpu::{
    BindGroupLayout, Device, RenderPipeline, TextureFormat, VertexAttribute,
    VertexBufferLayout, VertexStepMode,
};

/// Per-instance GPU data for the mesh pass.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MeshInstance {
    pub(crate) model: [[f32; 4]; 4], // offset  0 — 64 bytes
    pub(crate) tint:  [f32; 4],      // offset 64 — 16 bytes
}                                    // total: 80 bytes

pub(crate) const MESH_INSTANCE_SIZE: usize = size_of::<MeshInstance>(); // 80

pub(crate) fn create_mesh_pipeline(
    device:                    &Device,
    surface_format:            TextureFormat,
    camera_bind_group_layout:  &BindGroupLayout,
    texture_bind_group_layout: &BindGroupLayout,
) -> RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("mesh_shader.wgsl"));

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:              Some("Mesh Pipeline Layout"),
        bind_group_layouts: &[
            Some(camera_bind_group_layout),
            Some(texture_bind_group_layout),
        ],
        immediate_size: 0,
    });

    let vertex_attributes = [
        VertexAttribute { offset:  0, shader_location: 0, format: Float32x3 }, // position
        VertexAttribute { offset: 12, shader_location: 1, format: Float32x3 }, // normal
        VertexAttribute { offset: 24, shader_location: 2, format: Float32x2 }, // uv
    ];
    let vertex_buffer_layout = VertexBufferLayout {
        array_stride: size_of::<Vertex>() as u64, // 32 bytes
        step_mode:    VertexStepMode::Vertex,
        attributes:   &vertex_attributes,
    };

    let instance_attributes = [
        // model matrix — 4 rows of Float32x4, shader_locations 3..=6
        VertexAttribute { offset:  0, shader_location: 3, format: Float32x4 },
        VertexAttribute { offset: 16, shader_location: 4, format: Float32x4 },
        VertexAttribute { offset: 32, shader_location: 5, format: Float32x4 },
        VertexAttribute { offset: 48, shader_location: 6, format: Float32x4 },
        // tint — vec4 at byte 64
        VertexAttribute { offset: 64, shader_location: 7, format: Float32x4 },
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
