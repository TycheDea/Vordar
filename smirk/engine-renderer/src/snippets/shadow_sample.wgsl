// Shadow receiving — shared scene group.
//#const CASCADE_COUNT
@group(0) @binding(2) var<uniform> light_vp: array<mat4x4<f32>, CASCADE_COUNT>;
@group(0) @binding(3) var t_shadow: texture_depth_2d_array;
@group(0) @binding(4) var s_shadow: sampler_comparison;

//#const SHADOW_TEXEL

// Shrinks the containment test below the cascade's true NDC bound so a
// fragment near a tighter cascade's edge falls through to the next
// (coarser) cascade instead of PCF-sampling past its map edge (seam guard).
const CASCADE_EDGE_MARGIN: f32 = 0.02;

/// PCF 3×3 over the first (tightest) cascade whose fitted volume contains
/// `world_pos`; 1.0 = fully lit. Points outside every cascade are lit (the
/// map only covers the play area).
fn shadow_factor(world_pos: vec3<f32>) -> f32 {
    for (var c = 0u; c < CASCADE_COUNT; c++) {
        let lp  = light_vp[c] * vec4<f32>(world_pos, 1.0);
        let ndc = lp.xyz / lp.w;
        if (abs(ndc.x) >= 1.0 - CASCADE_EDGE_MARGIN || abs(ndc.y) >= 1.0 - CASCADE_EDGE_MARGIN || ndc.z >= 1.0 || ndc.z <= 0.0) {
            continue;
        }
        let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
        var sum = 0.0;
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let offset = vec2<f32>(f32(dx), f32(dy)) * SHADOW_TEXEL;
                sum += textureSampleCompareLevel(t_shadow, s_shadow, uv + offset, c, ndc.z);
            }
        }
        return sum / 9.0;
    }
    return 1.0;
}
