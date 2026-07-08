// Offscreen render harness tests — VQ-G1.
//
// Analytic assertions only (coverage %, brighter-than, uniformity) — never
// exact pixel values, so driver/adapter variance can't flake them. Every test
// skips cleanly when the machine has no GPU adapter.

use engine_renderer::instance::SdfInstance;
use engine_renderer::offscreen::{read_rgba8, render_sdf_scene, HeadlessGpu, SceneTarget};
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
