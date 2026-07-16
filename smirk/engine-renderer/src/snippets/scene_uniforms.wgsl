// ── Uniforms ──────────────────────────────────────────────────────────────────

struct Camera {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    right:         vec4<f32>,
    up:            vec4<f32>,
    eye:           vec4<f32>, // world-space camera position
    viewport:      vec4<f32>, // render target size in pixels (xy only)
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
    fog_height:         f32,
    fog_height_falloff: f32,
    _pad1:              u32,
    points:      array<PointLight, MAX_POINT_LIGHTS>,
}
@group(0) @binding(1)
var<uniform> light: LightUniform;

// SSAO: real blurred target when enabled, a white 1×1 fallback otherwise
// (see engine_renderer::ssao::WhiteAo) — shade_pbr multiplies ambient by it.
@group(0) @binding(5) var t_ssao: texture_2d<f32>;
@group(0) @binding(6) var s_ssao: sampler;
