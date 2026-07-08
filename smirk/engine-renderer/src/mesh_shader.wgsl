// Textured mesh pass — full PBR: Cook-Torrance GGX with normal mapping,
// metallic-roughness, emissive and AO (VQ-A2/C2/C4). Same camera/light
// uniforms as the other geometry passes so mixed scenes light consistently.

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

// Shadow receiving (VQ-D3) — shared scene group.
@group(0) @binding(2) var<uniform> light_vp: mat4x4<f32>;
@group(0) @binding(3) var t_shadow: texture_depth_2d;
@group(0) @binding(4) var s_shadow: sampler_comparison;

/// PCF 3×3 over the fitted sun map; 1.0 = fully lit. Points outside the
/// fitted volume are lit (the map only covers the play area).
fn shadow_factor(world_pos: vec3<f32>) -> f32 {
    let lp  = light_vp * vec4<f32>(world_pos, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv  = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0 || ndc.z <= 0.0) {
        return 1.0;
    }
    let texel = 1.0 / 2048.0;
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let offset = vec2<f32>(f32(dx), f32(dy)) * texel;
            sum += textureSampleCompareLevel(t_shadow, s_shadow, uv + offset, ndc.z);
        }
    }
    return sum / 9.0;
}

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

// ── Environment (group 2) — IBL ambient (VQ-D2) ─────────────────────────────

@group(2) @binding(0) var t_irradiance: texture_cube<f32>;
@group(2) @binding(1) var t_prefilter:  texture_cube<f32>;
@group(2) @binding(2) var t_brdf:       texture_2d<f32>;
@group(2) @binding(3) var s_env:        sampler;

const PREFILTER_MAX_MIP: f32 = 4.0; // PREFILTER_MIPS - 1

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

// ── Cook-Torrance GGX ────────────────────────────────────────────────────────

const PI: f32 = 3.14159265;

fn d_ggx(NdotH: f32, rough: f32) -> f32 {
    let a  = rough * rough;
    let a2 = a * a;
    let d  = NdotH * NdotH * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

fn g_smith(NdotV: f32, NdotL: f32, rough: f32) -> f32 {
    let r  = rough + 1.0;
    let k  = r * r / 8.0;
    let gv = NdotV / (NdotV * (1.0 - k) + k);
    let gl = NdotL / (NdotL * (1.0 - k) + k);
    return gv * gl;
}

fn f_schlick(VdotH: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - VdotH, 5.0);
}

/// Shared shading: albedo already tinted and in linear space. `shadow`
/// attenuates the direct sun term only (1.0 = fully lit).
fn shade_pbr(
    N: vec3<f32>, V: vec3<f32>, albedo: vec3<f32>,
    metallic: f32, roughness: f32, ao: f32, emissive: vec3<f32>, shadow: f32,
) -> vec3<f32> {
    let L = light.direction;
    let H = normalize(V + L);
    let NdotL = max(dot(N, L), 0.0);
    let NdotV = max(dot(N, V), 1e-4);
    let NdotH = max(dot(N, H), 0.0);
    let VdotH = max(dot(V, H), 0.0);

    let rough = clamp(roughness, 0.045, 1.0);
    let f0    = mix(vec3<f32>(0.04), albedo, metallic);

    let d = d_ggx(NdotH, rough);
    let g = g_smith(NdotV, NdotL, rough);
    let f = f_schlick(VdotH, f0);

    let specular = d * g * f / max(4.0 * NdotV * NdotL, 1e-4);
    let kd       = (vec3<f32>(1.0) - f) * (1.0 - metallic);

    let direct  = (kd * albedo + specular) * NdotL * light.color * shadow;

    // IBL ambient: diffuse irradiance + prefiltered specular with the
    // split-sum BRDF. light.ambient scales it (the day/night seam).
    let irr     = textureSample(t_irradiance, s_env, N).rgb;
    let diffuse = irr * albedo * (1.0 - metallic);
    let R       = reflect(-V, N);
    let pre     = textureSampleLevel(t_prefilter, s_env, R, rough * PREFILTER_MAX_MIP).rgb;
    let ab      = textureSample(t_brdf, s_env, vec2<f32>(NdotV, rough)).rg;
    let spec_ibl = pre * (f0 * ab.x + vec3<f32>(ab.y));
    let ambient  = light.ambient * (diffuse + spec_ibl) * ao;

    return ambient + direct + emissive;
}

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let albedo_s = textureSample(t_albedo, s_mat, in.uv);
    let albedo   = albedo_s.rgb * material.base_color.rgb * in.tint.rgb;

    let mr        = textureSample(t_mr, s_mat, in.uv);
    let metallic  = mr.b * material.mr.x;
    let roughness = mr.g * material.mr.y;
    let ao        = textureSample(t_ao, s_mat, in.uv).r;
    let emissive  = textureSample(t_emissive, s_mat, in.uv).rgb * material.emissive.rgb;

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
/// Exponential distance fog (VQ-A5); density 0 disables.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let dist = length(camera.eye.xyz - world_pos);
    let t = 1.0 - exp(-light.fog_density * dist);
    return mix(color, light.fog_color, t);
}

