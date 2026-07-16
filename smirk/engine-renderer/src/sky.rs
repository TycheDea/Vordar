// Skybox pass: cubemap background drawn at the far plane inside the scene
// pass (depth_write disabled, LessEqual compare) — not a post-process, hence
// living apart from post.rs's HDR-target/tonemap chain.

use wgpu::{Device, TextureFormat};

use crate::post::{HDR_FORMAT, SCENE_SAMPLES};

/// Skybox pipeline: cubemap background at the far plane inside the scene pass.
pub(crate) fn create_sky_pipeline(
    device:     &Device,
    camera_bgl: &wgpu::BindGroupLayout,
    sky_bgl:    &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label:  Some("sky.wgsl"),
        source: wgpu::ShaderSource::Wgsl(include_str!(concat!(env!("OUT_DIR"), "/sky.wgsl")).into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:              Some("Sky Pipeline Layout"),
        bind_group_layouts: &[Some(camera_bgl), Some(sky_bgl)],
        immediate_size:     0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("Sky Pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module:      &shader,
            entry_point: Some("vtx_main"),
            buffers:     &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format:              TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare:       Some(wgpu::CompareFunction::LessEqual),
            stencil:             Default::default(),
            bias:                Default::default(),
        }),
        multisample: wgpu::MultisampleState { count: SCENE_SAMPLES, ..Default::default() },
        fragment: Some(wgpu::FragmentState {
            module:      &shader,
            entry_point: Some("frag_main"),
            targets:     &[Some(wgpu::ColorTargetState {
                format:     HDR_FORMAT,
                blend:      Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview_mask: None,
        cache:          None,
    })
}

/// Bind group layout for the sky pass (cube + sampler).
pub(crate) fn create_sky_bind_group_layout(device: &Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label:   Some("Sky BGL"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled:   false,
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding:    1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count:      None,
            },
        ],
    })
}
