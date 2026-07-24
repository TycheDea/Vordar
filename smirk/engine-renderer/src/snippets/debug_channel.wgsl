//#include "snippets/srgb_oetf.wgsl"

/// Isolates one shading input as a viewable/measurable RGB frame, keyed by
/// `light.debug_mode` (0 = disabled, the shipped path). Albedo is sRGB-encoded
/// so the PNG matches the sRGB-format atlas bytes prop_audit.py measures;
/// roughness, metallic, normal and AO carry data rather than colour and stay raw.
fn debug_channel(albedo: vec3<f32>, metallic: f32, roughness: f32, ao: f32, N: vec3<f32>) -> vec3<f32> {
    switch light.debug_mode {
        case 1u: { return srgb_oetf(albedo); }
        case 2u: { return vec3<f32>(roughness); }
        case 3u: { return vec3<f32>(metallic); }
        case 4u: { return N * 0.5 + 0.5; }
        case 5u: { return vec3<f32>(ao); }
        default: { return vec3<f32>(0.0); }
    }
}
