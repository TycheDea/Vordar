// Image-based lighting bake passes, all one-time at environment load:
//   equirect_frag   — equirectangular .hdr → one cubemap face
//   irradiance_frag — cosine-convolved diffuse irradiance cube face
//   prefilter_frag  — GGX-prefiltered specular cube face (roughness per mip)
//   brdf_frag       — split-sum BRDF integration LUT (scale, bias) over
//                     (NdotV, roughness)
// Every pass draws the shared fullscreen triangle; `params` selects the cube
// face and roughness.

struct Params {
    face:      u32,
    roughness: f32,
    _pad0:     f32,
    _pad1:     f32,
}
@group(0) @binding(0) var t_src: texture_2d<f32>;   // equirect pass source
@group(0) @binding(1) var s_src: sampler;
@group(0) @binding(2) var<uniform> params: Params;

// Cube-pass variant binds a cubemap source instead (same slots, group 1).
@group(1) @binding(0) var t_cube: texture_cube<f32>;

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

const PI: f32 = 3.14159265;

// Convolution passes (irradiance, prefilter) clamp their base-cube samples so
// the HDRI's baked sun disc — orders of magnitude above the rest of the sky —
// doesn't double-count with the analytic key light or firefly the 256-sample
// GGX prefilter. The visible-sky path (equirect_frag / the base cube) stays
// unclamped so the sun disc still renders.
const IBL_SOURCE_CLAMP: f32 = 25.0;

/// World direction of a texel on cube face `face` at uv ∈ [0,1]² (wgpu/Vulkan
/// face order +X −X +Y −Y +Z −Z).
fn face_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let s = uv * 2.0 - 1.0; // u right, v down
    var d: vec3<f32>;
    switch face {
        case 0u:  { d = vec3<f32>( 1.0, -s.y, -s.x); }
        case 1u:  { d = vec3<f32>(-1.0, -s.y,  s.x); }
        case 2u:  { d = vec3<f32>( s.x,  1.0,  s.y); }
        case 3u:  { d = vec3<f32>( s.x, -1.0, -s.y); }
        case 4u:  { d = vec3<f32>( s.x, -s.y,  1.0); }
        default:  { d = vec3<f32>(-s.x, -s.y, -1.0); }
    }
    return normalize(d);
}

// ── equirect → cube ──────────────────────────────────────────────────────────

@fragment
fn equirect_frag(in: VertexOutput) -> @location(0) vec4<f32> {
    let d  = face_dir(params.face, in.uv);
    let eu = atan2(d.z, d.x) / (2.0 * PI) + 0.5;
    let ev = acos(clamp(d.y, -1.0, 1.0)) / PI;
    return vec4<f32>(textureSampleLevel(t_src, s_src, vec2<f32>(eu, ev), 0.0).rgb, 1.0);
}

// ── diffuse irradiance ───────────────────────────────────────────────────────

@fragment
fn irradiance_frag(in: VertexOutput) -> @location(0) vec4<f32> {
    let N = face_dir(params.face, in.uv);
    // Tangent frame around N.
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(N.y) > 0.99) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let T = normalize(cross(up, N));
    let B = cross(N, T);

    // Cosine-weighted hemisphere sum over a fixed spherical grid.
    var sum = vec3<f32>(0.0);
    var count = 0.0;
    for (var phi = 0.0; phi < 2.0 * PI; phi += PI / 16.0) {
        for (var theta = 0.0; theta < 0.5 * PI; theta += PI / 32.0) {
            let dir = cos(phi) * sin(theta) * T
                    + sin(phi) * sin(theta) * B
                    + cos(theta) * N;
            sum += min(textureSampleLevel(t_cube, s_src, dir, 0.0).rgb, vec3<f32>(IBL_SOURCE_CLAMP)) * cos(theta) * sin(theta);
            count += 1.0;
        }
    }
    return vec4<f32>(PI * sum / count, 1.0);
}

// ── GGX specular prefilter ───────────────────────────────────────────────────

fn radical_inverse_vdc(bits_in: u32) -> f32 {
    var bits = bits_in;
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}

fn hammersley(i: u32, n: u32) -> vec2<f32> {
    return vec2<f32>(f32(i) / f32(n), radical_inverse_vdc(i));
}

fn importance_sample_ggx(xi: vec2<f32>, n: vec3<f32>, roughness: f32) -> vec3<f32> {
    let a = roughness * roughness;
    let phi = 2.0 * PI * xi.x;
    let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
    let sin_theta = sqrt(1.0 - cos_theta * cos_theta);
    let h = vec3<f32>(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta);

    var up = vec3<f32>(0.0, 0.0, 1.0);
    if (abs(n.z) > 0.999) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let tangent   = normalize(cross(up, n));
    let bitangent = cross(n, tangent);
    return normalize(tangent * h.x + bitangent * h.y + n * h.z);
}

@fragment
fn prefilter_frag(in: VertexOutput) -> @location(0) vec4<f32> {
    let N = face_dir(params.face, in.uv);
    let R = N;
    let V = N;

    const SAMPLES: u32 = 256u;
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0u; i < SAMPLES; i++) {
        let xi = hammersley(i, SAMPLES);
        let H  = importance_sample_ggx(xi, N, params.roughness);
        let L  = normalize(2.0 * dot(V, H) * H - V);
        let NdotL = dot(N, L);
        if (NdotL > 0.0) {
            sum += min(textureSampleLevel(t_cube, s_src, L, 0.0).rgb, vec3<f32>(IBL_SOURCE_CLAMP)) * NdotL;
            weight += NdotL;
        }
    }
    return vec4<f32>(sum / max(weight, 1e-4), 1.0);
}

// ── split-sum BRDF LUT ───────────────────────────────────────────────────────

fn g_smith_ibl(NdotV: f32, NdotL: f32, roughness: f32) -> f32 {
    // k for IBL: a²/2 (Karis).
    let a = roughness * roughness;
    let k = a * a / 2.0;
    let gv = NdotV / (NdotV * (1.0 - k) + k);
    let gl = NdotL / (NdotL * (1.0 - k) + k);
    return gv * gl;
}

@fragment
fn brdf_frag(in: VertexOutput) -> @location(0) vec4<f32> {
    // LUT is sampled at vec2(NdotV, roughness) with the same uv convention
    // the bake writes, so no flip.
    let NdotV = max(in.uv.x, 1e-3);
    let roughness = in.uv.y;
    let V = vec3<f32>(sqrt(1.0 - NdotV * NdotV), 0.0, NdotV);
    let N = vec3<f32>(0.0, 0.0, 1.0);

    const SAMPLES: u32 = 512u;
    var scale = 0.0;
    var bias  = 0.0;
    for (var i = 0u; i < SAMPLES; i++) {
        let xi = hammersley(i, SAMPLES);
        let H  = importance_sample_ggx(xi, N, roughness);
        let L  = normalize(2.0 * dot(V, H) * H - V);
        let NdotL = max(L.z, 0.0);
        if (NdotL > 0.0) {
            let NdotH = max(H.z, 0.0);
            let VdotH = max(dot(V, H), 0.0);
            let g = g_smith_ibl(NdotV, NdotL, roughness);
            let g_vis = g * VdotH / (NdotH * NdotV);
            let fc = pow(1.0 - VdotH, 5.0);
            scale += (1.0 - fc) * g_vis;
            bias  += fc * g_vis;
        }
    }
    return vec4<f32>(scale / f32(SAMPLES), bias / f32(SAMPLES), 0.0, 1.0);
}
