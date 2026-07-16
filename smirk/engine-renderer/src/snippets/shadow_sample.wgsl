// Shadow receiving — shared scene group.
@group(0) @binding(2) var<uniform> light_vp: mat4x4<f32>;
@group(0) @binding(3) var t_shadow: texture_depth_2d;
@group(0) @binding(4) var s_shadow: sampler_comparison;

//#const SHADOW_TEXEL

/// PCF 3×3 over the fitted sun map; 1.0 = fully lit. Points outside the
/// fitted volume are lit (the map only covers the play area).
fn shadow_factor(world_pos: vec3<f32>) -> f32 {
    let lp  = light_vp * vec4<f32>(world_pos, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv  = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x <= 0.0 || uv.x >= 1.0 || uv.y <= 0.0 || uv.y >= 1.0 || ndc.z >= 1.0 || ndc.z <= 0.0) {
        return 1.0;
    }
    var sum = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let offset = vec2<f32>(f32(dx), f32(dy)) * SHADOW_TEXEL;
            sum += textureSampleCompareLevel(t_shadow, s_shadow, uv + offset, ndc.z);
        }
    }
    return sum / 9.0;
}
