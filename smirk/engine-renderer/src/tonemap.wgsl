// HDR → LDR tonemap: fullscreen pass sampling the resolved
// Rgba16Float scene, ACES-fitted curve (Narkowicz) with an exposure uniform.
// `post.encode` decides the transfer function: an sRGB swapchain view does
// its own hardware encode (encode = 0), while a plain-format offscreen
// target needs it done here (encode = 1). `post.passthrough` bypasses ACES
// and encode entirely, for debug channels that must read back as raw values.

//#include "snippets/srgb_oetf.wgsl"

@group(0) @binding(0) var t_hdr: texture_2d<f32>;
@group(0) @binding(1) var s_hdr: sampler;

struct PostParams {
    exposure:    f32,
    bloom:       f32, // bloom intensity (0 = off)
    passthrough: f32, // > 0.5: skip ACES + encode, return the HDR sample as-is
    encode:      f32, // > 0.5: apply the sRGB OETF (offscreen targets only)
}
@group(0) @binding(2) var<uniform> post: PostParams;
@group(0) @binding(3) var t_bloom: texture_2d<f32>;

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

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let scene = textureSample(t_hdr, s_hdr, in.uv);
    if post.passthrough > 0.5 {
        return scene;
    }
    // Bloom is already display-referred (bloom.wgsl's prefilter applies
    // exposure before thresholding) — only hdr needs it here.
    let bloom = textureSample(t_bloom, s_hdr, in.uv).rgb * post.bloom;
    let c = aces(scene.rgb * post.exposure + bloom);
    if post.encode > 0.5 {
        return vec4<f32>(srgb_oetf(c), scene.a);
    }
    return vec4<f32>(c, scene.a);
}
