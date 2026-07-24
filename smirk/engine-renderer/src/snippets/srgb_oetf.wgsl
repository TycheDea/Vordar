/// Linear → sRGB opto-electronic transfer function (IEC 61966-2-1) — the
/// same encode an sRGB view applies in hardware. Shared by debug_channel (the
/// Rgba8Unorm debug target has no hardware transfer function) and tonemap
/// (post.encode, for the same reason on offscreen targets).
fn srgb_oetf(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}
