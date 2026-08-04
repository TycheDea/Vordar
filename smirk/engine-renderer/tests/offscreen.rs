// Offscreen render harness tests.
//
// Analytic assertions only (coverage %, brighter-than, monotonic, uniformity
// bands) — never exact pixel values, so driver/adapter variance can't flake
// them. Every test skips cleanly when the machine has no GPU adapter.
// Frames go through the real chain: MSAA HDR → resolve → ACES tonemap.

use engine_renderer::instance::SdfInstance;
use engine_renderer::mesh::{load_gltf_data, AlphaMode, ImageData, MaterialData, MeshData, PrimitiveData, SharedImage, TextureSource};
use engine_renderer::offscreen::{
    create_mipped_rgba8, read_texture_mip, DebugChannel, HeadlessGpu, OffscreenRenderer, TestLight, TestPointLight,
};
use std::ops::Range;
use engine_renderer::texture::parse_dds;
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

/// Population variance of one channel over the whole image — a debug-channel
/// readback is a direct value passthrough (no tonemap/gamma, see
/// `debug_channel_roughness_reads_back_as_exact_byte`), so this measures real
/// per-pixel spread in that channel rather than an encoding artifact.
fn channel_variance(pixels: &[u8], channel: usize) -> f64 {
    let mean = channel_mean(pixels, channel);
    let sum_sq: f64 = pixels.iter().skip(channel).step_by(4)
        .map(|&v| { let d = v as f64 - mean; d * d })
        .sum();
    sum_sq / (pixels.len() / 4) as f64
}

/// Linear-light equivalent of a byte, on the same 0..255 scale, undoing the
/// sRGB OETF (IEC 61966-2-1) that the offscreen target now bakes in so it
/// matches the live swapchain's hardware encode. Assertions about physical
/// light — a ratio between samples, a "this is essentially black" floor —
/// read perceptual bytes off that target; decode back to this scale before
/// reasoning about them.
fn linear(byte: u8) -> f64 {
    let c = byte as f64 / 255.0;
    (if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }) * 255.0
}

/// `channel_mean`, decoded to linear light first.
fn channel_mean_linear(pixels: &[u8], channel: usize) -> f64 {
    let sum: f64 = pixels.iter().skip(channel).step_by(4).map(|&v| linear(v)).sum();
    sum / (pixels.len() / 4) as f64
}

/// `pixels` with every byte replaced by its `linear`-decoded equivalent.
fn linearize(pixels: &[u8]) -> Vec<u8> {
    pixels.iter().map(|&b| linear(b).round() as u8).collect()
}

/// True if every pixel within `rows` (row-major, `width` px wide, 4
/// bytes/pixel) differs by at most `tol` per channel between the two images.
fn rows_close(a: &[u8], b: &[u8], width: u32, rows: std::ops::Range<u32>, tol: i32) -> bool {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .enumerate()
        .filter(|(i, _)| rows.contains(&(*i as u32 / width)))
        .all(|(_, (pa, pb))| pa.iter().zip(pb).all(|(x, y)| (*x as i32 - *y as i32).abs() <= tol))
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

// ── PBR / mipmaps ────────────────────────────────────────────────────────────

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

/// GGX BRDF: on the same quad under the same sun, a smooth
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

    let peak = |img: &[u8]| img.chunks_exact(4).map(luminance).fold(0.0f64, f64::max);
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

/// A 1×1 RGBA8 texture usable as a uniform tangent-space normal map fixture.
fn solid_normal_texture(rgb: [u8; 3]) -> ImageData {
    ImageData { width: 1, height: 1, pixels: vec![rgb[0], rgb[1], rgb[2], 255] }
}

/// z-reconstruction (`z = sqrt(1 - x² - y²)`) must still let a tangent-space
/// normal map perturb N: a uniformly tilted map changes N·L versus a flat
/// one, so mean luminance under the default sun must differ by a clear
/// margin. RGBA8 only — proves the rewritten TBN path (shared by the future
/// BC5 path) still lights correctly.
#[test]
fn tilted_normal_map_changes_luminance_vs_flat() {
    let Some(mut r) = renderer_or_skip() else { return };

    let material = |normal_rgb: [u8; 3]| MaterialData {
        base_color_factor: [0.5, 0.5, 0.5, 1.0],
        roughness_factor:  0.6,
        metallic_factor:   0.0,
        normal_image:      Some(SharedImage::new(TextureSource::Rgba8(solid_normal_texture(normal_rgb)))),
        ..Default::default()
    };

    let target_flat = r.target(W, H);
    r.render_mesh(&target_flat, quad_with_material(40.0, material([128, 128, 255])), wgpu::Color::BLACK);
    let flat_mean = mean_luminance(&r.read(&target_flat));

    let target_tilted = r.target(W, H);
    r.render_mesh(&target_tilted, quad_with_material(40.0, material([200, 128, 235])), wgpu::Color::BLACK);
    let tilted_mean = mean_luminance(&r.read(&target_tilted));

    assert!(
        (flat_mean - tilted_mean).abs() > flat_mean * 0.05,
        "tilted normal map must change N·L and mean luminance: flat={flat_mean:.2} tilted={tilted_mean:.2}"
    );
}

// ── Geometric specular AA ────────────────────────────────────────────────────

/// Same ground quad as `quad_with_material` but with UV scaled by `tiles`, so
/// a small repeating normal map cycles `tiles` times across the surface (the
/// material sampler's Repeat address mode tiles it, see texture.rs's `make_sampler`).
fn quad_with_tiled_normal(extent: f32, material: MaterialData, tiles: f32) -> MeshData {
    let e = extent;
    let v = |x: f32, z: f32, u: f32, w: f32| MeshVertex {
        position: [x, 0.0, z],
        normal:   [0.0, 1.0, 0.0],
        uv:       [u * tiles, w * tiles],
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

/// A 2×2 tangent-space normal map checkerboarding between two tilts that are
/// mirror images of each other (mean flat), so repeating it via UV tiling
/// changes on-screen normal frequency without changing the surface's average
/// orientation.
fn checker_normal_texture() -> ImageData {
    let tilt_pos: [u8; 4] = [170, 128, 255, 255];
    let tilt_neg: [u8; 4] = [86, 128, 255, 255];
    let mut pixels = Vec::with_capacity(16);
    for row in [[tilt_pos, tilt_neg], [tilt_neg, tilt_pos]] {
        for px in row {
            pixels.extend_from_slice(&px);
        }
    }
    ImageData { width: 2, height: 2, pixels }
}

/// Karis/Tokuyoshi geometric specular AA: a shiny normal-mapped surface must
/// dim as the normal map's on-screen tiling density increases, because GSAA
/// folds the shading normal's screen-space derivative into roughness —
/// killing the shimmer a naive shader would show on fine normal-map detail.
#[test]
fn denser_normal_tiling_softens_specular_peak() {
    let Some(mut r) = renderer_or_skip() else { return };
    // Sun only — IBL ambient would flatten the lobe-shape comparison.
    r.set_light(TestLight {
        direction: Vec3::new(-1.0, 2.0, -1.0),
        color:     Vec3::new(0.3, 0.285, 0.255),
        ambient:   0.0,
    });

    let material = || MaterialData {
        base_color_factor: [0.5, 0.5, 0.5, 1.0],
        roughness_factor:  0.2,
        metallic_factor:   0.0,
        normal_image:      Some(SharedImage::new(TextureSource::Rgba8(checker_normal_texture()))),
        ..Default::default()
    };

    let mut render = |tiles: f32| -> Vec<u8> {
        let target = r.target(W, H);
        r.render_mesh(&target, quad_with_tiled_normal(40.0, material(), tiles), wgpu::Color::BLACK);
        r.read(&target)
    };

    let peak = |img: &[u8]| img.chunks_exact(4).map(luminance).fold(0.0f64, f64::max);

    // Measured peaks (256x256, roughness 0.2, sun at 0.3x): sparse
    // (tiles=4)=197.4 dense (tiles=64)=186.3, a 5.65% drop. The sun sits well
    // below the other PBR tests' intensity because at full brightness this
    // tilt's specular peak saturates to the 255 clip ceiling for both
    // densities, masking the GSAA effect entirely.
    let peak_sparse = peak(&render(4.0));
    let peak_dense  = peak(&render(64.0));
    assert!(
        peak_dense < peak_sparse,
        "denser normal tiling must soften the specular peak (GSAA): sparse={peak_sparse:.1} dense={peak_dense:.1}"
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

/// A compressed BC7 sRGB material renders through the real mesh pipeline:
/// decode + color-space handling must land red-dominant at the quad's
/// center, same as the RGBA8 path other tests in this file already cover.
/// Skips without a GPU adapter or without BC support (fallback adapters).
#[test]
fn compressed_bc7_texture_renders_through_real_pipeline() {
    let Some(mut r) = renderer_or_skip() else { return };
    if !r.gpu.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
        eprintln!("SKIP: adapter lacks TEXTURE_COMPRESSION_BC");
        return;
    }
    r.set_uniform_environment([1.0, 1.0, 1.0]);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });

    let img = parse_dds(include_bytes!("data/red8x8_bc7_srgb.dds")).expect("fixture parses");
    let material = MaterialData {
        base_color_image: Some(SharedImage::new(TextureSource::Compressed(img))),
        roughness_factor: 1.0,
        metallic_factor:  0.0,
        ..Default::default()
    };
    let target = r.target(W, H);
    r.render_mesh(&target, quad_with_material(40.0, material), wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let center = ((H / 2 * W + W / 2) * 4) as usize;
    let p = &pixels[center..center + 4];
    assert!(
        p[0] as u32 > 2 * p[1] as u32 && p[0] as u32 > 2 * p[2] as u32,
        "BC7 sRGB red fixture must dominate at center: {p:?}"
    );
}

/// The blit chain really downsamples — mip 1 of a 1-px checkerboard
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

/// A quad perpendicular to the camera's view axis, centered on the target
/// and sized to exactly fill the frustum at the default orbit's distance (34
/// units — the same constant `damaged_helmet_renders` frames the helmet
/// against). `right`/`up` are built the same way `Mat4::look_at_rh` derives
/// its camera-space x/y axes, so the quad projects to an axis-aligned
/// rectangle with no outer silhouette inside the frame: every background-
/// colored pixel in a render comes from the material's own alpha cutout,
/// never from geometry.
fn camera_filling_quad(material: MaterialData) -> MeshData {
    MeshData {
        primitives: vec![view_quad(1.0, material)],
        skeleton:   None,
        clips:      Vec::new(),
    }
}

/// Generalizes the frame-filling quad to any depth along the view ray:
/// same eye/right/up math as `camera_filling_quad`, but centered at
/// `dist_frac` of the way from the eye to the origin, with the half-extent
/// scaled so the quad still exactly fills the frame at that depth. Lets a
/// scene stack multiple depth-separated quads (opaque backdrop + one or more
/// blend layers) all facing the camera.
fn view_quad(dist_frac: f32, material: MaterialData) -> PrimitiveData {
    let radius = 34.0f32;
    let angle: f32 = std::f32::consts::FRAC_PI_4;
    let pitch = 0.8f32;
    let eye = Vec3::new(
        radius * angle.cos() * pitch.cos(),
        radius * pitch.sin(),
        radius * angle.sin() * pitch.cos(),
    );
    let forward = (-eye).normalize(); // target is the origin
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let normal = -forward;
    let center = eye + forward * (radius * dist_frac);

    let fovy = 45.0_f32.to_radians();
    let half = radius * dist_frac * (fovy / 2.0).tan() * 1.02; // 2% past the exact frustum edge

    let vert = |u: f32, v: f32| MeshVertex {
        position: (center + right * (u * 2.0 - 1.0) * half + up * (v * 2.0 - 1.0) * half).to_array(),
        normal:   normal.to_array(),
        uv:       [u, v],
        tangent:  [1.0, 0.0, 0.0, 1.0],
    };
    PrimitiveData {
        vertices: vec![vert(0.0, 0.0), vert(1.0, 0.0), vert(1.0, 1.0), vert(0.0, 1.0)],
        indices:  vec![0, 2, 1, 0, 3, 2],
        material,
        skin: None,
    }
}

/// 64² RGBA8: opaque white with an alpha ramp along the UV diagonal
/// (u - v), crossing the material's 0.5 cutoff exactly on the u==v line —
/// a masked edge that isn't aligned to the pixel or texel grid.
fn diagonal_alpha_texture(size: u32) -> ImageData {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32;
            let v = (y as f32 + 0.5) / size as f32;
            let a = ((u - v) + 0.5).clamp(0.0, 1.0);
            pixels.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    ImageData { width: size, height: size, pixels }
}

fn masked_billboard() -> MeshData {
    camera_filling_quad(MaterialData {
        alpha_mode:        AlphaMode::Mask(0.5),
        metallic_factor:   0.0,
        roughness_factor:  1.0,
        base_color_image:  Some(SharedImage::new(TextureSource::Rgba8(diagonal_alpha_texture(64)))),
        ..Default::default()
    })
}

/// Masked cutout edges are per-fragment discard today, which is a binary
/// keep/kill regardless of MSAA sample count: unlike a geometric silhouette
/// (`msaa_produces_intermediate_edge_pixels`), the boundary along the
/// texture's alpha ramp must also resolve to intermediate pixel values, not
/// a hard step between the lit body and the black background.
#[test]
fn masked_cutout_edge_has_intermediate_resolved_pixels() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_uniform_environment([1.0, 1.0, 1.0]);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });

    let target = r.target(W, H);
    r.render_mesh(&target, masked_billboard(), wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let body_peak = pixels.chunks_exact(4).map(|p| p[0]).max().unwrap();
    assert!(body_peak > 60, "cutout body must be visible, peak {body_peak}");
    let intermediates = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 12 && p[0] < body_peak - 12)
        .count();
    assert!(
        intermediates > 50,
        "alpha-to-coverage must leave intermediate pixels along the diagonal cutout edge, found {intermediates}"
    );
}

/// A Blend material must composite real coverage instead of the old
/// mask-approximated cutout: a red glass quad in front of a white opaque
/// quad must let the white show through (tinted red), not punch an opaque
/// red hole in the frame.
#[test]
fn blend_material_blends_instead_of_cutout() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_uniform_environment([1.0, 1.0, 1.0]);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });

    let white_opaque = view_quad(1.0, MaterialData {
        base_color_factor: [1.0, 1.0, 1.0, 1.0],
        roughness_factor:  1.0,
        metallic_factor:   0.0,
        ..Default::default()
    });
    let red_glass = view_quad(0.5, MaterialData {
        base_color_factor: [1.0, 0.0, 0.0, 0.6],
        alpha_mode:        AlphaMode::Blend,
        roughness_factor:  1.0,
        metallic_factor:   0.0,
        ..Default::default()
    });
    let data = MeshData {
        primitives: vec![white_opaque, red_glass],
        skeleton:   None,
        clips:      Vec::new(),
    };

    let target = r.target(W, H);
    r.render_mesh(&target, data, wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let (r_mean, g_mean) = (channel_mean_linear(&pixels, 0), channel_mean_linear(&pixels, 1));
    assert!(
        g_mean > 25.0,
        "white layer must show through the glass (not an opaque cutout), g_mean={g_mean:.1}"
    );
    assert!(
        r_mean > g_mean * 1.3,
        "the glass must tint the frame red: r={r_mean:.1} g={g_mean:.1}"
    );
}

/// Stacked transparents must composite back-to-front regardless of the
/// primitive vec's order: a near blue glass drawn before a far red glass
/// (deliberately adversarial order) over an opaque white backdrop must still
/// resolve as if drawn far-to-near — the near blue layer dominates. Drawing
/// in raw vec order instead would let the (nearer, but vec-first) blue layer
/// get overpainted by the farther red one, flipping the dominant channel.
#[test]
fn stacked_glass_composites_back_to_front() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_uniform_environment([1.0, 1.0, 1.0]);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });

    let white_opaque = view_quad(1.0, MaterialData {
        base_color_factor: [1.0, 1.0, 1.0, 1.0],
        roughness_factor:  1.0,
        metallic_factor:   0.0,
        ..Default::default()
    });
    let blue_near = view_quad(0.5, MaterialData {
        base_color_factor: [0.0, 0.0, 1.0, 0.5],
        alpha_mode:        AlphaMode::Blend,
        roughness_factor:  1.0,
        metallic_factor:   0.0,
        ..Default::default()
    });
    let red_far = view_quad(0.75, MaterialData {
        base_color_factor: [1.0, 0.0, 0.0, 0.5],
        alpha_mode:        AlphaMode::Blend,
        roughness_factor:  1.0,
        metallic_factor:   0.0,
        ..Default::default()
    });
    let data = MeshData {
        primitives: vec![white_opaque, blue_near, red_far], // adversarial: near before far
        skeleton:   None,
        clips:      Vec::new(),
    };

    let target = r.target(W, H);
    r.render_mesh(&target, data, wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let (r_mean, b_mean) = (channel_mean(&pixels, 0), channel_mean(&pixels, 2));
    assert!(
        b_mean > r_mean,
        "correct back-to-front order must let the near blue layer dominate: r={r_mean:.1} b={b_mean:.1}"
    );
}

// ── HDR / tonemap / MSAA / IBL ───────────────────────────────────────────────

/// HDR values survive to the tonemap — an 8× emissive quad tonemaps
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

/// MSAA 4× produces intermediate coverage values along a silhouette
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

/// White-furnace-style sanity: under a uniform white environment with
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
        .map(luminance)
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

// ── Bloom ────────────────────────────────────────────────────────────────────

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

/// VQ-C3's "HDR emissive > 1.0 blooms" must hold in display-referred space:
/// a 1.5-raw emissive displays at 0.75 under half exposure (below the 1.0
/// threshold) and must not bloom, even though its raw value is still > 1.0.
/// At exposure 1.0 the same raw value displays unchanged and still blooms.
#[test]
fn bloom_threshold_is_display_referred_after_exposure() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 0.0 });

    let scene = || {
        quad_with_material(3.0, MaterialData {
            base_color_factor: [0.0, 0.0, 0.0, 1.0],
            emissive_factor:   [1.0, 1.0, 1.0],
            emissive_strength: 1.5, // raw > 1.0, but *0.5 exposure displays at 0.75
            ..Default::default()
        })
    };
    let halo_count = |with: &[u8], without: &[u8]| {
        with.chunks_exact(4).zip(without.chunks_exact(4))
            .filter(|(a, b)| linear(a[1]) > 6.0 && linear(b[1]) <= 2.0)
            .count()
    };

    r.set_exposure(0.5);
    r.set_bloom_intensity(0.5);
    let target_on = r.target(W, H);
    r.render_mesh(&target_on, scene(), wgpu::Color::BLACK);
    let with_bloom = r.read(&target_on);
    r.set_bloom_intensity(0.0);
    let target_off = r.target(W, H);
    r.render_mesh(&target_off, scene(), wgpu::Color::BLACK);
    let without_bloom = r.read(&target_off);
    let halo_half_exposure = halo_count(&with_bloom, &without_bloom);
    assert_eq!(
        halo_half_exposure, 0,
        "1.5-raw emissive displays at 0.75 under 0.5 exposure — must not bloom, halo={halo_half_exposure}px"
    );

    r.set_exposure(1.0);
    r.set_bloom_intensity(0.5);
    let target_on = r.target(W, H);
    r.render_mesh(&target_on, scene(), wgpu::Color::BLACK);
    let with_bloom = r.read(&target_on);
    r.set_bloom_intensity(0.0);
    let target_off = r.target(W, H);
    r.render_mesh(&target_off, scene(), wgpu::Color::BLACK);
    let without_bloom = r.read(&target_off);
    let halo_full_exposure = halo_count(&with_bloom, &without_bloom);
    assert!(
        halo_full_exposure > 0,
        "1.5-raw emissive at exposure 1.0 must still bloom as before, halo={halo_full_exposure}px"
    );
}

// ── Shadows ──────────────────────────────────────────────────────────────────

/// A cube floating above a ground slab under a 45° sun casts a dark
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

/// Left half (u<0.5) fully transparent, right half fully opaque — a hard
/// MASK edge at the texture's u==0.5 line matching the material's 0.5 cutoff.
fn half_alpha_texture(size: u32) -> ImageData {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for _y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32;
            let a = if u < 0.5 { 0u8 } else { 255u8 };
            pixels.extend_from_slice(&[255, 255, 255, a]);
        }
    }
    ImageData { width: size, height: size, pixels }
}

/// A ground slab (extent 20, y=0) plus a horizontal MASK occluder (extent 8,
/// y=5) centered above it: the occluder's -x half is a cutout (alpha 0) and
/// its +x half opaque (alpha 255), so a correct shadow pass must let light
/// through the cutout half while the opaque half still casts one.
fn masked_occluder_scene() -> MeshData {
    let mut data = ground_quad(20.0, 1.0, 0.0);
    let e = 8.0;
    let v = |x: f32, z: f32, u: f32, w: f32| MeshVertex {
        position: [x, 5.0, z],
        normal:   [0.0, 1.0, 0.0],
        uv:       [u, w],
        tangent:  [1.0, 0.0, 0.0, 1.0],
    };
    data.primitives.push(PrimitiveData {
        vertices: vec![v(-e, -e, 0.0, 0.0), v(e, -e, 1.0, 0.0), v(e, e, 1.0, 1.0), v(-e, e, 0.0, 1.0)],
        indices:  vec![0, 2, 1, 0, 3, 2],
        material: MaterialData {
            alpha_mode:        AlphaMode::Mask(0.5),
            base_color_image:  Some(SharedImage::new(TextureSource::Rgba8(half_alpha_texture(64)))),
            roughness_factor:  1.0,
            metallic_factor:   0.0,
            ..Default::default()
        },
        skin: None,
    });
    data
}

/// A MASK primitive's cutout region must not cast a shadow: with a 45° sun
/// (`direction = (-1,1,0)`, so a caster at height h shadows the ground at
/// `x + h`) and a top-down orthographic camera (world x/z maps linearly to
/// pixel columns/rows via the ±20 ortho half-extent — no projection math
/// needed), `masked_occluder_scene`'s cutout half (x0 ∈ [-8,0)) shadows
/// ground x ∈ [-3,5), of which only x ∈ [-3,0) is camera-visible (outside
/// that the opaque half of the occluder itself blocks the view). That
/// visible slice must read as bright as untouched ground, not as dark as the
/// opaque half's real shadow (x ∈ (8,13], also camera-visible, beyond the
/// occluder's own footprint).
#[test]
fn masked_shadow_cutout_lets_light_through() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_camera_top_down();
    r.set_light(TestLight { direction: Vec3::new(-1.0, 1.0, 0.0), color: Vec3::ONE, ambient: 0.0 });

    let target = r.target(W, H);
    r.render_mesh(&target, masked_occluder_scene(), wgpu::Color::BLACK);
    let pixels = r.read(&target);

    // World x -> pixel column: x=-20 -> col 0, x=20 -> col W (ortho half-extent 20).
    let col = |x: f32| ((x + 20.0) / 40.0 * W as f32).round() as u32;
    let rows = H * 3 / 8..H * 5 / 8; // world z in roughly [-5,5], well inside the occluder's footprint

    let lit_ref      = luminance_tile_mean(&pixels, W, col(-18.0)..col(-15.0), rows.clone());
    let cutout_zone   = luminance_tile_mean(&pixels, W, col(-2.6)..col(-0.8), rows.clone());
    let shadowed_ref  = luminance_tile_mean(&pixels, W, col(9.0)..col(12.0), rows);

    assert!(
        shadowed_ref < lit_ref * 0.5,
        "sanity: the opaque half's real shadow must read clearly darker than lit ground: lit={lit_ref:.1} shadowed={shadowed_ref:.1}"
    );
    assert!(
        cutout_zone > lit_ref * 0.7,
        "sun must pass through the MASK cutout: lit={lit_ref:.1} cutout={cutout_zone:.1} (shadowed_ref={shadowed_ref:.1})"
    );
}

/// With the sky pass on, background pixels show the environment
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

/// A fogged zone's sky must blend toward the fog color near the horizon
/// instead of showing a hard seam against the fogged ground. `set_camera_level`
/// puts the boresight exactly on the horizon: the bottom half of the frustum
/// sees below-horizon rays (clamped, so the blend factor there is exactly 1),
/// the top rows sit near the zenith where the blend must stay negligible.
/// Density 0 must reproduce today's image bit-for-bit regardless of fog color.
#[test]
fn sky_fog_blends_toward_horizon_and_stays_bit_stable_at_zero_density() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.draw_sky = true;
    r.set_camera_level();

    let env = Vec3::new(0.05, 0.05, 0.4);
    let fog = Vec3::new(0.8, 0.5, 0.2);

    r.set_uniform_environment(env.to_array());
    let target = r.target(W, H);
    r.render_sdf(&target, &[], wgpu::Color::BLACK);
    let baseline = r.read(&target);

    r.set_fog(fog, 0.0);
    let target = r.target(W, H);
    r.render_sdf(&target, &[], wgpu::Color::BLACK);
    let zero_density = r.read(&target);
    assert_eq!(zero_density, baseline, "fog_density 0 must not perturb the image");

    r.set_fog(fog, 1.0);
    let target = r.target(W, H);
    r.render_sdf(&target, &[], wgpu::Color::BLACK);
    let high_density = r.read(&target);

    // Reference: sky texture == fog color, so any blend factor reproduces it.
    r.set_uniform_environment(fog.to_array());
    r.set_fog(fog, 0.0);
    let target = r.target(W, H);
    r.render_sdf(&target, &[], wgpu::Color::BLACK);
    let fog_reference = r.read(&target);

    assert!(
        rows_close(&high_density, &fog_reference, W, H / 2..H, 2),
        "below-horizon rows must converge on the fog color"
    );
    let zenith_band = 0..(H * 15 / 100);
    assert!(
        rows_close(&linearize(&high_density), &linearize(&zero_density), W, zenith_band, 18),
        "near-zenith rows must stay close to the unfogged sky"
    );
}

// ── Fog height ───────────────────────────────────────────────────────────────

/// Fog density is attenuated above a configured `fog_height`: the same
/// ground quad, same eye distance, renders through far more fog when the
/// fog height sits high above it (no height attenuation) than when the fog
/// height sits far below it (heavily attenuated) — height is the only
/// variable. `set_fog(_, 0.0)` + `set_fog_height(0.0, 0.0)` must still
/// reproduce today's unfogged render bit-for-bit.
#[test]
fn height_fog_attenuates_above_configured_fog_height() {
    let Some(mut r) = renderer_or_skip() else { return };
    let fog_color = Vec3::new(1.0, 0.0, 0.0);

    let target = r.target(W, H);
    r.render_mesh(&target, ground_quad(40.0, 0.9, 0.0), wgpu::Color::BLACK);
    let no_fog = r.read(&target);

    r.set_fog(fog_color, 0.05);
    r.set_fog_height(100.0, 0.15); // quad (y=0) far below fog height -> no attenuation
    let target = r.target(W, H);
    r.render_mesh(&target, ground_quad(40.0, 0.9, 0.0), wgpu::Color::BLACK);
    let below = r.read(&target);

    r.set_fog_height(-100.0, 0.15); // quad far above fog height -> heavily attenuated
    let target = r.target(W, H);
    r.render_mesh(&target, ground_quad(40.0, 0.9, 0.0), wgpu::Color::BLACK);
    let above = r.read(&target);

    let (below_r, above_r) = (channel_mean(&below, 0), channel_mean(&above, 0));
    assert!(
        below_r > above_r + 10.0,
        "quad below fog height must fog toward red much more: below={below_r:.1} above={above_r:.1}"
    );

    r.set_fog(fog_color, 0.0);
    r.set_fog_height(0.0, 0.0);
    let target = r.target(W, H);
    r.render_mesh(&target, ground_quad(40.0, 0.9, 0.0), wgpu::Color::BLACK);
    let zeroed = r.read(&target);
    assert_eq!(zeroed, no_fog, "density 0 + falloff 0 must reproduce the unfogged render exactly");
}

/// The BRDF LUT depends only on (NdotV, roughness), never on environment
/// data, so it must bake once at renderer init and every `Environment`
/// (zone crossing) shares that view rather than rebaking it.
#[test]
fn repeated_environment_loads_skip_redundant_brdf_bake() {
    let Some(mut r) = renderer_or_skip() else { return };
    let after_init = OffscreenRenderer::brdf_bake_count();

    for i in 0..5 {
        let v = 0.1 + i as f32 * 0.05;
        r.set_uniform_environment([v, v * 0.8, v * 1.1]);
    }

    assert_eq!(
        OffscreenRenderer::brdf_bake_count(), after_init,
        "5 environment loads (zone crossings) must not rebake the shared BRDF LUT"
    );
}

/// The IBL `Baker` (shader module + all four bake pipelines) is a pure
/// function of the device, never of the environment pixels, so it must
/// compile once at renderer init and every `Environment` (zone crossing)
/// reuses that baker rather than recompiling its pipelines.
#[test]
fn repeated_environment_loads_skip_redundant_baker_construction() {
    let Some(mut r) = renderer_or_skip() else { return };
    let after_init = OffscreenRenderer::baker_construction_count();

    for i in 0..3 {
        let v = 0.1 + i as f32 * 0.05;
        r.set_uniform_environment([v, v * 0.8, v * 1.1]);
    }

    assert_eq!(
        OffscreenRenderer::baker_construction_count(), after_init,
        "3 environment loads (zone crossings) must not reconstruct the shared Baker"
    );
}

// ── Point lights ─────────────────────────────────────────────────────────────

fn mean_luminance(pixels: &[u8]) -> f64 {
    pixels.chunks_exact(4).map(luminance).sum::<f64>() / (pixels.len() / 4) as f64
}

/// `mean_luminance`, decoded to linear light first (per-pixel decode has to
/// happen before averaging, not after, so this can't reuse byte-space
/// `luminance`).
fn mean_luminance_linear(pixels: &[u8]) -> f64 {
    pixels.chunks_exact(4)
        .map(|p| 0.2126 * linear(p[0]) + 0.7152 * linear(p[1]) + 0.0722 * linear(p[2]))
        .sum::<f64>() / (pixels.len() / 4) as f64
}

/// A point light brightens an otherwise-unlit ground plane, falls off
/// monotonically as it moves farther from the surface, has no effect once its
/// own position sits outside its radius, and carries its color through.
#[test]
fn point_light_brightens_falls_off_and_carries_color() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 0.0 });

    let mut render = |lights: &[TestPointLight]| -> Vec<u8> {
        r.set_point_lights(lights);
        let target = r.target(W, H);
        r.render_mesh(&target, ground_quad(20.0, 0.9, 0.0), wgpu::Color::BLACK);
        r.read(&target)
    };

    let cyan = Vec3::new(0.2, 0.8, 1.0);
    let light_at = |height: f32| TestPointLight {
        position:  Vec3::new(0.0, height, 0.0),
        color:     cyan,
        intensity: 30.0,
        radius:    30.0,
    };

    let unlit_mean = mean_luminance_linear(&render(&[]));

    let lit_2 = render(&[light_at(2.0)]);
    let lit_2_mean = mean_luminance_linear(&lit_2);
    // ACES tonemap compresses the raw ratio; measured ~4-8x at these
    // parameters, well above the 4x floor.
    assert!(
        lit_2_mean >= unlit_mean * 4.0,
        "point light must brighten the ground clearly: unlit={unlit_mean:.2} lit={lit_2_mean:.2}"
    );

    let lit_6_mean = mean_luminance_linear(&render(&[light_at(6.0)]));
    let lit_12_mean = mean_luminance_linear(&render(&[light_at(12.0)]));
    assert!(
        lit_2_mean > lit_6_mean && lit_6_mean > lit_12_mean,
        "luminance must fall off monotonically with distance: h2={lit_2_mean:.2} h6={lit_6_mean:.2} h12={lit_12_mean:.2}"
    );

    let out_of_radius_mean = mean_luminance_linear(&render(&[TestPointLight {
        position:  Vec3::new(0.0, 2.0, 0.0),
        color:     cyan,
        intensity: 30.0,
        radius:    1.0,
    }]));
    assert!(
        (out_of_radius_mean - unlit_mean).abs() < 1.0,
        "a light outside its own radius must reproduce the unlit image: unlit={unlit_mean:.2} got={out_of_radius_mean:.2}"
    );

    let (r_mean, b_mean) = (channel_mean(&lit_2, 0), channel_mean(&lit_2, 2));
    assert!(
        b_mean > r_mean,
        "cyan light's blue channel must exceed its red channel: r={r_mean:.2} b={b_mean:.2}"
    );
}

// ── SSAO ─────────────────────────────────────────────────────────────────────

/// Mean AO visibility (0..1, R32Float) over a pixel-space rectangle of the
/// full-res AO readback (`ao` is `ao_w`-wide, row-major, one f32/texel).
fn ao_tile_mean(ao: &[f32], ao_w: u32, x: Range<u32>, y: Range<u32>) -> f64 {
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for row in y {
        for col in x.clone() {
            sum += ao[(row * ao_w + col) as usize] as f64;
            count += 1;
        }
    }
    assert!(count > 0, "empty tile");
    sum / count as f64
}

/// The shared scene both SSAO tests frame: a 60×60 ground slab (top surface
/// at y=0) and a 6×1×6 box resting on it (footprint x/z in [-3,3]), with
/// the camera aimed obliquely at the box's +X wall base so the wall face and
/// the ground in front of it fill the frame — the crease line runs through
/// the frame center, so tiles need no world→pixel math. An angled view is
/// essential: GTAO integrates horizons from visible surfaces only, and its
/// effect radius (XeGTAO default ≈ 0.73 m) is shorter than the wall, so a
/// straight-down framing that hides the wall face shows no crease.
fn ssao_crease_scene() -> (SdfInstance, SdfInstance) {
    let ground = SdfInstance {
        model: Mat4::from_scale_rotation_translation(
            Vec3::new(60.0, 1.0, 60.0), glam::Quat::IDENTITY, Vec3::new(0.0, -0.5, 0.0),
        ).to_cols_array_2d(),
        color: [1.0, 1.0, 1.0], shape_type: 0, shape_params: [0.0; 4],
    };
    let box_on_ground = SdfInstance {
        model: Mat4::from_scale_rotation_translation(
            Vec3::new(6.0, 1.0, 6.0), glam::Quat::IDENTITY, Vec3::new(0.0, 0.5, 0.0),
        ).to_cols_array_2d(),
        color: [1.0, 1.0, 1.0], shape_type: 0, shape_params: [0.0; 4],
    };
    (ground, box_on_ground)
}

const SSAO_CREASE_EYE: Vec3 = Vec3::new(6.0, 2.0, 0.0);
const SSAO_CREASE_TARGET: Vec3 = Vec3::new(3.0, 0.0, 0.0);

/// The box-and-ground scene of `ssao_crease_scene`, with the sun off and
/// uniform env ambient: the AO texture over the ground just in front of the
/// wall must read clearly darker with the box present than the same pixels
/// over bare ground, while ground metres away (outside the effect radius)
/// stays untouched — the depth prepass + GTAO + denoise pipeline producing a
/// real, geometry-driven occlusion signal end to end.
#[test]
fn ssao_darkens_box_ground_contact_crease() {
    let Some(mut r) = renderer_or_skip() else { return };
    const SIZE: u32 = 512;
    r.set_ssao(true);
    r.set_camera_lookat(SSAO_CREASE_EYE, SSAO_CREASE_TARGET);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });
    let (ground, box_on_ground) = ssao_crease_scene();

    let target = r.target(SIZE, SIZE);
    r.render_sdf(&target, &[ground, box_on_ground], wgpu::Color::BLACK);
    let ao_with = r.ao_readback(&target).expect("set_ssao(true) must produce an AO readback");

    let target = r.target(SIZE, SIZE);
    r.render_sdf(&target, &[ground], wgpu::Color::BLACK);
    let ao_without = r.ao_readback(&target).expect("set_ssao(true) must produce an AO readback");

    // Rows just below the frame center: ground within ~25 cm of the wall
    // base (the boresight hits the crease), inside the effect radius.
    let crease_with    = ao_tile_mean(&ao_with,    SIZE, SIZE / 2 - 40..SIZE / 2 + 40, SIZE / 2 + 4..SIZE / 2 + 24);
    let crease_without = ao_tile_mean(&ao_without, SIZE, SIZE / 2 - 40..SIZE / 2 + 40, SIZE / 2 + 4..SIZE / 2 + 24);
    assert!(
        crease_with < crease_without * 0.9,
        "the wall must darken the ground at its base: with={crease_with:.3} without={crease_without:.3}"
    );

    // Rows near the bottom edge: ground ~1.5 m from the wall, outside the
    // effect radius — the box must not reach it.
    let open_with    = ao_tile_mean(&ao_with,    SIZE, SIZE / 2 - 40..SIZE / 2 + 40, SIZE - 60..SIZE - 20);
    let open_without = ao_tile_mean(&ao_without, SIZE, SIZE / 2 - 40..SIZE / 2 + 40, SIZE - 60..SIZE - 20);
    assert!(
        (open_with - open_without).abs() < 0.05 * open_without,
        "open ground outside the effect radius must stay within noise: with={open_with:.3} without={open_without:.3}"
    );
}

/// Mean luminance (`luminance`) over a pixel-space rectangle of a tonemapped
/// RGBA8 frame (`width`-wide, row-major, 4 bytes/pixel).
fn luminance_tile_mean(pixels: &[u8], width: u32, x: Range<u32>, y: Range<u32>) -> f64 {
    let mut sum = 0.0;
    let mut count = 0u64;
    for row in y {
        for col in x.clone() {
            let i = ((row * width + col) * 4) as usize;
            sum += luminance(&pixels[i..i + 4]);
            count += 1;
        }
    }
    assert!(count > 0, "empty tile");
    sum / count as f64
}

/// Same scene and framing as `ssao_darkens_box_ground_contact_crease`,
/// through the real tonemapped output rather than the raw AO readback: proves
/// `shade_pbr` actually consumes the AO texture to darken ambient (not just
/// that the AO pass produces one), and that the `set_ssao` toggle governs it.
/// SSAO on must read clearly darker at the contact crease than SSAO off,
/// while open ground outside the effect radius — where AO ≈ 1 either way —
/// stays within noise.
#[test]
fn ssao_darkens_final_image_crease_vs_open_ground() {
    let Some(mut r) = renderer_or_skip() else { return };
    const SIZE: u32 = 512;
    r.set_camera_lookat(SSAO_CREASE_EYE, SSAO_CREASE_TARGET);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });
    let (ground, box_on_ground) = ssao_crease_scene();

    r.set_ssao(false);
    let target = r.target(SIZE, SIZE);
    r.render_sdf(&target, &[ground, box_on_ground], wgpu::Color::BLACK);
    let off = r.read(&target);

    r.set_ssao(true);
    let target = r.target(SIZE, SIZE);
    r.render_sdf(&target, &[ground, box_on_ground], wgpu::Color::BLACK);
    let on = r.read(&target);

    // The same tiles as the AO-buffer test: crease rows just below the frame
    // center, open ground near the bottom edge.
    let crease_off = luminance_tile_mean(&off, SIZE, SIZE / 2 - 40..SIZE / 2 + 40, SIZE / 2 + 4..SIZE / 2 + 24);
    let crease_on  = luminance_tile_mean(&on,  SIZE, SIZE / 2 - 40..SIZE / 2 + 40, SIZE / 2 + 4..SIZE / 2 + 24);
    let open_off = luminance_tile_mean(&off, SIZE, SIZE / 2 - 40..SIZE / 2 + 40, SIZE - 60..SIZE - 20);
    let open_on  = luminance_tile_mean(&on,  SIZE, SIZE / 2 - 40..SIZE / 2 + 40, SIZE - 60..SIZE - 20);

    assert!(
        crease_on < crease_off * 0.95,
        "SSAO must darken the contact crease in the final image: off={crease_off:.1} on={crease_on:.1}"
    );
    assert!(
        (open_on - open_off).abs() < 0.05 * open_off,
        "open ground outside the effect radius must stay within noise: off={open_off:.1} on={open_on:.1}"
    );
}

/// Proves the debug-channel path end to end: uniform → shader switch → HDR
/// target → tonemap passthrough → `Rgba8Unorm` readback. With no transfer
/// function in that path, `byte = round(255 * value)` is an exact identity,
/// so a material's `roughness_factor: 0.6` must read back as G = 153 (±2 for
/// rounding/driver noise) at the quad's center. If this drifts, no debug
/// channel downstream can be trusted.
#[test]
fn debug_channel_roughness_reads_back_as_exact_byte() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_uniform_environment([1.0, 1.0, 1.0]);
    r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 1.0 });
    r.set_debug_channel(DebugChannel::Roughness);

    let material = MaterialData {
        roughness_factor: 0.6,
        metallic_factor:  0.0,
        ..Default::default()
    };
    let target = r.target(W, H);
    r.render_mesh(&target, camera_filling_quad(material), wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let center = ((H / 2 * W + W / 2) * 4) as usize;
    let g = pixels[center + 1] as i32;
    assert!((g - 153).abs() <= 2, "center G byte must be 153 ± 2, got {g}");
}

// ── World-space detail overlay ──────────────────────────────────────────────

/// A 2×2 tangent-space normal map checkerboarding between two mirror-image
/// tilts — spatially varying (unlike a 1×1 fixture) so triplanar-projecting
/// it across a surface produces real per-pixel normal variance. Reused as the
/// shared detail tile's normal map below.
fn detail_normal_tile() -> ImageData {
    checker_normal_texture()
}

/// A 2×2 albedo tile whose per-texel luminance alternates around the
/// overlay's 0.5 identity — the detail tile's color map. Spatially varying
/// for the same reason as `detail_normal_tile`.
fn detail_albedo_tile() -> ImageData {
    let a: [u8; 4] = [96, 96, 96, 255];
    let b: [u8; 4] = [160, 160, 160, 255];
    let mut pixels = Vec::with_capacity(16);
    for row in [[a, b], [b, a]] {
        for px in row {
            pixels.extend_from_slice(&px);
        }
    }
    ImageData { width: 2, height: 2, pixels }
}

/// The shared detail-overlay material the tests below bind at group 3 — a
/// synthetic non-flat tile standing in for the real CC0 photoscan (T1).
fn detail_tile_material() -> MaterialData {
    MaterialData {
        base_color_image: Some(SharedImage::new(TextureSource::Rgba8(detail_albedo_tile()))),
        normal_image:     Some(SharedImage::new(TextureSource::Rgba8(detail_normal_tile()))),
        ..Default::default()
    }
}

/// A flat, unmapped subject material (no base normal map) whose only source
/// of normal/roughness variation, if any, is the world-space detail overlay —
/// `detail` selects whether it opts in.
fn stone_quad_material(detail: bool) -> MaterialData {
    MaterialData {
        base_color_factor: [0.5, 0.5, 0.5, 1.0],
        roughness_factor:  0.5,
        metallic_factor:   0.0,
        detail,
        ..Default::default()
    }
}

/// `detail = false` must render byte-identically whether or not a real
/// (non-flat) tile is bound at group 3 — the uniform-gated branch in
/// `mesh_shader.wgsl` must never execute for opted-out content. Proves the
/// gate and the 1×1 neutral default together: an asset that never opts in
/// must render exactly as it did before this layer existed, regardless of
/// what the engine has bound for other, opted-in props.
#[test]
fn detail_layer_is_inert_without_opt_in() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_camera_lookat(Vec3::new(1.0, 2.0, 0.0), Vec3::ZERO);
    let scene = || quad_with_material(40.0, stone_quad_material(false));

    let target_neutral = r.target(W, H);
    r.render_mesh(&target_neutral, scene(), wgpu::Color::BLACK);
    let neutral_bound = r.read(&target_neutral);

    r.set_detail_material(detail_tile_material());
    let target_real = r.target(W, H);
    r.render_mesh(&target_real, scene(), wgpu::Color::BLACK);
    let real_tile_bound = r.read(&target_real);

    assert_eq!(
        neutral_bound, real_tile_bound,
        "detail=false must ignore whatever tile is bound at group 3"
    );
}

/// Opting in (`detail = true`) with a spatially-varying tile bound close up
/// must perturb both the shaded normal and roughness across the surface — a
/// flat quad with no base normal map has ~zero variance in either debug
/// channel until the world-space overlay adds it.
#[test]
fn detail_layer_perturbs_normal_and_roughness_when_opted_in() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_camera_lookat(Vec3::new(1.0, 2.0, 0.0), Vec3::ZERO);
    r.set_detail_material(detail_tile_material());

    let mut render = |channel: DebugChannel, detail: bool| -> Vec<u8> {
        r.set_debug_channel(channel);
        let target = r.target(W, H);
        r.render_mesh(&target, quad_with_material(40.0, stone_quad_material(detail)), wgpu::Color::BLACK);
        r.read(&target)
    };
    let variance3 = |p: &[u8]| channel_variance(p, 0) + channel_variance(p, 1) + channel_variance(p, 2);

    let var_normal_off = variance3(&render(DebugChannel::Normal, false));
    let var_normal_on  = variance3(&render(DebugChannel::Normal, true));
    let var_rough_off  = channel_variance(&render(DebugChannel::Roughness, false), 0);
    let var_rough_on   = channel_variance(&render(DebugChannel::Roughness, true), 0);

    assert!(var_normal_off < 0.5, "no base normal map, detail off: normal must be ~uniform, got variance {var_normal_off:.4}");
    assert!(
        var_normal_on > var_normal_off + 1.0,
        "opted-in detail must add normal variance: off={var_normal_off:.4} on={var_normal_on:.4}"
    );
    assert!(var_rough_off < 0.5, "constant roughness factor, detail off: must be ~uniform, got variance {var_rough_off:.4}");
    assert!(
        var_rough_on > var_rough_off + 1.0,
        "opted-in detail must add roughness variance: off={var_rough_off:.4} on={var_rough_on:.4}"
    );
}

/// The mandatory distance fade (`DETAIL_NORMAL_FADE_NEAR`/`_FAR` in
/// `snippets/detail_triplanar.wgsl`): the same opted-in quad shows real
/// normal variance close up (2 m, inside the fade-in band) and only a small
/// fraction of it far away (20 m, well beyond the 10 m cutoff) — the
/// aliasing requirement made machine-checkable, and a probe that fails if the
/// fade regresses to "always on" or "always off".
#[test]
fn detail_normal_fades_out_with_distance() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_detail_material(detail_tile_material());
    r.set_debug_channel(DebugChannel::Normal);

    let mut render_at = |eye_height: f32| -> Vec<u8> {
        r.set_camera_lookat(Vec3::new(eye_height * 0.5, eye_height, 0.0), Vec3::ZERO);
        let target = r.target(W, H);
        r.render_mesh(&target, quad_with_material(40.0, stone_quad_material(true)), wgpu::Color::BLACK);
        r.read(&target)
    };
    let variance3 = |p: &[u8]| channel_variance(p, 0) + channel_variance(p, 1) + channel_variance(p, 2);

    let var_close = variance3(&render_at(2.0));
    let var_far   = variance3(&render_at(20.0));

    assert!(var_close > 1.0, "close range (2 m) must show real detail variance, got {var_close:.4}");
    assert!(
        var_far < var_close * 0.1,
        "far range (20 m, beyond the 10 m fade cutoff) must be a small fraction of close: close={var_close:.4} far={var_far:.4}"
    );
}

/// A detail tile whose albedo is exactly the DC-neutral byte value
/// `content_lint`'s `detail_tile_is_dc_neutral` enforces on the real asset —
/// unlike `detail_albedo_tile`, which is spatially varying for the variance
/// tests above, this is flat so it isolates the overlay's identity point.
fn detail_dc_neutral_tile() -> MaterialData {
    MaterialData {
        base_color_image: Some(SharedImage::new(TextureSource::Rgba8(ImageData {
            width:  1,
            height: 1,
            pixels: vec![128, 128, 128, 255],
        }))),
        ..Default::default()
    }
}

/// The overlay's whole justification is "no visible pixel change on a
/// DC-neutral tile". A uniform darkening (or brightening) preserves variance,
/// which is why every other test in this file — measuring variance — cannot
/// see it: `DebugChannel::Albedo` reads the shaded albedo directly (no
/// lighting/tonemap confound), decoded to linear light before comparing, so a
/// multiplicative bias shows up as a mean shift with nothing else to explain
/// it. `DebugChannel::Roughness` is a direct byte passthrough (see
/// `debug_channel_roughness_reads_back_as_exact_byte`), so the same identity
/// check applies without a linear decode: `ratio == 1` on a DC-neutral tile
/// must leave `roughness_delta` at exactly 0.
#[test]
fn detail_layer_albedo_and_roughness_are_near_identity_on_dc_neutral_tile() {
    let Some(mut r) = renderer_or_skip() else { return };
    r.set_camera_lookat(Vec3::new(1.0, 2.0, 0.0), Vec3::ZERO);

    r.set_debug_channel(DebugChannel::Albedo);
    let target_off = r.target(W, H);
    r.render_mesh(&target_off, quad_with_material(40.0, stone_quad_material(false)), wgpu::Color::BLACK);
    let mean_off = channel_mean_linear(&r.read(&target_off), 0);

    r.set_detail_material(detail_dc_neutral_tile());
    let target_on = r.target(W, H);
    r.render_mesh(&target_on, quad_with_material(40.0, stone_quad_material(true)), wgpu::Color::BLACK);
    let mean_on = channel_mean_linear(&r.read(&target_on), 0);

    let rel_diff = (mean_on - mean_off).abs() / mean_off;
    assert!(
        rel_diff < 0.05,
        "DC-neutral detail tile must not change mean brightness: off={mean_off:.2} on={mean_on:.2} rel_diff={rel_diff:.4}"
    );

    r.set_debug_channel(DebugChannel::Roughness);
    let target_rough_off = r.target(W, H);
    r.render_mesh(&target_rough_off, quad_with_material(40.0, stone_quad_material(false)), wgpu::Color::BLACK);
    let rough_off = channel_mean(&r.read(&target_rough_off), 0);

    let target_rough_on = r.target(W, H);
    r.render_mesh(&target_rough_on, quad_with_material(40.0, stone_quad_material(true)), wgpu::Color::BLACK);
    let rough_on = channel_mean(&r.read(&target_rough_on), 0);

    let rough_diff = (rough_on - rough_off).abs();
    assert!(
        rough_diff <= 1.0,
        "DC-neutral detail tile must not change roughness: off={rough_off:.2} on={rough_on:.2} diff={rough_diff:.4}"
    );
}
