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

struct LightUniform {
    direction: vec3<f32>, // world-space, normalised, pointing TOWARD light
    _pad:      f32,
    color:     vec3<f32>,
    ambient:   f32,
    fog_color:   vec3<f32>,
    fog_density: f32,
}
@group(0) @binding(1)
var<uniform> light: LightUniform;
