// Headless offscreen render harness.
//
// Lets integration tests exercise the real scene pipelines (same WGSL, same
// pipeline factories, same HDR → MSAA-resolve → ACES-tonemap composition as
// RendererState) without a window or swapchain, then read pixels back and
// assert analytically (coverage %, darker-than, monotonic — never exact
// pixel values).
//
// Device requirements request TEXTURE_COMPRESSION_BC only when the adapter
// supports it (fallback adapters lack it), so BC-dependent tests must check
// `device.features().contains(...)` and skip rather than assume it's there.

use crate::camera::{self, Camera, CameraUniform, GpuPointLight, LightUniform, MAX_POINT_LIGHTS};
use crate::ibl::{Baker, Environment, EquirectImage};
use crate::instance::SdfInstance;
use crate::mesh::{self, MeshData};
use crate::mesh_pipeline::{self, MeshInstance};
use crate::mipgen::MipGenerator;
use crate::post::{TonemapPass, HDR_FORMAT, SCENE_SAMPLES};
use crate::sdf_pipeline::{self, INDICES};
use crate::shadow::{self, ShadowPipelines};
use crate::skinned_pipeline;
use crate::sky;
use crate::texture;
use glam::Vec3;
use wgpu::util::DeviceExt;

/// A GPU device with no surface attached. `None` when the machine has no
/// usable adapter (headless CI) — callers skip their test cleanly.
pub struct HeadlessGpu {
    pub device: wgpu::Device,
    pub queue:  wgpu::Queue,
}

impl HeadlessGpu {
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     None,
                force_fallback_adapter: false,
            },
        )).ok()?;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                required_features: adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC,
                ..Default::default()
            })
        ).ok()?;
        Some(Self { device, queue })
    }
}

/// An offscreen frame target mirroring the real chain: MSAA HDR color +
/// depth, single-sample HDR resolve, and the LDR output the tonemap pass
/// writes (readback-capable).
pub struct SceneTarget {
    pub width:  u32,
    pub height: u32,
    msaa_view:    wgpu::TextureView,
    depth_view:   wgpu::TextureView,
    resolve_view: wgpu::TextureView,
    output:       wgpu::Texture,
    output_view:  wgpu::TextureView,
}

impl SceneTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let tex = |label: &str, samples: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label:           Some(label),
                size,
                mip_level_count: 1,
                sample_count:    samples,
                dimension:       wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats:    &[],
            })
        };
        let msaa = tex("Offscreen MSAA", SCENE_SAMPLES, HDR_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let depth = tex(
            "Offscreen Depth", SCENE_SAMPLES, wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let resolve = tex(
            "Offscreen Resolve", 1, HDR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let output = tex(
            "Offscreen Output", 1, wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        Self {
            width,
            height,
            msaa_view:    msaa.create_view(&Default::default()),
            depth_view:   depth.create_view(&Default::default()),
            resolve_view: resolve.create_view(&Default::default()),
            output_view:  output.create_view(&Default::default()),
            output,
        }
    }
}

/// Directional light override for tests (defaults to the engine sun).
#[derive(Clone, Copy)]
pub struct TestLight {
    pub direction: Vec3,
    pub color:     Vec3,
    /// IBL ambient scale (1.0 = environment as authored).
    pub ambient:   f32,
}

/// Point light override for tests. `radius` is the distance at which
/// windowed inverse-square falloff reaches zero.
#[derive(Clone, Copy)]
pub struct TestPointLight {
    pub position:  Vec3,
    pub color:     Vec3,
    pub intensity: f32,
    pub radius:    f32,
}

/// The full offscreen scene renderer: real pipeline factories, a swappable
/// IBL environment, and the ACES tonemap — the same frame composition as
/// RendererState, minus the window.
pub struct OffscreenRenderer {
    pub gpu:        HeadlessGpu,
    aspect:         f32,
    camera_buffer:  wgpu::Buffer,
    material_bgl:   wgpu::BindGroupLayout,
    env_bgl:        wgpu::BindGroupLayout,
    sky_bgl:        wgpu::BindGroupLayout,
    sdf_pipeline:   wgpu::RenderPipeline,
    mesh_pipeline:  wgpu::RenderPipeline,
    mesh_transparent_pipeline: wgpu::RenderPipeline,
    sky_pipeline:   wgpu::RenderPipeline,
    tonemap:        TonemapPass,
    baker:          Baker,
    environment:    Environment,
    brdf_view:      wgpu::TextureView,
    mipgen:         MipGenerator,
    shadow_view:       wgpu::TextureView,
    shadow_pipelines:  ShadowPipelines,
    shadow_bind_group: wgpu::BindGroup,
    light_vp_buffer:   wgpu::Buffer,
    light_dir:         Vec3,
    _shadow_texture:   wgpu::Texture,
    // CPU copy of the full light uniform so set_light / set_fog /
    // set_point_lights can update their fields independently (the
    // RendererState pattern, state.rs:70-72).
    light_state:    LightUniform,
    light_buffer:   wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    vertex_buffer:  wgpu::Buffer,
    index_buffer:   wgpu::Buffer,
    white_bg:       wgpu::BindGroup,
    _white:         texture::ColorTexture,
    /// Sky pass on/off — off keeps the clear color as background so coverage
    /// assertions stay simple.
    pub draw_sky:   bool,
    /// World-space eye position, kept in lockstep with the GPU camera buffer
    /// so `render_mesh` can sort blend primitives back-to-front without a
    /// GPU readback.
    camera_eye:     Vec3,
}

impl OffscreenRenderer {
    /// `None` when no GPU adapter exists. Aspect is fixed per target at render
    /// time via the default orbit camera.
    pub fn new(aspect: f32) -> Option<Self> {
        let gpu = HeadlessGpu::new()?;
        let device = &gpu.device;

        let camera = Camera::new(aspect);
        let (shadow_texture, shadow_view) = shadow::create_shadow_texture(device);
        let (camera_buffer, light_buffer, light_vp_buffer, camera_bgl, camera_bind_group) =
            camera::create_gpu_resources(device, &camera, &shadow_view);

        let joint_bgl = skinned_pipeline::create_joint_bind_group_layout(device);
        let shadow_pipelines = ShadowPipelines::new(device, &joint_bgl);
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("Offscreen Shadow BG"),
            layout:  &shadow_pipelines.bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: light_vp_buffer.as_entire_binding(),
            }],
        });

        let texture_bgl  = sdf_pipeline::create_texture_bind_group_layout(device);
        let material_bgl = mesh_pipeline::create_material_bind_group_layout(device);
        let env_bgl      = crate::ibl::create_env_bind_group_layout(device);
        let sky_bgl      = sky::create_sky_bind_group_layout(device);
        let baker        = Baker::new(device);
        let brdf_view    = crate::ibl::bake_brdf_lut(device, &gpu.queue, &baker);
        let environment  = Environment::default_gray(device, &gpu.queue, &baker, &env_bgl, &sky_bgl, &brdf_view);
        let mipgen       = MipGenerator::new(device);

        let sdf_pipeline =
            sdf_pipeline::create_pipeline(device, HDR_FORMAT, &camera_bgl, &texture_bgl, &env_bgl);
        let mesh_pipeline =
            mesh_pipeline::create_mesh_pipeline(device, HDR_FORMAT, &camera_bgl, &material_bgl, &env_bgl, false);
        let mesh_transparent_pipeline =
            mesh_pipeline::create_mesh_pipeline(device, HDR_FORMAT, &camera_bgl, &material_bgl, &env_bgl, true);
        let sky_pipeline = sky::create_sky_pipeline(device, &camera_bgl, &sky_bgl);
        let mut tonemap = TonemapPass::new(device, wgpu::TextureFormat::Rgba8Unorm);
        tonemap.set_exposure(&gpu.queue, 1.0);

        let white    = texture::create_default_white(device, &gpu.queue);
        let white_bg = texture::create_bind_group(device, &texture_bgl, &white);
        let vertex_buffer = sdf_pipeline::create_vertex_buffer(device);
        let index_buffer  = sdf_pipeline::create_index_buffer(device);

        Some(Self {
            aspect,
            camera_buffer,
            vertex_buffer,
            index_buffer,
            shadow_view,
            shadow_pipelines,
            shadow_bind_group,
            light_vp_buffer,
            light_dir: Vec3::new(-1.0, 2.0, -1.0).normalize(),
            _shadow_texture: shadow_texture,
            light_state: LightUniform::default_sun(),
            material_bgl,
            env_bgl,
            sky_bgl,
            sdf_pipeline,
            mesh_pipeline,
            mesh_transparent_pipeline,
            sky_pipeline,
            tonemap,
            baker,
            environment,
            brdf_view,
            mipgen,
            light_buffer,
            camera_bind_group,
            white_bg,
            _white: white,
            draw_sky: false,
            camera_eye: camera.eye(),
            gpu,
        })
    }

    pub fn target(&self, width: u32, height: u32) -> SceneTarget {
        SceneTarget::new(&self.gpu.device, width, height)
    }

    /// Swap in a uniform-radiance environment (the white-furnace seam).
    pub fn set_uniform_environment(&mut self, rgb: [f32; 3]) {
        let pixels: Vec<f32> = (0..4 * 2).flat_map(|_| [rgb[0], rgb[1], rgb[2], 1.0]).collect();
        let image = EquirectImage::from_rgba_f32(4, 2, &pixels);
        self.environment = Environment::from_equirect_pixels(
            &self.gpu.device, &self.gpu.queue, &self.baker, &self.env_bgl, &self.sky_bgl, &self.brdf_view, &image,
        );
    }

    /// BRDF LUT bakes performed on the calling thread so far — proves
    /// repeated `Environment` construction (zone crossings) shares the LUT
    /// baked in `new` rather than rebaking it.
    pub fn brdf_bake_count() -> u32 {
        crate::ibl::brdf_bake_count()
    }

    /// `Baker` constructions (shader + pipeline compiles) performed on the
    /// calling thread so far — proves repeated `Environment` construction
    /// (zone crossings) reuses the baker built in `new` rather than
    /// recompiling its pipelines.
    pub fn baker_construction_count() -> u32 {
        crate::ibl::baker_construction_count()
    }

    /// Re-aims the camera to boresight elevation 0 (looking exactly at the
    /// horizon): the lower half of the frustum sees below-horizon rays, the
    /// upper half sees increasing elevation — the sky fog-blend test needs
    /// both, which the fixed default downward-pitched camera never produces.
    pub fn set_camera_level(&mut self) {
        let mut camera = Camera::new(self.aspect);
        camera.orbit(0.0, -0.8); // Camera::new's default pitch is 0.8; net pitch 0.0
        let cam_uniform = CameraUniform::from_camera(&camera);
        self.gpu.queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&[cam_uniform]));
        self.camera_eye = camera.eye();
    }

    /// Overrides distance-fog color/density only; direction/color/ambient/
    /// point lights stay whatever they last were (`default_sun` if `set_light`
    /// was never called).
    pub fn set_fog(&mut self, color: Vec3, density: f32) {
        self.light_state.fog_color = color.to_array();
        self.light_state.fog_density = density;
        self.gpu.queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&[self.light_state]));
    }

    /// Attenuates fog density above `height` by `exp(-falloff * max(y - height, 0))`;
    /// 0/0 reproduces pure distance fog.
    pub fn set_fog_height(&mut self, height: f32, falloff: f32) {
        self.light_state.fog_height = height;
        self.light_state.fog_height_falloff = falloff;
        self.gpu.queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&[self.light_state]));
    }

    pub fn set_light(&mut self, light: TestLight) {
        self.light_dir = light.direction.normalize();
        self.light_state.direction = self.light_dir.to_array();
        self.light_state.color     = light.color.to_array();
        self.light_state.ambient   = light.ambient;
        self.gpu.queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&[self.light_state]));
    }

    /// Sets up to `MAX_POINT_LIGHTS` point lights (extras truncated), leaving
    /// the sun/fog fields untouched.
    pub fn set_point_lights(&mut self, lights: &[TestPointLight]) {
        let count = lights.len().min(MAX_POINT_LIGHTS as usize);
        for (slot, light) in self.light_state.points.iter_mut().zip(lights.iter()) {
            *slot = GpuPointLight {
                position:  light.position.to_array(),
                radius:    light.radius,
                color:     light.color.to_array(),
                intensity: light.intensity,
            };
        }
        self.light_state.point_count = count as u32;
        self.gpu.queue.write_buffer(&self.light_buffer, 0, bytemuck::cast_slice(&[self.light_state]));
    }

    pub fn set_exposure(&mut self, exposure: f32) {
        self.tonemap.set_exposure(&self.gpu.queue, exposure);
    }

    /// Bloom composite strength; 0.0 disables bloom entirely.
    pub fn set_bloom_intensity(&mut self, intensity: f32) {
        self.tonemap.set_bloom_intensity(&self.gpu.queue, intensity);
    }

    /// Render SDF instances through the real primitive pipeline, then tonemap
    /// into the target's readable LDR output.
    pub fn render_sdf(&mut self, target: &SceneTarget, instances: &[SdfInstance], clear: wgpu::Color) {
        let instance_buffer = self.gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("Offscreen Instance Buffer"),
            contents: if instances.is_empty() { &[0u8; 96] } else { bytemuck::cast_slice(instances) },
            usage:    wgpu::BufferUsages::VERTEX,
        });
        self.compose(
            target,
            clear,
            |pass, this| {
                if !instances.is_empty() {
                    pass.set_pipeline(&this.shadow_pipelines.sdf);
                    pass.set_bind_group(0, &this.shadow_bind_group, &[]);
                    pass.set_vertex_buffer(0, this.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, instance_buffer.slice(..));
                    pass.set_index_buffer(this.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..INDICES.len() as u32, 0, 0..instances.len() as u32);
                }
            },
            |pass, this| {
                if !instances.is_empty() {
                    pass.set_pipeline(&this.sdf_pipeline);
                    pass.set_bind_group(0, &this.camera_bind_group, &[]);
                    pass.set_bind_group(1, &this.white_bg, &[]);
                    pass.set_bind_group(2, &this.environment.bind_group, &[]);
                    pass.set_vertex_buffer(0, this.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, instance_buffer.slice(..));
                    pass.set_index_buffer(this.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..INDICES.len() as u32, 0, 0..instances.len() as u32);
                }
            },
            |_pass, _this| {},
        );
    }

    /// Render static glTF mesh data through the real mesh pipeline (full PBR
    /// material bind groups, mipped textures), then tonemap.
    pub fn render_mesh(&mut self, target: &SceneTarget, data: MeshData, clear: wgpu::Color) {
        assert!(data.skeleton.is_none(), "render_mesh drives the static mesh path only");
        let gpu_mesh = mesh::upload_mesh(
            &self.gpu.device, &self.gpu.queue, &self.material_bgl, &self.mipgen, data,
        );
        let instance = MeshInstance {
            model: glam::Mat4::IDENTITY.to_cols_array_2d(),
            tint:  [1.0; 4],
        };
        let instance_buffer = self.gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("Offscreen Mesh Instance Buffer"),
            contents: bytemuck::cast_slice(&[instance]),
            usage:    wgpu::BufferUsages::VERTEX,
        });

        // Transparents don't cast shadows and don't draw in the opaque pass;
        // they draw back-to-front, after the sky, through their own pipeline.
        let opaque: Vec<usize> = gpu_mesh.primitives.iter().enumerate()
            .filter(|(_, p)| !p.blend)
            .map(|(i, _)| i)
            .collect();
        let mut blend: Vec<usize> = gpu_mesh.primitives.iter().enumerate()
            .filter(|(_, p)| p.blend)
            .map(|(i, _)| i)
            .collect();
        let eye = self.camera_eye;
        blend.sort_by(|&a, &b| {
            let da = eye.distance_squared(gpu_mesh.primitives[a].centroid());
            let db = eye.distance_squared(gpu_mesh.primitives[b].centroid());
            db.partial_cmp(&da).unwrap()
        });

        self.compose(
            target,
            clear,
            |pass, this| {
                pass.set_pipeline(&this.shadow_pipelines.mesh);
                pass.set_bind_group(0, &this.shadow_bind_group, &[]);
                pass.set_vertex_buffer(1, instance_buffer.slice(..));
                for &i in &opaque {
                    let prim = &gpu_mesh.primitives[i];
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, 0..1);
                }
            },
            |pass, this| {
                pass.set_pipeline(&this.mesh_pipeline);
                pass.set_bind_group(0, &this.camera_bind_group, &[]);
                pass.set_bind_group(2, &this.environment.bind_group, &[]);
                pass.set_vertex_buffer(1, instance_buffer.slice(..));
                for &i in &opaque {
                    let prim = &gpu_mesh.primitives[i];
                    pass.set_bind_group(1, &prim.material_bind_group, &[]);
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, 0..1);
                }
            },
            |pass, this| {
                pass.set_pipeline(&this.mesh_transparent_pipeline);
                pass.set_bind_group(0, &this.camera_bind_group, &[]);
                pass.set_bind_group(2, &this.environment.bind_group, &[]);
                pass.set_vertex_buffer(1, instance_buffer.slice(..));
                for &i in &blend {
                    let prim = &gpu_mesh.primitives[i];
                    pass.set_bind_group(1, &prim.material_bind_group, &[]);
                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                    pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..prim.index_count, 0, 0..1);
                }
            },
        );
    }

    /// Shared frame skeleton: shadow depth pre-pass, scene pass
    /// (MSAA→resolve, optional sky), then the ACES tonemap into the LDR
    /// output — the same composition as the real frame.
    fn compose(
        &mut self,
        target:           &SceneTarget,
        clear:            wgpu::Color,
        shadow_draw:      impl FnOnce(&mut wgpu::RenderPass<'_>, &Self),
        draw:             impl FnOnce(&mut wgpu::RenderPass<'_>, &Self),
        transparent_draw: impl FnOnce(&mut wgpu::RenderPass<'_>, &Self),
    ) {
        let bloom = crate::bloom::BloomPass::new(
            &self.gpu.device, &target.resolve_view, target.width, target.height,
        );
        bloom.set_exposure(&self.gpu.queue, self.tonemap.exposure());
        self.tonemap.set_source(&self.gpu.device, &target.resolve_view, &bloom.output_view);
        let light_vp = shadow::fit_light_vp(Vec3::ZERO, self.light_dir);
        self.gpu.queue.write_buffer(
            &self.light_vp_buffer, 0,
            bytemuck::cast_slice(&light_vp.to_cols_array()),
        );
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Offscreen Encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Offscreen Shadow Pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            shadow_draw(&mut pass, self);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Offscreen Main Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &target.msaa_view,
                    resolve_target: Some(&target.resolve_view),
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            draw(&mut pass, self);
            if self.draw_sky {
                pass.set_pipeline(&self.sky_pipeline);
                pass.set_bind_group(0, &self.camera_bind_group, &[]);
                pass.set_bind_group(1, &self.environment.sky_bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            transparent_draw(&mut pass, self);
        }
        bloom.encode(&mut encoder, None);
        self.tonemap.encode(&mut encoder, &target.output_view, None);
        self.gpu.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Read the target's tonemapped LDR output, rows unpadded.
    pub fn read(&self, target: &SceneTarget) -> Vec<u8> {
        read_texture_mip(&self.gpu, &target.output, 0)
    }
}

/// Upload RGBA8 pixels and build the full mip chain through the real blit
/// generator. Exposed so tests can assert on downsampled levels.
pub fn create_mipped_rgba8(
    gpu:    &HeadlessGpu,
    width:  u32,
    height: u32,
    pixels: &[u8],
    srgb:   bool,
) -> wgpu::Texture {
    let mipgen = MipGenerator::new(&gpu.device);
    texture::create_rgba_texture_mipped(&gpu.device, &gpu.queue, &mipgen, width, height, pixels, srgb)
        .texture
}

/// Read one mip level of a 4-bytes-per-pixel texture back to CPU memory, rows
/// unpadded (row-major). Blocks until the copy completes.
pub fn read_texture_mip(gpu: &HeadlessGpu, texture: &wgpu::Texture, mip: u32) -> Vec<u8> {
    const ROW_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT; // 256
    let width  = (texture.width() >> mip).max(1);
    let height = (texture.height() >> mip).max(1);
    let unpadded = width * 4;
    let padded   = unpadded.div_ceil(ROW_ALIGN) * ROW_ALIGN;

    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Readback Buffer"),
        size:               (padded * height) as u64,
        usage:              wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Readback Encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin:    wgpu::Origin3d::ZERO,
            aspect:    wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(std::iter::once(encoder.finish()));

    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("readback map failed"));
    gpu.device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
        .expect("device poll failed");

    let mapped = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((unpadded * height) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        pixels.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}
