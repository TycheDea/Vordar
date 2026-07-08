// GPU mip-chain generation — VQ-C1.
//
// A render-pass blit chain: for each level i, draw a fullscreen triangle into
// mip i sampling mip i-1 with a linear filter. Works on any renderable format
// (one lazily-created pipeline per format). The same blit skeleton is reused
// by the IBL prefilter (Phase 2) and bloom (Phase 4) chains.

use std::collections::HashMap;
use wgpu::{Device, Queue, Texture, TextureFormat};

pub(crate) struct MipGenerator {
    bgl:       wgpu::BindGroupLayout,
    sampler:   wgpu::Sampler,
    pipelines: HashMap<TextureFormat, wgpu::RenderPipeline>,
}

/// Formats the generator can blit. Built eagerly so `generate` takes `&self`
/// (callers hold it inside RendererState borrows). Extend when new render
/// formats need chains (IBL/bloom add their own passes on this skeleton).
const MIP_FORMATS: [TextureFormat; 2] =
    [TextureFormat::Rgba8Unorm, TextureFormat::Rgba8UnormSrgb];

/// Full mip count for a w×h texture (1×1 base ⇒ 1).
pub(crate) fn mip_level_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

impl MipGenerator {
    pub(crate) fn new(device: &Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("Mipgen BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty:         wgpu::BindingType::Texture {
                        multisampled:   false,
                        view_dimension: wgpu::TextureViewDimension::D2,
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
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:      Some("Mipgen Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::include_wgsl!("mipgen.wgsl"));

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Mipgen Pipeline Layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size:     0,
        });
        let pipelines = MIP_FORMATS
            .iter()
            .map(|&format| {
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label:  Some("Mipgen Pipeline"),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module:      &shader,
                        entry_point: Some("vtx_main"),
                        buffers:     &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    primitive:     Default::default(),
                    depth_stencil: None,
                    multisample:   Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module:      &shader,
                        entry_point: Some("frag_main"),
                        targets:     &[Some(wgpu::ColorTargetState {
                            format,
                            blend:      Some(wgpu::BlendState::REPLACE),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    multiview_mask: None,
                    cache:          None,
                });
                (format, pipeline)
            })
            .collect();

        Self { bgl, sampler, pipelines }
    }

    /// Fill mip levels 1..N of `texture` from level 0. The texture must have
    /// been created with RENDER_ATTACHMENT | TEXTURE_BINDING usage and a full
    /// `mip_level_count`.
    pub(crate) fn generate(&self, device: &Device, queue: &Queue, texture: &Texture) {
        let mips = texture.mip_level_count();
        if mips <= 1 {
            return;
        }
        let format = texture.format();
        if !self.pipelines.contains_key(&format) {
            log::warn!("mipgen: no pipeline for {format:?} — texture keeps a single level");
            return;
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mipgen Encoder"),
        });
        let views: Vec<wgpu::TextureView> = (0..mips)
            .map(|level| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label:             Some("Mipgen Level View"),
                    base_mip_level:    level,
                    mip_level_count:   Some(1),
                    ..Default::default()
                })
            })
            .collect();

        for level in 1..mips as usize {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label:   Some("Mipgen Bind Group"),
                layout:  &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding:  0,
                        resource: wgpu::BindingResource::TextureView(&views[level - 1]),
                    },
                    wgpu::BindGroupEntry {
                        binding:  1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Mipgen Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &views[level],
                    resolve_target: None,
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.pipelines[&format]);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit(std::iter::once(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_count_covers_full_chain() {
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_level_count(2, 2), 2);
        assert_eq!(mip_level_count(256, 256), 9);
        assert_eq!(mip_level_count(512, 256), 10);
        assert_eq!(mip_level_count(1000, 1000), 10); // non-pow2 rounds down
    }
}
