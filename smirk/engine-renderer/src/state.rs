use std::mem::size_of;
use std::sync::{Arc, Mutex};
use winit::window::Window;
use engine_core::traits::Resources;
use engine_app::config::WindowConfig;
use engine_app::winit_processor::WinitEventProcessor;
use crate::bloom;
use crate::camera::{self, Camera, CameraUniform, LightUniform};
use crate::gpu_timer;
use crate::ibl;
use crate::instance::{InstancePool, SdfInstance};
use crate::menu::MenuState;
use crate::mesh::{MeshDrawList, MeshStore, SkinnedDrawList, SocketConfig, SocketTransforms};
use crate::mesh_pipeline;
use crate::mipgen;
use crate::particle_pipeline;
use crate::post;
use crate::sdf_pipeline;
use crate::shadow;
use crate::skinned_pipeline;
use crate::sky;
use crate::texture::{self, ColorTexture};
use crate::ParticleDrawList;
use glam::Vec3 as GlamVec3;

const MAX_INSTANCES: usize = 65_536;
pub(crate) const MAX_MESH_INSTANCES: usize = 16_384;

pub(crate) struct RendererState {
    pub(crate) surface:           wgpu::Surface<'static>,
    pub(crate) device:            wgpu::Device,
    pub(crate) queue:             wgpu::Queue,
    pub(crate) config:            wgpu::SurfaceConfiguration,
    pub(crate) pipeline:          wgpu::RenderPipeline,
    pub(crate) vertex_buffer:     wgpu::Buffer,
    pub(crate) index_buffer:      wgpu::Buffer,
    pub(crate) instance_buffer:   wgpu::Buffer,
    // ── meshes ──
    pub(crate) mesh_pipeline:        wgpu::RenderPipeline,
    pub(crate) mesh_instance_buffer: wgpu::Buffer,
    // ── skinned meshes ──
    pub(crate) skinned_pipeline:        wgpu::RenderPipeline,
    pub(crate) skinned_instance_buffer: wgpu::Buffer,
    pub(crate) joint_buffer:            wgpu::Buffer,
    pub(crate) joint_bind_group:        wgpu::BindGroup,
    // ── particles ──
    pub(crate) particle_additive:        wgpu::RenderPipeline,
    pub(crate) particle_alpha:           wgpu::RenderPipeline,
    pub(crate) particle_fx_bgl:          wgpu::BindGroupLayout,
    pub(crate) particle_fx_bind_group:   wgpu::BindGroup,
    pub(crate) particle_params_buffer:   wgpu::Buffer,
    pub(crate) particle_atlas:           texture::ColorTexture,
    pub(crate) particle_instance_buffer: wgpu::Buffer,
    pub(crate) camera:            Camera,
    pub(crate) camera_buffer:     wgpu::Buffer,
    pub(crate) light_buffer:      wgpu::Buffer,
    pub(crate) camera_bind_group: wgpu::BindGroup,
    // ── HDR + post ──
    pub(crate) hdr:     post::HdrTargets,
    pub(crate) tonemap: post::TonemapPass,
    pub(crate) bloom:   bloom::BloomPass,
    pub(crate) gpu_timer: Option<gpu_timer::GpuTimer>,
    // ── shadows ──
    pub(crate) shadow_view:       wgpu::TextureView,
    pub(crate) shadow_pipelines:  shadow::ShadowPipelines,
    pub(crate) shadow_bind_group: wgpu::BindGroup,
    pub(crate) light_vp_buffer:   wgpu::Buffer,
    pub(crate) light_dir:         GlamVec3,
    _shadow_texture: wgpu::Texture,
    // CPU copy of the full light uniform so set_light / set_fog can update
    // their halves independently.
    pub(crate) light_state: LightUniform,
    // ── IBL environment ──
    pub(crate) env_bgl:      wgpu::BindGroupLayout,
    pub(crate) sky_bgl:      wgpu::BindGroupLayout,
    pub(crate) sky_pipeline: wgpu::RenderPipeline,
    // Shader module + bake pipelines are a pure function of the device —
    // built once here and shared by every `Environment` load rather than
    // recompiled per zone.
    pub(crate) baker:        ibl::Baker,
    pub(crate) environment:  ibl::Environment,
    // BRDF LUT is a pure function of (NdotV, roughness) — baked once here
    // and shared by every `Environment` rather than rebaked per zone.
    pub(crate) brdf_view:    wgpu::TextureView,
    // ── textures ──
    pub(crate) texture_bgl:          wgpu::BindGroupLayout,
    pub(crate) material_bgl:         wgpu::BindGroupLayout,
    pub(crate) mipgen:               mipgen::MipGenerator,
    pub(crate) texture_store:        Vec<(ColorTexture, wgpu::BindGroup)>,
    pub(crate) active_texture_idx:   usize,
    // ── egui ──
    pub(crate) egui_ctx:      egui::Context,
    pub(crate) egui_winit:    Arc<Mutex<egui_winit::State>>,
    pub(crate) egui_renderer: egui_wgpu::Renderer,
}

impl RendererState {
    fn init(window: Arc<Window>, vsync: bool) -> (Self, InstancePool, Arc<Mutex<egui_winit::State>>) {
        let (surface, device, queue, config, format, size) = create_surface_and_device(&window, vsync);

        let (camera, shadow_texture, shadow_view, camera_buffer, light_buffer, light_vp_buffer, camera_bgl, camera_bind_group) =
            create_camera_and_shadow_view(&device, size);

        let (vertex_buffer, index_buffer, texture_bgl, material_bgl, mipgen, default_tex, default_bg) =
            create_geometry_and_texture_resources(&device, &queue);

        // HDR scene targets + post chain: every scene pipeline
        // renders MSAA into Rgba16Float; the tonemap pass owns the swapchain.
        let (env_bgl, sky_bgl, baker, environment, brdf_view, sky_pipeline, hdr, gpu_timer, bloom, tonemap) =
            create_hdr_and_ibl_resources(&device, &queue, &camera_bgl, format, size);

        let (scene_format, render_pipeline, mesh_render_pipeline) =
            create_scene_pipelines(&device, &camera_bgl, &texture_bgl, &material_bgl, &env_bgl);

        let (joint_bgl, skinned_render_pipeline) =
            create_skinned_pipeline_resources(&device, scene_format, &camera_bgl, &material_bgl, &env_bgl);

        // Particle pass resources: atlas + soft-fade depth + params.
        let (particle_fx_bgl, particle_additive, particle_alpha, particle_atlas, particle_params_buffer, particle_fx_bind_group) =
            create_particle_resources(&device, &queue, scene_format, &camera_bgl, &hdr.depth_view, size);

        // Depth-only shadow variants of the three geometry pipelines.
        let (shadow_pipelines, shadow_bind_group) =
            create_shadow_pass_resources(&device, &joint_bgl, &light_vp_buffer);

        let (instance_buffer, mesh_instance_buffer, skinned_instance_buffer, particle_instance_buffer, joint_buffer, joint_bind_group) =
            create_instance_buffers(&device, &joint_bgl);

        // ── egui ──────────────────────────────────────────────────────────────
        let (egui_ctx, egui_winit, egui_renderer) = create_egui_resources(&device, format, &window);

        // Index 0 in texture_store is always the default white texture.
        let texture_store = vec![(default_tex, default_bg)];

        (
            Self {
                surface, device, queue, config,
                pipeline: render_pipeline,
                vertex_buffer, index_buffer, instance_buffer,
                mesh_pipeline: mesh_render_pipeline,
                mesh_instance_buffer,
                skinned_pipeline: skinned_render_pipeline,
                skinned_instance_buffer,
                joint_buffer,
                joint_bind_group,
                particle_additive,
                particle_alpha,
                particle_fx_bgl,
                particle_fx_bind_group,
                particle_params_buffer,
                particle_atlas,
                particle_instance_buffer,
                camera, camera_buffer, light_buffer, camera_bind_group,
                hdr, tonemap, bloom, gpu_timer,
                shadow_view,
                shadow_pipelines,
                shadow_bind_group,
                light_vp_buffer,
                light_dir: GlamVec3::new(-1.0, 2.0, -1.0).normalize(),
                _shadow_texture: shadow_texture,
                light_state: LightUniform::default_sun(),
                env_bgl, sky_bgl, sky_pipeline, baker, environment, brdf_view,
                texture_bgl,
                material_bgl,
                mipgen,
                texture_store,
                active_texture_idx: 0,
                egui_ctx,
                egui_winit: egui_winit.clone(),
                egui_renderer,
            },
            InstancePool::new(MAX_INSTANCES),
            egui_winit,
        )
    }

    pub(crate) fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 { return; }
        self.config.width  = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.hdr = post::HdrTargets::new(&self.device, w, h);
        self.bloom = bloom::BloomPass::new(&self.device, &self.hdr.resolve_view, w, h);
        self.bloom.set_exposure(&self.queue, self.tonemap.exposure());
        self.tonemap.set_source(&self.device, &self.hdr.resolve_view, &self.bloom.output_view);
        // The particle pass samples the (re-created) scene depth.
        self.particle_fx_bind_group = create_particle_fx_bind_group(
            &self.device, &self.particle_fx_bgl, &self.particle_atlas,
            &self.hdr.depth_view, &self.particle_params_buffer,
        );
        self.queue.write_buffer(
            &self.particle_params_buffer, 0,
            bytemuck::cast_slice(&[w as f32, h as f32, 0.6, 0.0]),
        );
        self.camera.aspect = w as f32 / h as f32;
        let uniform = CameraUniform::from_camera(&self.camera);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }
}

fn create_surface_and_device(
    window: &Arc<Window>,
    vsync:  bool,
) -> (wgpu::Surface<'static>, wgpu::Device, wgpu::Queue, wgpu::SurfaceConfiguration, wgpu::TextureFormat, winit::dpi::PhysicalSize<u32>) {
    let instance = wgpu::Instance::default();

    // Arc<Window> yields Surface<'static> — no lifetime tied to the local borrow
    let surface = instance.create_surface(window.clone()).expect("failed to create surface");

    let adapter = pollster::block_on(instance.request_adapter(
        &wgpu::RequestAdapterOptions {
            power_preference:       wgpu::PowerPreference::HighPerformance,
            compatible_surface:     Some(&surface),
            force_fallback_adapter: false,
        },
    )).expect("no suitable GPU adapter found");

    let (device, queue) = pollster::block_on(
        adapter.request_device(&wgpu::DeviceDescriptor {
            // Timestamps are optional (dev-overlay GPU timing).
            required_features: wgpu::Features::TEXTURE_COMPRESSION_BC
                | (adapter.features() & wgpu::Features::TIMESTAMP_QUERY),
            ..Default::default()
        })
    ).expect("failed to acquire device (TEXTURE_COMPRESSION_BC required — desktop GPU needed)");

    let caps   = surface.get_capabilities(&adapter);
    let format = caps.formats.iter().copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);
    let size = window.inner_size();
    let config = wgpu::SurfaceConfiguration {
        usage:        wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width:        size.width,
        height:       size.height,
        present_mode: if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync },
        alpha_mode:   caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    (surface, device, queue, config, format, size)
}

fn create_camera_and_shadow_view(
    device: &wgpu::Device,
    size:   winit::dpi::PhysicalSize<u32>,
) -> (Camera, wgpu::Texture, wgpu::TextureView, wgpu::Buffer, wgpu::Buffer, wgpu::Buffer, wgpu::BindGroupLayout, wgpu::BindGroup) {
    let camera = Camera::new(size.width as f32 / size.height as f32);
    let (shadow_texture, shadow_view) = shadow::create_shadow_texture(device);
    let (camera_buffer, light_buffer, light_vp_buffer, camera_bgl, camera_bind_group) =
        camera::create_gpu_resources(device, &camera, &shadow_view);
    (camera, shadow_texture, shadow_view, camera_buffer, light_buffer, light_vp_buffer, camera_bgl, camera_bind_group)
}

fn create_geometry_and_texture_resources(
    device: &wgpu::Device,
    queue:  &wgpu::Queue,
) -> (wgpu::Buffer, wgpu::Buffer, wgpu::BindGroupLayout, wgpu::BindGroupLayout, mipgen::MipGenerator, ColorTexture, wgpu::BindGroup) {
    let vertex_buffer = sdf_pipeline::create_vertex_buffer(device);
    let index_buffer  = sdf_pipeline::create_index_buffer(device);
    let texture_bgl   = sdf_pipeline::create_texture_bind_group_layout(device);
    let material_bgl  = mesh_pipeline::create_material_bind_group_layout(device);
    let mipgen        = mipgen::MipGenerator::new(device);
    let default_tex   = texture::create_default_white(device, queue);
    let default_bg    = texture::create_bind_group(device, &texture_bgl, &default_tex);
    (vertex_buffer, index_buffer, texture_bgl, material_bgl, mipgen, default_tex, default_bg)
}

fn create_hdr_and_ibl_resources(
    device:     &wgpu::Device,
    queue:      &wgpu::Queue,
    camera_bgl: &wgpu::BindGroupLayout,
    format:     wgpu::TextureFormat,
    size:       winit::dpi::PhysicalSize<u32>,
) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout, ibl::Baker, ibl::Environment, wgpu::TextureView, wgpu::RenderPipeline, post::HdrTargets, Option<gpu_timer::GpuTimer>, bloom::BloomPass, post::TonemapPass) {
    let env_bgl = ibl::create_env_bind_group_layout(device);
    let sky_bgl = sky::create_sky_bind_group_layout(device);
    let baker = ibl::Baker::new(device);
    let brdf_view = ibl::bake_brdf_lut(device, queue, &baker);
    let environment = ibl::Environment::default_gray(device, queue, &baker, &env_bgl, &sky_bgl, &brdf_view);
    let sky_pipeline = sky::create_sky_pipeline(device, camera_bgl, &sky_bgl);
    let hdr = post::HdrTargets::new(device, size.width, size.height);
    let gpu_timer = gpu_timer::GpuTimer::new(device, queue);
    let bloom = bloom::BloomPass::new(device, &hdr.resolve_view, size.width, size.height);
    let mut tonemap = post::TonemapPass::new(device, format);
    tonemap.set_source(device, &hdr.resolve_view, &bloom.output_view);
    tonemap.set_exposure(queue, 1.0);
    (env_bgl, sky_bgl, baker, environment, brdf_view, sky_pipeline, hdr, gpu_timer, bloom, tonemap)
}

fn create_scene_pipelines(
    device:       &wgpu::Device,
    camera_bgl:   &wgpu::BindGroupLayout,
    texture_bgl:  &wgpu::BindGroupLayout,
    material_bgl: &wgpu::BindGroupLayout,
    env_bgl:      &wgpu::BindGroupLayout,
) -> (wgpu::TextureFormat, wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let scene_format = post::HDR_FORMAT;
    let render_pipeline =
        sdf_pipeline::create_pipeline(device, scene_format, camera_bgl, texture_bgl, env_bgl);
    let mesh_render_pipeline =
        mesh_pipeline::create_mesh_pipeline(device, scene_format, camera_bgl, material_bgl, env_bgl);
    (scene_format, render_pipeline, mesh_render_pipeline)
}

fn create_skinned_pipeline_resources(
    device:       &wgpu::Device,
    scene_format: wgpu::TextureFormat,
    camera_bgl:   &wgpu::BindGroupLayout,
    material_bgl: &wgpu::BindGroupLayout,
    env_bgl:      &wgpu::BindGroupLayout,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let joint_bgl = skinned_pipeline::create_joint_bind_group_layout(device);
    let skinned_render_pipeline = skinned_pipeline::create_skinned_pipeline(
        device, scene_format, camera_bgl, material_bgl, &joint_bgl, env_bgl,
    );
    (joint_bgl, skinned_render_pipeline)
}

fn create_particle_resources(
    device:      &wgpu::Device,
    queue:       &wgpu::Queue,
    scene_format: wgpu::TextureFormat,
    camera_bgl:  &wgpu::BindGroupLayout,
    depth_view:  &wgpu::TextureView,
    size:        winit::dpi::PhysicalSize<u32>,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline, wgpu::RenderPipeline, ColorTexture, wgpu::Buffer, wgpu::BindGroup) {
    let particle_fx_bgl = particle_pipeline::create_particle_fx_bind_group_layout(device);
    let (particle_additive, particle_alpha) = particle_pipeline::create_particle_pipelines(
        device, scene_format, camera_bgl, &particle_fx_bgl,
    );
    let particle_atlas = particle_pipeline::create_particle_atlas(device, queue);
    let particle_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Particle FX Params"),
        size:               16,
        usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let particle_fx_bind_group = create_particle_fx_bind_group(
        device, &particle_fx_bgl, &particle_atlas, depth_view, &particle_params_buffer,
    );
    queue.write_buffer(
        &particle_params_buffer, 0,
        bytemuck::cast_slice(&[size.width as f32, size.height as f32, 0.6, 0.0]),
    );
    (particle_fx_bgl, particle_additive, particle_alpha, particle_atlas, particle_params_buffer, particle_fx_bind_group)
}

fn create_shadow_pass_resources(
    device:          &wgpu::Device,
    joint_bgl:       &wgpu::BindGroupLayout,
    light_vp_buffer: &wgpu::Buffer,
) -> (shadow::ShadowPipelines, wgpu::BindGroup) {
    let shadow_pipelines = shadow::ShadowPipelines::new(device, joint_bgl);
    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:   Some("Shadow Cast Bind Group"),
        layout:  &shadow_pipelines.bgl,
        entries: &[wgpu::BindGroupEntry {
            binding:  0,
            resource: light_vp_buffer.as_entire_binding(),
        }],
    });
    (shadow_pipelines, shadow_bind_group)
}

fn create_instance_buffers(
    device:    &wgpu::Device,
    joint_bgl: &wgpu::BindGroupLayout,
) -> (wgpu::Buffer, wgpu::Buffer, wgpu::Buffer, wgpu::Buffer, wgpu::Buffer, wgpu::BindGroup) {
    let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Instance Buffer"),
        size:               (MAX_INSTANCES * size_of::<SdfInstance>()) as u64,
        usage:              wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mesh_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Mesh Instance Buffer"),
        size:               (MAX_MESH_INSTANCES * mesh_pipeline::MESH_INSTANCE_SIZE) as u64,
        usage:              wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let skinned_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Skinned Instance Buffer"),
        size:               (skinned_pipeline::MAX_SKINNED_INSTANCES * skinned_pipeline::SKINNED_INSTANCE_SIZE) as u64,
        usage:              wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let particle_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Particle Instance Buffer"),
        size:               (particle_pipeline::MAX_PARTICLES * particle_pipeline::PARTICLE_INSTANCE_SIZE) as u64,
        usage:              wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let joint_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label:              Some("Joint Palette Buffer"),
        size:               (skinned_pipeline::MAX_JOINT_MATRICES * size_of::<[[f32; 4]; 4]>()) as u64,
        usage:              wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let joint_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:   Some("Joint Palette Bind Group"),
        layout:  joint_bgl,
        entries: &[wgpu::BindGroupEntry {
            binding:  0,
            resource: joint_buffer.as_entire_binding(),
        }],
    });

    (instance_buffer, mesh_instance_buffer, skinned_instance_buffer, particle_instance_buffer, joint_buffer, joint_bind_group)
}

fn create_egui_resources(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    window: &Arc<Window>,
) -> (egui::Context, Arc<Mutex<egui_winit::State>>, egui_wgpu::Renderer) {
    let egui_ctx = egui::Context::default();
    // Apply dark/transparent theme for the menu.
    egui_ctx.set_visuals(egui::Visuals {
        panel_fill:            egui::Color32::TRANSPARENT,
        window_fill:           egui::Color32::TRANSPARENT,
        window_shadow:         egui::Shadow::NONE,
        window_stroke:         egui::Stroke::NONE,
        override_text_color:   Some(egui::Color32::WHITE),
        ..egui::Visuals::dark()
    });

    let native_ppp = Some(window.scale_factor() as f32);
    let egui_winit_state = egui_winit::State::new(
        egui_ctx.clone(),
        egui::ViewportId::ROOT,
        window.as_ref(),
        native_ppp,
        None,
        None,
    );
    let egui_winit = Arc::new(Mutex::new(egui_winit_state));

    let egui_renderer = egui_wgpu::Renderer::new(
        device, format, egui_wgpu::RendererOptions::default(),
    );

    (egui_ctx, egui_winit, egui_renderer)
}

fn create_particle_fx_bind_group(
    device:  &wgpu::Device,
    layout:  &wgpu::BindGroupLayout,
    atlas:   &texture::ColorTexture,
    depth:   &wgpu::TextureView,
    params:  &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:   Some("Particle FX Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&atlas.view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&atlas.sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
            wgpu::BindGroupEntry { binding: 3, resource: params.as_entire_binding() },
        ],
    })
}

pub(crate) fn init(window: &Arc<Window>, resources: &mut Resources) {
    let vsync = resources.get::<WindowConfig>().map(|c| c.vsync).unwrap_or(true);
    let (state, pool, egui_winit) = RendererState::init(window.clone(), vsync);

    // Register the egui winit event processor so app_loop can forward winit events.
    resources.insert(WinitEventProcessor::new(move |w, e| {
        egui_winit.lock().unwrap().on_window_event(w, e).consumed
    }));

    // Insert MenuState seeded from current config (if present).
    let menu_state = resources.get::<WindowConfig>()
        .map(MenuState::new)
        .unwrap_or_default();
    resources.insert(menu_state);

    resources.insert(state);
    resources.insert(pool);
    resources.insert(MeshStore::default());
    resources.insert(MeshDrawList::default());
    resources.insert(SkinnedDrawList::default());
    resources.insert(SocketConfig::default());
    resources.insert(SocketTransforms::default());
    resources.insert(ParticleDrawList::default());
}

pub(crate) fn on_resize(w: u32, h: u32, resources: &mut Resources) {
    resources.get_mut::<RendererState>()
        .expect("RendererState not in resources")
        .resize(w, h);
}
