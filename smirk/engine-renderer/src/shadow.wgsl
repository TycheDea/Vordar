// Shadow depth pre-pass: scene geometry rendered from the sun's
// fitted orthographic view into a depth-only target. Three vertex entry
// points — one per geometry pipeline's vertex/instance layout. No fragment
// stage; slope-scaled bias lives in the pipeline's DepthBiasState.

@group(0) @binding(0)
var<uniform> light_vp: mat4x4<f32>;

// Skinned variant only (group 1 = the shared joint palette).
@group(1) @binding(0) var<storage, read> joints: array<mat4x4<f32>>;

// ── SDF primitive layout (unit cube + SdfInstance) ───────────────────────────

@vertex
fn sdf_vtx(
    @location(0) position: vec3<f32>,
    @location(3) model_0:  vec4<f32>,
    @location(4) model_1:  vec4<f32>,
    @location(5) model_2:  vec4<f32>,
    @location(6) model_3:  vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
    return light_vp * (model * vec4<f32>(position, 1.0));
}

// ── static mesh layout (MeshVertex + MeshInstance) ───────────────────────────

@vertex
fn mesh_vtx(
    @location(0) position: vec3<f32>,
    @location(4) model_0:  vec4<f32>,
    @location(5) model_1:  vec4<f32>,
    @location(6) model_2:  vec4<f32>,
    @location(7) model_3:  vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
    return light_vp * (model * vec4<f32>(position, 1.0));
}

// ── skinned mesh layout (SkinnedVertex + SkinnedMeshInstance + palette) ──────

@vertex
fn skinned_vtx(
    @location(0) position:   vec3<f32>,
    @location(4) in_joints:  vec4<u32>,
    @location(5) in_weights: vec4<f32>,
    @location(6) model_0:    vec4<f32>,
    @location(7) model_1:    vec4<f32>,
    @location(8) model_2:    vec4<f32>,
    @location(9) model_3:    vec4<f32>,
    @location(11) joint_base: u32,
) -> @builtin(position) vec4<f32> {
    let skin = in_weights.x * joints[joint_base + in_joints.x]
             + in_weights.y * joints[joint_base + in_joints.y]
             + in_weights.z * joints[joint_base + in_joints.z]
             + in_weights.w * joints[joint_base + in_joints.w];
    let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
    return light_vp * (model * (skin * vec4<f32>(position, 1.0)));
}
