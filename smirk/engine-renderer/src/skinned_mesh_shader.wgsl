// Skinned mesh pass — linear-blend skinning on top of the static mesh shader.
// Same camera/light (group 0) and base-color texture (group 1); adds a
// joint-palette storage buffer (group 2). Each instance owns a contiguous
// block of joint matrices indexed by `joint_base`, so instancing survives.

@group(0) @binding(0)
var<uniform> view_proj: mat4x4<f32>;

struct LightUniform {
    direction: vec3<f32>,
    _pad:      f32,
    color:     vec3<f32>,
    ambient:   f32,
}
@group(0) @binding(1)
var<uniform> light: LightUniform;

@group(1) @binding(0) var t_color: texture_2d<f32>;
@group(1) @binding(1) var s_color: sampler;

@group(2) @binding(0) var<storage, read> joints: array<mat4x4<f32>>;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       normal:   vec3<f32>,
    @location(1)       uv:       vec2<f32>,
    @location(2)       tint:     vec4<f32>,
}

@vertex
fn vtx_main(
    // per-vertex
    @location(0) position:   vec3<f32>,
    @location(1) in_normal:  vec3<f32>,
    @location(2) in_uv:      vec2<f32>,
    @location(3) in_joints:  vec4<u32>,
    @location(4) in_weights: vec4<f32>,
    // per-instance
    @location(5) model_0:    vec4<f32>,
    @location(6) model_1:    vec4<f32>,
    @location(7) model_2:    vec4<f32>,
    @location(8) model_3:    vec4<f32>,
    @location(9) inst_tint:  vec4<f32>,
    @location(10) joint_base: u32,
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
    out.clip_pos = view_proj * world;
    out.normal   = normalize(norm_mat * (skin3 * in_normal));
    out.uv       = in_uv;
    out.tint     = inst_tint;
    return out;
}

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex  = textureSample(t_color, s_color, in.uv);
    let base = tex.rgb * in.tint.rgb;

    let N    = normalize(in.normal);
    let diff = max(dot(N, light.direction), 0.0);
    let lit  = light.ambient + diff * (1.0 - light.ambient);

    return vec4<f32>(base * lit * light.color, 1.0);
}
