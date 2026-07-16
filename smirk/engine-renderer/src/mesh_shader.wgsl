// Textured mesh pass — full PBR: Cook-Torrance GGX with normal mapping,
// metallic-roughness, emissive and AO. Same camera/light uniforms as the
// other geometry passes so mixed scenes light consistently.

//#include "snippets/scene_uniforms.wgsl"

//#include "snippets/shadow_sample.wgsl"

// ── Material (group 1) ───────────────────────────────────────────────────────

@group(1) @binding(0) var t_albedo:   texture_2d<f32>; // sRGB
@group(1) @binding(1) var s_mat:      sampler;
@group(1) @binding(2) var t_normal:   texture_2d<f32>; // linear, tangent-space
@group(1) @binding(3) var t_mr:       texture_2d<f32>; // linear: g=roughness, b=metallic
@group(1) @binding(4) var t_emissive: texture_2d<f32>; // sRGB
@group(1) @binding(5) var t_ao:       texture_2d<f32>; // linear: r=occlusion

struct MaterialUniform {
    base_color: vec4<f32>,
    emissive:   vec4<f32>, // rgb premultiplied by KHR emissive_strength
    mr:         vec4<f32>, // x=metallic factor, y=roughness factor
}
@group(1) @binding(6)
var<uniform> material: MaterialUniform;

// ── Environment (group 2) — IBL ambient ─────────────────────────────────────

@group(2) @binding(0) var t_irradiance: texture_cube<f32>;
@group(2) @binding(1) var t_prefilter:  texture_cube<f32>;
@group(2) @binding(2) var t_brdf:       texture_2d<f32>;
@group(2) @binding(3) var s_env:        sampler;

//#const PREFILTER_MAX_MIP

struct VertexOutput {
    @builtin(position) clip_pos:  vec4<f32>,
    @location(0)       normal:    vec3<f32>,
    @location(1)       uv:        vec2<f32>,
    @location(2)       tint:      vec4<f32>,
    @location(3)       world_pos: vec3<f32>,
    @location(4)       tangent:   vec4<f32>,
}

@vertex
fn vtx_main(
    // per-vertex
    @location(0) position:   vec3<f32>,
    @location(1) in_normal:  vec3<f32>,
    @location(2) in_uv:      vec2<f32>,
    @location(3) in_tangent: vec4<f32>,
    // per-instance
    @location(4) model_0:    vec4<f32>,
    @location(5) model_1:    vec4<f32>,
    @location(6) model_2:    vec4<f32>,
    @location(7) model_3:    vec4<f32>,
    @location(8) inst_tint:  vec4<f32>,
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
    out.clip_pos  = camera.view_proj * world;
    out.normal    = normalize(norm_mat * in_normal);
    out.uv        = in_uv;
    out.tint      = inst_tint;
    out.world_pos = world.xyz;
    out.tangent   = vec4<f32>(normalize(norm_mat * in_tangent.xyz), in_tangent.w);
    return out;
}

//#include "snippets/pbr_common.wgsl"

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let albedo_s = textureSample(t_albedo, s_mat, in.uv);
    let albedo   = albedo_s.rgb * material.base_color.rgb * in.tint.rgb;

    let mr        = textureSample(t_mr, s_mat, in.uv);
    let metallic  = mr.b * material.mr.x;
    let roughness = mr.g * material.mr.y;
    let ao        = textureSample(t_ao, s_mat, in.uv).r;
    let emissive  = textureSample(t_emissive, s_mat, in.uv).rgb * material.emissive.rgb;

    // Alpha cutoff (glTF MASK; BLEND approximated as MASK) — mr.z of 0 means
    // opaque. Placed after all textureSample calls to keep them in trivially
    // uniform control flow.
    if albedo_s.a * material.base_color.a < material.mr.z {
        discard;
    }

    // Normal mapping — tangent w = 0 means "no tangent basis" (degenerate UVs).
    let Nv = normalize(in.normal);
    var N  = Nv;
    if (abs(in.tangent.w) > 0.5) {
        let T  = normalize(in.tangent.xyz - Nv * dot(in.tangent.xyz, Nv));
        let B  = cross(Nv, T) * in.tangent.w;
        let nm = textureSample(t_normal, s_mat, in.uv).xyz * 2.0 - 1.0;
        N = normalize(T * nm.x + B * nm.y + Nv * nm.z);
    }

    let V = normalize(camera.eye.xyz - in.world_pos);
    let shadow = shadow_factor(in.world_pos);
    let color = shade_pbr(N, V, albedo, metallic, roughness, ao, emissive, shadow);
    return vec4<f32>(apply_fog(color, in.world_pos), 1.0);
}
/// Exponential distance fog; density 0 disables.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let dist = length(camera.eye.xyz - world_pos);
    let t = 1.0 - exp(-light.fog_density * dist);
    return mix(color, light.fog_color, t);
}
