// Billboard particle pass — camera-facing quads with additive blending.
//
// Unlike the three opaque pipelines this one reads depth without writing it
// (particles occlude behind geometry but never punch holes in each other) and
// blends One+One, which is order-independent — no back-to-front sorting.
// The quad is expanded in the vertex shader from the camera's right/up basis
// (CameraUniform); the soft-disc falloff is procedural in the fragment shader,
// so there is no texture bind group at all.

use std::mem::size_of;
use wgpu::VertexFormat::Float32x4;
use wgpu::{BindGroupLayout, Device, RenderPipeline, TextureFormat, VertexAttribute, VertexBufferLayout, VertexStepMode};

/// One particle on the GPU. `color` is the already-faded RGB (the client
/// premultiplies fade into it); alpha is unused by the additive blend.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleInstance {
    pub position: [f32; 3], // offset  0 — world-space center
    pub size:     f32,      // offset 12 — half-extent of the quad
    pub color:    [f32; 4], // offset 16
}                           // total: 32 bytes

pub const MAX_PARTICLES: usize = 4096;
pub(crate) const PARTICLE_INSTANCE_SIZE: usize = size_of::<ParticleInstance>(); // 32

pub(crate) fn create_particle_pipeline(
    device:                   &Device,
    surface_format:           TextureFormat,
    camera_bind_group_layout: &BindGroupLayout,
) -> RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("particle_shader.wgsl"));

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:              Some("Particle Pipeline Layout"),
        bind_group_layouts: &[Some(camera_bind_group_layout)],
        immediate_size:     0,
    });

    let instance_attributes = [
        VertexAttribute { offset: 0,  shader_location: 0, format: Float32x4 }, // position + size
        VertexAttribute { offset: 16, shader_location: 1, format: Float32x4 }, // color
    ];
    let instance_buffer_layout = VertexBufferLayout {
        array_stride: PARTICLE_INSTANCE_SIZE as u64,
        step_mode:    VertexStepMode::Instance,
        attributes:   &instance_attributes,
    };

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("Particle Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module:      &shader,
            entry_point: Some("vtx_main"),
            buffers:     &[instance_buffer_layout],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format:              TextureFormat::Depth32Float,
            depth_write_enabled: Some(false), // read-only: occluded by geometry, never occludes
            depth_compare:       Some(wgpu::CompareFunction::Less),
            stencil:             Default::default(),
            bias:                Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module:      &shader,
            entry_point: Some("frag_main"),
            targets:     &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend:  Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation:  wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation:  wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache:          None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_instance_is_tightly_packed() {
        assert_eq!(PARTICLE_INSTANCE_SIZE, 32);
    }
}
