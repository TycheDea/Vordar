// Image-based lighting: bake a Radiance .hdr equirect into the cubemaps the
// PBR shaders consume — base environment (sky), diffuse irradiance, and the
// GGX-prefiltered specular chain — through render passes (the mipgen blit
// skeleton with IBL fragments), once per environment load. The split-sum
// BRDF LUT depends only on (NdotV, roughness), never the equirect, so it
// bakes once (`bake_brdf_lut`) and every `Environment` shares that view.

use half::f16;
use wgpu::util::DeviceExt;
use wgpu::{Device, Queue, TextureFormat};

const CUBE_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
pub(crate) const ENV_SIZE: u32 = 512; // base cubemap (sky)
const IRRADIANCE_SIZE: u32 = 32;
const PREFILTER_SIZE: u32 = 128;
/// Prefilter mip count → shaders map roughness 0..1 onto mips 0..(N-1).
pub(crate) const PREFILTER_MIPS: u32 = 5;
const BRDF_SIZE: u32 = 512;

/// Bind group layout the geometry shaders consume (their `env` group):
/// irradiance cube, prefiltered cube, BRDF LUT, sampler.
pub(crate) fn create_env_bind_group_layout(device: &Device) -> wgpu::BindGroupLayout {
    let cube = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled:   false,
            view_dimension: wgpu::TextureViewDimension::Cube,
            sample_type:    wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label:   Some("Environment BGL"),
        entries: &[
            cube(0), // irradiance
            cube(1), // prefiltered specular
            wgpu::BindGroupLayoutEntry {
                binding:    2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled:   false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            }, // BRDF LUT
            wgpu::BindGroupLayoutEntry {
                binding:    3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count:      None,
            },
        ],
    })
}

/// A loaded environment: the geometry env bind group + the sky-pass bind
/// group, with every texture they reference kept alive. Swapped per zone.
pub(crate) struct Environment {
    pub(crate) bind_group:     wgpu::BindGroup,
    pub(crate) sky_bind_group: wgpu::BindGroup,
    _cubemap:    wgpu::Texture,
    _irradiance: wgpu::Texture,
    _prefilter:  wgpu::Texture,
    _sampler:    wgpu::Sampler,
}

impl Environment {
    /// Load a Radiance .hdr equirect from disk and bake the full IBL set.
    pub(crate) fn from_hdr(
        device:     &Device,
        queue:      &Queue,
        baker:      &Baker,
        env_layout: &wgpu::BindGroupLayout,
        sky_layout: &wgpu::BindGroupLayout,
        brdf_view:  &wgpu::TextureView,
        path:       &str,
    ) -> Result<Self, String> {
        let img = image::open(path).map_err(|e| format!("{path}: {e}"))?.into_rgb32f();
        let (w, h) = (img.width(), img.height());
        let pixels: Vec<f32> = img
            .pixels()
            .flat_map(|p| [p.0[0], p.0[1], p.0[2], 1.0])
            .collect();
        Ok(Self::from_equirect_pixels(device, queue, baker, env_layout, sky_layout, brdf_view, w, h, &pixels))
    }

    /// Bake from raw RGBA f32 equirect pixels (also the test seam: a uniform
    /// image gives a white-furnace environment). `brdf_view` is the LUT baked
    /// once at renderer init (see `bake_brdf_lut`) and shared across every
    /// `Environment` — it depends only on (NdotV, roughness), never on the
    /// pixels baked here. `baker` is the same shared pipeline set, built once
    /// at renderer init (see `Baker::new`).
    pub(crate) fn from_equirect_pixels(
        device:     &Device,
        queue:      &Queue,
        baker:      &Baker,
        env_layout: &wgpu::BindGroupLayout,
        sky_layout: &wgpu::BindGroupLayout,
        brdf_view:  &wgpu::TextureView,
        width:      u32,
        height:     u32,
        rgba_f32:   &[f32],
    ) -> Self {
        // Upload the equirect as Rgba16Float (filterable everywhere).
        let halves: Vec<f16> = rgba_f32.iter().map(|&v| f16::from_f32(v)).collect();
        let equirect = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("IBL Equirect"),
            size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          TextureFormat::Rgba16Float,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture:   &equirect,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&halves),
            wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(width * 8),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );

        let equirect_view = equirect.create_view(&Default::default());

        // Every face of every stage records into one encoder — one queue
        // submit per environment load instead of 42. wgpu inserts usage
        // transitions between render passes, so the cubemap the equirect
        // passes write is legal to sample in the irradiance/prefilter passes
        // recorded into the same encoder afterward.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("IBL Bake Encoder"),
        });

        // 1. equirect → base cubemap.
        let cubemap = create_cube(device, "IBL Cubemap", ENV_SIZE, 1);
        for face in 0..6 {
            baker.bake_face(
                device, &mut encoder, &baker.equirect_pipeline,
                Some(&equirect_view), None,
                &cubemap, face, 0, 0.0,
            );
        }
        let base_cube_view = cube_view(&cubemap);

        // 2. diffuse irradiance.
        let irradiance = create_cube(device, "IBL Irradiance", IRRADIANCE_SIZE, 1);
        for face in 0..6 {
            baker.bake_face(
                device, &mut encoder, &baker.irradiance_pipeline,
                None, Some(&base_cube_view),
                &irradiance, face, 0, 0.0,
            );
        }

        // 3. GGX prefiltered specular chain.
        let prefilter = create_cube(device, "IBL Prefilter", PREFILTER_SIZE, PREFILTER_MIPS);
        for mip in 0..PREFILTER_MIPS {
            let roughness = mip as f32 / (PREFILTER_MIPS - 1) as f32;
            for face in 0..6 {
                baker.bake_face(
                    device, &mut encoder, &baker.prefilter_pipeline,
                    None, Some(&base_cube_view),
                    &prefilter, face, mip, roughness,
                );
            }
        }

        queue.submit(std::iter::once(encoder.finish()));

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:          Some("Environment Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Linear,
            min_filter:     wgpu::FilterMode::Linear,
            mipmap_filter:  wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("Environment Bind Group"),
            layout:  env_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&cube_view(&irradiance)) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&cube_view(&prefilter)) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(brdf_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let sky_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("Sky Bind Group"),
            layout:  sky_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&base_cube_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        Self {
            bind_group,
            sky_bind_group,
            _cubemap:    cubemap,
            _irradiance: irradiance,
            _prefilter:  prefilter,
            _sampler:    sampler,
        }
    }

    /// Fallback environment when a zone sets no HDRI: a uniform mid-gray sky
    /// giving flat, non-directional IBL ambient.
    pub(crate) fn default_gray(
        device:     &Device,
        queue:      &Queue,
        baker:      &Baker,
        env_layout: &wgpu::BindGroupLayout,
        sky_layout: &wgpu::BindGroupLayout,
        brdf_view:  &wgpu::TextureView,
    ) -> Self {
        let v = 0.18f32;
        let pixels: Vec<f32> = (0..4 * 2).flat_map(|_| [v, v, v, 1.0]).collect();
        Self::from_equirect_pixels(device, queue, baker, env_layout, sky_layout, brdf_view, 4, 2, &pixels)
    }
}

/// Bakes the split-sum BRDF LUT once — a pure function of (NdotV,
/// roughness) with no environment input (`ibl.wgsl:155-183`), so every
/// `Environment` shares this view instead of rebaking it per zone crossing.
pub(crate) fn bake_brdf_lut(device: &Device, queue: &Queue, baker: &Baker) -> wgpu::TextureView {
    let brdf = device.create_texture(&wgpu::TextureDescriptor {
        label:           Some("IBL BRDF LUT"),
        size:            wgpu::Extent3d { width: BRDF_SIZE, height: BRDF_SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count:    1,
        dimension:       wgpu::TextureDimension::D2,
        format:          TextureFormat::Rgba16Float,
        usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats:    &[],
    });
    baker.bake_2d(device, queue, &baker.brdf_pipeline, &brdf);
    #[cfg(feature = "offscreen")]
    BAKE_COUNT.with(|c| c.set(c.get() + 1));
    brdf.create_view(&Default::default())
}

// Thread-local rather than a process-global counter: the default test
// harness runs each `#[test]` on its own thread, so a test's count of its
// own renderer's bakes can't be polluted by other tests' renderers running
// concurrently on other threads.
#[cfg(feature = "offscreen")]
thread_local! {
    static BAKE_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// BRDF LUT bakes performed on the calling thread so far — lets tests assert
/// that repeated `Environment` construction shares the LUT rather than
/// rebaking it.
#[cfg(feature = "offscreen")]
pub(crate) fn brdf_bake_count() -> u32 {
    BAKE_COUNT.with(|c| c.get())
}

#[cfg(feature = "offscreen")]
thread_local! {
    static BAKER_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// `Baker` constructions (shader module + pipeline compiles) performed on the
/// calling thread so far — lets tests assert that repeated `Environment`
/// construction (zone crossings) reuses the baker built at renderer init
/// rather than recompiling its pipelines.
#[cfg(feature = "offscreen")]
pub(crate) fn baker_construction_count() -> u32 {
    BAKER_COUNT.with(|c| c.get())
}

fn create_cube(device: &Device, label: &str, size: u32, mips: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label:           Some(label),
        size:            wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 6 },
        mip_level_count: mips,
        sample_count:    1,
        dimension:       wgpu::TextureDimension::D2,
        format:          CUBE_FORMAT,
        usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats:    &[],
    })
}

fn cube_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label:     Some("Cube View"),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    face:      u32,
    roughness: f32,
    _pad:      [f32; 2],
}

/// The four bake pipelines plus their layouts, built once per device and
/// shared by reference across every environment bake.
pub(crate) struct Baker {
    src_bgl:             wgpu::BindGroupLayout, // t_src + s_src + params
    cube_bgl:            wgpu::BindGroupLayout, // t_cube
    sampler:             wgpu::Sampler,
    dummy_2d:            wgpu::Texture,
    dummy_cube:          wgpu::Texture,
    equirect_pipeline:   wgpu::RenderPipeline,
    irradiance_pipeline: wgpu::RenderPipeline,
    prefilter_pipeline:  wgpu::RenderPipeline,
    brdf_pipeline:       wgpu::RenderPipeline,
}

impl Baker {
    pub(crate) fn new(device: &Device) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("ibl.wgsl"));

        let src_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("IBL Src BGL"),
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
        let cube_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("IBL Cube BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled:   false,
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("IBL Pipeline Layout"),
            bind_group_layouts: &[Some(&src_bgl), Some(&cube_bgl)],
            immediate_size:     0,
        });

        let make = |entry: &str| {
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
                        format:     CUBE_FORMAT,
                        blend:      Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache:          None,
            })
        };

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:          Some("IBL Bake Sampler"),
            address_mode_u: wgpu::AddressMode::Repeat, // equirect wraps in u
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Linear,
            min_filter:     wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Dummies fill the unused slot of each pass (equirect ignores the
        // cube group; the cube passes ignore t_src).
        let dummy_2d = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("IBL Dummy 2D"),
            size:            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          CUBE_FORMAT,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats:    &[],
        });
        let dummy_cube = create_cube(device, "IBL Dummy Cube", 1, 1);

        #[cfg(feature = "offscreen")]
        BAKER_COUNT.with(|c| c.set(c.get() + 1));

        Self {
            equirect_pipeline:   make("equirect_frag"),
            irradiance_pipeline: make("irradiance_frag"),
            prefilter_pipeline:  make("prefilter_frag"),
            brdf_pipeline:       make("brdf_frag"),
            src_bgl,
            cube_bgl,
            sampler,
            dummy_2d,
            dummy_cube,
        }
    }

    fn bind_groups(
        &self,
        device: &Device,
        src_2d:   Option<&wgpu::TextureView>,
        src_cube: Option<&wgpu::TextureView>,
        face:      u32,
        roughness: f32,
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("IBL Params"),
            contents: bytemuck::cast_slice(&[Params { face, roughness, _pad: [0.0; 2] }]),
            usage:    wgpu::BufferUsages::UNIFORM,
        });
        let dummy_2d_view = self.dummy_2d.create_view(&Default::default());
        let dummy_cube_view = cube_view(&self.dummy_cube);
        let src = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("IBL Src BG"),
            layout:  &self.src_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding:  0,
                    resource: wgpu::BindingResource::TextureView(src_2d.unwrap_or(&dummy_2d_view)),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
            ],
        });
        let cube = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("IBL Cube BG"),
            layout:  &self.cube_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: wgpu::BindingResource::TextureView(src_cube.unwrap_or(&dummy_cube_view)),
            }],
        });
        (src, cube)
    }

    #[allow(clippy::too_many_arguments)]
    fn bake_face(
        &self,
        device:   &Device,
        encoder:  &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        src_2d:   Option<&wgpu::TextureView>,
        src_cube: Option<&wgpu::TextureView>,
        dst:       &wgpu::Texture,
        face:      u32,
        mip:       u32,
        roughness: f32,
    ) {
        let (src_bg, cube_bg) = self.bind_groups(device, src_2d, src_cube, face, roughness);
        let target = dst.create_view(&wgpu::TextureViewDescriptor {
            label:             Some("IBL Face Target"),
            dimension:         Some(wgpu::TextureViewDimension::D2),
            base_mip_level:    mip,
            mip_level_count:   Some(1),
            base_array_layer:  face,
            array_layer_count: Some(1),
            ..Default::default()
        });
        self.record(encoder, pipeline, &src_bg, &cube_bg, &target);
    }

    fn bake_2d(&self, device: &Device, queue: &Queue, pipeline: &wgpu::RenderPipeline, dst: &wgpu::Texture) {
        let (src_bg, cube_bg) = self.bind_groups(device, None, None, 0, 0.0);
        let target = dst.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("IBL Bake Encoder"),
        });
        self.record(&mut encoder, pipeline, &src_bg, &cube_bg, &target);
        queue.submit(std::iter::once(encoder.finish()));
    }

    fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        src_bg:  &wgpu::BindGroup,
        cube_bg: &wgpu::BindGroup,
        target:  &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("IBL Bake Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           target,
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, src_bg, &[]);
        pass.set_bind_group(1, cube_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}
