// Headless interior evidence for the chapel's broken-vault-vs-enclosed-volume
// design question: orbit-camera containment (analytic ray-vs-box sweep, plus
// rendered frames), interior IBL brightness (covered/open/exterior), roof
// shadow casting, and a candle point-light read, against chapter03's graybox
// chapel. Not a ship-gate tool (zone_review is); throwaway probe logic (the
// roof ablation filter, the containment sweep) lives here, while the reusable
// bit (loading chapter geometry as mesh primitives) lives in
// `vordar_client::chapter_geometry`, shared with zone_review.

use engine_renderer::mesh::{MeshData, PrimitiveData};
use engine_renderer::offscreen::{OffscreenRenderer, TestLight, TestPointLight};
use glam::Vec3;
use image::RgbaImage;
use vordar_client::chapter_geometry::load_chapter_prims;
use vordar_client::ground::{generate_ground, load_ground_material};
use vordar_game::zones::{resolve_sun_color, resolve_sun_dir, ZoneVisuals};

const OUT: &str = "target/town-probe";
const HDRI: &str = "content/textures/env/castilian_plateau_dusk_2k.hdr";
const GROUND_DIR: &str = "content/textures/ground/cracked_earth";
const FOG_COLOR: Vec3 = Vec3::new(2.0, 0.64, 0.25);
const FOG_DENSITY: f32 = 0.0055;
const GROUND_SIZE: f32 = 400.0;
const GROUND_TILE: f32 = 7.0;

const NAVE_CENTER: Vec3 = Vec3::new(-22.0, 0.0, -13.0);
const NEAR_DOOR: Vec3 = Vec3::new(-15.0, 0.0, -13.0);
const MIN_RADIUS: f32 = 4.0;
const MAX_RADIUS: f32 = 100.0;
const DEFAULT_RADIUS: f32 = 34.0;
const DEFAULT_PITCH: f32 = 0.8;
const EYE_HEIGHT: f32 = 1.6;

// ── Orbit geometry (mirrors Camera::recompute_eye) ───────────────────────────

/// Unit direction from `target` to the orbit eye at (`angle`, `pitch`) —
/// `Camera::recompute_eye`'s spherical parametrization, magnitude 1 (so a
/// ray-vs-box hit distance along it is the radius directly). Ignores the
/// `MIN_EYE_Y` ground clamp: that only ever raises the eye, so skipping it
/// is the conservative direction for a containment check.
fn orbit_dir(angle: f32, pitch: f32) -> Vec3 {
    Vec3::new(angle.cos() * pitch.cos(), pitch.sin(), angle.sin() * pitch.cos())
}

fn orbit_eye(target: Vec3, radius: f32, angle: f32, pitch: f32) -> Vec3 {
    target + radius * orbit_dir(angle, pitch)
}

/// Slab-method ray-vs-AABB: entry distance along the unit `dir` from
/// `origin`, or `None` if the ray misses the box (or the box is entirely
/// behind the origin).
fn ray_aabb(origin: Vec3, dir: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let (mut t_min, mut t_max) = (0.0f32, f32::INFINITY);
    for axis in 0..3 {
        let (o, d, mn, mx) = (origin[axis], dir[axis], min[axis], max[axis]);
        if d.abs() < 1e-8 {
            if o < mn || o > mx {
                return None;
            }
        } else {
            let (mut t1, mut t2) = ((mn - o) / d, (mx - o) / d);
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            t_min = t_min.max(t1);
            t_max = t_max.min(t2);
            if t_min > t_max {
                return None;
            }
        }
    }
    Some(t_min)
}

/// World-space AABB of one primitive's vertices (chapter boxes are
/// axis-aligned by construction — no rotation is authored on any chapter03
/// prefab — so this recovers each box's exact bounds).
fn prim_aabb(prim: &PrimitiveData) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for v in &prim.vertices {
        let p = Vec3::from_array(v.position);
        min = min.min(p);
        max = max.max(p);
    }
    (min, max)
}

/// Largest radius at (`angle`, `pitch`) before the orbit ray from `target`
/// enters any chapel box — the containment ceiling a player could reach
/// before the camera starts clipping through geometry in that direction.
fn max_safe_radius(target: Vec3, angle: f32, pitch: f32, boxes: &[(Vec3, Vec3)]) -> f32 {
    let dir = orbit_dir(angle, pitch);
    boxes.iter()
        .filter_map(|&(mn, mx)| ray_aabb(target, dir, mn, mx))
        .fold(MAX_RADIUS, f32::min)
}

/// Sweeps yaw at `pitch`, returns (worst-case safe radius, the angle that
/// produced it) in degrees for the printout.
fn worst_case(target: Vec3, pitch: f32, boxes: &[(Vec3, Vec3)]) -> (f32, f32) {
    let mut best = (MAX_RADIUS, 0.0f32);
    let mut deg = 0.0f32;
    while deg < 360.0 {
        let angle = deg.to_radians();
        let r = max_safe_radius(target, angle, pitch, boxes);
        if r < best.0 {
            best = (r, deg);
        }
        deg += 1.0;
    }
    best
}

fn print_containment(boxes: &[(Vec3, Vec3)]) {
    println!("=== P-C.1 orbit-camera containment ===");
    println!("zoom range: [{MIN_RADIUS}, {MAX_RADIUS}] m; default radius {DEFAULT_RADIUS} m, default pitch {DEFAULT_PITCH} rad");
    for (label, target) in [("nave center (-22,-13)", NAVE_CENTER), ("near door (-15,-13)", NEAR_DOOR)] {
        println!("-- {label} --");
        for pitch in [-1.4f32, 0.0, DEFAULT_PITCH, 1.4] {
            let (safe_r, angle_deg) = worst_case(target, pitch, boxes);
            println!(
                "  pitch {pitch:+.2} rad: worst-case safe radius = {safe_r:.2} m (at yaw {angle_deg:.0} deg) — \
                 min-zoom({MIN_RADIUS}) {} — default-zoom({DEFAULT_RADIUS}) {}",
                if MIN_RADIUS <= safe_r { "CONTAINED" } else { "CLIPS" },
                if DEFAULT_RADIUS <= safe_r { "CONTAINED" } else { "CLIPS" },
            );
        }
    }
}

// ── Rendering ────────────────────────────────────────────────────────────────

fn ground_prims() -> Vec<PrimitiveData> {
    let material = load_ground_material(GROUND_DIR).unwrap_or_else(|e| panic!("ground: {e}"));
    generate_ground(GROUND_SIZE, GROUND_TILE, material).primitives
}

fn render_frame(r: &mut OffscreenRenderer, prims: Vec<PrimitiveData>, eye: Vec3, target: Vec3, w: u32, h: u32) -> (RgbaImage, Vec<u8>) {
    r.set_camera_lookat(eye, target);
    let rt = r.target(w, h);
    r.render_mesh(&rt, MeshData { primitives: prims, skeleton: None, clips: Vec::new() }, wgpu::Color::BLACK);
    let pixels = r.read(&rt);
    let img = RgbaImage::from_raw(w, h, pixels.clone()).expect("readback size matches WxH");
    (img, pixels)
}

fn save(img: &RgbaImage, name: &str) {
    let path = format!("{OUT}/{name}.png");
    img.save(&path).unwrap_or_else(|e| panic!("save {path}: {e}"));
    println!("  wrote {path}");
}

/// Undoes the sRGB OETF the tonemap pass bakes in (matches the swapchain's
/// hardware encode) — same decode the offscreen test suite uses before
/// reasoning about physical light.
fn linear_byte(byte: u8) -> f64 {
    let c = byte as f64 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn channel_mean_linear(pixels: &[u8], channel: usize) -> f64 {
    let sum: f64 = pixels.iter().skip(channel).step_by(4).map(|&v| linear_byte(v)).sum();
    sum / (pixels.len() / 4) as f64
}

fn report_means(label: &str, pixels: &[u8]) -> (f64, f64, f64) {
    let (r, g, b) = (channel_mean_linear(pixels, 0), channel_mean_linear(pixels, 1), channel_mean_linear(pixels, 2));
    println!("  {label}: linear mean R={r:.4} G={g:.4} B={b:.4}");
    (r, g, b)
}

fn setup(r: &mut OffscreenRenderer) {
    r.load_environment_hdr(HDRI).unwrap_or_else(|e| panic!("HDRI: {e}"));
    r.draw_sky = true;
    r.set_fog(FOG_COLOR, FOG_DENSITY);
    let visuals = ZoneVisuals::default();
    r.set_light(TestLight { direction: resolve_sun_dir(&visuals), color: resolve_sun_color(&visuals), ambient: visuals.ambient });
    // RendererState defaults SSAO on (state.rs) — match it so the IBL/shadow
    // reads reflect live gameplay, not just this harness's own default-off.
    r.set_ssao(true);
}

fn main() {
    std::fs::create_dir_all(OUT).unwrap_or_else(|e| panic!("mkdir {OUT}: {e}"));
    let (w, h) = (1280u32, 720u32);

    let chapter_prims = load_chapter_prims("chapter03");
    let boxes: Vec<(Vec3, Vec3)> = chapter_prims.iter().map(prim_aabb).collect();

    // ---- P-C.1: containment (analytic, no GPU) ----
    print_containment(&boxes);

    let Some(mut r) = OffscreenRenderer::new(w as f32 / h as f32) else {
        eprintln!("chapel_probe: no GPU adapter available — skipping renders");
        return;
    };
    setup(&mut r);

    // Rendered evidence for the containment numbers: default zoom/pitch
    // toward the nave's short (worst-case) axis, the computed safe radius at
    // the same pitch, and the worst near-door angle at minimum zoom.
    println!("=== P-C.1 rendered frames ===");
    let (safe_r, _) = worst_case(NAVE_CENTER, DEFAULT_PITCH, &boxes);
    let (_, door_worst_deg) = worst_case(NEAR_DOOR, DEFAULT_PITCH, &boxes);
    let shots = [
        ("containment_default_radius34_pitch08", NAVE_CENTER, DEFAULT_RADIUS, 90.0f32, DEFAULT_PITCH),
        ("containment_safe_radius_pitch08", NAVE_CENTER, (safe_r - 0.2).max(0.5), 90.0, DEFAULT_PITCH),
        ("containment_near_door_minzoom", NEAR_DOOR, MIN_RADIUS, door_worst_deg, DEFAULT_PITCH),
    ];
    for (name, target, radius, angle_deg, pitch) in shots {
        let eye = orbit_eye(target, radius, angle_deg.to_radians(), pitch).max(Vec3::new(f32::NEG_INFINITY, 0.0, f32::NEG_INFINITY));
        let mut prims = ground_prims();
        prims.extend(load_chapter_prims("chapter03"));
        let (img, _) = render_frame(&mut r, prims, eye, target, w, h);
        println!("  {name}: eye={eye:?} target={target:?} radius={radius:.2} angle={angle_deg:.0}deg pitch={pitch:.2}");
        save(&img, name);
    }

    // ---- P-C.2/P-C.3: IBL brightness + roof shadow ----
    // Steep look-up framing isolates "how much sky is visible overhead" —
    // the mechanism both the IBL-leak and shadow questions turn on — at
    // three positions: under the west roof, under the open east vault, and
    // outside the chapel entirely (plaza, no building overhead).
    println!("=== P-C.2 interior IBL brightness (covered / open / exterior) ===");
    // Near-vertical (tiny x so the camera basis stays non-degenerate): from
    // the room's z-centerline this frames mostly ceiling/sky, side walls
    // only at the frame edges — an earlier, shallower tilt (1.5,6,0) let the
    // nearby whitewashed walls dominate the frame and swamp the roof/sky
    // signal this probe needs.
    let up = Vec3::new(0.05, 10.0, 0.0);
    let conditions = [
        ("covered", Vec3::new(-26.0, EYE_HEIGHT, -13.0)),
        ("open",    Vec3::new(-18.0, EYE_HEIGHT, -13.0)),
        ("exterior", Vec3::new(0.0, EYE_HEIGHT, 30.0)),
    ];
    for (name, eye) in conditions {
        let mut prims = ground_prims();
        prims.extend(load_chapter_prims("chapter03"));
        let (img, pixels) = render_frame(&mut r, prims, eye, eye + up, w, h);
        report_means(name, &pixels);
        save(&img, &format!("ibl_{name}_lookup"));
    }

    println!("=== P-C.3 roof shadow (with vs without the roof slab) ===");
    let shadow_eye = Vec3::new(-26.0, 5.0, -13.0);
    let shadow_target = Vec3::new(-26.0, -0.5, -13.0); // straight down at the floor under the roof
    {
        let mut prims = ground_prims();
        prims.extend(load_chapter_prims("chapter03"));
        let (img, pixels) = render_frame(&mut r, prims, shadow_eye, shadow_target, w, h);
        report_means("with_roof", &pixels);
        save(&img, "shadow_with_roof");
    }
    {
        let mut prims = ground_prims();
        // Roof cubes are the only chapter03 geometry above y=10.9 (chapel_roof
        // spans y 11.0..11.6) — ablate just that piece to isolate its effect.
        prims.extend(load_chapter_prims("chapter03").into_iter().filter(|p| {
            p.vertices.iter().all(|v| v.position[1] < 10.9)
        }));
        let (img, pixels) = render_frame(&mut r, prims, shadow_eye, shadow_target, w, h);
        report_means("without_roof", &pixels);
        save(&img, "shadow_without_roof");
    }

    // ---- P-C.4: candle PointLight read ----
    // Same color/intensity/radius as content/prefabs/portal.ron's authored
    // candle-gold PointLight block, placed at interior candle height.
    println!("=== P-C.4 candle PointLight read ===");
    let candle_eye = Vec3::new(-26.0, EYE_HEIGHT, -13.0);
    let candle_target = Vec3::new(-22.0, 1.3, -13.0);
    r.set_point_lights(&[]);
    {
        let mut prims = ground_prims();
        prims.extend(load_chapter_prims("chapter03"));
        let (img, pixels) = render_frame(&mut r, prims, candle_eye, candle_target, w, h);
        report_means("candle_off", &pixels);
        save(&img, "candle_off");
    }
    r.set_point_lights(&[TestPointLight { position: candle_target, color: Vec3::new(1.05, 0.825, 0.3), intensity: 8.0, radius: 9.0 }]);
    {
        let mut prims = ground_prims();
        prims.extend(load_chapter_prims("chapter03"));
        let (img, pixels) = render_frame(&mut r, prims, candle_eye, candle_target, w, h);
        report_means("candle_on", &pixels);
        save(&img, "candle_on");
    }

    println!("chapel_probe: done, frames in {OUT}");
}
