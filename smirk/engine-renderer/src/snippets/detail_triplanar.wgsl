// World-space triplanar detail overlay: a tiling, high-frequency stone-grain
// layer sampled at a real-world period rather than in UV space, so its texel
// density is independent of the atlas and the prop's UV utilization (see the
// detail-phase plan). Composed onto the base bump via Mikkelsen surface
// gradients (JCGT 2020) — the base bump is UV-tangent-space and this one is
// world-space triplanar, different frames with no shared RNM basis, so
// gradients (which add across frames) combine them instead. Detail roughness
// derives from the detail albedo's luminance — no third texture.
//
// group(3): the shared detail tile, bound once per pass (not per primitive)
// since every opted-in material samples the same global tile.
@group(3) @binding(0) var t_detail_albedo: texture_2d<f32>; // sRGB, BC7
@group(3) @binding(1) var t_detail_normal: texture_2d<f32>; // linear, tangent-space, z reconstructed from xy — BC5
@group(3) @binding(2) var s_detail:        sampler;

const DETAIL_PERIOD: f32 = 0.45; // world metres per tile — a 2K-square tile at this period resolves to ~0.22 mm/texel
const DETAIL_BLEND_SHARPNESS: f32 = 3.0; // triplanar weight exponent: pow(abs(N), k), normalized
const DETAIL_NORMAL_STRENGTH: f32 = 1.0;
const DETAIL_ROUGHNESS_STRENGTH: f32 = 0.4;
// Linear decode of the sRGB 0.5 grey mean that content_lint's
// detail_tile_is_dc_neutral enforces on the tile — the tile is a BC7_UNORM_SRGB
// texture, so textureSample returns linear and the overlay's identity point is
// this value, not 0.5.
const DETAIL_ALBEDO_NEUTRAL: f32 = 0.2140;
const DETAIL_ALBEDO_STRENGTH: f32 = 0.5;
const DETAIL_NORMAL_FADE_NEAR: f32 = 4.0;
const DETAIL_NORMAL_FADE_FAR:  f32 = 10.0;
const DETAIL_COLOR_FADE_NEAR:  f32 = 12.0;
const DETAIL_COLOR_FADE_FAR:   f32 = 24.0;

struct DetailSample {
    surf_grad:       vec3<f32>, // world-space surface gradient, tangent to Nv, already distance-faded
    albedo_scale:    f32,       // multiplicative overlay (1.0 = no change), already distance-faded
    roughness_delta: f32,       // additive roughness perturbation, already distance-faded
}

/// Tangent-space normal-map sample → 2D derivative: `-xy/z`, the same
/// formula `mesh_shader.wgsl`'s base bump uses (OpenGL +Y convention, no
/// vertical flip — Mikkelsen's `TspaceNormalToDerivative` with
/// `gFlipVertDeriv = false`). The `max(z, 1e-3)` guard is unreachable with
/// real BC5 data (z = 0 needs xy on the unit circle exactly) — this function
/// divides by z where the base bump's own formula never does, so it is the
/// one place this overlay's math isn't a strict identity with line 120's.
fn detail_deriv(uv: vec2<f32>) -> vec2<f32> {
    let n = textureSample(t_detail_normal, s_detail, uv).xy * 2.0 - 1.0;
    let z = sqrt(max(1.0 - dot(n, n), 0.0));
    return -n / max(z, 1e-3);
}

/// Triplanar detail sample at `world_pos`, blended by `Nv`-derived plane
/// weights and projected tangent to `Nv` — Mikkelsen's
/// `SurfgradFromTriplanarProjection` + `SurfgradFromVolumeGradient` (JCGT
/// 2020), verified against github.com/mmikk/hextile-demo's
/// surfgrad_framework.h and shader_lighting.hlsl's `CommonTriplanarNormal`.
/// Each plane's UV is a continuous affine function of `world_pos`, so
/// hardware mip derivatives are correct without extra work; the signed `-z`
/// terms in `uv_x`/`uv_y` below are not arbitrary — they're the exact sign
/// pairing (matched against `CommonTriplanarNormal`'s `sp_x`/`sp_y`/`sp_z`
/// plus its post-fetch corrections) that keeps the blended gradient
/// continuous across a corner where two planes both contribute, rather than
/// each plane independently and inconsistently mirrored.
fn sample_detail(world_pos: vec3<f32>, Nv: vec3<f32>, eye_dist: f32) -> DetailSample {
    let pos = world_pos / DETAIL_PERIOD;

    let uv_x = vec2<f32>(-pos.z, pos.y);
    let uv_y = vec2<f32>(pos.x, -pos.z);
    let uv_z = vec2<f32>(pos.x, pos.y);

    let raw_x = detail_deriv(uv_x);
    let raw_y = detail_deriv(uv_y);
    let raw_z = detail_deriv(uv_z);
    // Corrects each plane's raw (u,v)-space derivative to a world-position
    // derivative, so the fixed x/y/z assembly below (SurfgradFromTriplanarProjection)
    // is valid regardless of the sign each plane's UV carries.
    let deriv_x = vec2<f32>(-raw_x.x, -raw_x.y);
    let deriv_y = raw_y;
    let deriv_z = vec2<f32>(raw_z.x, -raw_z.y);

    let w_raw = pow(abs(Nv), vec3<f32>(DETAIL_BLEND_SHARPNESS));
    let weights = w_raw / max(w_raw.x + w_raw.y + w_raw.z, 1e-6);

    // SurfgradFromTriplanarProjection: (z,y)-plane, (x,z)-plane, (x,y)-plane
    // assembled into one world-space volume gradient.
    let grad = vec3<f32>(
        weights.z * deriv_z.x + weights.y * deriv_y.x,
        weights.z * deriv_z.y + weights.x * deriv_x.y,
        weights.x * deriv_x.x + weights.y * deriv_y.y,
    );
    // SurfgradFromVolumeGradient: project tangent to the true surface normal.
    let surf_grad = grad - dot(Nv, grad) * Nv;

    // Same three plane weights as the normal blend above; luminance only —
    // the overlay is desaturated (see `ratio` below) so the tile's own chroma
    // never drags onto the macro atlas's colour story.
    let tap = textureSample(t_detail_albedo, s_detail, uv_x).rgb * weights.x
            + textureSample(t_detail_albedo, s_detail, uv_y).rgb * weights.y
            + textureSample(t_detail_albedo, s_detail, uv_z).rgb * weights.z;
    let luma = dot(tap, vec3<f32>(0.2126, 0.7152, 0.0722));

    // Fades computed unconditionally (never gate the taps above on distance —
    // that would make textureSample's implicit derivatives non-uniform across
    // a fragment quad); only the contribution is scaled to zero.
    let normal_fade = 1.0 - smoothstep(DETAIL_NORMAL_FADE_NEAR, DETAIL_NORMAL_FADE_FAR, eye_dist);
    let color_fade  = 1.0 - smoothstep(DETAIL_COLOR_FADE_NEAR, DETAIL_COLOR_FADE_FAR, eye_dist);

    // Albedo overlay: a scalar ratio about DETAIL_ALBEDO_NEUTRAL, scaled by
    // strength and fade. Clamp band keeps a single bright or dark texel from
    // blowing out or crushing the base albedo.
    let ratio = clamp(luma / DETAIL_ALBEDO_NEUTRAL, 0.5, 1.6);

    var out: DetailSample;
    out.surf_grad       = surf_grad * DETAIL_NORMAL_STRENGTH * normal_fade;
    out.albedo_scale    = mix(1.0, ratio, DETAIL_ALBEDO_STRENGTH * color_fade);
    out.roughness_delta = (1.0 - ratio) * DETAIL_ROUGHNESS_STRENGTH * color_fade;
    return out;
}
