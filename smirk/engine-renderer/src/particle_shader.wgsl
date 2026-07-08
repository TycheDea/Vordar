// Billboard particles: a per-instance quad expanded toward the camera, shaded
// as a procedural soft disc and blended additively (One+One). Color arrives
// pre-faded from the CPU sim, so the fragment shader only shapes the falloff.

struct Camera {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    right:         vec4<f32>, // world-space billboard basis
    up:            vec4<f32>,
}
@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       color:    vec4<f32>,
    @location(1)       corner:   vec2<f32>, // -1..1 quad coords
}

@vertex
fn vtx_main(
    @builtin(vertex_index) vi: u32,
    @location(0) pos_size: vec4<f32>, // xyz = world center, w = half-extent
    @location(1) color:    vec4<f32>,
) -> VertexOutput {
    // Triangle-strip corners: (-1,-1) (1,-1) (-1,1) (1,1).
    let cx = f32(vi & 1u) * 2.0 - 1.0;
    let cy = f32(vi >> 1u) * 2.0 - 1.0;
    let world = pos_size.xyz
        + (camera.right.xyz * cx + camera.up.xyz * cy) * pos_size.w;

    var out: VertexOutput;
    out.clip_pos = camera.view_proj * vec4<f32>(world, 1.0);
    out.color = color;
    out.corner = vec2<f32>(cx, cy);
    return out;
}

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Soft disc: quadratic falloff from center to the quad edge. Additive
    // blend ignores alpha; energy lives in the RGB.
    let k = max(1.0 - dot(in.corner, in.corner), 0.0);
    return vec4<f32>(in.color.rgb * k * k, 1.0);
}
