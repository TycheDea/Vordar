// Skybox pass (VQ-D2): the zone's IBL cubemap rendered as the background.
// Fullscreen triangle pinned to the far plane (z = w ⇒ depth 1.0), depth
// test LessEqual with writes off, so geometry always wins. Unprojects each
// pixel through inv_view_proj to a world-space view ray.

struct Camera {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    right:         vec4<f32>,
    up:            vec4<f32>,
    eye:           vec4<f32>,
}
@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0) var t_sky: texture_cube<f32>;
@group(1) @binding(1) var s_sky: sampler;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       ndc:      vec2<f32>,
}

@vertex
fn vtx_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32((vi >> 1u) & 1u) * 4 - 1);
    out.clip_pos = vec4<f32>(x, y, 1.0, 1.0); // z = w ⇒ far plane
    out.ndc      = vec2<f32>(x, y);
    return out;
}

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Unproject two depths to build the ray (works for both projections).
    let p0 = camera.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let p1 = camera.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(p1.xyz / p1.w - p0.xyz / p0.w);
    return vec4<f32>(textureSample(t_sky, s_sky, dir).rgb, 1.0);
}
