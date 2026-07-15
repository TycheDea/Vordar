// Billboard particles: atlas-textured quads expanded toward the
// camera (or velocity-aligned when the instance carries stretch), soft-faded
// against the scene depth. Color arrives pre-faded from the CPU sim; the
// alpha channel drives the premultiplied-alpha variant, ignored by additive.

struct Camera {
    view_proj:     mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    right:         vec4<f32>, // world-space billboard basis
    up:            vec4<f32>,
    eye:           vec4<f32>,
}
@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0) var t_atlas: texture_2d<f32>;
@group(1) @binding(1) var s_atlas: sampler;
@group(1) @binding(2) var t_depth: texture_depth_multisampled_2d;

struct FxParams {
    viewport:  vec2<f32>,
    fade_range: f32, // world units of the soft depth fade
    _pad:      f32,
}
@group(1) @binding(3)
var<uniform> fx: FxParams;

const ATLAS_GRID: f32 = 4.0;

struct VertexOutput {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0)       color:    vec4<f32>,
    @location(1)       uv:       vec2<f32>,
    @location(2)       world_pos: vec3<f32>,
}

@vertex
fn vtx_main(
    @location(0) pos_size: vec4<f32>,
    @location(1) in_color: vec4<f32>,
    @location(2) stretch:  vec4<f32>,
    @location(3) cell:     vec4<u32>,
    @builtin(vertex_index) vi: u32,
) -> VertexOutput {
    // Triangle-strip corner in {-1,1}².
    let qx = f32(i32(vi & 1u) * 2 - 1);
    let qy = f32(i32((vi >> 1u) & 1u) * 2 - 1);

    let center = pos_size.xyz;
    let size   = pos_size.w;

    // Round billboard: camera basis. Stretched: local x follows the world
    // velocity (projected), local y completes the basis with the view dir.
    var axis_x = camera.right.xyz;
    var axis_y = camera.up.xyz;
    var len_x  = size;
    if (stretch.w > 0.0 && dot(stretch.xyz, stretch.xyz) > 1e-6) {
        let view = normalize(camera.eye.xyz - center);
        let vel  = stretch.xyz - view * dot(stretch.xyz, view); // screen-plane component
        if (dot(vel, vel) > 1e-6) {
            axis_x = normalize(vel);
            axis_y = normalize(cross(view, axis_x));
            len_x  = size * (1.0 + stretch.w);
        }
    }
    let world = center + axis_x * (qx * len_x) + axis_y * (qy * size);

    // Atlas cell uv.
    let c   = f32(cell.x % u32(ATLAS_GRID * ATLAS_GRID));
    let col = c % ATLAS_GRID;
    let row = floor(c / ATLAS_GRID);
    let local = vec2<f32>(qx, qy) * 0.5 + vec2<f32>(0.5);
    let uv = (vec2<f32>(col, row) + local) / ATLAS_GRID;

    var out: VertexOutput;
    out.clip_pos  = camera.view_proj * vec4<f32>(world, 1.0);
    out.color     = in_color;
    out.uv        = uv;
    out.world_pos = world;
    return out;
}

/// World-space distance from the eye for a framebuffer position + depth.
fn eye_distance(frag_xy: vec2<f32>, depth: f32) -> f32 {
    let ndc = vec3<f32>(
        frag_xy.x / fx.viewport.x * 2.0 - 1.0,
        1.0 - frag_xy.y / fx.viewport.y * 2.0,
        depth,
    );
    let p = camera.inv_view_proj * vec4<f32>(ndc, 1.0);
    return length(p.xyz / p.w - camera.eye.xyz);
}

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = textureSample(t_atlas, s_atlas, in.uv).r;

    // Soft particles: fade where the quad nears scene geometry (sample 0 of
    // the MSAA depth is plenty for a fade).
    let coords = vec2<i32>(in.clip_pos.xy);
    let scene_depth = textureLoad(t_depth, coords, 0);
    let scene_dist  = eye_distance(in.clip_pos.xy, scene_depth);
    let frag_dist   = eye_distance(in.clip_pos.xy, in.clip_pos.z);
    let soft = clamp((scene_dist - frag_dist) / max(fx.fade_range, 1e-3), 0.0, 1.0);

    // Premultiplied output: additive ignores alpha; the alpha variant blends
    // with OneMinusSrcAlpha.
    let a = in.color.a * t * soft;
    return vec4<f32>(in.color.rgb * t * soft, a);
}
