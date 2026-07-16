// Skybox pass: the zone's IBL cubemap rendered as the background.
// Fullscreen triangle pinned to the far plane (z = w ⇒ depth 1.0), depth
// test LessEqual with writes off, so geometry always wins. Unprojects each
// pixel through inv_view_proj to a world-space view ray.

//#include "snippets/scene_uniforms.wgsl"

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

// Elevation falloff: dies out by ~15° (sin 15° ≈ 0.26, exp(-0.26 * 12) ≈ 0.04).
const HORIZON_FALLOFF: f32 = 12.0;
// Zone fog densities run ~0.005 (content/zones/zones.ron) — this scale
// saturates density_on to 1 for any zone that opts in, while fog_density 0
// keeps density_on exactly 0 so the unfogged image stays bit-stable.
const DENSITY_ON_SCALE: f32 = 1000.0;

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Unproject two depths to build the ray (works for both projections).
    let p0 = camera.inv_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let p1 = camera.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(p1.xyz / p1.w - p0.xyz / p0.w);
    let sky = textureSample(t_sky, s_sky, dir).rgb;

    let density_on = saturate(light.fog_density * DENSITY_ON_SCALE);
    let horizon     = saturate(exp(-max(dir.y, 0.0) * HORIZON_FALLOFF)) * density_on;
    return vec4<f32>(mix(sky, light.fog_color, horizon), 1.0);
}
