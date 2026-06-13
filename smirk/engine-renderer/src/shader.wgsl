// ── Uniforms ──────────────────────────────────────────────────────────────────

@group(0) @binding(0)
var<uniform> view_proj: mat4x4<f32>;

struct LightUniform {
    direction: vec3<f32>, // world-space, normalised, pointing TOWARD light
    _pad:      f32,
    color:     vec3<f32>,
    ambient:   f32,
}
@group(0) @binding(1)
var<uniform> light: LightUniform;

// ── Texture (group 1) ─────────────────────────────────────────────────────────

@group(1) @binding(0) var t_color: texture_2d<f32>;
@group(1) @binding(1) var s_color: sampler;

// ── Vertex I/O ────────────────────────────────────────────────────────────────

struct VertexOutput {
    @builtin(position)               clip_pos:     vec4<f32>,
    @location(0)                     color:        vec3<f32>,
    @location(1) @interpolate(flat)  shape_type:   u32,
    @location(2)                     local_pos:    vec3<f32>,
    @location(3)                     normal:       vec3<f32>,
    @location(4)                     world_pos:    vec3<f32>,
    @location(5)                     shape_params: vec4<f32>,
    @location(6)                     uv:           vec2<f32>,
}

@vertex
fn vtx_main(
    // per-vertex
    @location(0)                    position:     vec3<f32>,
    @location(1)                    in_normal:    vec3<f32>,
    @location(2)                    in_uv:        vec2<f32>,
    // per-instance (locations shifted +1 to make room for UV)
    @location(3)                    model_0:      vec4<f32>,
    @location(4)                    model_1:      vec4<f32>,
    @location(5)                    model_2:      vec4<f32>,
    @location(6)                    model_3:      vec4<f32>,
    @location(7)                    inst_color:   vec3<f32>,
    @location(8) @interpolate(flat) shape_type:   u32,
    @location(9)                    shape_params: vec4<f32>,
) -> VertexOutput {
    let model   = mat4x4<f32>(model_0, model_1, model_2, model_3);
    let world   = model * vec4<f32>(position, 1.0);
    // Normal transform: inverse-transpose of the upper-left 3×3 (R * S^-1).
    // For TRS matrices S^-1 = col / |col|^2 — avoids WGSL inverse() which is unavailable.
    let col0     = model[0].xyz;
    let col1     = model[1].xyz;
    let col2     = model[2].xyz;
    let norm_mat = mat3x3<f32>(col0 / dot(col0, col0), col1 / dot(col1, col1), col2 / dot(col2, col2));

    var out: VertexOutput;
    out.clip_pos     = view_proj * world;
    out.color        = inst_color;
    out.shape_type   = shape_type;
    out.local_pos    = position;
    out.normal       = normalize(norm_mat * in_normal);
    out.world_pos    = world.xyz;
    out.shape_params = shape_params;
    out.uv           = in_uv;
    return out;
}

// ── Fragment ──────────────────────────────────────────────────────────────────

@fragment
fn frag_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample texture and multiply by instance color.
    // Default white texture (1×1, all 1s) leaves instance color unchanged.
    let tex_color = textureSample(t_color, s_color, in.uv);
    var base = tex_color.rgb * in.color;

    // shape_type 6 = procedural checker floor.
    // shape_params.x = tile frequency (tiles per world unit).
    if in.shape_type == 6u {
        let tile    = in.world_pos.xz * in.shape_params.x;
        let pattern = floor(tile.x) + floor(tile.y);
        if fract(pattern * 0.5) > 0.25 { base *= 0.6; }
    }

    // shape_type 7 = readable world ground: instance color is the biome base;
    // a soft 2-unit checker breaks uniformity, a bright gridline every
    // shape_params.x units gives position, and the two world axes through the
    // origin give orientation (X axis warm, Z axis cool).
    if in.shape_type == 7u {
        let p = in.world_pos.xz;
        let checker = floor(p.x * 0.5) + floor(p.y * 0.5);
        if fract(checker * 0.5) > 0.25 { base *= 0.90; }
        let period = max(in.shape_params.x, 1.0);
        // Distance to the nearest gridline (nearest multiple of period).
        let cell = abs(fract(p / period + vec2<f32>(0.5)) - vec2<f32>(0.5)) * period;
        if min(cell.x, cell.y) < 0.07 { base *= 1.45; }
        if abs(p.y) < 0.14 { base = mix(base, vec3<f32>(0.95, 0.55, 0.25), 0.8); }
        if abs(p.x) < 0.14 { base = mix(base, vec3<f32>(0.35, 0.60, 0.95), 0.8); }
    }

    // Lambert diffuse + ambient
    let N    = normalize(in.normal);
    let diff = max(dot(N, light.direction), 0.0);
    let lit  = light.ambient + diff * (1.0 - light.ambient);

    return vec4<f32>(base * lit * light.color, 1.0);
}
