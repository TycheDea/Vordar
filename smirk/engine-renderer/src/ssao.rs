// Depth prepass + SSAO: a full-res single-sample depth-only pass renders
// opaque geometry from the main camera, then a half-res pass reconstructs
// position/normal from that depth and darkens creases where nearby geometry
// blocks the hemisphere above a surface. shade_pbr (pbr_common.wgsl) samples
// the blurred result to scale IBL ambient only; `WhiteAo` is the neutral
// stand-in bound whenever SSAO is disabled.

use wgpu::{BindGroupLayout, Device, TextureFormat};

/// Sample radius (world-space metres) the hemisphere kernel reaches. Wider
/// than a textbook 0.5m default: occlusion strength falls off with distance
/// from the occluder much faster than the radius itself (few kernel
/// directions reach a far corner of the hemisphere), so a small radius
/// collapses the crease's strongly-occluded band to a sub-pixel strip with
/// nothing to average over at these render sizes (see the
/// `ssao_darkens_box_ground_contact_crease` offscreen test) — 3.0 keeps that
/// near-field band several AO-texels wide.
pub(crate) const SSAO_RADIUS: f32 = 3.0;
const SSAO_BIAS: f32 = 0.02;

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

/// The full-res prepass depth plus the half-res raw/blurred AO targets. AO
/// renders at half the prepass resolution; the blur denoises the raw pass's
/// per-pixel hash-rotated kernel noise.
pub(crate) struct SsaoTargets {
    pub(crate) width:      u32,
    pub(crate) height:     u32,
    pub(crate) ao_width:   u32,
    pub(crate) ao_height:  u32,
    pub(crate) prepass_depth_view: wgpu::TextureView,
    pub(crate) raw_ao_view:        wgpu::TextureView,
    pub(crate) blurred_ao_view:    wgpu::TextureView,
    /// Exposed for CPU readback (offscreen tests only — production frames
    /// never read this back).
    pub(crate) blurred_ao: wgpu::Texture,
    _prepass_depth: wgpu::Texture,
    _raw_ao:        wgpu::Texture,
}

impl SsaoTargets {
    pub(crate) fn new(device: &Device, width: u32, height: u32) -> Self {
        let width  = width.max(1);
        let height = height.max(1);
        let ao_width  = (width / 2).max(1);
        let ao_height = (height / 2).max(1);

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

        let ao_tex = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label:           Some(label),
                size:            wgpu::Extent3d { width: ao_width, height: ao_height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count:    1,
                dimension:       wgpu::TextureDimension::D2,
                format:          TextureFormat::R8Unorm,
                // COPY_SRC: the offscreen test harness reads the blurred
                // target back; harmless for the production frame path.
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let raw_ao     = ao_tex("SSAO Raw");
        let blurred_ao = ao_tex("SSAO Blurred");

        Self {
            width, height, ao_width, ao_height,
            prepass_depth_view: prepass_depth.create_view(&Default::default()),
            raw_ao_view:        raw_ao.create_view(&Default::default()),
            blurred_ao_view:    blurred_ao.create_view(&Default::default()),
            blurred_ao,
            _prepass_depth: prepass_depth,
            _raw_ao:        raw_ao,
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

/// Linear-filtering sampler for the AO texture (real or white-fallback),
/// clamped so a screen UV rounding slightly past [0,1] at the frame edge
/// samples the edge texel rather than wrapping.
pub(crate) fn create_ao_sampler(device: &Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label:          Some("SSAO Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter:     wgpu::FilterMode::Linear,
        min_filter:     wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SsaoParamsUniform {
    screen_size: [f32; 2],
    radius:      f32,
    bias:        f32,
}

/// Fullscreen hemisphere-kernel occlusion pass: samples the depth prepass,
/// writes raw (unblurred) AO.
pub(crate) struct SsaoPass {
    pipeline:      wgpu::RenderPipeline,
    bgl:           wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
    bind_group:    Option<wgpu::BindGroup>,
}

impl SsaoPass {
    pub(crate) fn new(device: &Device, camera_bgl: &BindGroupLayout, shader: &wgpu::ShaderModule) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("SSAO BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled:   false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type:    wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
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
            label:              Some("SSAO Pipeline Layout"),
            bind_group_layouts: &[Some(camera_bgl), Some(&bgl)],
            immediate_size:     0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("SSAO Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:      shader,
                entry_point: Some("vtx_main"),
                buffers:     &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive:     Default::default(),
            depth_stencil: None,
            multisample:   Default::default(),
            fragment: Some(wgpu::FragmentState {
                module:      shader,
                entry_point: Some("ssao_frag"),
                targets:     &[Some(wgpu::ColorTargetState {
                    format:     TextureFormat::R8Unorm,
                    blend:      None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache:          None,
        });
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("SSAO Params"),
            size:               std::mem::size_of::<SsaoParamsUniform>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { pipeline, bgl, params_buffer, bind_group: None }
    }

    /// (Re)point the pass at the current `SsaoTargets` — call at init and on
    /// every resize, since the depth view it binds is rebuilt there too.
    pub(crate) fn set_target(&mut self, device: &Device, queue: &wgpu::Queue, targets: &SsaoTargets) {
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[SsaoParamsUniform {
            screen_size: [targets.ao_width as f32, targets.ao_height as f32],
            radius:      SSAO_RADIUS,
            bias:        SSAO_BIAS,
        }]));
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("SSAO Bind Group"),
            layout:  &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&targets.prepass_depth_view) },
                wgpu::BindGroupEntry { binding: 1, resource: self.params_buffer.as_entire_binding() },
            ],
        }));
    }

    /// The params buffer — `BlurPass` shares it (both passes want the same
    /// screen-size/radius/bias uniform, per `ssao.wgsl`).
    pub(crate) fn params_buffer(&self) -> &wgpu::Buffer {
        &self.params_buffer
    }

    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, camera_bind_group: &wgpu::BindGroup, dst: &wgpu::TextureView) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            log::error!("SsaoPass::encode before set_target — frame dropped");
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           dst,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Fullscreen 3×3 box blur denoising `SsaoPass`'s raw output.
pub(crate) struct BlurPass {
    pipeline:   wgpu::RenderPipeline,
    bgl:        wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,
}

impl BlurPass {
    pub(crate) fn new(device: &Device, shader: &wgpu::ShaderModule) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("SSAO Blur BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled:   false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type:    wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
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
        // Group 0 is left unused: blur_frag never reaches `camera`/`light`.
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("SSAO Blur Pipeline Layout"),
            bind_group_layouts: &[None, Some(&bgl)],
            immediate_size:     0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("SSAO Blur Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module:      shader,
                entry_point: Some("vtx_main"),
                buffers:     &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive:     Default::default(),
            depth_stencil: None,
            multisample:   Default::default(),
            fragment: Some(wgpu::FragmentState {
                module:      shader,
                entry_point: Some("blur_frag"),
                targets:     &[Some(wgpu::ColorTargetState {
                    format:     TextureFormat::R8Unorm,
                    blend:      None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache:          None,
        });
        Self { pipeline, bgl, bind_group: None }
    }

    /// (Re)point the pass at the current raw AO view — call at init and on
    /// every resize. `params_buffer` is `SsaoPass::params_buffer()`, shared
    /// so both passes read the same screen size.
    pub(crate) fn set_target(&mut self, device: &Device, raw_ao_view: &wgpu::TextureView, params_buffer: &wgpu::Buffer) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("SSAO Blur Bind Group"),
            layout:  &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(raw_ao_view) },
                wgpu::BindGroupEntry { binding: 1, resource: params_buffer.as_entire_binding() },
            ],
        }));
    }

    pub(crate) fn encode(&self, encoder: &mut wgpu::CommandEncoder, dst: &wgpu::TextureView) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            log::error!("BlurPass::encode before set_target — frame dropped");
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Blur Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view:           dst,
                resolve_target: None,
                depth_slice:    None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Compiles `ssao.wgsl` once — `SsaoPass` and `BlurPass` share the module
/// (its two fragment entries), matching `ssao.wgsl`'s own doc comment.
pub(crate) fn create_shader(device: &Device) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label:  Some("SSAO Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!(concat!(env!("OUT_DIR"), "/ssao.wgsl")).into()),
    })
}
