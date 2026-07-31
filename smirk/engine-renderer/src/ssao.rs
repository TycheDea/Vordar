// Depth prepass + GTAO: a full-res single-sample depth-only pass renders
// opaque geometry from the main camera, then three compute passes (gtao.wgsl:
// depth prefilter → horizon-slice GTAO → edge-aware spatial denoise) produce
// a full-res AO texture. shade_pbr (pbr_common.wgsl) samples the denoised
// result to scale IBL ambient only; `WhiteAo` is the neutral stand-in bound
// whenever SSAO is disabled.

use wgpu::{BindGroupLayout, Device, TextureFormat};

/// Depth-only pipeline variants of the three geometry pipelines, rendering
/// into `SsaoTargets`' full-res depth from the main camera (group 0 is the
/// same scene bind group the main pipelines use) instead of a light's view.
pub(crate) struct DepthPrepassPipelines {
    pub(crate) sdf:     wgpu::RenderPipeline,
    pub(crate) mesh:    wgpu::RenderPipeline,
    pub(crate) skinned: wgpu::RenderPipeline,
}

impl DepthPrepassPipelines {
    pub(crate) fn new(device: &Device, camera_bgl: &BindGroupLayout, joint_bgl: &BindGroupLayout) -> Self {
        use crate::instance::SdfInstance;
        use crate::mesh_pipeline::{MeshVertex, MESH_INSTANCE_SIZE};
        use crate::sdf_pipeline::Vertex;
        use crate::skinned_pipeline::{SKINNED_INSTANCE_SIZE, SKINNED_VERTEX_SIZE};
        use std::mem::size_of;
        use wgpu::VertexFormat::{Float32x3, Float32x4, Uint16x4, Uint32};
        use wgpu::{VertexAttribute, VertexBufferLayout, VertexStepMode};

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("Depth Prepass Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!(concat!(env!("OUT_DIR"), "/depth_prepass.wgsl")).into()),
        });

        let layout_static = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Depth Prepass Static Layout"),
            bind_group_layouts: &[Some(camera_bgl)],
            immediate_size:     0,
        });
        let layout_skinned = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("Depth Prepass Skinned Layout"),
            bind_group_layouts: &[Some(camera_bgl), Some(joint_bgl)],
            immediate_size:     0,
        });

        let make = |label: &str,
                    layout: &wgpu::PipelineLayout,
                    entry: &str,
                    buffers: &[VertexBufferLayout]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label:  Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module:      &shader,
                    entry_point: Some(entry),
                    buffers,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format:              TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare:       Some(wgpu::CompareFunction::Less),
                    stencil:             Default::default(),
                    bias:                wgpu::DepthBiasState::default(),
                }),
                multisample:    Default::default(),
                fragment:       None, // depth-only
                multiview_mask: None,
                cache:          None,
            })
        };

        // Vertex/instance layouts mirror shadow.rs's — same buffers, same
        // subset of attributes (position, model rows, skin data).
        let sdf_vertex = [
            VertexAttribute { offset: 0, shader_location: 0, format: Float32x3 },
        ];
        let sdf_instance = [
            VertexAttribute { offset:  0, shader_location: 3, format: Float32x4 },
            VertexAttribute { offset: 16, shader_location: 4, format: Float32x4 },
            VertexAttribute { offset: 32, shader_location: 5, format: Float32x4 },
            VertexAttribute { offset: 48, shader_location: 6, format: Float32x4 },
        ];
        let mesh_vertex = [
            VertexAttribute { offset: 0, shader_location: 0, format: Float32x3 },
        ];
        let mesh_instance = [
            VertexAttribute { offset:  0, shader_location: 4, format: Float32x4 },
            VertexAttribute { offset: 16, shader_location: 5, format: Float32x4 },
            VertexAttribute { offset: 32, shader_location: 6, format: Float32x4 },
            VertexAttribute { offset: 48, shader_location: 7, format: Float32x4 },
        ];
        let skinned_vertex = [
            VertexAttribute { offset:  0, shader_location: 0, format: Float32x3 },
            VertexAttribute { offset: 48, shader_location: 4, format: Uint16x4 },
            VertexAttribute { offset: 56, shader_location: 5, format: Float32x4 },
        ];
        let skinned_instance = [
            VertexAttribute { offset:  0, shader_location: 6, format: Float32x4 },
            VertexAttribute { offset: 16, shader_location: 7, format: Float32x4 },
            VertexAttribute { offset: 32, shader_location: 8, format: Float32x4 },
            VertexAttribute { offset: 48, shader_location: 9, format: Float32x4 },
            VertexAttribute { offset: 80, shader_location: 11, format: Uint32 },
        ];

        let sdf = make(
            "Depth Prepass SDF Pipeline",
            &layout_static,
            "sdf_vtx",
            &[
                VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode:    VertexStepMode::Vertex,
                    attributes:   &sdf_vertex,
                },
                VertexBufferLayout {
                    array_stride: size_of::<SdfInstance>() as u64,
                    step_mode:    VertexStepMode::Instance,
                    attributes:   &sdf_instance,
                },
            ],
        );
        let mesh = make(
            "Depth Prepass Mesh Pipeline",
            &layout_static,
            "mesh_vtx",
            &[
                VertexBufferLayout {
                    array_stride: size_of::<MeshVertex>() as u64,
                    step_mode:    VertexStepMode::Vertex,
                    attributes:   &mesh_vertex,
                },
                VertexBufferLayout {
                    array_stride: MESH_INSTANCE_SIZE as u64,
                    step_mode:    VertexStepMode::Instance,
                    attributes:   &mesh_instance,
                },
            ],
        );
        let skinned = make(
            "Depth Prepass Skinned Pipeline",
            &layout_skinned,
            "skinned_vtx",
            &[
                VertexBufferLayout {
                    array_stride: SKINNED_VERTEX_SIZE as u64,
                    step_mode:    VertexStepMode::Vertex,
                    attributes:   &skinned_vertex,
                },
                VertexBufferLayout {
                    array_stride: SKINNED_INSTANCE_SIZE as u64,
                    step_mode:    VertexStepMode::Instance,
                    attributes:   &skinned_instance,
                },
            ],
        );

        Self { sdf, mesh, skinned }
    }
}

/// Depth mip levels in the prefiltered chain (XeGTAO's XE_GTAO_DEPTH_MIP_LEVELS).
const DEPTH_MIP_COUNT: u32 = 5;

/// The full-res prepass depth plus the GTAO intermediates: the linearized
/// depth mip chain, packed depth-difference edges, the noisy AO the main pass
/// writes, and the denoised AO the scene bind group samples.
pub(crate) struct SsaoTargets {
    pub(crate) width:  u32,
    pub(crate) height: u32,
    pub(crate) prepass_depth_view: wgpu::TextureView,
    /// Per-mip storage views the prefilter pass writes.
    pub(crate) depth_mip_views: Vec<wgpu::TextureView>,
    /// All-mips sampled view the GTAO main pass reads.
    pub(crate) depth_mips_view: wgpu::TextureView,
    pub(crate) edges_view:    wgpu::TextureView,
    pub(crate) noisy_ao_view: wgpu::TextureView,
    pub(crate) ao_view:       wgpu::TextureView,
    /// Exposed for CPU readback (offscreen tests only — production frames
    /// never read this back).
    pub(crate) ao: wgpu::Texture,
    _prepass_depth: wgpu::Texture,
    _depth_mips:    wgpu::Texture,
    _edges:         wgpu::Texture,
    _noisy_ao:      wgpu::Texture,
}

impl SsaoTargets {
    pub(crate) fn new(device: &Device, width: u32, height: u32) -> Self {
        let width  = width.max(1);
        let height = height.max(1);

        let prepass_depth = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("SSAO Prepass Depth"),
            size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          TextureFormat::Depth32Float,
            usage:           wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats:    &[],
        });

        let storage_tex = |label: &str, format: TextureFormat, mips: u32, extra: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label:           Some(label),
                size:            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: mips,
                sample_count:    1,
                dimension:       wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | extra,
                view_formats: &[],
            })
        };
        let depth_mips = storage_tex("GTAO Depth Mips", TextureFormat::R32Float, DEPTH_MIP_COUNT, wgpu::TextureUsages::empty());
        let edges      = storage_tex("GTAO Edges", TextureFormat::R32Uint, 1, wgpu::TextureUsages::empty());
        let noisy_ao   = storage_tex("GTAO Noisy AO", TextureFormat::R32Float, 1, wgpu::TextureUsages::empty());
        // COPY_SRC: the offscreen test harness reads the denoised target
        // back; harmless for the production frame path.
        let ao = storage_tex("GTAO AO", TextureFormat::R32Float, 1, wgpu::TextureUsages::COPY_SRC);

        let depth_mip_views = (0..DEPTH_MIP_COUNT)
            .map(|mip| {
                depth_mips.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level:  mip,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        Self {
            width, height,
            prepass_depth_view: prepass_depth.create_view(&Default::default()),
            depth_mip_views,
            depth_mips_view: depth_mips.create_view(&Default::default()),
            edges_view:      edges.create_view(&Default::default()),
            noisy_ao_view:   noisy_ao.create_view(&Default::default()),
            ao_view:         ao.create_view(&Default::default()),
            ao,
            _prepass_depth: prepass_depth,
            _depth_mips:    depth_mips,
            _edges:         edges,
            _noisy_ao:      noisy_ao,
        }
    }
}

/// A 1×1 always-white AO texture — the neutral fallback the scene bind group
/// points at whenever SSAO is disabled, so shade_pbr's `ao * ssao` term
/// reduces to plain material AO.
pub(crate) struct WhiteAo {
    _texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
}

impl WhiteAo {
    pub(crate) fn new(device: &Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("SSAO White Fallback"),
            size:            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          TextureFormat::R8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture:   &texture,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            &[255u8],
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(1), rows_per_image: Some(1) },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        let view = texture.create_view(&Default::default());
        Self { _texture: texture, view }
    }
}

/// Nearest sampler for the AO texture (real or white-fallback) — the AO
/// target is R32Float (non-filterable without an extra feature) and full-res,
/// so shading reads texels 1:1 anyway. Clamped so a screen UV rounding
/// slightly past [0,1] at the frame edge samples the edge texel rather than
/// wrapping.
pub(crate) fn create_ao_sampler(device: &Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label:          Some("SSAO Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter:     wgpu::FilterMode::Nearest,
        min_filter:     wgpu::FilterMode::Nearest,
        ..Default::default()
    })
}

/// The three GTAO compute pipelines (gtao.wgsl) and their per-target bind
/// groups: depth prefilter → GTAO main → spatial denoise.
pub(crate) struct GtaoPasses {
    prefilter: wgpu::ComputePipeline,
    main:      wgpu::ComputePipeline,
    denoise:   wgpu::ComputePipeline,
    prefilter_bgl: wgpu::BindGroupLayout,
    gtao_bgl:      wgpu::BindGroupLayout,
    denoise_bgl:   wgpu::BindGroupLayout,
    sampler:       wgpu::Sampler,
    hilbert_view:  wgpu::TextureView,
    _hilbert:      wgpu::Texture,
    bind_groups:   Option<GtaoBindGroups>,
}

struct GtaoBindGroups {
    prefilter: wgpu::BindGroup,
    main:      wgpu::BindGroup,
    denoise:   wgpu::BindGroup,
    width:     u32,
    height:    u32,
}

impl GtaoPasses {
    pub(crate) fn new(device: &Device, queue: &wgpu::Queue, camera_bgl: &BindGroupLayout) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("GTAO Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!(concat!(env!("OUT_DIR"), "/gtao.wgsl")).into()),
        });

        let storage_entry = |binding: u32, format: TextureFormat| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access:         wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        };
        let texture_entry = |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                multisampled:   false,
                view_dimension: wgpu::TextureViewDimension::D2,
                sample_type,
            },
            count: None,
        };

        let prefilter_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("GTAO Prefilter BGL"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Depth),
                storage_entry(1, TextureFormat::R32Float),
                storage_entry(2, TextureFormat::R32Float),
                storage_entry(3, TextureFormat::R32Float),
                storage_entry(4, TextureFormat::R32Float),
                storage_entry(5, TextureFormat::R32Float),
            ],
        });
        let gtao_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("GTAO Main BGL"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(1, wgpu::TextureSampleType::Uint),
                wgpu::BindGroupLayoutEntry {
                    binding:    2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count:      None,
                },
                storage_entry(3, TextureFormat::R32Float),
                storage_entry(4, TextureFormat::R32Uint),
            ],
        });
        let denoise_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("GTAO Denoise BGL"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(1, wgpu::TextureSampleType::Uint),
                storage_entry(2, TextureFormat::R32Float),
            ],
        });

        let compute = |label: &str, entry: &str, layouts: &[Option<&wgpu::BindGroupLayout>]| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label:              Some(label),
                bind_group_layouts: layouts,
                immediate_size:     0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label:       Some(label),
                layout:      Some(&layout),
                module:      &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache:       None,
            })
        };
        let prefilter = compute("GTAO Prefilter Pipeline", "prefilter_depth", &[Some(camera_bgl), Some(&prefilter_bgl)]);
        let main      = compute("GTAO Main Pipeline", "gtao", &[Some(camera_bgl), None, Some(&gtao_bgl)]);
        let denoise   = compute("GTAO Denoise Pipeline", "denoise", &[None, None, None, Some(&denoise_bgl)]);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:          Some("GTAO Point Clamp Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let (hilbert, hilbert_view) = create_hilbert_lut(device, queue);

        Self {
            prefilter, main, denoise,
            prefilter_bgl, gtao_bgl, denoise_bgl,
            sampler,
            hilbert_view,
            _hilbert:    hilbert,
            bind_groups: None,
        }
    }

    /// (Re)point the passes at the current `SsaoTargets` — call at init and
    /// on every resize, since every view they bind is rebuilt there too.
    pub(crate) fn set_target(&mut self, device: &Device, targets: &SsaoTargets) {
        let prefilter = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("GTAO Prefilter Bind Group"),
            layout:  &self.prefilter_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&targets.prepass_depth_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&targets.depth_mip_views[0]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&targets.depth_mip_views[1]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&targets.depth_mip_views[2]) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&targets.depth_mip_views[3]) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&targets.depth_mip_views[4]) },
            ],
        });
        let main = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("GTAO Main Bind Group"),
            layout:  &self.gtao_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&targets.depth_mips_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.hilbert_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&targets.noisy_ao_view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&targets.edges_view) },
            ],
        });
        let denoise = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("GTAO Denoise Bind Group"),
            layout:  &self.denoise_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&targets.noisy_ao_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&targets.edges_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&targets.ao_view) },
            ],
        });
        self.bind_groups = Some(GtaoBindGroups {
            prefilter, main, denoise,
            width:  targets.width,
            height: targets.height,
        });
    }

    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, camera_bind_group: &wgpu::BindGroup) {
        let Some(groups) = self.bind_groups.as_ref() else {
            log::error!("GtaoPasses::encode before set_target — frame dropped");
            return;
        };
        // The prefilter handles 2×2 source texels per invocation at
        // workgroup 8×8 (16×16 texels per group); the other two are 1:1.
        let (w, h) = (groups.width, groups.height);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("GTAO Prefilter Pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.prefilter);
            pass.set_bind_group(0, camera_bind_group, &[]);
            pass.set_bind_group(1, &groups.prefilter, &[]);
            pass.dispatch_workgroups(w.div_ceil(16), h.div_ceil(16), 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("GTAO Main Pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.main);
            pass.set_bind_group(0, camera_bind_group, &[]);
            pass.set_bind_group(2, &groups.main, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("GTAO Denoise Pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.denoise);
            pass.set_bind_group(3, &groups.denoise, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
    }
}

const HILBERT_WIDTH: u16 = 64;

/// The 64×64 Hilbert-curve index LUT the GTAO pass turns into a low-bias
/// per-pixel noise pair via the R2 sequence (XeGTAO's spatial noise).
fn create_hilbert_lut(device: &Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::TextureView) {
    let mut data = [0u16; (HILBERT_WIDTH as usize) * (HILBERT_WIDTH as usize)];
    for y in 0..HILBERT_WIDTH {
        for x in 0..HILBERT_WIDTH {
            data[y as usize * HILBERT_WIDTH as usize + x as usize] = hilbert_index(x, y);
        }
    }

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label:           Some("GTAO Hilbert LUT"),
        size:            wgpu::Extent3d { width: HILBERT_WIDTH as u32, height: HILBERT_WIDTH as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count:    1,
        dimension:       wgpu::TextureDimension::D2,
        format:          TextureFormat::R16Uint,
        usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats:    &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture:   &texture,
            mip_level: 0,
            origin:    wgpu::Origin3d::ZERO,
            aspect:    wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&data),
        wgpu::TexelCopyBufferLayout {
            offset:         0,
            bytes_per_row:  Some(HILBERT_WIDTH as u32 * 2),
            rows_per_image: Some(HILBERT_WIDTH as u32),
        },
        wgpu::Extent3d { width: HILBERT_WIDTH as u32, height: HILBERT_WIDTH as u32, depth_or_array_layers: 1 },
    );
    let view = texture.create_view(&Default::default());
    (texture, view)
}

// https://www.shadertoy.com/view/3tB3z3
fn hilbert_index(mut x: u16, mut y: u16) -> u16 {
    let mut index = 0;
    let mut level: u16 = HILBERT_WIDTH / 2;
    while level > 0 {
        let region_x = (x & level > 0) as u16;
        let region_y = (y & level > 0) as u16;
        index += level * level * ((3 * region_x) ^ region_y);

        if region_y == 0 {
            if region_x == 1 {
                x = HILBERT_WIDTH - 1 - x;
                y = HILBERT_WIDTH - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }

        level /= 2;
    }
    index
}
