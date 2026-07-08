// Skinned mesh pass — linear-blend skinning on top of the PBR mesh shader.
// Same camera/light (group 0) and material set (group 1); adds a
// joint-palette storage buffer (group 2). Each instance owns a contiguous
// block of joint matrices indexed by `joint_base`, so instancing survives.

struct Camera {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    right:         vec4<f32>,
    up:            vec4<f32>,
    eye:           vec4<f32>,
}
@group(0) @binding(0)
var<uniform> camera: Camera;

struct LightUniform {
    direction: vec3<f32>,
    _pad:      f32,
    color:     vec3<f32>,
    ambient:   f32,
}
@group(0) @binding(1)
var<uniform> light: LightUniform;

// Shadow receiving (VQ-D3) — shared scene group.
@group(0) @binding(2) var<uniform> light_vp: mat4x4<f32>;
@group(0) @binding(3) var t_shadow: texture_depth_2d;
@group(0) @binding(4) var s_shadow: sampler_comparison;

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

@group(1) @binding(0) var t_albedo:   texture_2d<f32>;
@group(1) @binding(1) var s_mat:      sampler;
@group(1) @binding(2) var t_normal:   texture_2d<f32>;
@group(1) @binding(3) var t_mr:       texture_2d<f32>;
@group(1) @binding(4) var t_emissive: texture_2d<f32>;
@group(1) @binding(5) var t_ao:       texture_2d<f32>;

struct MaterialUniform {
    base_color: vec4<f32>,
    emissive:   vec4<f32>,
    mr:         vec4<f32>,
}
@group(1) @binding(6)
var<uniform> material: MaterialUniform;

@group(2) @binding(0) var<storage, read> joints: array<mat4x4<f32>>;

// ── Environment (group 3) — IBL ambient (VQ-D2) ─────────────────────────────

@group(3) @binding(0) var t_irradiance: texture_cube<f32>;
@group(3) @binding(1) var t_prefilter:  texture_cube<f32>;
@group(3) @binding(2) var t_brdf:       texture_2d<f32>;
@group(3) @binding(3) var s_env:        sampler;

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
    @location(4) in_joints:  vec4<u32>,
    @location(5) in_weights: vec4<f32>,
    // per-instance
    @location(6) model_0:    vec4<f32>,
    @location(7) model_1:    vec4<f32>,
    @location(8) model_2:    vec4<f32>,
    @location(9) model_3:    vec4<f32>,
    @location(10) inst_tint: vec4<f32>,
    @location(11) joint_base: u32,
) -> VertexOutput {
    // Blend the four influencing joint matrices by weight.
    let skin = in_weights.x * joints[joint_base + in_joints.x]
             + in_weights.y * joints[joint_base + in_joints.y]
             + in_weights.z * joints[joint_base + in_joints.z]
             + in_weights.w * joints[joint_base + in_joints.w];

    let model   = mat4x4<f32>(model_0, model_1, model_2, model_3);
    let skinned = skin * vec4<f32>(position, 1.0);
    let world   = model * skinned;

    // Normal: skin rotation (upper 3×3), then the model's inverse-transpose
    // (col / |col|² — the same TRS trick as the static pass).
    let skin3 = mat3x3<f32>(skin[0].xyz, skin[1].xyz, skin[2].xyz);
    let mc0 = model[0].xyz;
    let mc1 = model[1].xyz;
    let mc2 = model[2].xyz;
    let norm_mat = mat3x3<f32>(mc0 / dot(mc0, mc0), mc1 / dot(mc1, mc1), mc2 / dot(mc2, mc2));

    var out: VertexOutput;
    out.clip_pos  = camera.view_proj * world;
    out.normal    = normalize(norm_mat * (skin3 * in_normal));
    out.uv        = in_uv;
    out.tint      = inst_tint;
    out.world_pos = world.xyz;
    out.tangent   = vec4<f32>(normalize(norm_mat * (skin3 * in_tangent.xyz)), in_tangent.w);
    return out;
}

// ── Cook-Torrance GGX (same math as mesh_shader.wgsl — WGSL has no includes) ─

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

    // IBL ambient (same as mesh_shader.wgsl).
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
    return vec4<f32>(shade_pbr(N, V, albedo, metallic, roughness, ao, emissive, shadow), 1.0);
}
