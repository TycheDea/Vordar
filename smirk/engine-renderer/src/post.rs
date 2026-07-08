// Post chain (VQ-D1): the HDR render targets the scene passes draw into, and
// the tonemap pass that resolves them onto the swapchain. Bloom (Phase 4)
// composites into the tonemap pass.

use wgpu::{Device, Queue, TextureFormat};

/// Scene color format — HDR, filterable, universally 4×-MSAA-capable.
pub(crate) const HDR_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
/// Scene MSAA sample count (VQ-D4). WebGPU guarantees 4× support for
/// rgba16float + depth32float; a knob here is the documented fallback seam.
pub(crate) const SCENE_SAMPLES: u32 = 4;

/// The offscreen targets one frame of scene rendering uses: multisampled HDR
/// color + depth, and the single-sample HDR resolve the tonemap pass reads.
pub(crate) struct HdrTargets {
    pub(crate) msaa_view:    wgpu::TextureView,
    pub(crate) depth_view:   wgpu::TextureView,
    pub(crate) resolve_view: wgpu::TextureView,
    _msaa:    wgpu::Texture,
    _depth:   wgpu::Texture,
    _resolve: wgpu::Texture,
}

impl HdrTargets {
    pub(crate) fn new(device: &Device, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 };
        let msaa = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("HDR MSAA Color"),
            size,
            mip_level_count: 1,
            sample_count:    SCENE_SAMPLES,
            dimension:       wgpu::TextureDimension::D2,
            format:          HDR_FORMAT,
            usage:           wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats:    &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("HDR MSAA Depth"),
            size,
            mip_level_count: 1,
            sample_count:    SCENE_SAMPLES,
            dimension:       wgpu::TextureDimension::D2,
            format:          TextureFormat::Depth32Float,
            // TEXTURE_BINDING: the particle pass samples scene depth for the
            // soft fade while the attachment is read-only.
            usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats:    &[],
        });
        let resolve = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("HDR Resolve"),
            size,
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          HDR_FORMAT,
            usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats:    &[],
        });
        Self {
            msaa_view:    msaa.create_view(&Default::default()),
            depth_view:   depth.create_view(&Default::default()),
            resolve_view: resolve.create_view(&Default::default()),
            _msaa:    msaa,
            _depth:   depth,
            _resolve: resolve,
        }
    }
}

/// Fullscreen ACES tonemap: HDR resolve (+ bloom) → LDR swapchain.
pub(crate) struct TonemapPass {
    pipeline:        wgpu::RenderPipeline,
    bgl:             wgpu::BindGroupLayout,
    sampler:         wgpu::Sampler,
    exposure_buffer: wgpu::Buffer,
    bind_group:      Option<wgpu::BindGroup>,
    exposure:        f32,
    bloom_intensity: f32,
}

impl TonemapPass {
    pub(crate) fn new(device: &Device, output_format: TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("tonemap.wgsl"));
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("Tonemap BGL"),
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
                wgpu::BindGroupLayoutEntry {
                    binding:    3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled:   false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                }, // bloom
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Tonemap Pipeline Layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size:     0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("Tonemap Pipeline"),
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
                    format:     output_format,
                    blend:      Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache:          None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:      Some("Tonemap Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let exposure_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Exposure Uniform"),
            size:               16,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bgl,
            sampler,
            exposure_buffer,
            bind_group:      None,
            exposure:        1.0,
            bloom_intensity: 0.12,
        }
    }

    /// (Re)point the pass at the scene's HDR resolve + bloom output — call at
    /// init and on every resize.
    pub(crate) fn set_source(
        &mut self,
        device:     &Device,
        hdr_view:   &wgpu::TextureView,
        bloom_view: &wgpu::TextureView,
    ) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("Tonemap Bind Group"),
            layout:  &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(hdr_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.exposure_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(bloom_view) },
            ],
        }));
    }

    pub(crate) fn set_exposure(&mut self, queue: &Queue, exposure: f32) {
        self.exposure = exposure;
        self.upload_params(queue);
    }

    pub(crate) fn set_bloom_intensity(&mut self, queue: &Queue, intensity: f32) {
        self.bloom_intensity = intensity;
        self.upload_params(queue);
    }

    fn upload_params(&self, queue: &Queue) {
        queue.write_buffer(
            &self.exposure_buffer, 0,
            bytemuck::cast_slice(&[self.exposure, self.bloom_intensity, 0.0, 0.0]),
        );
    }

    /// Draw the tonemapped scene onto `dst` (the swapchain view).
    /// `timestamp_writes` lets the frame timer close on this pass.
    pub(crate) fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        dst: &wgpu::TextureView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            log::error!("TonemapPass::encode before set_source — frame dropped");
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Tonemap Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           dst,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Skybox pipeline: cubemap background at the far plane inside the scene pass.
pub(crate) fn create_sky_pipeline(
    device:     &Device,
    camera_bgl: &wgpu::BindGroupLayout,
    sky_bgl:    &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::include_wgsl!("sky.wgsl"));
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

/// GPU frame timing for the dev overlay (Phase 8): two timestamps bracket
/// the frame (shadow-pass begin → tonemap-pass end). `None` when the adapter
/// lacks TIMESTAMP_QUERY. Sampled sparsely and read with a blocking map —
/// dev-overlay-only cost.
pub(crate) struct GpuTimer {
    pub(crate) query_set: wgpu::QuerySet,
    resolve:   wgpu::Buffer,
    staging:   wgpu::Buffer,
    period_ns: f32,
}

impl GpuTimer {
    pub(crate) fn new(device: &Device, queue: &Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("Frame Timestamps"),
            ty:    wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Timestamp Resolve"),
            size:               16,
            usage:              wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("Timestamp Staging"),
            size:               16,
            usage:              wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Some(Self { query_set, resolve, staging, period_ns: queue.get_timestamp_period() })
    }

    pub(crate) fn begin_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set:                    &self.query_set,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index:       None,
        }
    }

    pub(crate) fn end_writes(&self) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set:                    &self.query_set,
            beginning_of_pass_write_index: None,
            end_of_pass_write_index:       Some(1),
        }
    }

    pub(crate) fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.query_set, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, 16);
    }

    /// Blocking read of the last resolved pair → frame GPU milliseconds.
    pub(crate) fn read_blocking(&self, device: &Device) -> Option<f32> {
        let slice = self.staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
            .ok()?;
        let data = slice.get_mapped_range();
        let stamps: &[u64] = bytemuck::cast_slice(&data);
        let ms = (stamps[1].saturating_sub(stamps[0])) as f32 * self.period_ns * 1e-6;
        drop(data);
        self.staging.unmap();
        Some(ms)
    }
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
