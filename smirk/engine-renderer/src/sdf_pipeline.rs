// SDF-primitive pipeline: the unit cube (`Vertex`, `VERTICES`, `INDICES`)
// every shape instances, and `create_pipeline`, the render pipeline that
// steps `SdfInstance` (instance.rs) through shape_type/shape_params.

use crate::instance::SdfInstance;
use std::mem::size_of;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::VertexFormat::{Float32x2, Float32x3, Float32x4, Uint32};
use wgpu::{
    BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType,
    Buffer, BufferUsages, Device, RenderPipeline, SamplerBindingType, ShaderStages,
    TextureFormat, TextureSampleType, TextureViewDimension, VertexAttribute,
    VertexBufferLayout, VertexStepMode,
};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal:   [f32; 3],
    pub uv:       [f32; 2],  // stride: 32 bytes
}

// 24 vertices — 4 per face, each face sharing one outward normal.
// Needed for correct per-face shading; 8-vertex shared cubes can't have face normals.
pub(crate) const VERTICES: &[Vertex] = &[
    // back  (z = -0.5), normal (0, 0, -1)  — verts 0-3
    Vertex { position: [-0.5, -0.5, -0.5], normal: [0.0,  0.0, -1.0], uv: [0.0, 1.0] },
    Vertex { position: [ 0.5, -0.5, -0.5], normal: [0.0,  0.0, -1.0], uv: [1.0, 1.0] },
    Vertex { position: [ 0.5,  0.5, -0.5], normal: [0.0,  0.0, -1.0], uv: [1.0, 0.0] },
    Vertex { position: [-0.5,  0.5, -0.5], normal: [0.0,  0.0, -1.0], uv: [0.0, 0.0] },
    // front (z = +0.5), normal (0, 0, +1)  — verts 4-7
    Vertex { position: [ 0.5, -0.5,  0.5], normal: [0.0,  0.0,  1.0], uv: [0.0, 1.0] },
    Vertex { position: [-0.5, -0.5,  0.5], normal: [0.0,  0.0,  1.0], uv: [1.0, 1.0] },
    Vertex { position: [-0.5,  0.5,  0.5], normal: [0.0,  0.0,  1.0], uv: [1.0, 0.0] },
    Vertex { position: [ 0.5,  0.5,  0.5], normal: [0.0,  0.0,  1.0], uv: [0.0, 0.0] },
    // right (x = +0.5), normal (+1, 0, 0)  — verts 8-11
    Vertex { position: [ 0.5, -0.5,  0.5], normal: [1.0,  0.0,  0.0], uv: [0.0, 1.0] },
    Vertex { position: [ 0.5, -0.5, -0.5], normal: [1.0,  0.0,  0.0], uv: [1.0, 1.0] },
    Vertex { position: [ 0.5,  0.5, -0.5], normal: [1.0,  0.0,  0.0], uv: [1.0, 0.0] },
    Vertex { position: [ 0.5,  0.5,  0.5], normal: [1.0,  0.0,  0.0], uv: [0.0, 0.0] },
    // left  (x = -0.5), normal (-1, 0, 0)  — verts 12-15
    Vertex { position: [-0.5, -0.5, -0.5], normal: [-1.0, 0.0,  0.0], uv: [0.0, 1.0] },
    Vertex { position: [-0.5, -0.5,  0.5], normal: [-1.0, 0.0,  0.0], uv: [1.0, 1.0] },
    Vertex { position: [-0.5,  0.5,  0.5], normal: [-1.0, 0.0,  0.0], uv: [1.0, 0.0] },
    Vertex { position: [-0.5,  0.5, -0.5], normal: [-1.0, 0.0,  0.0], uv: [0.0, 0.0] },
    // top   (y = +0.5), normal (0, +1, 0)  — verts 16-19
    Vertex { position: [-0.5,  0.5, -0.5], normal: [0.0,  1.0,  0.0], uv: [0.0, 0.0] },
    Vertex { position: [ 0.5,  0.5, -0.5], normal: [0.0,  1.0,  0.0], uv: [1.0, 0.0] },
    Vertex { position: [ 0.5,  0.5,  0.5], normal: [0.0,  1.0,  0.0], uv: [1.0, 1.0] },
    Vertex { position: [-0.5,  0.5,  0.5], normal: [0.0,  1.0,  0.0], uv: [0.0, 1.0] },
    // bottom (y = -0.5), normal (0, -1, 0) — verts 20-23
    Vertex { position: [-0.5, -0.5,  0.5], normal: [0.0, -1.0,  0.0], uv: [0.0, 0.0] },
    Vertex { position: [ 0.5, -0.5,  0.5], normal: [0.0, -1.0,  0.0], uv: [1.0, 0.0] },
    Vertex { position: [ 0.5, -0.5, -0.5], normal: [0.0, -1.0,  0.0], uv: [1.0, 1.0] },
    Vertex { position: [-0.5, -0.5, -0.5], normal: [0.0, -1.0,  0.0], uv: [0.0, 1.0] },
];

pub(crate) const INDICES: &[u16] = &[
     0,  1,  2,   0,  2,  3,  // back
     4,  5,  6,   4,  6,  7,  // front
     8,  9, 10,   8, 10, 11,  // right
    12, 13, 14,  12, 14, 15,  // left
    16, 17, 18,  16, 18, 19,  // top
    20, 21, 22,  20, 22, 23,  // bottom
];

pub(crate) fn create_vertex_buffer(device: &Device) -> Buffer {
    device.create_buffer_init(&BufferInitDescriptor {
        label:    Some("Vertex Buffer"),
        contents: bytemuck::cast_slice(VERTICES),
        usage:    BufferUsages::VERTEX,
    })
}

pub(crate) fn create_index_buffer(device: &Device) -> Buffer {
    device.create_buffer_init(&BufferInitDescriptor {
        label:    Some("Cube Index Buffer"),
        contents: bytemuck::cast_slice(INDICES),
        usage:    BufferUsages::INDEX,
    })
}

/// Bind group layout for the color texture (group 1).
pub(crate) fn create_texture_bind_group_layout(device: &Device) -> BindGroupLayout {
    device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label:   Some("Texture BGL"),
        entries: &[
            BindGroupLayoutEntry {
                binding:    0,
                visibility: ShaderStages::FRAGMENT,
                ty:         BindingType::Texture {
                    multisampled:   false,
                    view_dimension: TextureViewDimension::D2,
                    sample_type:    TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding:    1,
                visibility: ShaderStages::FRAGMENT,
                ty:         BindingType::Sampler(SamplerBindingType::Filtering),
                count:      None,
            },
        ],
    })
}

pub(crate) fn create_pipeline(
    device:              &Device,
    surface_format:      TextureFormat,
    camera_bind_group_layout:  &BindGroupLayout,
    texture_bind_group_layout: &BindGroupLayout,
    env_bind_group_layout:     &BindGroupLayout,
) -> RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label:  Some("shader.wgsl"),
        source: wgpu::ShaderSource::Wgsl(include_str!(concat!(env!("OUT_DIR"), "/shader.wgsl")).into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:             Some("Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(camera_bind_group_layout),
            Some(texture_bind_group_layout),
            Some(env_bind_group_layout),
        ],
        immediate_size: 0,
    });

    // Instance locations shifted +1 to make room for UV at vertex location 2.
    let instance_attributes = [
        // model matrix — 4 rows of Float32x4, shader_locations 3..=6
        VertexAttribute { offset:  0, shader_location: 3, format: Float32x4 }, // row 0
        VertexAttribute { offset: 16, shader_location: 4, format: Float32x4 }, // row 1
        VertexAttribute { offset: 32, shader_location: 5, format: Float32x4 }, // row 2
        VertexAttribute { offset: 48, shader_location: 6, format: Float32x4 }, // row 3
        // color — vec3 at byte 64
        VertexAttribute { offset: 64, shader_location: 7, format: Float32x3 },
        // shape_type — u32 at byte 76
        VertexAttribute { offset: 76, shader_location: 8, format: Uint32 },
        // shape_params — vec4 at byte 80
        VertexAttribute { offset: 80, shader_location: 9, format: Float32x4 },
    ];
    let instance_buffer_layout = VertexBufferLayout {
        array_stride: size_of::<SdfInstance>() as u64, // 96 bytes
        step_mode:    VertexStepMode::Instance,
        attributes:   &instance_attributes,
    };

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

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("Render Pipeline"),
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
