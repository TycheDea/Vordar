// Skinned mesh pass — linear-blend skinning on top of the PBR mesh shader.
// Same camera/light (group 0) and material set (group 1); adds a
// joint-palette storage buffer (group 2). Each instance owns a contiguous
// block of joint matrices indexed by `joint_base`, so instancing survives.

//#include "snippets/scene_uniforms.wgsl"

//#include "snippets/shadow_sample.wgsl"

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

// ── Environment (group 3) — IBL ambient ─────────────────────────────────────

@group(3) @binding(0) var t_irradiance: texture_cube<f32>;
@group(3) @binding(1) var t_prefilter:  texture_cube<f32>;
@group(3) @binding(2) var t_brdf:       texture_2d<f32>;
@group(3) @binding(3) var s_env:        sampler;

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

    let alpha_test = albedo_s.a * material.base_color.a;
    var out_alpha = 1.0;
    if material.mr.w > 0.5 {
        // BLEND: real coverage; rgb premultiplied at return.
        out_alpha = alpha_test;
    } else if material.mr.z > 0.0 {
        // Alpha cutoff (glTF MASK) — mr.z of 0 means opaque, no cutout.
        // fwidth() must run before any discard so a fragment quad's
        // derivative stays defined; a cutout material sharpens alpha to a
        // per-sample coverage value instead of a binary keep/discard, letting
        // alpha-to-coverage anti-alias the cutout edge. A texel with no raw
        // alpha still discards outright — zero coverage has nothing to blend.
        out_alpha = saturate((alpha_test - material.mr.z) / max(fwidth(alpha_test), 1e-4) + 0.5);
        if alpha_test <= 0.0 {
            discard;
        }
    }

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
    let color = shade_pbr(in.world_pos, N, V, albedo, metallic, roughness, ao, emissive, shadow);
    var rgb = apply_fog(color, in.world_pos);
    if material.mr.w > 0.5 { rgb = rgb * out_alpha; }
    return vec4<f32>(rgb, out_alpha);
}
/// Exponential distance fog; density 0 disables.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let dist = length(camera.eye.xyz - world_pos);
    let t = 1.0 - exp(-light.fog_density * dist);
    return mix(color, light.fog_color, t);
}
