use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingType, Buffer, BufferBindingType, BufferUsages,
    SamplerBindingType, ShaderStages, TextureViewDimension,
};

/// Which projection the camera uses. Cycle at runtime with the C key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProjectionMode {
    Perspective,
    Isometric,  // orthographic + 35° pitch + 45° yaw
    TopDown,    // orthographic + straight down
}

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
            zfar: 200.0,
            // Pulled back far enough to read the battlefield, not just the
            // player's feet (Phase 7.5). Mouse wheel adjusts via zoom().
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
    /// CameraConfig (see zoom_camera in lib.rs).
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
        self.eye.y = self.target.y + self.radius * self.pitch.sin();
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
}

impl CameraUniform {
    pub(crate) fn from_camera(camera: &Camera) -> Self {
        let (right, up) = camera.basis();
        let vp = camera.build_view_projection_matrix();
        Self {
            view_proj:     vp.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            right:         [right.x, right.y, right.z, 0.0],
            up:            [up.x, up.y, up.z, 0.0],
            eye:           [camera.eye.x, camera.eye.y, camera.eye.z, 1.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct LightUniform {
    pub(crate) direction: [f32; 3], // world-space, normalised, pointing TOWARD light
    pub(crate) _pad:      f32,
    pub(crate) color:     [f32; 3], // RGB light intensity
    pub(crate) ambient:   f32,      // IBL ambient scale (1.0 = environment as authored)
    // Distance fog (VQ-A5): linear-space color, exponential density.
    pub(crate) fog_color:   [f32; 3],
    pub(crate) fog_density: f32,
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
        }
    }
}

/// Creates uniform buffers (camera + light + shadow light_vp), the scene
/// bind group layout, and the bind group. Shadow resources live here
/// (bindings 2–4) because the skinned pipeline already uses the default max
/// of 4 bind groups.
/// Returns (camera_buffer, light_buffer, light_vp_buffer, layout, bind_group).
pub(crate) fn create_gpu_resources(
    device:      &wgpu::Device,
    camera:      &Camera,
    shadow_view: &wgpu::TextureView,
) -> (Buffer, Buffer, Buffer, BindGroupLayout, BindGroup) {
    let cam_uniform = CameraUniform::from_camera(camera);
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

    let light_vp_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label:    Some("Light VP Uniform"),
        contents: bytemuck::cast_slice(&glam::Mat4::IDENTITY.to_cols_array()),
        usage:    BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });

    let shadow_sampler = crate::shadow::create_shadow_sampler(device);

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
            // Shadow receiving (VQ-D3): sun view-proj + depth map + comparison sampler.
            uniform_entry(2, ShaderStages::FRAGMENT),
            BindGroupLayoutEntry {
                binding:    3,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    multisampled:   false,
                    view_dimension: TextureViewDimension::D2,
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
        ],
    });

    let bind_group = device.create_bind_group(&BindGroupDescriptor {
        label:  Some("Scene Bind Group"),
        layout: &layout,
        entries: &[
            BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
            BindGroupEntry { binding: 1, resource: light_buffer.as_entire_binding() },
            BindGroupEntry { binding: 2, resource: light_vp_buffer.as_entire_binding() },
            BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(shadow_view) },
            BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
        ],
    });

    (camera_buffer, light_buffer, light_vp_buffer, layout, bind_group)
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
}