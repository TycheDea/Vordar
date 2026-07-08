// Textured mesh pass — same camera/light uniforms and Lambert model as
// shader.wgsl, but real vertex geometry with per-primitive base-color
// textures instead of the instanced unit cube.

@group(0) @binding(0)
var<uniform> view_proj: mat4x4<f32>;

struct LightUniform {
    direction: vec3<f32>, // world-space, normalised, pointing TOWARD light
    _pad:      f32,
    color:     vec3<f32>,
    ambient:   f32,
}
@group(0) @binding(1)
var<uniform> light: LightUniform;

@group(1) @binding(0) var t_color: texture_2d<f32>;
@group(1) @binding(1) var s_color: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       normal:   vec3<f32>,
    @location(1)       uv:       vec2<f32>,
    @location(2)       tint:     vec4<f32>,
}

@vertex
fn vtx_main(
    // per-vertex
    @location(0) position:  vec3<f32>,
    @location(1) in_normal: vec3<f32>,
    @location(2) in_uv:     vec2<f32>,
    // per-instance
    @location(3) model_0:   vec4<f32>,
    @location(4) model_1:   vec4<f32>,
    @location(5) model_2:   vec4<f32>,
    @location(6) model_3:   vec4<f32>,
    @location(7) inst_tint: vec4<f32>,
) -> VertexOutput {
    let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
    let world = model * vec4<f32>(position, 1.0);
    // Normal transform: inverse-transpose of the upper-left 3×3 (R * S^-1).
    // For TRS matrices S^-1 = col / |col|^2 — avoids WGSL inverse() which is unavailable.
    let col0     = model[0].xyz;
    let col1     = model[1].xyz;
    let col2     = model[2].xyz;
    let norm_mat = mat3x3<f32>(col0 / dot(col0, col0), col1 / dot(col1, col1), col2 / dot(col2, col2));

    var out: VertexOutput;
    out.clip_pos = view_proj * world;
    out.normal   = normalize(norm_mat * in_normal);
    out.uv       = in_uv;
    out.tint     = inst_tint;
    return out;
}

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex  = textureSample(t_color, s_color, in.uv);
    let base = tex.rgb * in.tint.rgb;

    // Lambert diffuse + ambient — identical to the primitive pass so mixed
    // scenes light consistently.
    let N    = normalize(in.normal);
    let diff = max(dot(N, light.direction), 0.0);
    let lit  = light.ambient + diff * (1.0 - light.ambient);

    return vec4<f32>(base * lit * light.color, 1.0);
}
