// SSAO: reconstructs position/normal from the depth prepass and darkens
// creases where nearby geometry blocks the hemisphere above a surface. AO
// multiplies IBL ambient only — nothing samples this texture yet.
//
// `ssao_frag` and `blur_frag` share `@group(1) @binding(0)` for their own
// input texture (depth vs. raw AO) — each entry point only reaches one of
// the two declarations, which WGSL validates per entry point, not per module.

//#include "snippets/scene_uniforms.wgsl"

struct SsaoParams {
    screen_size: vec2<f32>, // blur target dimensions (half-res), texel clamp
    radius:      f32,       // world-space sample radius, metres
    bias:        f32,       // depth bias against acne
}

const KERNEL_SIZE: u32 = 16u;
const KERNEL: array<vec3<f32>, 16> = array<vec3<f32>, 16>(
    vec3<f32>(-0.0452, -0.0829,  0.0328),
    vec3<f32>(-0.0470,  0.0828,  0.0407),
    vec3<f32>(-0.0263, -0.0666,  0.0888),
    vec3<f32>(-0.0399,  0.1143,  0.0517),
    vec3<f32>(-0.0007, -0.1022,  0.1182),
    vec3<f32>(-0.0414,  0.1518,  0.1027),
    vec3<f32>( 0.1746,  0.0772,  0.1221),
    vec3<f32>(-0.2597,  0.0313,  0.0756),
    vec3<f32>( 0.2833,  0.1549,  0.0373),
    vec3<f32>( 0.1812, -0.0338,  0.3377),
    vec3<f32>(-0.0945,  0.1293,  0.4222),
    vec3<f32>(-0.0764,  0.5035,  0.1291),
    vec3<f32>(-0.4999, -0.1197,  0.3214),
    vec3<f32>( 0.2071, -0.5550,  0.3618),
    vec3<f32>(-0.4056, -0.6099,  0.2933),
    vec3<f32>(-0.3136,  0.2603,  0.7924),
);

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

@vertex
fn vtx_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32((vi >> 1u) & 1u) * 4 - 1);
    out.clip_pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv       = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

fn hash12(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(12.9898, 78.233));
    return fract(sin(h) * 43758.5453);
}

// A per-pixel unit vector spread over the full sphere (not confined to one
// plane): world-space normals are frequently axis-aligned (a ground's +Y, a
// box wall's ±X/±Z), and a rotation seed built from only two fixed axes
// degenerates against those — Gram-Schmidt cancels it to a single fixed
// axis instead of actually rotating.
fn hash_sphere(p: vec2<f32>) -> vec3<f32> {
    let angle = hash12(p) * 6.2831853;
    let z     = hash12(p + vec2<f32>(37.0, 17.0)) * 2.0 - 1.0;
    let r     = sqrt(max(1.0 - z * z, 0.0));
    return vec3<f32>(r * cos(angle), r * sin(angle), z);
}

fn depth_to_world(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc   = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = camera.inv_view_proj * ndc;
    return world.xyz / world.w;
}

// ── SSAO: hemisphere kernel occlusion against the depth prepass ──────────────

@group(1) @binding(0) var t_depth: texture_depth_2d;
@group(1) @binding(1) var<uniform> params: SsaoParams;

fn depth_at(uv: vec2<f32>) -> f32 {
    let dims  = vec2<f32>(textureDimensions(t_depth, 0));
    let texel = vec2<i32>(clamp(uv, vec2<f32>(0.0), vec2<f32>(0.999999)) * dims);
    return textureLoad(t_depth, texel, 0);
}

@fragment
fn ssao_frag(in: VertexOutput) -> @location(0) f32 {
    let depth = depth_at(in.uv);
    if (depth >= 1.0) {
        return 1.0; // sky / far plane: fully unoccluded
    }
    let world_pos = depth_to_world(in.uv, depth);

    // Forward is recoverable from the orthonormal (right, up) basis already
    // carried by CameraUniform, so no extra field is needed.
    let forward = normalize(cross(camera.up.xyz, camera.right.xyz));
    var normal = normalize(cross(dpdy(world_pos), dpdx(world_pos)));
    if (dot(normal, camera.eye.xyz - world_pos) < 0.0) {
        normal = -normal;
    }

    let rand_vec  = hash_sphere(in.clip_pos.xy);
    let tangent   = normalize(rand_vec - normal * dot(rand_vec, normal));
    let bitangent = cross(normal, tangent);
    let tbn       = mat3x3<f32>(tangent, bitangent, normal);

    let current_lin = dot(world_pos - camera.eye.xyz, forward);
    var occlusion = 0.0;
    for (var i = 0u; i < KERNEL_SIZE; i = i + 1u) {
        let sample_pos  = world_pos + (tbn * KERNEL[i]) * params.radius;
        let sample_clip = camera.view_proj * vec4<f32>(sample_pos, 1.0);
        let sample_ndc  = sample_clip.xyz / sample_clip.w;
        let sample_uv   = vec2<f32>(sample_ndc.x * 0.5 + 0.5, 0.5 - sample_ndc.y * 0.5);
        if (sample_uv.x < 0.0 || sample_uv.x > 1.0 || sample_uv.y < 0.0 || sample_uv.y > 1.0) {
            continue;
        }

        let actual_world = depth_to_world(sample_uv, depth_at(sample_uv));
        let actual_lin    = dot(actual_world - camera.eye.xyz, forward);
        let sample_lin    = dot(sample_pos - camera.eye.xyz, forward);

        let range_check = clamp(params.radius / max(abs(current_lin - actual_lin), 1e-4), 0.0, 1.0);
        if (actual_lin <= sample_lin - params.bias) {
            occlusion += range_check;
        }
    }
    return 1.0 - occlusion / f32(KERNEL_SIZE);
}

// ── Blur: 3×3 box filter denoising the raw AO ────────────────────────────────

@group(1) @binding(0) var t_ao: texture_2d<f32>;

@fragment
fn blur_frag(in: VertexOutput) -> @location(0) f32 {
    let dims  = vec2<i32>(params.screen_size);
    let coord = vec2<i32>(in.clip_pos.xy);
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = clamp(coord + vec2<i32>(dx, dy), vec2<i32>(0), dims - vec2<i32>(1));
            sum += textureLoad(t_ao, c, 0).r;
        }
    }
    return sum / 9.0;
}
