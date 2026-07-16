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

/// Shared shading: albedo already tinted and in linear space. `shadow`
/// attenuates the direct sun term only (1.0 = fully lit).
fn shade_pbr(
    N: vec3<f32>, V: vec3<f32>, albedo: vec3<f32>,
    metallic: f32, roughness: f32, ao: f32, emissive: vec3<f32>, shadow: f32,
) -> vec3<f32> {
    let L = light.direction;
    let H = normalize(V + L);
    let NdotL = max(dot(N, L), 0.0);
    let NdotV = max(dot(N, V), 1e-4);
    let NdotH = max(dot(N, H), 0.0);
    let VdotH = max(dot(V, H), 0.0);

    let rough = clamp(roughness, 0.045, 1.0);
    let f0    = mix(vec3<f32>(0.04), albedo, metallic);

    let d = d_ggx(NdotH, rough);
    let g = g_smith(NdotV, NdotL, rough);
    let f = f_schlick(VdotH, f0);

    let specular = d * g * f / max(4.0 * NdotV * NdotL, 1e-4);
    let kd       = (vec3<f32>(1.0) - f) * (1.0 - metallic);

    let direct  = (kd * albedo + specular) * NdotL * light.color * shadow;

    // IBL ambient: diffuse irradiance + prefiltered specular with the
    // split-sum BRDF. light.ambient scales it (the day/night seam).
    let irr     = textureSample(t_irradiance, s_env, N).rgb;
    let diffuse = irr * albedo * (1.0 - metallic);
    let R       = reflect(-V, N);
    let pre     = textureSampleLevel(t_prefilter, s_env, R, rough * PREFILTER_MAX_MIP).rgb;
    let ab      = textureSample(t_brdf, s_env, vec2<f32>(NdotV, rough)).rg;
    let spec_ibl = pre * (f0 * ab.x + vec3<f32>(ab.y));
    let ambient  = light.ambient * (diffuse + spec_ibl) * ao;

    return ambient + direct + emissive;
}
