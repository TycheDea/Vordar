pub mod anim;
pub(crate) mod bloom;
pub mod camera;
pub mod dev_overlay;
pub(crate) mod ibl;
pub mod instance;
pub mod menu;
pub mod mesh;
pub(crate) mod mesh_pipeline;
pub(crate) mod mipgen;
pub mod offscreen;
pub mod particle_pipeline;
pub(crate) mod post;
pub(crate) mod shadow;
pub mod tangent;
pub mod pipeline;
pub(crate) mod skinned_pipeline;
pub mod texture;
pub mod ui_layers;

pub use dev_overlay::DevOverlaySystem;
pub use menu::{MenuState, MenuSystem};
pub use mesh::{MeshDrawList, MeshRenderSyncSystem, MeshStore, SkinnedDrawList, SocketConfig, SocketTransforms};
pub use mesh_pipeline::MeshVertex;
pub use particle_pipeline::{ParticleInstance, MAX_PARTICLES};
pub use ui_layers::UiLayers;

use std::mem::size_of;
use std::sync::{Arc, Mutex};
use winit::window::Window;
use engine_core::traits::Resources;
use engine_core::World;
use engine_app::app::App;
use engine_app::plugin::Plugin;
use engine_app::config::WindowConfig;
use engine_app::scheduler::{InterpolationAlpha, Phase, System, SystemOrder};
use engine_app::input::KeyboardState;
use engine_app::winit_processor::WinitEventProcessor;
use engine_core::traits::DespawnQueue;
use engine_core::components::{PreviousTransform, RenderShape, RenderShapeType, ShapeGroup, Transform};
use glam::Mat4;
use winit::keyboard::KeyCode;
use crate::camera::{Camera, CameraUniform, LightUniform, ProjectionMode};
use crate::menu::{draw_menu, MenuAction, MenuScreen, SettingsDraft}; // SettingsDraft used in apply_pending
use glam::Vec3 as GlamVec3;
use crate::instance::{InstancePool, InstanceSlot, ShapeGroupSlots, SdfInstance, INSTANCE_SIZE};
use crate::pipeline::INDICES;
use texture::ColorTexture;

/// Opaque handle to a loaded GPU texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureHandle(usize);

/// Camera dolly limits — presentation tuning owned by the game. Insert your
/// own to override; absent, the defaults below apply.
pub struct CameraConfig {
    pub min_radius: f32,
    pub max_radius: f32,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self { min_radius: 16.0, max_radius: 55.0 }
    }
}

const MAX_INSTANCES: usize = 65_536;
const MAX_MESH_INSTANCES: usize = 16_384;

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
    pub(crate) particle_pipeline:        wgpu::RenderPipeline,
    pub(crate) particle_instance_buffer: wgpu::Buffer,
    pub(crate) camera:            Camera,
    pub(crate) camera_buffer:     wgpu::Buffer,
    pub(crate) light_buffer:      wgpu::Buffer,
    pub(crate) camera_bind_group: wgpu::BindGroup,
    // ── HDR + post (VQ-D1/D4) ──
    pub(crate) hdr:     post::HdrTargets,
    pub(crate) tonemap: post::TonemapPass,
    pub(crate) bloom:   bloom::BloomPass,
    // ── shadows (VQ-D3) ──
    pub(crate) shadow_view:       wgpu::TextureView,
    pub(crate) shadow_pipelines:  shadow::ShadowPipelines,
    pub(crate) shadow_bind_group: wgpu::BindGroup,
    pub(crate) light_vp_buffer:   wgpu::Buffer,
    pub(crate) light_dir:         GlamVec3,
    _shadow_texture: wgpu::Texture,
    // CPU copy of the full light uniform so set_light / set_fog can update
    // their halves independently.
    pub(crate) light_state: LightUniform,
    // ── IBL environment (VQ-D2) ──
    pub(crate) env_bgl:      wgpu::BindGroupLayout,
    pub(crate) sky_bgl:      wgpu::BindGroupLayout,
    pub(crate) sky_pipeline: wgpu::RenderPipeline,
    pub(crate) environment:  ibl::Environment,
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
                required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
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

        let camera = Camera::new(size.width as f32 / size.height as f32);
        let (shadow_texture, shadow_view) = shadow::create_shadow_texture(&device);
        let (camera_buffer, light_buffer, light_vp_buffer, camera_bgl, camera_bind_group) =
            camera::create_gpu_resources(&device, &camera, &shadow_view);

        let vertex_buffer   = pipeline::create_vertex_buffer(&device);
        let index_buffer    = pipeline::create_index_buffer(&device);
        let texture_bgl     = pipeline::create_texture_bind_group_layout(&device);
        let material_bgl    = mesh_pipeline::create_material_bind_group_layout(&device);
        let mipgen          = mipgen::MipGenerator::new(&device);
        let default_tex     = texture::create_default_white(&device, &queue);
        let default_bg      = texture::create_bind_group(&device, &texture_bgl, &default_tex);

        // HDR scene targets + post chain (VQ-D1/D4): every scene pipeline
        // renders MSAA into Rgba16Float; the tonemap pass owns the swapchain.
        let env_bgl = ibl::create_env_bind_group_layout(&device);
        let sky_bgl = post::create_sky_bind_group_layout(&device);
        let environment = ibl::Environment::default_gray(&device, &queue, &env_bgl, &sky_bgl);
        let sky_pipeline = post::create_sky_pipeline(&device, &camera_bgl, &sky_bgl);
        let hdr = post::HdrTargets::new(&device, size.width, size.height);
        let bloom = bloom::BloomPass::new(&device, &hdr.resolve_view, size.width, size.height);
        let mut tonemap = post::TonemapPass::new(&device, format);
        tonemap.set_source(&device, &hdr.resolve_view, &bloom.output_view);
        tonemap.set_exposure(&queue, 1.0);

        let scene_format = post::HDR_FORMAT;
        let render_pipeline =
            pipeline::create_pipeline(&device, scene_format, &camera_bgl, &texture_bgl, &env_bgl);
        let mesh_render_pipeline =
            mesh_pipeline::create_mesh_pipeline(&device, scene_format, &camera_bgl, &material_bgl, &env_bgl);

        let joint_bgl = skinned_pipeline::create_joint_bind_group_layout(&device);
        let skinned_render_pipeline = skinned_pipeline::create_skinned_pipeline(
            &device, scene_format, &camera_bgl, &material_bgl, &joint_bgl, &env_bgl,
        );
        let particle_render_pipeline =
            particle_pipeline::create_particle_pipeline(&device, scene_format, &camera_bgl);

        // Depth-only shadow variants of the three geometry pipelines (VQ-D3).
        let shadow_pipelines = shadow::ShadowPipelines::new(&device, &joint_bgl);
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("Shadow Cast Bind Group"),
            layout:  &shadow_pipelines.bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: light_vp_buffer.as_entire_binding(),
            }],
        });

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
            layout:  &joint_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding:  0,
                resource: joint_buffer.as_entire_binding(),
            }],
        });

        // ── egui ──────────────────────────────────────────────────────────────
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
            &device, format, egui_wgpu::RendererOptions::default(),
        );

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
                particle_pipeline: particle_render_pipeline,
                particle_instance_buffer,
                camera, camera_buffer, light_buffer, camera_bind_group,
                hdr, tonemap, bloom,
                shadow_view,
                shadow_pipelines,
                shadow_bind_group,
                light_vp_buffer,
                light_dir: GlamVec3::new(-1.0, 2.0, -1.0).normalize(),
                _shadow_texture: shadow_texture,
                light_state: LightUniform::default_sun(),
                env_bgl, sky_bgl, sky_pipeline, environment,
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
        self.tonemap.set_source(&self.device, &self.hdr.resolve_view, &self.bloom.output_view);
        self.camera.aspect = w as f32 / h as f32;
        let uniform = CameraUniform::from_camera(&self.camera);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }
}

/// Allocate a render slot for a new entity. Call from SpawnQueue callbacks.
pub fn alloc_render_slot(resources: &mut Resources) -> InstanceSlot {
    let pool = resources.get_mut::<InstancePool>().expect("InstancePool not in resources");
    InstanceSlot(pool.alloc())
}

/// Allocate N render slots for a ShapeGroup entity. `count` must equal `group.shapes.len()`.
pub fn alloc_shape_group_slots(count: usize, resources: &mut Resources) -> ShapeGroupSlots {
    let pool = resources.get_mut::<InstancePool>().expect("InstancePool not in resources");
    ShapeGroupSlots((0..count).map(|_| pool.alloc()).collect())
}

/// Free a render slot when an entity is despawned. Call from DespawnQueue hooks.
pub fn free_render_slot(slot: InstanceSlot, resources: &mut Resources) {
    let pool = resources.get_mut::<InstancePool>().expect("InstancePool not in resources");
    pool.free(slot.0);
}

/// Update camera target and/or orbit angle/pitch, then upload the uniform once.
/// Pass `target = None` to skip moving the target (orbit only).
pub fn update_camera(target: Option<GlamVec3>, yaw_delta: f32, pitch_delta: f32, resources: &mut Resources) {
    let state = resources
        .get_mut::<RendererState>()
        .expect("RendererState not in resources");
    if let Some(t) = target { state.camera.target = t; }
    state.camera.orbit(yaw_delta, pitch_delta);
    let uniform = CameraUniform::from_camera(&state.camera);
    state.queue.write_buffer(&state.camera_buffer, 0, bytemuck::cast_slice(&[uniform]));
}

/// Convenience wrapper — move target only, no orbit.
pub fn set_camera_target(target: GlamVec3, resources: &mut Resources) {
    update_camera(Some(target), 0.0, 0.0, resources);
}

/// Dolly the camera in (+delta) or out (−delta is toward the target), then
/// upload the uniform. Clamped to CameraConfig; works in every projection.
pub fn zoom_camera(delta: f32, resources: &mut Resources) {
    let (min_radius, max_radius) = resources
        .get::<CameraConfig>()
        .map(|c| (c.min_radius, c.max_radius))
        .unwrap_or_else(|| {
            let d = CameraConfig::default();
            (d.min_radius, d.max_radius)
        });
    let state = resources
        .get_mut::<RendererState>()
        .expect("RendererState not in resources");
    state.camera.zoom(delta, min_radius, max_radius);
    let uniform = CameraUniform::from_camera(&state.camera);
    state.queue.write_buffer(&state.camera_buffer, 0, bytemuck::cast_slice(&[uniform]));
}

/// Where a screen-pixel ray hits the ground plane (y = 0), in world space.
/// None when the renderer is absent, the cursor ray misses the plane, or the
/// hit would be behind the camera.
pub fn screen_to_ground(screen_px: (f32, f32), resources: &Resources) -> Option<GlamVec3> {
    let state = resources.get::<RendererState>()?;
    let (w, h) = (state.config.width as f32, state.config.height as f32);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let ndc = glam::Vec2::new(
        screen_px.0 / w * 2.0 - 1.0,
        1.0 - screen_px.1 / h * 2.0,
    );
    unproject_to_ground(state.camera.build_view_projection_matrix(), ndc)
}

/// Unproject an NDC point through inv(view_proj) (wgpu clip z ∈ [0,1]) and
/// intersect the resulting ray with the y = 0 plane. Pure — unit-tested.
pub fn unproject_to_ground(view_proj: Mat4, ndc: glam::Vec2) -> Option<GlamVec3> {
    let inv = view_proj.inverse();
    let p0 = inv.project_point3(GlamVec3::new(ndc.x, ndc.y, 0.0));
    let p1 = inv.project_point3(GlamVec3::new(ndc.x, ndc.y, 1.0));
    let d = p1 - p0;
    // Relative threshold: a near-horizontal ray "hits" the plane absurdly far
    // away through float noise alone — treat it as a miss.
    if d.y.abs() < 1e-4 * d.length() {
        return None;
    }
    let t = -p0.y / d.y;
    (t >= 0.0).then(|| p0 + d * t)
}

/// Load a Radiance .hdr equirect as the zone's environment: IBL ambient for
/// every lit pass and the visible sky (VQ-D2). Returns false (keeping the
/// previous environment) when the file is missing or invalid.
pub fn set_environment(path: &str, resources: &mut Resources) -> bool {
    // Headless / pre-window: nothing to do (same contract as the sync systems).
    let Some(state) = resources.get_mut::<RendererState>() else { return false };
    match ibl::Environment::from_hdr(&state.device, &state.queue, &state.env_bgl, &state.sky_bgl, path) {
        Ok(env) => {
            state.environment = env;
            true
        }
        Err(e) => {
            log::error!("set_environment failed: {e}");
            false
        }
    }
}

/// Exposure applied in the tonemap pass (VQ-D1). 1.0 is neutral; the
/// day/night system may drive it.
pub fn set_exposure(exposure: f32, resources: &mut Resources) {
    let Some(state) = resources.get_mut::<RendererState>() else { return };
    state.tonemap.set_exposure(&state.queue, exposure);
}

/// Override the directional light. dir is the world-space vector pointing TOWARD the light
/// (will be normalised here). color is RGB intensity. ambient scales the IBL
/// ambient term (1.0 = the environment as authored — the day/night seam).
pub fn set_light(dir: GlamVec3, color: GlamVec3, ambient: f32, resources: &mut Resources) {
    let state = resources.get_mut::<RendererState>()
        .expect("RendererState not in resources");
    let dir = dir.normalize();
    state.light_dir = dir; // shadow fitting reads the CPU copy
    state.light_state.direction = dir.to_array();
    state.light_state.color     = color.to_array();
    state.light_state.ambient   = ambient;
    state.queue.write_buffer(&state.light_buffer, 0, bytemuck::cast_slice(&[state.light_state]));
}

/// Upload procedurally-built mesh data (e.g. the zone ground) under a
/// synthetic asset key; entities reference it via `RenderMesh { asset: key }`.
/// Returns false headless (no renderer — nothing to draw into).
pub fn register_procedural_mesh(key: &str, data: mesh::MeshData, resources: &mut Resources) -> bool {
    let Some(mut store) = resources.get_mut::<MeshStore>().map(std::mem::take) else {
        return false;
    };
    let ok = match resources.get::<RendererState>() {
        Some(state) => {
            store.register(&state.device, &state.queue, &state.material_bgl, &state.mipgen, key, data);
            true
        }
        None => false,
    };
    resources.insert(store);
    ok
}

/// Distance fog for the current zone (VQ-A5): linear-space color, exponential
/// density per world unit (0.0 disables).
pub fn set_fog(color: GlamVec3, density: f32, resources: &mut Resources) {
    let Some(state) = resources.get_mut::<RendererState>() else { return };
    state.light_state.fog_color   = color.to_array();
    state.light_state.fog_density = density;
    state.queue.write_buffer(&state.light_buffer, 0, bytemuck::cast_slice(&[state.light_state]));
}

/// Create a procedural checkerboard texture without any asset files.
/// Useful for testing the texture pipeline immediately.
pub fn create_checker_texture(
    size:    u32,
    tile_size: u32,
    color_a: [u8; 4],
    color_b: [u8; 4],
    resources: &mut Resources,
) -> TextureHandle {
    let state = resources.get_mut::<RendererState>().expect("RendererState not in resources");
    let tex = texture::create_checker_texture(&state.device, &state.queue, size, tile_size, color_a, color_b);
    let bg  = texture::create_bind_group(&state.device, &state.texture_bgl, &tex);
    let idx = state.texture_store.len();
    state.texture_store.push((tex, bg));
    TextureHandle(idx)
}

/// Load a BC7 DDS texture from `path`. Returns a handle to use with `set_texture`.
/// Logs a warning and returns `None` if the file is missing or invalid.
pub fn load_texture(path: &str, resources: &mut Resources) -> Option<TextureHandle> {
    let state = resources.get_mut::<RendererState>().expect("RendererState not in resources");
    match texture::load_dds(&state.device, &state.queue, path) {
        Ok(tex) => {
            let bg  = texture::create_bind_group(&state.device, &state.texture_bgl, &tex);
            let idx = state.texture_store.len();
            state.texture_store.push((tex, bg));
            Some(TextureHandle(idx))
        }
        Err(e) => {
            log::warn!("load_texture failed: {e}");
            None
        }
    }
}

/// Set the active texture for the next render pass.
pub fn set_texture(handle: TextureHandle, resources: &mut Resources) {
    let state = resources.get_mut::<RendererState>().expect("RendererState not in resources");
    state.active_texture_idx = handle.0;
}

/// Reset to the default white texture (instance colors render unaffected).
pub fn clear_texture(resources: &mut Resources) {
    let state = resources.get_mut::<RendererState>().expect("RendererState not in resources");
    state.active_texture_idx = 0;
}

/// Returns the camera's current yaw angle (radians). Used by movement systems to
/// align player input with the camera-facing direction.
/// Returns 0.0 if RendererState is not yet initialised.
pub fn camera_yaw(resources: &Resources) -> f32 {
    resources
        .get::<RendererState>()
        .map(|s| s.camera.angle)
        .unwrap_or(0.0)
}

/// Returns `(forward, right)` world-space XZ vectors for WASD input, accounting for
/// the current camera projection mode and yaw.
/// In TopDown mode the up vector is NEG_Z so the axes are fixed: W = -Z, D = +X.
pub fn camera_movement_axes(resources: &Resources) -> (GlamVec3, GlamVec3) {
    let state = match resources.get::<RendererState>() {
        Some(s) => s,
        None => return (GlamVec3::NEG_Z, GlamVec3::X),
    };
    match state.camera.mode {
        ProjectionMode::TopDown => (GlamVec3::NEG_Z, GlamVec3::X),
        _ => {
            let yaw = state.camera.angle;
            let forward = GlamVec3::new(-yaw.cos(), 0.0, -yaw.sin());
            let right   = GlamVec3::new( yaw.sin(), 0.0, -yaw.cos());
            (forward, right)
        }
    }
}

pub fn init(window: &Arc<Window>, resources: &mut Resources) {
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

/// World-space particle instances for this display frame. A game-side system
/// (the client's particle sim) rebuilds `instances` every frame in
/// Phase::RenderSync; RenderSystem uploads and draws them additively after the
/// opaque passes. Anything past MAX_PARTICLES is ignored.
#[derive(Default)]
pub struct ParticleDrawList {
    pub instances: Vec<ParticleInstance>,
}

pub fn on_resize(w: u32, h: u32, resources: &mut Resources) {
    resources.get_mut::<RendererState>()
        .expect("RendererState not in resources")
        .resize(w, h);
}

// ── Systems ───────────────────────────────────────────────────────────────────

/// Saves each entity's current position into PreviousTransform at the start of
/// every fixed step. Register in Phase::Update, SystemOrder::First so it runs
/// before any movement system mutates Transform.
pub struct SaveTransformSystem;

impl System for SaveTransformSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, _delta: f32) {
        for (transform, prev) in world.query::<(&Transform, &mut PreviousTransform)>().iter() {
            prev.position = transform.position;
        }
    }
}

pub struct RenderSyncSystem;

impl System for RenderSyncSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let alpha = resources.get::<InterpolationAlpha>().map(|a| a.0).unwrap_or(1.0);
        let pool = resources.get_mut::<InstancePool>()
            .expect("InstancePool not in resources");

        for (transform, prev, render_shape, slot) in
            world.query::<(&Transform, Option<&PreviousTransform>, &RenderShape, &InstanceSlot)>().iter()
        {
            // Lerp position if PreviousTransform is present; otherwise use current.
            let render_pos = match prev {
                Some(p) => p.position.lerp(transform.position, alpha),
                None    => transform.position,
            };
            let render_transform = Transform {
                position: render_pos,
                rotation: transform.rotation,
                scale:    transform.scale,
            };
            let (shape_type, shape_params) = shape_to_gpu(render_shape.shape);
            let new_inst = SdfInstance {
                model:       render_transform.to_model_matrix().to_cols_array_2d(),
                color:       render_shape.color.to_array(),
                shape_type,
                shape_params,
            };
            if bytemuck::bytes_of(&pool.slots[slot.0]) != bytemuck::bytes_of(&new_inst) {
                pool.slots[slot.0] = new_inst;
                pool.dirty[slot.0] = true;
            }
        }

        for (transform, prev, group, slots) in
            world.query::<(&Transform, Option<&PreviousTransform>, &ShapeGroup, &ShapeGroupSlots)>().iter()
        {
            let render_pos = match prev {
                Some(p) => p.position.lerp(transform.position, alpha),
                None    => transform.position,
            };
            let parent_model = Transform {
                position: render_pos,
                rotation: transform.rotation,
                scale:    transform.scale,
            }.to_model_matrix();

            for (sub, key) in group.shapes.iter().zip(slots.0.iter()) {
                let sub_model = parent_model
                    * Mat4::from_scale_rotation_translation(sub.scale, sub.rotation, sub.offset);
                let (shape_type, shape_params) = shape_to_gpu(sub.shape);
                let new_inst = SdfInstance {
                    model:       sub_model.to_cols_array_2d(),
                    color:       sub.color.to_array(),
                    shape_type,
                    shape_params,
                };
                if bytemuck::bytes_of(&pool.slots[*key]) != bytemuck::bytes_of(&new_inst) {
                    pool.slots[*key] = new_inst;
                    pool.dirty[*key] = true;
                }
            }
        }
    }
}

pub struct RenderSystem {
    // TODO: replace gpu_buf (full-copy buffer) with two scratch fields:
    //   gpu_buf:      Vec<SdfInstance>  — dirty instance data only (reused across frames)
    //   dirty_ranges: Vec<(u64, usize)> — (byte_offset_in_buffer, instance_count) per dirty range
    //
    //   These are pre-allocated scratch — clear + fill each frame, never reallocate at steady state.
    //   Kotlin analogue: two reusable ArrayList fields on the system class, cleared each frame.
    gpu_buf:      Vec<SdfInstance>,
    dirty_ranges: Vec<(u64, usize)>,
    /// Deferred actions collected from egui during the last frame.
    pending_menu: Vec<MenuAction>,
}

impl RenderSystem {
    pub fn new() -> Self {
        // TODO: init dirty_ranges: Vec::new() alongside gpu_buf and pending_menu.
        Self { gpu_buf: Vec::new(), dirty_ranges: Vec::new(), pending_menu: Vec::new() }
    }
}

impl System for RenderSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        // ── Apply deferred menu actions from the last frame ───────────────────
        self.apply_pending_menu_actions(resources);

        // ── Collect dirty ranges ──────────────────────────────────────────────
        self.gpu_buf.clear();
        self.dirty_ranges.clear();
        let slot_count = {
            let pool = resources.get::<InstancePool>().expect("InstancePool not in resources");
            let mut i = 0;
            while i < pool.slots.len() {
                if pool.dirty[i] {
                    let start = i;
                    while i < pool.slots.len() && pool.dirty[i] { i += 1; }
                    self.dirty_ranges.push((start as u64 * INSTANCE_SIZE as u64, i - start));
                    self.gpu_buf.extend_from_slice(&pool.slots[start..i]);
                } else {
                    i += 1;
                }
            }
            pool.slots.len()
        };
        {
            let pool = resources.get_mut::<InstancePool>().expect("InstancePool not in resources");
            pool.dirty.iter_mut().for_each(|d| *d = false);
        }

        // ── Snapshot lightweight state for egui draw (before mut borrow) ─────
        let window       = resources.get::<Arc<Window>>().cloned();
        let menu_snap    = resources.get::<MenuState>().cloned();
        let dev_lines    = resources.get::<engine_app::dev_stats::DevStats>()
            .filter(|s| s.open)
            .map(|s| s.display_lines());
        let monitor_fps  = window.as_ref()
            .and_then(|w| w.current_monitor())
            .and_then(|m| m.refresh_rate_millihertz())
            .map(|mhz| (mhz / 1000).max(30));

        // ── Egui frame: engine UI + game-registered UiLayers ─────────────────
        // Runs against Arc-clone handles BEFORE the RendererState mut borrow,
        // so game layers get read access to Resources.
        let egui_frame = if let Some(ref w) = window {
            let (egui_ctx, egui_winit) = {
                let s = resources.get::<RendererState>()
                    .expect("RendererState not in resources");
                (s.egui_ctx.clone(), s.egui_winit.clone())
            };
            let raw_input = egui_winit.lock().unwrap().take_egui_input(w);
            let mut menu_actions: Vec<MenuAction> = Vec::new();

            // begin_pass/end_pass, NOT run_ui: run_ui wraps the frame in a
            // full-screen background Ui and allocates it as a central panel,
            // which makes egui claim the whole viewport — egui-winit then
            // consumes every unrelated click and wheel event (game input
            // died unless another button was already held).
            egui_ctx.begin_pass(raw_input);
            if let Some(ref lines) = dev_lines {
                dev_overlay::draw_dev_overlay(&egui_ctx, lines);
            }
            if let Some(m) = menu_snap.as_ref() {
                if m.open {
                    draw_menu(&egui_ctx, m, monitor_fps, &mut menu_actions);
                }
            }
            // Game UI layers (minimap, action bar, ...). Taken out so the
            // callbacks can read Resources while the registry is borrowed.
            let mut layers = resources.get_mut::<UiLayers>()
                .map(std::mem::take)
                .unwrap_or_default();
            for layer in layers.layers.iter_mut() {
                layer(&egui_ctx, resources);
            }
            if let Some(slot) = resources.get_mut::<UiLayers>() {
                *slot = layers;
            }
            let full_output = egui_ctx.end_pass();

            // Handle platform output (clipboard, cursor, etc.)
            egui_winit.lock().unwrap()
                .handle_platform_output(w, full_output.platform_output.clone());

            let ppp = full_output.pixels_per_point;
            let prims = egui_ctx.tessellate(full_output.shapes.clone(), ppp);
            self.pending_menu = menu_actions;
            Some((full_output, prims, ppp))
        } else {
            None
        };

        // Mesh draw lists + store, taken out so they outlive the RendererState
        // borrow below (returned at the end of the frame).
        let mesh_list     = resources.get_mut::<MeshDrawList>().map(std::mem::take);
        let skinned_list  = resources.get_mut::<SkinnedDrawList>().map(std::mem::take);
        let mesh_store    = resources.get_mut::<MeshStore>().map(std::mem::take);
        let particle_list = resources.get_mut::<ParticleDrawList>().map(std::mem::take);

        // ── All GPU work inside one mutable borrow of RendererState ───────────
        let state = resources.get_mut::<RendererState>()
            .expect("RendererState not in resources");

        let mut buf_pos = 0usize;
        for &(offset, count) in &self.dirty_ranges {
            let data = bytemuck::cast_slice(&self.gpu_buf[buf_pos..buf_pos + count]);
            state.queue.write_buffer(&state.instance_buffer, offset, data);
            buf_pos += count;
        }

        if let Some(list) = mesh_list.as_ref().filter(|l| !l.instances.is_empty()) {
            let n = list.instances.len().min(MAX_MESH_INSTANCES);
            state.queue.write_buffer(
                &state.mesh_instance_buffer, 0,
                bytemuck::cast_slice(&list.instances[..n]),
            );
        }

        // Skinned instances + joint palette.
        if let Some(list) = skinned_list.as_ref().filter(|l| !l.instances.is_empty()) {
            state.queue.write_buffer(
                &state.skinned_instance_buffer, 0,
                bytemuck::cast_slice(&list.instances),
            );
            if !list.joints.is_empty() {
                state.queue.write_buffer(
                    &state.joint_buffer, 0,
                    bytemuck::cast_slice(&list.joints),
                );
            }
        }

        // Particles.
        let particle_count = particle_list
            .as_ref()
            .map(|l| l.instances.len().min(particle_pipeline::MAX_PARTICLES))
            .unwrap_or(0);
        if particle_count > 0 {
            let list = particle_list.as_ref().expect("count > 0");
            state.queue.write_buffer(
                &state.particle_instance_buffer, 0,
                bytemuck::cast_slice(&list.instances[..particle_count]),
            );
        }

        // wgpu 29: get_current_texture() returns CurrentSurfaceTexture enum
        let surface_texture = match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                state.resize(state.config.width, state.config.height);
                restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
                return;
            }
        };

        let view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // ── Egui GPU upload ───────────────────────────────────────────────────
        let (egui_output, egui_primitives, egui_screen) = if let Some((full_output, prims, ppp)) = egui_frame {
            let sd = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [state.config.width, state.config.height],
                pixels_per_point: ppp,
            };
            // Upload new egui textures
            for (id, delta) in &full_output.textures_delta.set {
                state.egui_renderer.update_texture(&state.device, &state.queue, *id, delta);
            }
            (Some(full_output), Some(prims), Some(sd))
        } else {
            (None, None, None)
        };

        // ── Build command encoder ─────────────────────────────────────────────
        let mut encoder = state.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("Render Encoder") }
        );

        // Shadow pre-pass (VQ-D3): fit the sun's ortho volume around the
        // camera target (texel-snapped) and render depth-only variants of
        // every opaque draw. Particles don't cast.
        {
            let light_vp = shadow::fit_light_vp(state.camera.target, state.light_dir);
            state.queue.write_buffer(
                &state.light_vp_buffer, 0,
                bytemuck::cast_slice(&light_vp.to_cols_array()),
            );

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shadow Pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &state.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });

            // SDF primitives.
            pass.set_pipeline(&state.shadow_pipelines.sdf);
            pass.set_bind_group(0, &state.shadow_bind_group, &[]);
            pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, state.instance_buffer.slice(..));
            pass.set_index_buffer(state.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..INDICES.len() as u32, 0, 0..slot_count as u32);

            // Static meshes.
            if let (Some(list), Some(store)) = (mesh_list.as_ref(), mesh_store.as_ref()) {
                if !list.instances.is_empty() {
                    pass.set_pipeline(&state.shadow_pipelines.mesh);
                    pass.set_vertex_buffer(1, state.mesh_instance_buffer.slice(..));
                    for &(mesh_idx, first, count) in &list.ranges {
                        if first as usize >= MAX_MESH_INSTANCES { break; }
                        let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                        let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                        for prim in &gpu_mesh.primitives {
                            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                        }
                    }
                }
            }

            // Skinned meshes (re-binds the shared joint palette).
            if let (Some(list), Some(store)) = (skinned_list.as_ref(), mesh_store.as_ref()) {
                if !list.instances.is_empty() {
                    pass.set_pipeline(&state.shadow_pipelines.skinned);
                    pass.set_bind_group(1, &state.joint_bind_group, &[]);
                    pass.set_vertex_buffer(1, state.skinned_instance_buffer.slice(..));
                    for &(mesh_idx, first, count) in &list.ranges {
                        let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                        for prim in &gpu_mesh.primitives {
                            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                        }
                    }
                }
            }
        }

        // Main 3D pass — MSAA HDR, resolved for the tonemap pass (VQ-D1/D4).
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &state.hdr.msaa_view,
                    resolve_target: Some(&state.hdr.resolve_view),
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard, // resolve target keeps the frame
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &state.hdr.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let tex_bg = &state.texture_store[state.active_texture_idx].1;
            pass.set_pipeline(&state.pipeline);
            pass.set_bind_group(0, &state.camera_bind_group, &[]);
            pass.set_bind_group(1, tex_bg, &[]);
            pass.set_bind_group(2, &state.environment.bind_group, &[]);
            pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, state.instance_buffer.slice(..));
            pass.set_index_buffer(state.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..INDICES.len() as u32, 0, 0..slot_count as u32);

            // Mesh pass — same render pass and camera bind group, real geometry.
            // Ranges are sorted by first-instance, so overflow past the buffer
            // cap ends the loop rather than wrapping.
            if let (Some(list), Some(store)) = (mesh_list.as_ref(), mesh_store.as_ref()) {
                if !list.instances.is_empty() {
                    pass.set_pipeline(&state.mesh_pipeline);
                    pass.set_bind_group(2, &state.environment.bind_group, &[]);
                    pass.set_vertex_buffer(1, state.mesh_instance_buffer.slice(..));
                    for &(mesh_idx, first, count) in &list.ranges {
                        if first as usize >= MAX_MESH_INSTANCES { break; }
                        let count = count.min(MAX_MESH_INSTANCES as u32 - first);
                        let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                        for prim in &gpu_mesh.primitives {
                            pass.set_bind_group(1, &prim.material_bind_group, &[]);
                            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                        }
                    }
                }
            }

            // Skinned mesh pass — same camera bind group, plus the joint
            // palette (group 2). Instances index their own joint block via the
            // joint_base instance attribute, so one draw per mesh still works.
            if let (Some(list), Some(store)) = (skinned_list.as_ref(), mesh_store.as_ref()) {
                if !list.instances.is_empty() {
                    pass.set_pipeline(&state.skinned_pipeline);
                    pass.set_bind_group(0, &state.camera_bind_group, &[]);
                    pass.set_bind_group(2, &state.joint_bind_group, &[]);
                    pass.set_bind_group(3, &state.environment.bind_group, &[]);
                    pass.set_vertex_buffer(1, state.skinned_instance_buffer.slice(..));
                    for &(mesh_idx, first, count) in &list.ranges {
                        let Some(gpu_mesh) = store.meshes.get(mesh_idx) else { continue };
                        for prim in &gpu_mesh.primitives {
                            pass.set_bind_group(1, &prim.material_bind_group, &[]);
                            pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                            pass.set_index_buffer(prim.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                            pass.draw_indexed(0..prim.index_count, 0, first..first + count);
                        }
                    }
                }
            }

            // Sky pass — the IBL cubemap as background, pinned to the far
            // plane behind everything opaque (VQ-D2).
            pass.set_pipeline(&state.sky_pipeline);
            pass.set_bind_group(0, &state.camera_bind_group, &[]);
            pass.set_bind_group(1, &state.environment.sky_bind_group, &[]);
            pass.draw(0..3, 0..1);

            // Particle pass — additive billboards, drawn last so the read-only
            // depth test sees all opaque geometry and they blend over the sky.
            if particle_count > 0 {
                pass.set_pipeline(&state.particle_pipeline);
                pass.set_bind_group(0, &state.camera_bind_group, &[]);
                pass.set_vertex_buffer(0, state.particle_instance_buffer.slice(..));
                pass.draw(0..4, 0..particle_count as u32);
            }
        }

        // Bloom chain from the HDR resolve, then tonemap (ACES + exposure +
        // bloom composite) onto the swapchain.
        state.bloom.encode(&mut encoder);
        state.tonemap.encode(&mut encoder, &view);

        // Egui overlay pass (Load existing pixels — don't clear the 3D scene)
        if let (Some(prims), Some(sd)) = (egui_primitives.as_ref(), egui_screen.as_ref()) {
            state.egui_renderer.update_buffers(
                &state.device, &state.queue, &mut encoder, prims, sd,
            );
            let rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &view,
                    resolve_target: None,
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Load, // preserve 3D scene below
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            let mut rpass_static = rpass.forget_lifetime();
            state.egui_renderer.render(&mut rpass_static, prims, sd);
        }

        // Free textures, submit, present
        if let Some(ref fo) = egui_output {
            for id in &fo.textures_delta.free {
                state.egui_renderer.free_texture(id);
            }
        }
        state.queue.submit(std::iter::once(encoder.finish()));
        surface_texture.present();

        restore_mesh_resources(resources, mesh_list, skinned_list, mesh_store, particle_list);
    }
}

/// Return the taken mesh draw lists/store to Resources — called on every exit
/// path of RenderSystem::run so loaded meshes survive skipped frames.
fn restore_mesh_resources(
    resources: &mut Resources,
    list:      Option<MeshDrawList>,
    skinned:   Option<SkinnedDrawList>,
    store:     Option<MeshStore>,
    particles: Option<ParticleDrawList>,
) {
    if let Some(l) = list      { resources.insert(l); }
    if let Some(s) = skinned   { resources.insert(s); }
    if let Some(s) = store     { resources.insert(s); }
    if let Some(p) = particles { resources.insert(p); }
}

impl RenderSystem {
    fn apply_pending_menu_actions(&mut self, resources: &mut Resources) {
        use engine_app::config::{Resolution, WindowMode};
        use std::sync::Arc;
        use winit::window::Window;

        // Collect UpdateDraft actions — only keep the last one (multiple can arrive per frame).
        let mut latest_draft: Option<SettingsDraft> = None;
        let mut other_actions: Vec<MenuAction> = Vec::new();
        for action in self.pending_menu.drain(..) {
            match action {
                MenuAction::UpdateDraft(d) => { latest_draft = Some(d); }
                other => other_actions.push(other),
            }
        }

        // Apply latest draft (from egui widget mutations)
        if let Some(d) = latest_draft {
            if let Some(m) = resources.get_mut::<MenuState>() {
                m.draft = d;
            }
        }

        for action in other_actions {
            match action {
                MenuAction::Resume => {
                    if let Some(m) = resources.get_mut::<MenuState>() { m.open = false; }
                }
                MenuAction::OpenSettings => {
                    let draft = resources.get::<WindowConfig>().map(SettingsDraft::from_config);
                    if let Some(m) = resources.get_mut::<MenuState>() {
                        if let Some(d) = draft { m.draft = d; }
                        m.screen   = MenuScreen::Settings;
                        m.selected = 0;
                    }
                }
                MenuAction::SaveAndBack => {
                    let draft = resources.get::<MenuState>().map(|m| m.draft.clone());
                    if let Some(d) = draft {
                        let new_cfg = d.into_config();
                        if let Some(w) = resources.get::<Arc<Window>>() {
                            w.set_title(&new_cfg.title);
                            // Only request size change when windowed — fullscreen modes ignore it.
                            if matches!(new_cfg.mode, WindowMode::Windowed) {
                                if let Resolution::Fixed(ww, wh) = new_cfg.resolution {
                                    let _ = w.request_inner_size(
                                        winit::dpi::PhysicalSize::new(ww, wh),
                                    );
                                }
                            }
                            let fullscreen = match new_cfg.mode {
                                WindowMode::Windowed   => None,
                                WindowMode::Borderless =>
                                    Some(winit::window::Fullscreen::Borderless(None)),
                                WindowMode::Fullscreen =>
                                    w.current_monitor()
                                     .and_then(|m| m.video_modes().next())
                                     .map(winit::window::Fullscreen::Exclusive)
                                     .or(Some(winit::window::Fullscreen::Borderless(None))),
                            };
                            w.set_fullscreen(fullscreen);
                        }
                        // Apply vsync change to the wgpu surface
                        if let Some(state) = resources.get_mut::<RendererState>() {
                            state.config.present_mode = if new_cfg.vsync {
                                wgpu::PresentMode::AutoVsync
                            } else {
                                wgpu::PresentMode::AutoNoVsync
                            };
                            state.surface.configure(&state.device, &state.config);
                        }
                        resources.insert(new_cfg);
                    }
                    if let Some(m) = resources.get_mut::<MenuState>() {
                        m.screen   = MenuScreen::Main;
                        m.selected = 1;
                    }
                }
                MenuAction::CancelAndBack => {
                    // Discard draft — reset from current config
                    let draft = resources.get::<WindowConfig>().map(SettingsDraft::from_config);
                    if let Some(m) = resources.get_mut::<MenuState>() {
                        if let Some(d) = draft { m.draft = d; }
                        m.screen   = MenuScreen::Main;
                        m.selected = 1;
                    }
                }
                MenuAction::Quit => std::process::exit(0),
                MenuAction::HoverSelect(i) => {
                    if let Some(m) = resources.get_mut::<MenuState>() {
                        m.selected = i;
                    }
                }
                MenuAction::UpdateDraft(_) => unreachable!(), // handled above
            }
        }
    }
}

/// Cycles the camera projection mode (Perspective → Isometric → TopDown → …) on C press.
/// Register in Phase::PostUpdate, SystemOrder::First so it runs before CameraFollowSystem.
pub struct CycleCameraSystem {
    was_pressed: bool,
}

impl CycleCameraSystem {
    pub fn new() -> Self { Self { was_pressed: false } }
}

impl System for CycleCameraSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let pressed = resources
            .get::<KeyboardState>()
            .map(|kb| kb.is_pressed(KeyCode::KeyC))
            .unwrap_or(false);

        if pressed && !self.was_pressed {
            let state = resources.get_mut::<RendererState>()
                .expect("RendererState not in resources");
            state.camera.cycle_projection();
            let uniform = CameraUniform::from_camera(&state.camera);
            state.queue.write_buffer(&state.camera_buffer, 0, bytemuck::cast_slice(&[uniform]));
        }

        self.was_pressed = pressed;
    }
}

/// Frees render slots for entities queued for despawn — must run before DespawnFlushSystem.
/// Register via `register_render_cleanup(app)`.
pub struct RenderSlotDespawnSystem;

impl System for RenderSlotDespawnSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Collect entities without holding the DespawnQueue borrow into the loop.
        let entities: Vec<_> = resources
            .get::<DespawnQueue>()
            .map(|q| q.0.iter().map(|(e, _)| *e).collect())
            .unwrap_or_default();

        let pool = resources.get_mut::<InstancePool>()
            .expect("InstancePool not in resources");

        for entity in entities {
            if let Ok(slot) = world.get::<&InstanceSlot>(entity) {
                pool.free(slot.0);
            }
            if let Ok(slots) = world.get::<&ShapeGroupSlots>(entity) {
                for &key in &slots.0 { pool.free(key); }
            }
        }
    }
}

/// Allocates GPU instance slots for entities that have a RenderShape/ShapeGroup
/// but no slot yet — entities spawned from data (prefabs) need no renderer access
/// at spawn time. Runs in Phase::RenderSync, First, so freshly flushed entities
/// are visible the same frame. Steady-state cost is ~zero: the matched archetype
/// set is empty once every renderable entity holds its slot.
pub struct RenderSlotAttachSystem;

impl System for RenderSlotAttachSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Collect first — the query borrow must end before insert_one mutates the world.
        let singles: Vec<hecs::Entity> = world
            .query::<(hecs::Entity, &RenderShape)>().without::<&InstanceSlot>()
            .iter().map(|(e, _)| e).collect();
        let groups: Vec<(hecs::Entity, usize)> = world
            .query::<(hecs::Entity, &ShapeGroup)>().without::<&ShapeGroupSlots>()
            .iter().map(|(e, g)| (e, g.shapes.len())).collect();

        for entity in singles {
            let slot = alloc_render_slot(resources);
            let _ = world.insert_one(entity, slot);
        }
        for (entity, count) in groups {
            let slots = alloc_shape_group_slots(count, resources);
            let _ = world.insert_one(entity, slots);
        }
    }
}

/// Registers the full renderer: window/init callbacks and all render systems.
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.on_window_ready(init)
            .on_resize_fn(on_resize)
            // Save position snapshot before movement systems run.
            .add_system(SaveTransformSystem,       Phase::Update,       SystemOrder::First)
            // Free render slots before DespawnFlushSystem removes entities.
            .add_system(RenderSlotDespawnSystem,   Phase::DespawnFlush, SystemOrder::First)
            // Keyboard navigation for the pause menu.
            .add_system(MenuSystem,                Phase::PostUpdate,   SystemOrder::First)
            // C key cycles Perspective → Isometric → TopDown.
            .add_system(CycleCameraSystem::new(),  Phase::PostUpdate,   SystemOrder::First)
            // F3 toggles the dev stats overlay.
            .add_system(DevOverlaySystem::new(),   Phase::PostUpdate,   SystemOrder::First)
            // Attach slots to slotless renderables, then sync transforms to the GPU pool.
            .add_system(RenderSlotAttachSystem,    Phase::RenderSync,   SystemOrder::First)
            .add_system(RenderSyncSystem,          Phase::RenderSync,   SystemOrder::Default)
            .add_system(MeshRenderSyncSystem::new(), Phase::RenderSync, SystemOrder::Default)
            .add_system(RenderSystem::new(),       Phase::Render,       SystemOrder::Default);
    }
}

fn shape_to_gpu(shape: RenderShapeType) -> (u32, [f32; 4]) {
    match shape {
        RenderShapeType::Cube                         => (0, [0.0; 4]),
        RenderShapeType::Sphere                       => (1, [0.0; 4]),
        RenderShapeType::Diamond                      => (2, [0.0; 4]),
        RenderShapeType::RoundedBox { corner_radius } => (3, [corner_radius, 0.0, 0.0, 0.0]),
        RenderShapeType::Cylinder                     => (4, [0.0; 4]),
        RenderShapeType::Capsule                      => (5, [0.0; 4]),
        RenderShapeType::Custom { shape_type, params } => (shape_type, params),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    fn unproject_topdown_ortho_hits_expected_ground_point() {
        // Straight-down ortho, ±10 extent, up = -Z (north at screen top).
        let proj = Mat4::orthographic_rh(-10.0, 10.0, -10.0, 10.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(
            GlamVec3::new(0.0, 50.0, 0.0),
            GlamVec3::ZERO,
            GlamVec3::NEG_Z,
        );
        let vp = proj * view;

        let center = unproject_to_ground(vp, Vec2::ZERO).unwrap();
        assert!(center.distance(GlamVec3::ZERO) < 1e-3, "center ray must hit origin: {center}");

        // NDC +x = screen right = world +X; NDC +y = screen up = world -Z.
        let off = unproject_to_ground(vp, Vec2::new(0.5, 0.5)).unwrap();
        assert!(off.distance(GlamVec3::new(5.0, 0.0, -5.0)) < 1e-3, "got {off}");
    }

    #[test]
    fn unproject_horizontal_ray_misses_ground() {
        let proj = Mat4::perspective_rh(45f32.to_radians(), 1.0, 0.1, 100.0);
        // Eye at y=5 looking horizontally: the center ray never reaches y=0.
        let view = Mat4::look_at_rh(
            GlamVec3::new(0.0, 5.0, 0.0),
            GlamVec3::new(10.0, 5.0, 0.0),
            GlamVec3::Y,
        );
        assert!(unproject_to_ground(proj * view, Vec2::ZERO).is_none());
    }
}
