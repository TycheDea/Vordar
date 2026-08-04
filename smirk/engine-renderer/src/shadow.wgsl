// Shadow depth pre-pass: scene geometry rendered from the sun's
// fitted orthographic view into a depth-only target. Three vertex entry
// points — one per geometry pipeline's vertex/instance layout. No fragment
// stage; slope-scaled bias lives in the pipeline's DepthBiasState.

@group(0) @binding(0)
var<uniform> light_vp: mat4x4<f32>;

// Skinned variant only (group 1 = the shared joint palette).
@group(1) @binding(0) var<storage, read> joints: array<mat4x4<f32>>;

// ── MASK material variants ────────────────────────────────────────────────
// glTF alphaMode MASK geometry must not cast a solid-quad shadow: the
// fragment stage discards cutout texels so the cascade's depth-only target
// is left unwritten there, matching the main pass's cutout silhouette. The
// static pipeline binds material at group 1 (group 0 is light_vp); the
// skinned pipeline binds it at group 2 (group 1 is already the joint
// palette above).

struct MaskedOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

struct MaskMaterial {
    base_color: vec4<f32>,
    emissive:   vec4<f32>,
    mr:         vec4<f32>, // z = mask cutoff
}

@group(1) @binding(0) var mesh_albedo:  texture_2d<f32>;
@group(1) @binding(1) var mesh_sampler: sampler;
@group(1) @binding(6) var<uniform> mesh_material: MaskMaterial;

@group(2) @binding(0) var skin_albedo:  texture_2d<f32>;
@group(2) @binding(1) var skin_sampler: sampler;
@group(2) @binding(6) var<uniform> skin_material: MaskMaterial;

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

// ── static mesh, MASK material ────────────────────────────────────────────

@vertex
fn mesh_vtx_masked(
    @location(0) position: vec3<f32>,
    @location(1) uv:       vec2<f32>,
    @location(4) model_0:  vec4<f32>,
    @location(5) model_1:  vec4<f32>,
    @location(6) model_2:  vec4<f32>,
    @location(7) model_3:  vec4<f32>,
) -> MaskedOut {
    let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
    var out: MaskedOut;
    out.clip_pos = light_vp * (model * vec4<f32>(position, 1.0));
    out.uv = uv;
    return out;
}

@fragment
fn mesh_frag_masked(in: MaskedOut) {
    let alpha = textureSample(mesh_albedo, mesh_sampler, in.uv).a * mesh_material.base_color.a;
    if alpha < mesh_material.mr.z {
        discard;
    }
}

// ── skinned mesh, MASK material ───────────────────────────────────────────

@vertex
fn skinned_vtx_masked(
    @location(0) position:   vec3<f32>,
    @location(1) uv:         vec2<f32>,
    @location(4) in_joints:  vec4<u32>,
    @location(5) in_weights: vec4<f32>,
    @location(6) model_0:    vec4<f32>,
    @location(7) model_1:    vec4<f32>,
    @location(8) model_2:    vec4<f32>,
    @location(9) model_3:    vec4<f32>,
    @location(11) joint_base: u32,
) -> MaskedOut {
    let skin = in_weights.x * joints[joint_base + in_joints.x]
             + in_weights.y * joints[joint_base + in_joints.y]
             + in_weights.z * joints[joint_base + in_joints.z]
             + in_weights.w * joints[joint_base + in_joints.w];
    let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
    var out: MaskedOut;
    out.clip_pos = light_vp * (model * (skin * vec4<f32>(position, 1.0)));
    out.uv = uv;
    return out;
}

@fragment
fn skinned_frag_masked(in: MaskedOut) {
    let alpha = textureSample(skin_albedo, skin_sampler, in.uv).a * skin_material.base_color.a;
    if alpha < skin_material.mr.z {
        discard;
    }
}
