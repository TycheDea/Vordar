// ── Uniforms ──────────────────────────────────────────────────────────────────

struct Camera {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    right:         vec4<f32>,
    up:            vec4<f32>,
    eye:           vec4<f32>, // world-space camera position
}
@group(0) @binding(0)
var<uniform> camera: Camera;

//#const MAX_POINT_LIGHTS
struct PointLight {
    position:  vec3<f32>,
    radius:    f32, // distance at which falloff reaches zero
    color:     vec3<f32>,
    intensity: f32,
}

struct LightUniform {
    direction: vec3<f32>, // world-space, normalised, pointing TOWARD light
    _pad:      f32,
    color:     vec3<f32>,
    ambient:   f32,
    fog_color:   vec3<f32>,
    fog_density: f32,
    point_count: u32,
    _pad0:       u32,
    _pad1:       u32,
    _pad2:       u32,
    points:      array<PointLight, MAX_POINT_LIGHTS>,
}
@group(0) @binding(1)
var<uniform> light: LightUniform;
