// Offscreen render harness tests — VQ-G1.
//
// Analytic assertions only (coverage %, brighter-than, uniformity) — never
// exact pixel values, so driver/adapter variance can't flake them. Every test
// skips cleanly when the machine has no GPU adapter.

use engine_renderer::instance::SdfInstance;
use engine_renderer::mesh::{load_gltf_data, MaterialData, MeshData, PrimitiveData};
use engine_renderer::offscreen::{
    create_mipped_rgba8, read_rgba8, read_texture_mip, render_mesh_scene, render_sdf_scene,
    HeadlessGpu, SceneTarget,
};
use engine_renderer::MeshVertex;
use glam::{Mat4, Vec3};

const W: u32 = 256;
const H: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn gpu_or_skip() -> Option<HeadlessGpu> {
    let gpu = HeadlessGpu::new();
    if gpu.is_none() {
        eprintln!("SKIP: no GPU adapter available — offscreen tests need one");
    }
    gpu
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

/// Mean of one channel (0=r,1=g,2=b,3=a) over the whole image.
fn channel_mean(pixels: &[u8], channel: usize) -> f64 {
    let sum: u64 = pixels.iter().skip(channel).step_by(4).map(|&v| v as u64).sum();
    sum as f64 / (pixels.len() / 4) as f64
}

#[test]
fn clear_only_render_is_uniform() {
    let Some(gpu) = gpu_or_skip() else { return };
    let target = SceneTarget::new(&gpu.device, W, H, FORMAT);
    render_sdf_scene(&gpu, &target, &[], wgpu::Color { r: 0.2, g: 0.4, b: 0.6, a: 1.0 });
    let pixels = read_rgba8(&gpu, &target);

    assert_eq!(pixels.len(), (W * H * 4) as usize);
    let first: [u8; 4] = pixels[0..4].try_into().unwrap();
    assert!(
        pixels.chunks_exact(4).all(|p| p == first),
        "clear-only frame must be uniform, first pixel {first:?}"
    );
    // Channel ordering sanity: b > g > r for this clear color.
    assert!(first[2] > first[1] && first[1] > first[0], "got {first:?}");
}

#[test]
fn cube_renders_with_coverage_and_color() {
    let Some(gpu) = gpu_or_skip() else { return };
    let target = SceneTarget::new(&gpu.device, W, H, FORMAT);

    // Big red cube at the camera target (origin). Default orbit camera looks
    // at the origin, so the cube lands mid-frame.
    let instances = [cube_at(Vec3::ZERO, 10.0, [1.0, 0.0, 0.0])];
    render_sdf_scene(&gpu, &target, &instances, wgpu::Color::BLACK);
    let pixels = read_rgba8(&gpu, &target);

    // Coverage: a 10-unit cube seen from the default radius-34 orbit covers a
    // solid chunk of a 256² frame but never all of it.
    let covered = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count();
    let total = (W * H) as usize;
    assert!(
        covered > total / 50 && covered < total * 9 / 10,
        "cube coverage {covered}/{total} outside sane bounds"
    );

    // Color: red instance on black clear ⇒ red mean dominates.
    let (r, g, b) = (channel_mean(&pixels, 0), channel_mean(&pixels, 1), channel_mean(&pixels, 2));
    assert!(r > g * 2.0 && r > b * 2.0, "red cube must dominate: r={r:.1} g={g:.1} b={b:.1}");
}

// ── Phase 1: PBR / mipmaps ───────────────────────────────────────────────────

/// A ground quad at y=0 spanning ±extent, normal +Y, with a uniform material.
/// No images, so tangents come from the generator and every map binds its
/// 1×1 neutral default — the factors drive the BRDF.
fn ground_quad(extent: f32, roughness: f32, metallic: f32) -> MeshData {
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
            material: MaterialData {
                base_color_factor: [0.5, 0.5, 0.5, 1.0],
                roughness_factor:  roughness,
                metallic_factor:   metallic,
                ..Default::default()
            },
            skin: None,
        }],
        skeleton: None,
        clips:    Vec::new(),
    }
}

fn luminance(p: &[u8]) -> f64 {
    0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64
}

/// VQ-G1 for the GGX BRDF: on the same quad under the same sun, a smooth
/// surface concentrates specular energy (brighter peak, smaller hotspot) than
/// a fully rough one — monotonic, no exact pixels.
#[test]
fn smooth_surface_has_tighter_brighter_specular_peak_than_rough() {
    let Some(gpu) = gpu_or_skip() else { return };

    let render = |roughness: f32| -> Vec<u8> {
        let target = SceneTarget::new(&gpu.device, W, H, FORMAT);
        render_mesh_scene(&gpu, &target, ground_quad(40.0, roughness, 0.0), wgpu::Color::BLACK);
        read_rgba8(&gpu, &target)
    };
    let smooth = render(0.05);
    let rough  = render(1.0);

    let peak = |img: &[u8]| {
        img.chunks_exact(4)
            .map(|p| luminance(p))
            .fold(0.0f64, f64::max)
    };
    let (peak_smooth, peak_rough) = (peak(&smooth), peak(&rough));
    assert!(
        peak_smooth > peak_rough * 1.15,
        "smooth peak must outshine rough: smooth={peak_smooth:.1} rough={peak_rough:.1}"
    );

    // Hotspot size: pixels within 90% of each image's own peak. The smooth
    // lobe is concentrated; the rough lobe spreads.
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

/// Metal reflects tinted specular and has no diffuse: a fully metallic quad
/// shades differently from a dielectric one everywhere it is lit.
#[test]
fn metallic_changes_response_vs_dielectric() {
    let Some(gpu) = gpu_or_skip() else { return };
    let render = |metallic: f32| {
        let target = SceneTarget::new(&gpu.device, W, H, FORMAT);
        render_mesh_scene(&gpu, &target, ground_quad(40.0, 0.4, metallic), wgpu::Color::BLACK);
        read_rgba8(&gpu, &target)
    };
    let dielectric = render(0.0);
    let metal      = render(1.0);

    // Same geometry in both frames, so compare within the quad's footprint
    // (taken from the dielectric render — diffuse lights all of it). Off the
    // specular lobe a metal has no diffuse term, so its dark percentile falls
    // well below the dielectric's uniform diffuse floor.
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
        metal_p10 < dielectric_p10 * 0.6,
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
    let Some(gpu) = gpu_or_skip() else { return };

    let mut data = load_gltf_data(path).expect("helmet parses");
    let m = &data.primitives[0].material;
    assert!(m.base_color_image.is_some(), "helmet has albedo");
    assert!(m.normal_image.is_some(), "helmet has a normal map");
    assert!(m.metallic_roughness_image.is_some(), "helmet has MR");
    assert!(m.emissive_image.is_some(), "helmet has emissive");

    // The helmet is ~1 unit tall at the origin; scale it up to read from the
    // default radius-34 orbit camera.
    for prim in &mut data.primitives {
        for v in &mut prim.vertices {
            for c in &mut v.position {
                *c *= 12.0;
            }
        }
    }
    let target = SceneTarget::new(&gpu.device, W, H, FORMAT);
    render_mesh_scene(&gpu, &target, data, wgpu::Color::BLACK);
    let pixels = read_rgba8(&gpu, &target);

    let covered = pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count();
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
    let Some(gpu) = gpu_or_skip() else { return };

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

#[test]
fn nearer_cube_occludes_farther_cube() {
    let Some(gpu) = gpu_or_skip() else { return };

    // Scene A: green cube alone at origin.
    let green = cube_at(Vec3::ZERO, 8.0, [0.0, 1.0, 0.0]);
    let target_a = SceneTarget::new(&gpu.device, W, H, FORMAT);
    render_sdf_scene(&gpu, &target_a, &[green], wgpu::Color::BLACK);
    let green_alone = channel_mean(&read_rgba8(&gpu, &target_a), 1);

    // Scene B: same green cube, plus a red cube between it and the camera.
    // Default camera sits at positive X/Z (yaw π/4, radius 34, pitch 0.8),
    // so "toward the camera" is toward +X+Z and up.
    let red_front = cube_at(Vec3::new(8.0, 8.0, 8.0), 10.0, [1.0, 0.0, 0.0]);
    let target_b = SceneTarget::new(&gpu.device, W, H, FORMAT);
    render_sdf_scene(&gpu, &target_b, &[green, red_front], wgpu::Color::BLACK);
    let green_occluded = channel_mean(&read_rgba8(&gpu, &target_b), 1);

    // Depth test must remove green energy — monotonic, not exact.
    assert!(
        green_occluded < green_alone * 0.8,
        "occluder must reduce green mean: alone={green_alone:.2} occluded={green_occluded:.2}"
    );
}
