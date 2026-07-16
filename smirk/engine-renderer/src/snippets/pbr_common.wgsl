// ── Cook-Torrance GGX ────────────────────────────────────────────────────────

const PI: f32 = 3.14159265;

fn d_ggx(NdotH: f32, rough: f32) -> f32 {
    let a  = rough * rough;
    let a2 = a * a;
    let d  = NdotH * NdotH * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

fn g_smith(NdotV: f32, NdotL: f32, rough: f32) -> f32 {
    let r  = rough + 1.0;
    let k  = r * r / 8.0;
    let gv = NdotV / (NdotV * (1.0 - k) + k);
    let gl = NdotL / (NdotL * (1.0 - k) + k);
    return gv * gl;
}

fn f_schlick(VdotH: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - VdotH, 5.0);
}

/// Cook-Torrance direct term for one light direction `L` (unit vector,
/// pointing toward the light), scaled by neither light color nor intensity —
/// callers apply those, since the sun and each point light carry them
/// differently.
fn direct_brdf(N: vec3<f32>, V: vec3<f32>, L: vec3<f32>, albedo: vec3<f32>, metallic: f32, rough: f32) -> vec3<f32> {
    let H = normalize(V + L);
    let NdotL = max(dot(N, L), 0.0);
    let NdotV = max(dot(N, V), 1e-4);
    let NdotH = max(dot(N, H), 0.0);
    let VdotH = max(dot(V, H), 0.0);

    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    let d = d_ggx(NdotH, rough);
    let g = g_smith(NdotV, NdotL, rough);
    let f = f_schlick(VdotH, f0);

    let specular = d * g * f / max(4.0 * NdotV * NdotL, 1e-4);
    let kd       = (vec3<f32>(1.0) - f) * (1.0 - metallic);

    return (kd * albedo + specular) * NdotL;
}

// Karis/Tokuyoshi geometric specular AA: fold the shading normal's
// screen-space variance into roughness so sub-pixel normal detail does not
// alias the specular lobe. Zero on flat surfaces (derivatives vanish).
fn specular_aa_roughness(N: vec3<f32>, rough: f32) -> f32 {
    let dndu = dpdx(N);
    let dndv = dpdy(N);
    let variance = 0.25 * (dot(dndu, dndu) + dot(dndv, dndv));
    let kernel   = min(2.0 * variance, 0.18);
    let a2       = rough * rough;
    return sqrt(clamp(a2 + kernel, 0.0, 1.0));
}

/// Shared shading: albedo already tinted and in linear space. `P` is the
/// fragment's world position (the point-light loop's falloff origin).
/// `shadow` attenuates the direct sun term only (1.0 = fully lit).
/// `screen_uv` (clip-space position over the viewport) samples the SSAO
/// texture, which scales ambient only — never direct light or emissive.
fn shade_pbr(
    P: vec3<f32>, N: vec3<f32>, V: vec3<f32>, albedo: vec3<f32>,
    metallic: f32, roughness: f32, ao: f32, emissive: vec3<f32>, shadow: f32,
    screen_uv: vec2<f32>,
) -> vec3<f32> {
    let rough = specular_aa_roughness(N, clamp(roughness, 0.045, 1.0));

    var direct = direct_brdf(N, V, light.direction, albedo, metallic, rough) * light.color * shadow;

    for (var i = 0u; i < light.point_count; i++) {
        let pl = light.points[i];
        let to_l = pl.position - P;
        let d = length(to_l);
        if d < pl.radius {
            let window = saturate(1.0 - pow(d / pl.radius, 4.0));
            let att = window * window / (d * d + 0.01);
            direct += direct_brdf(N, V, to_l / max(d, 1e-4), albedo, metallic, rough) * pl.color * pl.intensity * att;
        }
    }

    // IBL ambient: diffuse irradiance + prefiltered specular with the
    // split-sum BRDF. light.ambient scales it (the day/night seam).
    let NdotV   = max(dot(N, V), 1e-4);
    let f0      = mix(vec3<f32>(0.04), albedo, metallic);
    let irr     = textureSample(t_irradiance, s_env, N).rgb;
    let diffuse = irr * albedo * (1.0 - metallic);
    let R       = reflect(-V, N);
    let pre     = textureSampleLevel(t_prefilter, s_env, R, rough * PREFILTER_MAX_MIP).rgb;
    let ab      = textureSample(t_brdf, s_env, vec2<f32>(NdotV, rough)).rg;
    let spec_ibl = pre * (f0 * ab.x + vec3<f32>(ab.y));
    // Single mip (level 0) — textureSampleLevel skips the implicit-derivative
    // LOD naga would otherwise compute for a texture this shape has no use for.
    let ssao    = textureSampleLevel(t_ssao, s_ssao, screen_uv, 0.0).r;
    let ambient  = light.ambient * (diffuse + spec_ibl) * ao * ssao;

    return ambient + direct + emissive;
}
