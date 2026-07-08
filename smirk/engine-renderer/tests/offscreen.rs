// Offscreen render harness tests — VQ-G1.
//
// Analytic assertions only (coverage %, brighter-than, monotonic, uniformity
// bands) — never exact pixel values, so driver/adapter variance can't flake
// them. Every test skips cleanly when the machine has no GPU adapter.
// Frames go through the real chain: MSAA HDR → resolve → ACES tonemap.

use engine_renderer::instance::SdfInstance;
use engine_renderer::mesh::{load_gltf_data, MaterialData, MeshData, PrimitiveData};
use engine_renderer::offscreen::{
    create_mipped_rgba8, read_texture_mip, HeadlessGpu, OffscreenRenderer, TestLight,
};
use engine_renderer::MeshVertex;
use glam::{Mat4, Vec3};

const W: u32 = 256;
const H: u32 = 256;

fn renderer_or_skip() -> Option<OffscreenRenderer> {
    let r = OffscreenRenderer::new(W as f32 / H as f32);
    if r.is_none() {
        eprintln!("SKIP: no GPU adapter available — offscreen tests need one");
    }
    r
}

fn cube_at(position: Vec3, scale: f32, color: [f32; 3]) -> SdfInstance {
    SdfInstance {
        model: Mat4::from_scale_rotation_translation(
            Vec3::splat(scale),
            glam::Quat::IDENTITY,
            position,
        )
        .to_cols_array_2d(),
        color,
        shape_type: 0, // cube
        shape_params: [0.0; 4],
    }
}

fn luminance(p: &[u8]) -> f64 {
    0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64
}

/// Mean of one channel (0=r,1=g,2=b,3=a) over the whole image.
fn channel_mean(pixels: &[u8], channel: usize) -> f64 {
    let sum: u64 = pixels.iter().skip(channel).step_by(4).map(|&v| v as u64).sum();
    sum as f64 / (pixels.len() / 4) as f64
}

fn coverage(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count()
}

// ── basics ───────────────────────────────────────────────────────────────────

#[test]
fn clear_only_render_is_uniform() {
    let Some(mut r) = renderer_or_skip() else { return };
    let target = r.target(W, H);
    r.render_sdf(&target, &[], wgpu::Color { r: 0.02, g: 0.08, b: 0.18, a: 1.0 });
    let pixels = r.read(&target);

    assert_eq!(pixels.len(), (W * H * 4) as usize);
    let first: [u8; 4] = pixels[0..4].try_into().unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == first),
        "clear-only frame must be uniform, first pixel {first:?}"
    );
    // Channel ordering survives the tonemap: b > g > r for this clear color.
    assert!(first[2] > first[1] && first[1] > first[0], "got {first:?}");
}

#[test]
fn cube_renders_with_coverage_and_color() {
    let Some(mut r) = renderer_or_skip() else { return };
    let target = r.target(W, H);

    // Big red cube at the camera target (origin). Default orbit camera looks
    // at the origin, so the cube lands mid-frame.
    let instances = [cube_at(Vec3::ZERO, 10.0, [1.0, 0.0, 0.0])];
    r.render_sdf(&target, &instances, wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let covered = coverage(&pixels);
    let total = (W * H) as usize;
    assert!(
        covered > total / 50 && covered < total * 9 / 10,
        "cube coverage {covered}/{total} outside sane bounds"
    );

    // Color: red instance on black clear ⇒ red mean dominates.
    let (r_m, g_m, b_m) = (channel_mean(&pixels, 0), channel_mean(&pixels, 1), channel_mean(&pixels, 2));
    assert!(r_m > g_m * 1.5 && r_m > b_m * 1.5, "red cube must dominate: r={r_m:.1} g={g_m:.1} b={b_m:.1}");
}

#[test]
fn nearer_cube_occludes_farther_cube() {
    let Some(mut r) = renderer_or_skip() else { return };

    // Scene A: green cube alone at origin.
    let green = cube_at(Vec3::ZERO, 8.0, [0.0, 1.0, 0.0]);
    let target_a = r.target(W, H);
    r.render_sdf(&target_a, &[green], wgpu::Color::BLACK);
    let green_alone = channel_mean(&r.read(&target_a), 1);

    // Scene B: same green cube, plus a red cube between it and the camera
    // (default camera sits at positive X/Z, elevated).
    let red_front = cube_at(Vec3::new(8.0, 8.0, 8.0), 10.0, [1.0, 0.0, 0.0]);
    let target_b = r.target(W, H);
    r.render_sdf(&target_b, &[green, red_front], wgpu::Color::BLACK);
    let green_occluded = channel_mean(&r.read(&target_b), 1);

    assert!(
        green_occluded < green_alone * 0.8,
        "occluder must reduce green mean: alone={green_alone:.2} occluded={green_occluded:.2}"
    );
}

// ── Phase 1: PBR / mipmaps ───────────────────────────────────────────────────

/// A ground quad at y=0 spanning ±extent, normal +Y, with a uniform material.
fn ground_quad(extent: f32, roughness: f32, metallic: f32) -> MeshData {
    quad_with_material(extent, MaterialData {
        base_color_factor: [0.5, 0.5, 0.5, 1.0],
        roughness_factor:  roughness,
        metallic_factor:   metallic,
        ..Default::default()
    })
}

fn quad_with_material(extent: f32, material: MaterialData) -> MeshData {
    let e = extent;
    let v = |x: f32, z: f32, u: f32, w: f32| MeshVertex {
        position: [x, 0.0, z],
        normal:   [0.0, 1.0, 0.0],
        uv:       [u, w],
        tangent:  [1.0, 0.0, 0.0, 1.0],
    };
    MeshData {
        primitives: vec![PrimitiveData {
            vertices: vec![v(-e, -e, 0.0, 0.0), v(e, -e, 1.0, 0.0), v(e, e, 1.0, 1.0), v(-e, e, 0.0, 1.0)],
            indices:  vec![0, 2, 1, 0, 3, 2],
            material,
            skin: None,
        }],
        skeleton: None,
        clips:    Vec::new(),
    }
}

/// VQ-G1 for the GGX BRDF: on the same quad under the same sun, a smooth
/// surface concentrates specular energy (brighter peak, smaller hotspot) than
/// a fully rough one — monotonic, no exact pixels.
#[test]
fn smooth_surface_has_tighter_brighter_specular_peak_than_rough() {
    let Some(mut r) = renderer_or_skip() else { return };
    // Sun only — IBL ambient would flatten the lobe-shape comparison.
    r.set_light(TestLight {
        direction: Vec3::new(-1.0, 2.0, -1.0),
        color:     Vec3::new(1.0, 0.95, 0.85),
        ambient:   0.0,
    });

    let mut render = |roughness: f32| -> Vec<u8> {
        let target = r.target(W, H);
        r.render_mesh(&target, ground_quad(40.0, roughness, 0.0), wgpu::Color::BLACK);
        r.read(&target)
    };
    // 0.2 keeps the lobe several pixels wide (0.05 collapses it sub-pixel and
    // the MSAA resolve can miss it entirely).
    let smooth = render(0.2);
    let rough  = render(1.0);

    let peak = |img: &[u8]| img.chunks_exact(4).map(|p| luminance(p)).fold(0.0f64, f64::max);
    let (peak_smooth, peak_rough) = (peak(&smooth), peak(&rough));
    assert!(
        peak_smooth > peak_rough * 1.15,
        "smooth peak must outshine rough: smooth={peak_smooth:.1} rough={peak_rough:.1}"
    );

    let hotspot = |img: &[u8]| {
        let p = peak(img);
        img.chunks_exact(4).filter(|px| luminance(px) > 0.9 * p).count()
    };
    let (hot_smooth, hot_rough) = (hotspot(&smooth), hotspot(&rough));
    assert!(
        hot_smooth < hot_rough,
        "smooth hotspot must be tighter: smooth={hot_smooth}px rough={hot_rough}px"
    );
}

/// Off the specular lobe a metal has no diffuse term, so its dark percentile
/// falls well below the dielectric's uniform diffuse floor.
#[test]
fn metallic_changes_response_vs_dielectric() {
    let Some(mut r) = renderer_or_skip() else { return };
    // Sun only — a gray env's specular IBL would relight the metal.
    r.set_light(TestLight {
        direction: Vec3::new(-1.0, 2.0, -1.0),
        color:     Vec3::new(1.0, 0.95, 0.85),
        ambient:   0.0,
    });
    let mut render = |metallic: f32| {
        let target = r.target(W, H);
        r.render_mesh(&target, ground_quad(40.0, 0.4, metallic), wgpu::Color::BLACK);
        r.read(&target)
    };
    let dielectric = render(0.0);
    let metal      = render(1.0);

    let footprint: Vec<usize> = dielectric
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, p)| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .map(|(i, _)| i)
        .collect();
    assert!(footprint.len() > 1000, "quad must cover a useful area");

    let p10 = |img: &[u8]| {
        let mut lums: Vec<f64> = footprint
            .iter()
            .map(|&i| luminance(&img[i * 4..i * 4 + 4]))
            .collect();
        lums.sort_by(|a, b| a.partial_cmp(b).unwrap());
        lums[lums.len() / 10]
    };
    let (metal_p10, dielectric_p10) = (p10(&metal), p10(&dielectric));
    assert!(
        metal_p10 < dielectric_p10 * 0.7,
        "off-lobe metal is darker (no diffuse): metal p10={metal_p10:.1} dielectric p10={dielectric_p10:.1}"
    );
}

/// Khronos DamagedHelmet: a real full-PBR asset (all five maps + tangents)
/// loads and renders with sane coverage through the mesh pipeline.
#[test]
fn damaged_helmet_renders() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../content/source/test/DamagedHelmet.glb"
    );
    if !std::path::Path::new(path).exists() {
        eprintln!("SKIP: DamagedHelmet fixture missing");
        return;
    }
    let Some(mut r) = renderer_or_skip() else { return };

    let mut data = load_gltf_data(path).expect("helmet parses");
    let m = &data.primitives[0].material;
    assert!(m.base_color_image.is_some(), "helmet has albedo");
    assert!(m.normal_image.is_some(), "helmet has a normal map");
    assert!(m.metallic_roughness_image.is_some(), "helmet has MR");
    assert!(m.emissive_image.is_some(), "helmet has emissive");

    // ~1 unit tall at the origin; scale up to read from the radius-34 orbit.
    for prim in &mut data.primitives {
        for v in &mut prim.vertices {
            for c in &mut v.position {
                *c *= 12.0;
            }
        }
    }
    let target = r.target(W, H);
    r.render_mesh(&target, data, wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let covered = coverage(&pixels);
    let total = (W * H) as usize;
    assert!(
        covered > total / 50 && covered < total * 9 / 10,
        "helmet coverage {covered}/{total} outside sane bounds"
    );
}

/// VQ-C1: the blit chain really downsamples — mip 1 of a 1-px checkerboard
/// averages toward mid-gray, far from both extremes.
#[test]
fn mip_chain_downsamples_checkerboard_to_gray() {
    let Some(gpu) = HeadlessGpu::new() else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };

    const S: u32 = 64;
    let mut pixels = Vec::with_capacity((S * S * 4) as usize);
    for y in 0..S {
        for x in 0..S {
            let v = if (x + y) % 2 == 0 { 255u8 } else { 0u8 };
            pixels.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let texture = create_mipped_rgba8(&gpu, S, S, &pixels, false);
    assert_eq!(texture.mip_level_count(), 7, "full chain for 64²");

    let mip1 = read_texture_mip(&gpu, &texture, 1);
    assert_eq!(mip1.len(), (32 * 32 * 4) as usize);
    for px in mip1.chunks_exact(4) {
        assert!(
            (96..=160).contains(&px[0]),
            "mip1 of a 1px checkerboard is mid-gray, got {px:?}"
        );
    }
}

// ── Phase 2: HDR / tonemap / MSAA / IBL ─────────────────────────────────────

/// VQ-D1: HDR values survive to the tonemap — an 8× emissive quad tonemaps
/// brighter than a 1× one (monotonic) yet stays below clipping.
#[test]
fn hdr_emissive_tonemaps_monotonically_without_clipping() {
    let Some(mut r) = renderer_or_skip() else { return };
    // Kill sun + ambient so only emissive contributes; half exposure keeps
    // ACES(8x) below its saturation point so headroom is measurable.
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 0.0 });
    r.set_exposure(0.5);

    let mut render = |strength: f32| -> Vec<u8> {
        let target = r.target(W, H);
        r.render_mesh(
            &target,
            quad_with_material(40.0, MaterialData {
                base_color_factor: [0.0, 0.0, 0.0, 1.0],
                emissive_factor:   [1.0, 1.0, 1.0],
                emissive_strength: strength,
                ..Default::default()
            }),
            wgpu::Color::BLACK,
        );
        r.read(&target)
    };
    let dim    = render(1.0);
    let bright = render(8.0);

    let peak = |img: &[u8]| img.chunks_exact(4).map(|p| p[1]).max().unwrap();
    let (p1, p8) = (peak(&dim), peak(&bright));
    assert!(p8 > p1, "8x emissive must tonemap brighter: 1x={p1} 8x={p8}");
    assert!(p8 <= 254, "ACES must not clip 8x emissive to pure white, got {p8}");
    assert!(p1 > 100, "1x white emissive is clearly visible, got {p1}");
}

/// VQ-D4: MSAA 4× produces intermediate coverage values along a silhouette
/// edge that 1× sampling cannot (a black frame with a lit cube must contain
/// edge pixels strictly between background and body brightness).
#[test]
fn msaa_produces_intermediate_edge_pixels() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 0.0 });

    // Pure white emissive-like: use a white cube lit only by uniform env so
    // every face shades the same; edges then blend body ↔ black background.
    r.set_uniform_environment([1.0, 1.0, 1.0]);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });
    let target = r.target(W, H);
    r.render_sdf(&target, &[cube_at(Vec3::ZERO, 10.0, [1.0, 1.0, 1.0])], wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let body_peak = pixels.chunks_exact(4).map(|p| p[0]).max().unwrap();
    assert!(body_peak > 60, "cube must be visible, peak {body_peak}");
    let intermediates = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 12 && p[0] < body_peak - 12)
        .count();
    assert!(
        intermediates > 50,
        "4x MSAA must leave intermediate edge pixels, found {intermediates}"
    );
}

/// VQ-D2 white-furnace-style sanity: under a uniform white environment with
/// no sun, a fully rough white quad shades nearly flat (IBL irradiance of a
/// uniform sky is constant) and clearly non-black.
#[test]
fn uniform_white_environment_lights_surfaces_uniformly() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_uniform_environment([1.0, 1.0, 1.0]);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });

    let target = r.target(W, H);
    r.render_mesh(
        &target,
        quad_with_material(40.0, MaterialData {
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            roughness_factor:  1.0,
            metallic_factor:   0.0,
            ..Default::default()
        }),
        wgpu::Color::BLACK,
    );
    let pixels = r.read(&target);

    let footprint: Vec<f64> = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 8)
        .map(|p| luminance(p))
        .collect();
    assert!(footprint.len() > 1000, "quad visible");
    let mean = footprint.iter().sum::<f64>() / footprint.len() as f64;
    assert!(mean > 80.0, "white furnace lights the quad, mean {mean:.1}");
    let max = footprint.iter().cloned().fold(0.0f64, f64::max);
    let min = footprint.iter().cloned().fold(f64::MAX, f64::min);
    assert!(
        max - min < 0.35 * mean,
        "uniform env ⇒ near-flat shading: min={min:.1} max={max:.1} mean={mean:.1}"
    );
}

// ── Phase 4: bloom ───────────────────────────────────────────────────────────

/// A small HDR-bright quad on black spreads energy beyond its own rect with
/// bloom on; the identical render with bloom off keeps that region black.
#[test]
fn bloom_spreads_hdr_energy_beyond_the_emitter() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 0.0 });

    let scene = || {
        quad_with_material(3.0, MaterialData {
            base_color_factor: [0.0, 0.0, 0.0, 1.0],
            emissive_factor:   [1.0, 1.0, 1.0],
            emissive_strength: 8.0, // over the bloom threshold (1.0)
            ..Default::default()
        })
    };

    r.set_bloom_intensity(0.5);
    let target_on = r.target(W, H);
    r.render_mesh(&target_on, scene(), wgpu::Color::BLACK);
    let with_bloom = r.read(&target_on);

    r.set_bloom_intensity(0.0);
    let target_off = r.target(W, H);
    r.render_mesh(&target_off, scene(), wgpu::Color::BLACK);
    let without_bloom = r.read(&target_off);

    // Halo = pixels lit with bloom but black without it.
    let mut halo = 0usize;
    for (a, b) in with_bloom.chunks_exact(4).zip(without_bloom.chunks_exact(4)) {
        if a[1] > 6 && b[1] <= 2 {
            halo += 1;
        }
    }
    assert!(halo > 300, "bloom must spread beyond the emitter: halo={halo}px");
}

// ── Phase 3: shadows ─────────────────────────────────────────────────────────

/// VQ-D3: a cube floating above a ground slab under a 45° sun casts a dark
/// band — pixel-wise darker than the identical scene without the caster,
/// over an area clearly larger than the caster's own silhouette; far ground
/// stays untouched.
#[test]
fn floating_cube_casts_shadow_band_on_ground() {
    let Some(mut r) = renderer_or_skip() else { return };
    // 45° sun, no IBL so the shadow contrast is unambiguous.
    r.set_light(TestLight {
        direction: Vec3::new(-1.0, 1.0, 0.0),
        color:     Vec3::new(1.0, 1.0, 1.0),
        ambient:   0.0,
    });

    // Ground: a flat white slab; caster: a red cube 6 units up.
    let ground = cube_at(Vec3::new(0.0, -0.55, 0.0), 1.0, [1.0, 1.0, 1.0]);
    let ground = SdfInstance {
        model: Mat4::from_scale_rotation_translation(
            Vec3::new(60.0, 1.0, 60.0),
            glam::Quat::IDENTITY,
            Vec3::new(0.0, -0.55, 0.0),
        )
        .to_cols_array_2d(),
        ..ground
    };
    let caster = cube_at(Vec3::new(0.0, 6.0, 0.0), 4.0, [1.0, 0.0, 0.0]);

    let target_without = r.target(W, H);
    r.render_sdf(&target_without, &[ground], wgpu::Color::BLACK);
    let without = r.read(&target_without);

    let target_with = r.target(W, H);
    r.render_sdf(&target_with, &[ground, caster], wgpu::Color::BLACK);
    let with = r.read(&target_with);

    // Pixel-wise: green channel only (the red caster has no green, so green
    // loss = ground darkened by shadow or occlusion).
    let mut darkened = 0usize;
    let mut unchanged_far = 0usize;
    let mut checked_far = 0usize;
    for (i, (a, b)) in without.chunks_exact(4).zip(with.chunks_exact(4)).enumerate() {
        let (ga, gb) = (a[1] as i32, b[1] as i32);
        if ga > 40 && gb < ga * 6 / 10 {
            darkened += 1;
        }
        // Left edge column region is far from cube + shadow (sun from -x
        // throws the shadow toward +x): must be untouched.
        let x = i % W as usize;
        if x < 20 && ga > 40 {
            checked_far += 1;
            if (ga - gb).abs() <= 10 {
                unchanged_far += 1;
            }
        }
    }
    // The caster's own silhouette also removes green; require the darkened
    // area to be clearly larger than the cube footprint alone.
    let cube_footprint = with
        .chunks_exact(4)
        .filter(|p| p[0] > 40 && p[1] < p[0] / 3)
        .count();
    assert!(
        darkened > cube_footprint + 300,
        "shadow band must extend beyond the caster: darkened={darkened} footprint={cube_footprint}"
    );
    assert!(checked_far > 100, "far-ground control region present");
    assert!(
        unchanged_far as f64 > checked_far as f64 * 0.95,
        "far ground unchanged: {unchanged_far}/{checked_far}"
    );
}

/// VQ-D2: with the sky pass on, background pixels show the environment
/// rather than the black clear.
#[test]
fn sky_pass_fills_background_with_environment() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_uniform_environment([0.6, 0.7, 0.9]);
    r.draw_sky = true;

    let target = r.target(W, H);
    r.render_sdf(&target, &[], wgpu::Color::BLACK);
    let pixels = r.read(&target);

    // Every pixel is sky — non-black, blue-tinted.
    let (r_m, g_m, b_m) = (channel_mean(&pixels, 0), channel_mean(&pixels, 1), channel_mean(&pixels, 2));
    assert!(b_m > 60.0, "sky visible, b mean {b_m:.1}");
    assert!(b_m > g_m && g_m > r_m, "sky tint ordering holds: {r_m:.1} {g_m:.1} {b_m:.1}");
}
