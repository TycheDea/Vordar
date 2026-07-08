// Dual-filter Kawase bloom (VQ-C3 payoff): soft-knee prefilter from the HDR
// resolve, then a downsample/upsample chain (Bjørge dual-filter taps). The
// upsample legs blend additively; the tonemap pass composites the result.

@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var s_src: sampler;

struct Params {
    // x = threshold, y = knee (prefilter); x = source texel w, y = texel h
    // (down/up passes).
    a: f32,
    b: f32,
    _pad0: f32,
    _pad1: f32,
}
@group(0) @binding(2) var<uniform> params: Params;

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

// ── soft-knee prefilter ──────────────────────────────────────────────────────

@fragment
fn prefilter_frag(in: VertexOutput) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(t_src, s_src, in.uv, 0.0).rgb;
    let threshold = params.a;
    let knee      = max(params.b, 1e-4);
    let brightness = max(c.r, max(c.g, c.b));
    var soft = clamp(brightness - threshold + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee);
    let contribution = max(soft, brightness - threshold) / max(brightness, 1e-4);
    return vec4<f32>(c * max(contribution, 0.0), 1.0);
}

// ── dual-filter downsample (5 taps) ──────────────────────────────────────────

@fragment
fn down_frag(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = vec2<f32>(params.a, params.b); // source texel
    var sum = textureSampleLevel(t_src, s_src, in.uv, 0.0).rgb * 4.0;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>( d.x,  d.y), 0.0).rgb;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(-d.x,  d.y), 0.0).rgb;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>( d.x, -d.y), 0.0).rgb;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(-d.x, -d.y), 0.0).rgb;
    return vec4<f32>(sum / 8.0, 1.0);
}

// ── dual-filter upsample (8 taps, blended additively by the pipeline) ────────

@fragment
fn up_frag(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = vec2<f32>(params.a, params.b); // source texel
    var sum = vec3<f32>(0.0);
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(-d.x * 2.0, 0.0), 0.0).rgb;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(-d.x,  d.y), 0.0).rgb * 2.0;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(0.0,  d.y * 2.0), 0.0).rgb;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>( d.x,  d.y), 0.0).rgb * 2.0;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>( d.x * 2.0, 0.0), 0.0).rgb;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>( d.x, -d.y), 0.0).rgb * 2.0;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(0.0, -d.y * 2.0), 0.0).rgb;
    sum += textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(-d.x, -d.y), 0.0).rgb * 2.0;
    return vec4<f32>(sum / 12.0, 1.0);
}
