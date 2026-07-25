//! The game-facing API: free functions over Resources — the only
//! supported way for game/client code to poke the renderer.

use crate::camera::CameraUniform;
use crate::camera::ProjectionMode;
use crate::ibl;
use crate::instance::{InstancePool, InstanceSlot, ShapeGroupSlots};
use crate::mesh::{self, MeshStore};
use crate::mesh_pipeline;
use crate::state::RendererState;
use crate::texture;
use engine_core::traits::Resources;
use glam::Mat4;
use glam::Vec3 as GlamVec3;

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

/// Allocate a render slot for a new entity. Call from SpawnQueue callbacks.
pub fn alloc_render_slot(resources: &mut Resources) -> InstanceSlot {
    let pool = resources.expect_mut::<InstancePool>();
    InstanceSlot(pool.alloc())
}

/// Allocate N render slots for a ShapeGroup entity. `count` must equal `group.shapes.len()`.
pub fn alloc_shape_group_slots(count: usize, resources: &mut Resources) -> ShapeGroupSlots {
    let pool = resources.expect_mut::<InstancePool>();
    ShapeGroupSlots((0..count).map(|_| pool.alloc()).collect())
}

/// Free a render slot when an entity is despawned. Call from DespawnQueue hooks.
pub fn free_render_slot(slot: InstanceSlot, resources: &mut Resources) {
    let pool = resources.expect_mut::<InstancePool>();
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
    let uniform = CameraUniform::from_camera(&state.camera, (state.config.width, state.config.height));
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
    let uniform = CameraUniform::from_camera(&state.camera, (state.config.width, state.config.height));
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

/// Request a Radiance .hdr equirect as the zone's environment: IBL ambient
/// for every lit pass and the visible sky. Decode + f16 conversion run on a
/// detached thread; the bake and swap happen on the main thread the frame
/// the pixels arrive (`RenderSystem::run` polls every frame) — the previous
/// environment stays visible until then. Failure (logged when it surfaces)
/// keeps the previous environment. Returns false headless (no renderer to
/// request against); returns true otherwise, including when `path` is
/// already applied or already pending (no-op).
pub fn set_environment(path: &str, resources: &mut Resources) -> bool {
    // Headless / pre-window: nothing to do (same contract as the sync systems).
    let Some(state) = resources.get_mut::<RendererState>() else { return false };
    if state.current_env_path.as_deref() == Some(path) {
        return true;
    }
    if state.pending_env.as_ref().is_some_and(|p| p.path == path) {
        return true;
    }
    // Last write wins: the stale thread's send lands in a dropped receiver.
    state.pending_env = Some(ibl::PendingEnvironment::spawn(path));
    true
}

/// Exposure applied in the tonemap pass. 1.0 is neutral; the
/// day/night system may drive it.
pub fn set_exposure(exposure: f32, resources: &mut Resources) {
    let Some(state) = resources.get_mut::<RendererState>() else { return };
    state.tonemap.set_exposure(&state.queue, exposure);
    state.bloom.set_exposure(&state.queue, exposure);
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

/// Enqueue a background job producing procedural mesh data (e.g. the zone
/// ground) under a synthetic asset key; entities can reference it via
/// `RenderMesh { asset: key }` right away — `MeshStore::integrate` uploads
/// the result once the job completes. A key already known in any state is a
/// no-op (procedural data is deterministic per key). Returns false headless
/// (no `MeshStore` — renderer init never ran).
pub fn request_procedural_mesh(
    key: &str,
    job: impl FnOnce() -> Result<mesh::MeshData, String> + Send + 'static,
    resources: &mut Resources,
) -> bool {
    let Some(store) = resources.get_mut::<MeshStore>() else {
        return false;
    };
    store.request_job(key, job);
    true
}

/// Swaps in the shared tile for the world-space triplanar detail overlay
/// (mesh pipeline group 3) — one global material every opted-in prop samples,
/// not a per-primitive bind group. Takes `MaterialData`, not a directory: the
/// content-convention loader already exists and is tested
/// (`client::ground::load_ground_material`), so this stays ignorant of it and
/// reuses `mesh::slot_texture` — the same 1×1-neutral-or-real-texture rule
/// every material texture slot already follows. A no-op headless (no
/// renderer to swap into).
pub fn set_detail_material(material: mesh::MaterialData, resources: &mut Resources) {
    let Some(state) = resources.get_mut::<RendererState>() else { return };
    let albedo = mesh::slot_texture(
        &state.device, &state.queue, &state.mipgen,
        &material.base_color_image, true, [128, 128, 128, 255],
    );
    let normal = mesh::slot_texture(
        &state.device, &state.queue, &state.mipgen,
        &material.normal_image, false, [128, 128, 255, 255],
    );
    state.detail_bind_group = mesh_pipeline::create_detail_bind_group(&state.device, &state.detail_bgl, &albedo, &normal);
    state.detail_textures = vec![albedo, normal];
}

/// Distance fog for the current zone: linear-space color, exponential
/// density per world unit (0.0 disables).
pub fn set_fog(color: GlamVec3, density: f32, resources: &mut Resources) {
    let Some(state) = resources.get_mut::<RendererState>() else { return };
    state.light_state.fog_color   = color.to_array();
    state.light_state.fog_density = density;
    state.queue.write_buffer(&state.light_buffer, 0, bytemuck::cast_slice(&[state.light_state]));
}

/// Attenuates fog density above `height` by `exp(-falloff * max(y - height, 0))`;
/// 0/0 reproduces pure distance fog.
pub fn set_fog_height(height: f32, falloff: f32, resources: &mut Resources) {
    let Some(state) = resources.get_mut::<RendererState>() else { return };
    state.light_state.fog_height         = height;
    state.light_state.fog_height_falloff = falloff;
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
    let state = resources.expect_mut::<RendererState>();
    let tex = texture::create_checker_texture(&state.device, &state.queue, size, tile_size, color_a, color_b);
    let bg  = texture::create_bind_group(&state.device, &state.texture_bgl, &tex);
    let idx = state.texture_store.len();
    state.texture_store.push((tex, bg));
    TextureHandle(idx)
}

/// Load a BC7 DDS texture from `path`. `srgb` picks Bc7RgbaUnormSrgb (for
/// sRGB-encoded images like color/albedo, the default) vs Bc7RgbaUnorm (for
/// linear data). Returns a handle to use with `set_texture`. Logs a warning
/// and returns `None` if the file is missing or invalid.
pub fn load_texture(path: &str, srgb: bool, resources: &mut Resources) -> Option<TextureHandle> {
    let state = resources.expect_mut::<RendererState>();
    match texture::load_dds(&state.device, &state.queue, path, srgb) {
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
    let state = resources.expect_mut::<RendererState>();
    state.active_texture_idx = handle.0;
}

/// Reset to the default white texture (instance colors render unaffected).
pub fn clear_texture(resources: &mut Resources) {
    let state = resources.expect_mut::<RendererState>();
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

/// Returns the camera's current world-space eye position. Used to
/// depth-sort alpha-blended particles back-to-front.
/// Returns `Vec3::ZERO` if RendererState is not yet initialised.
pub fn camera_eye(resources: &Resources) -> GlamVec3 {
    resources
        .get::<RendererState>()
        .map(|s| s.camera.eye())
        .unwrap_or(GlamVec3::ZERO)
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
