// VQ-A5: the shipped "start" zone key must read near-neutral grey overcast
// (sky/fog R/B close to 1, R never running warm of B), not the retired amber
// dusk look. Skips without a GPU adapter.

use engine_renderer::offscreen::OffscreenRenderer;
use std::path::{Path, PathBuf};
use vordar_game::zones::load_zones;

const W: u32 = 256;
const H: u32 = 256;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Undoes the sRGB OETF the tonemap pass bakes in (matches the swapchain's
/// hardware encode) — same decode chapel_probe.rs/offscreen.rs use before
/// reasoning about physical light.
fn linear_byte(byte: u8) -> f64 {
    let c = byte as f64 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn channel_mean_linear(pixels: &[u8], channel: usize) -> f64 {
    let sum: f64 = pixels.iter().skip(channel).step_by(4).map(|&v| linear_byte(v)).sum();
    sum / (pixels.len() / 4) as f64
}

/// VQ-A5: the shipped overcast look reads near-neutral (R/B close to 1, R
/// never running warm of B) — a regression back to the retired amber dusk
/// key fails this loudly. Bands: measured R/B 0.859 ± ~20%; the retired
/// dusk constants measure R/B 1.938, an order outside either bound.
#[test]
fn start_zone_sky_is_near_neutral_not_amber() {
    let Some(mut r) = OffscreenRenderer::new(W as f32 / H as f32) else {
        eprintln!("SKIP: no GPU adapter available");
        return;
    };

    let root = repo_root();
    let def = load_zones(root.join("content/zones/zones.ron").to_str().unwrap());
    let visuals = &def.zones.iter().find(|z| z.name == "start").expect("start zone exists").visuals;
    let hdri = visuals.env.as_deref().expect("start zone authors env");
    let hdri = root.join(hdri);
    let hdri = hdri.to_str().unwrap();
    r.load_environment_hdr(hdri).unwrap_or_else(|e| panic!("HDRI {hdri}: {e}"));
    r.draw_sky = true;
    r.set_camera_level();
    r.set_fog(visuals.fog_color, visuals.fog_density);
    r.set_fog_height(visuals.fog_height, visuals.fog_height_falloff);
    r.set_exposure(visuals.exposure);

    let target = r.target(W, H);
    r.render_sdf(&target, &[], wgpu::Color::BLACK);
    let pixels = r.read(&target);

    let (rm, gm, bm) = (
        channel_mean_linear(&pixels, 0),
        channel_mean_linear(&pixels, 1),
        channel_mean_linear(&pixels, 2),
    );
    let ratio = rm / bm;
    println!("start zone sky+fog linear means: R={rm:.4} G={gm:.4} B={bm:.4} R/B={ratio:.4}");
    assert!(
        (0.65..=1.05).contains(&ratio),
        "R/B {ratio:.4} (R={rm:.4} B={bm:.4}) drifted off VQ-A5's near-neutral overcast band"
    );
    assert!(rm - bm < 0.05, "R-B {:.4} too warm for VQ-A5's overcast look", rm - bm);
}
