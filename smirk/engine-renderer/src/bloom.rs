// Dual-filter Kawase bloom. A half-resolution Rgba16Float mip chain:
// soft-knee prefilter from the HDR resolve into mip 0, downsample through
// the chain, then additive tent upsamples back to mip 0, which the tonemap
// pass composites.

use crate::gpu_timer::{GpuPass, GpuTimer};
use crate::post::HDR_FORMAT;
use wgpu::util::DeviceExt;
use wgpu::{Device, Queue};

pub(crate) const BLOOM_LEVELS: u32 = 6;
const THRESHOLD: f32 = 1.0; // display-referred: raw HDR * exposure > 1.0 blooms
const KNEE: f32 = 0.5;

pub(crate) struct BloomPass {
    prefilter: wgpu::RenderPipeline,
    down:      wgpu::RenderPipeline,
    up:        wgpu::RenderPipeline, // additive blend
    /// (bind group sampling the stage source, target mip view) per stage:
    /// [prefilter, down 0→1 … , up N→N-1 …].
    stages: Vec<(wgpu::BindGroup, wgpu::TextureView)>,
    /// Mip 0 of the chain — what the tonemap pass composites.
    pub(crate) output_view: wgpu::TextureView,
    /// Prefilter's params buffer (COPY_DST) — `set_exposure` rewrites its
    /// spare slot without rebuilding the bind group.
    prefilter_params: wgpu::Buffer,
    _chain: wgpu::Texture,
}

impl BloomPass {
    pub(crate) fn new(device: &Device, hdr_resolve: &wgpu::TextureView, width: u32, height: u32) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("bloom.wgsl"));
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("Bloom BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
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
                wgpu::BindGroupLayoutEntry {
                    binding:    2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty:                 wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size:   None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Bloom Pipeline Layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size:     0,
        });

        let make = |entry: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:  Some(entry),
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
                    entry_point: Some(entry),
                    targets:     &[Some(wgpu::ColorTargetState {
                        format:     HDR_FORMAT,
                        blend:      Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache:          None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation:  wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let prefilter = make("prefilter_frag", wgpu::BlendState::REPLACE);
        let down      = make("down_frag", wgpu::BlendState::REPLACE);
        let up        = make("up_frag", additive);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:          Some("Bloom Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Linear,
            min_filter:     wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Half-resolution chain with BLOOM_LEVELS mips.
        let (cw, ch) = ((width / 2).max(1), (height / 2).max(1));
        let levels = BLOOM_LEVELS.min(crate::mipgen::mip_level_count(cw, ch));
        let chain = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("Bloom Chain"),
            size:            wgpu::Extent3d { width: cw, height: ch, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          HDR_FORMAT,
            usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats:    &[],
        });
        let mip_view = |level: u32| {
            chain.create_view(&wgpu::TextureViewDescriptor {
                label:           Some("Bloom Mip View"),
                base_mip_level:  level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };

        let params_buffer = |a: f32, b: f32, c: f32, usage: wgpu::BufferUsages| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:    Some("Bloom Params"),
                contents: bytemuck::cast_slice(&[a, b, c, 0.0]),
                usage,
            })
        };
        let bind = |src: &wgpu::TextureView, params: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label:   Some("Bloom Stage BG"),
                layout:  &bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                ],
            })
        };

        let mut stages = Vec::new();
        // Prefilter: HDR resolve → mip 0. Exposure starts neutral;
        // set_exposure rewrites it later without touching this bind group.
        let prefilter_params = params_buffer(
            THRESHOLD, KNEE, 1.0,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        stages.push((bind(hdr_resolve, &prefilter_params), mip_view(0)));
        // Down chain: mip i-1 → mip i.
        for i in 1..levels {
            let sw = (cw >> (i - 1)).max(1) as f32;
            let sh = (ch >> (i - 1)).max(1) as f32;
            let params = params_buffer(0.5 / sw, 0.5 / sh, 0.0, wgpu::BufferUsages::UNIFORM);
            stages.push((bind(&mip_view(i - 1), &params), mip_view(i)));
        }
        // Up chain: mip i → mip i-1 (additive).
        for i in (1..levels).rev() {
            let sw = (cw >> i).max(1) as f32;
            let sh = (ch >> i).max(1) as f32;
            let params = params_buffer(0.5 / sw, 0.5 / sh, 0.0, wgpu::BufferUsages::UNIFORM);
            stages.push((bind(&mip_view(i), &params), mip_view(i - 1)));
        }

        Self {
            prefilter,
            down,
            up,
            stages,
            output_view: mip_view(0),
            prefilter_params,
            _chain: chain,
        }
    }

    /// Rewrite the prefilter's exposure multiplier — the same value the
    /// tonemap pass applies to the HDR resolve (tonemap.wgsl composites
    /// `hdr * exposure + bloom`), so the threshold sees display-referred
    /// brightness instead of raw scene values.
    pub(crate) fn set_exposure(&self, queue: &Queue, exposure: f32) {
        queue.write_buffer(&self.prefilter_params, 8, bytemuck::cast_slice(&[exposure]));
    }

    /// Record the full bloom chain. Call between the main-pass resolve and
    /// the tonemap pass. `timer` brackets the whole chain (first stage's
    /// begin write, last stage's end write) under `GpuPass::Bloom` — a
    /// single stage gets both writes on its one pass.
    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, timer: Option<&GpuTimer>) {
        let levels = self.stages.len().div_ceil(2); // prefilter + (levels-1) down + (levels-1) up
        let last = self.stages.len().saturating_sub(1);
        for (i, (bind_group, target)) in self.stages.iter().enumerate() {
            let (pipeline, load) = if i == 0 {
                (&self.prefilter, wgpu::LoadOp::Clear(wgpu::Color::BLACK))
            } else if i < levels {
                (&self.down, wgpu::LoadOp::Clear(wgpu::Color::BLACK))
            } else {
                (&self.up, wgpu::LoadOp::Load) // additive over the down result
            };
            let timestamp_writes = match timer {
                Some(t) if i == 0 && i == last => Some(t.pass_writes(GpuPass::Bloom)),
                Some(t) if i == 0              => Some(t.begin_writes(GpuPass::Bloom)),
                Some(t) if i == last           => Some(t.end_writes(GpuPass::Bloom)),
                _                               => None,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Stage"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           target,
                    resolve_target: None,
                    depth_slice:    None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes,
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
