// The procedural zone ground renders through the real mesh pipeline with
// visible texture variation — not a flat slab. Skips without a GPU adapter
// or the ground texture set.

use engine_renderer::mesh::TextureSource;
use engine_renderer::offscreen::OffscreenRenderer;
use vordar_client::ground::{generate_ground, load_ground_material};

#[test]
fn zone_ground_renders_with_texture_variation() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/textures/ground/cracked_earth");
    if !std::path::Path::new(dir).exists() {
        eprintln!("SKIP: ground texture set missing");
        return;
    }

    let material = load_ground_material(dir).expect("texture set loads");
    // cracked_earth ships a .dds sidecar for every map slot: the finder must
    // prefer it over the PNG source. Needs no GPU, so it's checked whether
    // or not the render below runs.
    let base_color_compressed = matches!(material.base_color_image, Some(TextureSource::Compressed(_)));
    let normal_compressed = matches!(material.normal_image, Some(TextureSource::Compressed(_)));
    let mr_compressed = matches!(material.metallic_roughness_image, Some(TextureSource::Compressed(_)));

    let Some(mut r) = OffscreenRenderer::new(1.0) else {
        eprintln!("SKIP: no GPU adapter");
        return;
    };
    if !r.gpu.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
        eprintln!("SKIP: adapter lacks TEXTURE_COMPRESSION_BC");
        return;
    }

    let data = generate_ground(400.0, 7.0, material);
    let target = r.target(256, 256);
    r.render_mesh(&target, data, wgpu::Color::BLACK);
    let pixels = r.read(&target);

    // The camera looks down at textured ground: nearly the whole frame is
    // covered, and luminance varies well beyond a flat-color slab.
    let lums: Vec<f64> = pixels
        .chunks_exact(4)
        .map(|p| 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64)
        .collect();
    let covered = lums.iter().filter(|&&l| l > 4.0).count();
    assert!(covered > lums.len() * 8 / 10, "ground fills the frame: {covered}");

    let mean = lums.iter().sum::<f64>() / lums.len() as f64;
    let variance = lums.iter().map(|l| (l - mean) * (l - mean)).sum::<f64>() / lums.len() as f64;
    assert!(
        variance.sqrt() > 4.0,
        "textured ground must vary (VQ-A2): stddev {:.2}, mean {mean:.2}",
        variance.sqrt()
    );

    assert!(base_color_compressed, "diff_2048.dds exists in cracked_earth — base color should load Compressed");
    assert!(normal_compressed, "nor_gl_2048.dds exists in cracked_earth — normal should load Compressed");
    assert!(mr_compressed, "rough_2048_mr.dds exists in cracked_earth — metallic-roughness should load Compressed");
}
