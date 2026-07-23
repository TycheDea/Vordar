// Camera state and its GPU-side uniforms: `Camera` (position/target/
// projection), the `CameraUniform`/`LightUniform` buffers group 0 binds for
// every scene pipeline, and `CycleCameraSystem` (the 'C' key handler that
// steps ProjectionMode Perspective -> Isometric -> TopDown).

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, BufferUsages,
    SamplerBindingType, ShaderStages, TextureViewDimension,
};
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;

/// Which projection the camera uses. Cycle at runtime with the C key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProjectionMode {
    Perspective,
    Isometric,  // orthographic + 35° pitch + 45° yaw
    TopDown,    // orthographic + straight down
}

/// World-space floor for the camera eye — the ground plane `unproject_to_ground` assumes.
const MIN_EYE_Y: f32 = 0.0;

pub(crate) struct Camera {
    eye: glam::Vec3,
    pub(crate) target: glam::Vec3,
    pub(crate) aspect: f32,
    fovy: f32,
    znear: f32,
    zfar: f32,
    radius: f32,
    pub(crate) angle: f32,
    pitch: f32,
    pub(crate) mode: ProjectionMode,
    ortho_half_height: f32,
}

impl Camera {
    pub(crate) fn new(aspect: f32) -> Self {
        let mut cam = Self {
            eye: glam::Vec3::ZERO,
            target: glam::Vec3::ZERO,
            aspect,
            fovy: 45.0_f32.to_radians(),
            znear: 0.1,
            zfar: 400.0,
            // Pulled back far enough to read the battlefield, not just the
            // player's feet. Mouse wheel adjusts via zoom().
            radius: 34.0,
            angle:  std::f32::consts::FRAC_PI_4,
            pitch:  0.8,
            mode: ProjectionMode::Perspective,
            ortho_half_height: 20.0,
        };
        cam.recompute_eye();
        cam
    }

    pub(crate) fn build_view_projection_matrix(&self) -> glam::Mat4 {
        let projection = match self.mode {
            ProjectionMode::Perspective => {
                glam::Mat4::perspective_rh(self.fovy, self.aspect, self.znear, self.zfar)
            }
            ProjectionMode::Isometric | ProjectionMode::TopDown => {
                let hh = self.ortho_half_height;
                let hw = hh * self.aspect;
                glam::Mat4::orthographic_rh(-hw, hw, -hh, hh, self.znear, self.zfar)
            }
        };
        // Top-down: eye is directly above target so look-dir = -Y;
        // use NEG_Z as up so north (–Z) points to the top of the screen.
        let up = match self.mode {
            ProjectionMode::TopDown => glam::Vec3::NEG_Z,
            _ => glam::Vec3::Y,
        };
        let view = glam::Mat4::look_at_rh(self.eye, self.target, up);
        projection * view
    }

    /// Dolly in/out. Scales the orthographic extent in lockstep so zoom
    /// works in every projection mode. Limits come from the game's
    /// CameraConfig (see `zoom_camera`).
    pub(crate) fn zoom(&mut self, delta: f32, min_radius: f32, max_radius: f32) {
        let before = self.radius;
        self.radius = (self.radius + delta).clamp(min_radius, max_radius);
        self.ortho_half_height *= self.radius / before;
        self.recompute_eye();
    }

    pub(crate) fn orbit(&mut self, yaw_delta: f32, pitch_delta: f32) {
        self.angle += yaw_delta;
        // Only clamp when the user is actively pitching — preserves mode-set values.
        if pitch_delta != 0.0 {
            self.pitch = (self.pitch + pitch_delta).clamp(-1.4, 1.4);
        }
        self.recompute_eye();
    }

    /// Frame a world-space AABB in perspective: target its center and pull
    /// back so its bounding sphere fills `fill` (0..1) of the vertical FOV.
    pub(crate) fn fit_bounds(&mut self, min: glam::Vec3, max: glam::Vec3, fill: f32) {
        self.target = (min + max) * 0.5;
        let sphere = (max - min).length() * 0.5;
        self.radius = sphere / (fill * (self.fovy * 0.5).tan() * self.aspect.min(1.0));
        self.recompute_eye();
    }

    /// Advance to the next projection mode and snap pitch/yaw to canonical values.
    pub(crate) fn cycle_projection(&mut self) {
        self.mode = match self.mode {
            ProjectionMode::Perspective => {
                self.pitch = 0.6155; // arctan(1/√2) ≈ 35° — true isometric
                self.angle = std::f32::consts::FRAC_PI_4;
                ProjectionMode::Isometric
            }
            ProjectionMode::Isometric => {
                self.pitch = std::f32::consts::FRAC_PI_2;
                self.angle = 0.0;
                ProjectionMode::TopDown
            }
            ProjectionMode::TopDown => {
                self.pitch = 0.8;
                self.angle = std::f32::consts::FRAC_PI_4;
                ProjectionMode::Perspective
            }
        };
        self.recompute_eye();
    }

    fn recompute_eye(&mut self) {
        self.eye.x = self.target.x + self.radius * self.angle.cos() * self.pitch.cos();
        self.eye.z = self.target.z + self.radius * self.angle.sin() * self.pitch.cos();
        // Floored at the ground plane (also assumed by unproject_to_ground) so
        // pitching down at any zoom level can't put the eye underground.
        self.eye.y = (self.target.y + self.radius * self.pitch.sin()).max(MIN_EYE_Y);
    }

    /// The camera's world-space eye position.
    pub(crate) fn eye(&self) -> glam::Vec3 {
        self.eye
    }

    /// The camera's world-space right and up vectors — the plane billboards
    /// (particles) expand in. Derived from the same look-at inputs as
    /// `build_view_projection_matrix`, including the TopDown up-flip.
    pub(crate) fn basis(&self) -> (glam::Vec3, glam::Vec3) {
        let world_up = match self.mode {
            ProjectionMode::TopDown => glam::Vec3::NEG_Z,
            _ => glam::Vec3::Y,
        };
        let forward = (self.target - self.eye)
            .try_normalize()
            .unwrap_or(glam::Vec3::NEG_Z);
        let right = forward
            .cross(world_up)
            .try_normalize()
            .unwrap_or(glam::Vec3::X);
        let up = right.cross(forward);
        (right, up)
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    /// Inverse of view_proj — the skybox pass unprojects NDC to a world-space
    /// view ray (WGSL has no inverse()).
    inv_view_proj: [[f32; 4]; 4],
    right:     [f32; 4],
    up:        [f32; 4],
    /// World-space eye position — the PBR shaders' view vector origin.
    eye:       [f32; 4],
    /// Render target size in pixels (xy only) — shade_pbr divides
    /// `clip_pos.xy` by this to get the SSAO texture's screen UV.
    viewport:  [f32; 4],
}

impl CameraUniform {
    pub(crate) fn from_camera(camera: &Camera, viewport: (u32, u32)) -> Self {
        let (right, up) = camera.basis();
        let vp = camera.build_view_projection_matrix();
        Self {
            view_proj:     vp.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            right:         [right.x, right.y, right.z, 0.0],
            up:            [up.x, up.y, up.z, 0.0],
            eye:           [camera.eye.x, camera.eye.y, camera.eye.z, 1.0],
            viewport:      [viewport.0 as f32, viewport.1 as f32, 0.0, 0.0],
        }
    }

    /// Rewrites just the viewport field of an already-uploaded camera
    /// buffer — the offscreen harness's camera-orientation resets don't know
    /// the eventual render target's pixel size, so `compose` corrects it
    /// here right before every draw.
    pub(crate) fn write_viewport(queue: &wgpu::Queue, camera_buffer: &Buffer, width: u32, height: u32) {
        let offset = std::mem::offset_of!(CameraUniform, viewport) as wgpu::BufferAddress;
        queue.write_buffer(camera_buffer, offset, bytemuck::cast_slice(&[width as f32, height as f32, 0.0f32, 0.0f32]));
    }
}

/// Cap on point lights carried per frame — mirrored into WGSL's `LightUniform`
/// array length by build.rs so the two sides can never drift apart.
pub(crate) const MAX_POINT_LIGHTS: u32 = 16;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuPointLight {
    pub(crate) position:  [f32; 3],
    pub(crate) radius:    f32, // distance at which falloff reaches zero
    pub(crate) color:     [f32; 3],
    pub(crate) intensity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightUniform {
    pub(crate) direction: [f32; 3], // world-space, normalised, pointing TOWARD light
    pub(crate) _pad:      f32,
    pub(crate) color:     [f32; 3], // RGB light intensity
    pub(crate) ambient:   f32,      // IBL ambient scale (1.0 = environment as authored)
    // Distance fog: linear-space color, exponential density.
    pub(crate) fog_color:   [f32; 3],
    pub(crate) fog_density: f32,
    pub(crate) point_count: u32,
    // Height fog: density is attenuated above fog_height by
    // exp(-fog_height_falloff * max(y - fog_height, 0)); 0/0 reproduces
    // pure distance fog.
    pub(crate) fog_height:         f32,
    pub(crate) fog_height_falloff: f32,
    pub(crate) _pad2:              u32,
    pub(crate) points:      [GpuPointLight; MAX_POINT_LIGHTS as usize],
}

impl LightUniform {
    pub(crate) fn default_sun() -> Self {
        let dir = glam::Vec3::new(-1.0, 2.0, -1.0).normalize();
        Self {
            direction:   dir.to_array(),
            _pad:        0.0,
            color:       [1.0, 0.95, 0.85],
            ambient:     1.0,
            fog_color:   [0.30, 0.26, 0.28], // dusk haze
            fog_density: 0.0,                // zones opt in via set_fog
            point_count: 0,
            fog_height:         0.0,
            fog_height_falloff: 0.0, // zones opt in via set_fog_height
            _pad2:              0,
            points:      [GpuPointLight { position: [0.0; 3], radius: 0.0, color: [0.0; 3], intensity: 0.0 };
                MAX_POINT_LIGHTS as usize],
        }
    }
}

/// Creates uniform buffers (camera + light + shadow light_vp) and the scene
/// bind group layout (bindings 0–6: camera/light uniforms, shadow receiving,
/// SSAO). Shadow resources live at 2–4 and SSAO at 5–6 because the skinned
/// pipeline already uses the default max of 4 bind groups. Building the
/// bind group itself is a separate step (`create_scene_bind_group`) since
/// the AO view it binds isn't built until after the SSAO passes, which in
/// turn need this layout to exist first.
/// Returns (camera_buffer, light_buffer, light_vp_buffer, layout).
pub(crate) fn create_scene_buffers_and_layout(
    device:   &wgpu::Device,
    camera:   &Camera,
    viewport: (u32, u32),
) -> (Buffer, Buffer, Buffer, BindGroupLayout) {
    let cam_uniform = CameraUniform::from_camera(camera, viewport);
    let camera_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label:    Some("Camera Uniform"),
        contents: bytemuck::cast_slice(&[cam_uniform]),
        usage:    BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });

    let light_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label:    Some("Light Uniform"),
        contents: bytemuck::cast_slice(&[LightUniform::default_sun()]),
        usage:    BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });

    let identity_cascades: Vec<f32> = (0..crate::shadow::CASCADE_COUNT)
        .flat_map(|_| glam::Mat4::IDENTITY.to_cols_array())
        .collect();
    let light_vp_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label:    Some("Light VP Uniform"),
        contents: bytemuck::cast_slice(&identity_cascades),
        usage:    BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });

    let uniform_entry = |binding: u32, visibility: ShaderStages| BindGroupLayoutEntry {
        binding,
        visibility,
        ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    };
    let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("Scene Bind Group Layout"),
        entries: &[
            // Fragment too: the PBR shaders read camera.eye for the view vector.
            uniform_entry(0, ShaderStages::VERTEX.union(ShaderStages::FRAGMENT)),
            uniform_entry(1, ShaderStages::FRAGMENT),
            // Shadow receiving: sun view-proj + depth map + comparison sampler.
            uniform_entry(2, ShaderStages::FRAGMENT),
            BindGroupLayoutEntry {
                binding:    3,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    multisampled:   false,
                    view_dimension: TextureViewDimension::D2Array,
                    sample_type:    wgpu::TextureSampleType::Depth,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding:    4,
                visibility: ShaderStages::FRAGMENT,
                ty:         BindingType::Sampler(SamplerBindingType::Comparison),
                count:      None,
            },
            // SSAO: real blurred target when enabled, a white 1×1 fallback
            // otherwise (see ssao::WhiteAo).
            BindGroupLayoutEntry {
                binding:    5,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    multisampled:   false,
                    view_dimension: TextureViewDimension::D2,
                    sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding:    6,
                visibility: ShaderStages::FRAGMENT,
                ty:         BindingType::Sampler(SamplerBindingType::Filtering),
                count:      None,
            },
        ],
    });

    (camera_buffer, light_buffer, light_vp_buffer, layout)
}

/// (Re)builds the scene bind group against whichever AO view/sampler the
/// caller currently wants bound — called at construction and again whenever
/// that view's identity changes (a resize, or the offscreen harness's SSAO
/// toggle), since a bind group keeps its bound view alive rather than
/// tracking the field that replaced it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_scene_bind_group(
    device:            &wgpu::Device,
    layout:            &BindGroupLayout,
    camera_buffer:     &Buffer,
    light_buffer:      &Buffer,
    light_vp_buffer:   &Buffer,
    shadow_array_view: &wgpu::TextureView,
    ao_view:           &wgpu::TextureView,
    ao_sampler:        &wgpu::Sampler,
) -> BindGroup {
    let shadow_sampler = crate::shadow::create_shadow_sampler(device);
    device.create_bind_group(&BindGroupDescriptor {
        label:  Some("Scene Bind Group"),
        layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
            BindGroupEntry { binding: 1, resource: light_buffer.as_entire_binding() },
            BindGroupEntry { binding: 2, resource: light_vp_buffer.as_entire_binding() },
            BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(shadow_array_view) },
            BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
            BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(ao_view) },
            BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(ao_sampler) },
        ],
    })
}

/// Cycles the camera projection mode (Perspective → Isometric → TopDown → …) on C press.
/// Register in Phase::PostUpdate, SystemOrder::First so it runs before CameraFollowSystem.
pub struct CycleCameraSystem;

impl System for CycleCameraSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let pressed = resources
            .get::<engine_app::input::KeyboardState>()
            .map(|kb| kb.just_pressed(winit::keyboard::KeyCode::KeyC))
            .unwrap_or(false);

        if pressed {
            let state = resources.get_mut::<crate::RendererState>()
                .expect("RendererState not in resources");
            state.camera.cycle_projection();
            let uniform = CameraUniform::from_camera(&state.camera, (state.config.width, state.config.height));
            state.queue.write_buffer(&state.camera_buffer, 0, bytemuck::cast_slice(&[uniform]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basis_is_orthonormal_in_every_projection_mode() {
        let mut cam = Camera::new(16.0 / 9.0);
        for _ in 0..3 {
            let (right, up) = cam.basis();
            let forward = (cam.target - cam.eye).normalize();
            assert!((right.length() - 1.0).abs() < 1e-4, "{:?}: |right| = 1", cam.mode);
            assert!((up.length() - 1.0).abs() < 1e-4, "{:?}: |up| = 1", cam.mode);
            assert!(right.dot(up).abs() < 1e-4, "{:?}: right ⊥ up", cam.mode);
            assert!(right.dot(forward).abs() < 1e-4, "{:?}: right ⊥ forward", cam.mode);
            assert!(up.dot(forward).abs() < 1e-4, "{:?}: up ⊥ forward", cam.mode);
            cam.cycle_projection();
        }
    }

    #[test]
    fn topdown_basis_matches_screen_axes() {
        // TopDown looks straight down with north (-Z) at the top of the screen,
        // so a billboard's right is +X and its up is -Z.
        let mut cam = Camera::new(1.0);
        cam.cycle_projection(); // Perspective -> Isometric
        cam.cycle_projection(); // Isometric -> TopDown
        assert_eq!(cam.mode, ProjectionMode::TopDown);
        let (right, up) = cam.basis();
        assert!(right.abs_diff_eq(glam::Vec3::X, 1e-3), "right = +X, got {right}");
        assert!(up.abs_diff_eq(glam::Vec3::NEG_Z, 1e-3), "up = -Z, got {up}");
    }

    #[test]
    fn eye_never_dips_below_ground_across_pitch_and_zoom_extremes() {
        let (min_radius, max_radius) = (4.0_f32, 100.0_f32);
        for &radius in &[min_radius, 10.0, 34.0, max_radius] {
            let mut cam = Camera::new(16.0 / 9.0);
            cam.zoom(radius - cam.radius, min_radius, max_radius);
            // Walk pitch from clamp extreme to clamp extreme in small steps,
            // checking every step — not just the endpoints — the way holding
            // an arrow key actually drives it.
            for _ in 0..600 {
                cam.orbit(0.0, -0.01);
                assert!(cam.eye.y >= 0.0, "radius={radius} pitch={} eye.y={}", cam.pitch, cam.eye.y);
            }
            for _ in 0..600 {
                cam.orbit(0.0, 0.01);
                assert!(cam.eye.y >= 0.0, "radius={radius} pitch={} eye.y={}", cam.pitch, cam.eye.y);
            }
        }
    }
}