// asset_inspect: render one prop across a lighting x debug-channel x distance
// matrix for vision review — the eyes-fix companion to zone_review.rs's
// in-scene shots, built because one beauty pass under one light can't
// separate "bad material" from "bad lighting", and a downscaled contact
// sheet can't resolve texel-level defects (see
// tasks/lessons/2026-07-23-review-in-engine-at-gameplay-framing.md). Every
// (lighting, channel) pair gets its own directory of full-resolution PNGs —
// those are the evidence — plus one small index sheet; the sheet alone is
// never sufficient to judge an asset. Mirrors turntable.rs/zone_review.rs's
// offscreen-harness shape one crate over; lives in the client (not the
// engine) because `dusk` needs presentation::{SUN_DIR, SUN_COLOR} and
// content/zones/zones.ron.

use engine_renderer::anim::LocalTransform;
use engine_renderer::mesh::{load_gltf_data, MaterialData, MeshData, PrimitiveData};
use engine_renderer::offscreen::{DebugChannel, OffscreenRenderer, TestLight};
use engine_renderer::review;
use glam::Vec3;
use image::RgbaImage;
use std::path::{Path, PathBuf};
use std::process::exit;
use vordar_client::presentation::{SUN_COLOR, SUN_DIR};
use vordar_game::zones::load_zones;

const ZONES_PATH: &str = "content/zones/zones.ron";
const HDRI_DUSK: &str = "content/textures/env/castilian_plateau_dusk_2k.hdr";

/// Single-prop framing extent — matches turntable.rs/gear_render.rs
/// (zone_review.rs's wider whole-zone fallback doesn't apply here).
const GROUND_EXTENT: f32 = 40.0;

/// Eye height for `gameplay`/`macro` framing — average human eye height
/// above the ground, matching zone_review.rs's close-up shots.
const EYE_HEIGHT: f32 = 1.6;
/// `gameplay` camera-to-near-surface distance — the live game's actual
/// walk-up inspection range (close enough to read texel density), matching
/// zone_review.rs's CLOSE_DISTANCE.
const CLOSE_DISTANCE: f32 = 2.3;
/// `macro` camera-to-near-surface distance — close enough to read
/// individual texels rather than overall material response.
const MACRO_DISTANCE: f32 = 0.6;

const REFERENCE_MODEL: &str = "content/models/props/rock_face_01/rock_face_01_1k.gltf";
/// The scale zones.ron:63 actually places rock_face_01 at — a material
/// comparison at a fixed metric distance is only honest at shipped scale.
const REFERENCE_SCALE: f32 = 4.0;

/// Sheet tiles are capped at this size — a 1024² frame downscaled straight
/// to a contact-sheet cell reintroduces the blindness this tool exists to
/// fix; the full-res PNG beside it is the actual evidence.
const SHEET_TILE: u32 = 512;
/// Sheet long edge is capped near this size — a contact sheet gets
/// downscaled to ~1568 px on its long edge before a vision model sees it, so
/// this keeps the sheet from arriving pre-shrunk past that ceiling too.
const MAX_SHEET_EDGE: u32 = 1536;

/// Matches `TonemapPass::new`'s shipped default (post.rs) — restored
/// explicitly per lighting group since `furnace` zeroes it and presets can
/// run in any order within one invocation.
const DEFAULT_BLOOM: f32 = 0.12;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lighting { Studio, Furnace, Raking, Dusk }

impl Lighting {
    fn name(self) -> &'static str {
        match self {
            Lighting::Studio  => "studio",
            Lighting::Furnace => "furnace",
            Lighting::Raking  => "raking",
            Lighting::Dusk    => "dusk",
        }
    }

    /// Whether this preset's `beauty`/`clay` render shows the sky as
    /// background. Every other channel forces it off regardless (see main).
    fn draw_sky(self) -> bool {
        matches!(self, Lighting::Furnace | Lighting::Dusk)
    }

    fn clear_color(self) -> wgpu::Color {
        match self {
            Lighting::Studio => wgpu::Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 },
            _ => wgpu::Color::BLACK,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Channel { Beauty, Albedo, Rough, Metal, Normal, Ao, Clay }

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Channel::Beauty => "beauty",
            Channel::Albedo => "albedo",
            Channel::Rough  => "rough",
            Channel::Metal  => "metal",
            Channel::Normal => "normal",
            Channel::Ao     => "ao",
            Channel::Clay   => "clay",
        }
    }

    /// `Clay` still renders the shipped Beauty path — only its material
    /// inputs are substituted on the CPU (see `clay_override`).
    fn debug_channel(self) -> DebugChannel {
        match self {
            Channel::Beauty | Channel::Clay => DebugChannel::Beauty,
            Channel::Albedo => DebugChannel::Albedo,
            Channel::Rough  => DebugChannel::Roughness,
            Channel::Metal  => DebugChannel::Metallic,
            Channel::Normal => DebugChannel::Normal,
            Channel::Ao     => DebugChannel::Occlusion,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Distance { Full, Gameplay, Macro }

impl Distance {
    fn name(self) -> &'static str {
        match self {
            Distance::Full     => "full",
            Distance::Gameplay => "gameplay",
            Distance::Macro    => "macro",
        }
    }
}

struct Args {
    model:     String,
    out:       String,
    size:      (u32, u32),
    scale:     f32,
    lighting:  Vec<Lighting>,
    channel:   Vec<Channel>,
    angles:    u32,
    distance:  Vec<Distance>,
    reference: bool,
}

fn usage(msg: &str) -> ! {
    eprintln!("asset_inspect: {msg}");
    eprintln!(
        "usage: asset_inspect <glb|gltf> [--out DIR] [--size WxH] [--scale S] \
         [--lighting studio,furnace,raking,dusk] \
         [--channel beauty,albedo,rough,metal,normal,ao,clay] \
         [--angles N] [--distance full,gameplay,macro] [--reference]"
    );
    exit(2);
}

fn die(e: String) -> ! {
    eprintln!("asset_inspect: {e}");
    exit(1);
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn parse_list<T>(s: &str, one: impl Fn(&str) -> Option<T>) -> Option<Vec<T>> {
    let v: Vec<T> = s.split(',').map(&one).collect::<Option<Vec<T>>>()?;
    (!v.is_empty()).then_some(v)
}

fn parse_lighting(s: &str) -> Option<Lighting> {
    Some(match s {
        "studio"  => Lighting::Studio,
        "furnace" => Lighting::Furnace,
        "raking"  => Lighting::Raking,
        "dusk"    => Lighting::Dusk,
        _ => return None,
    })
}

fn parse_channel(s: &str) -> Option<Channel> {
    Some(match s {
        "beauty" => Channel::Beauty,
        "albedo" => Channel::Albedo,
        "rough"  => Channel::Rough,
        "metal"  => Channel::Metal,
        "normal" => Channel::Normal,
        "ao"     => Channel::Ao,
        "clay"   => Channel::Clay,
        _ => return None,
    })
}

fn parse_distance(s: &str) -> Option<Distance> {
    Some(match s {
        "full"     => Distance::Full,
        "gameplay" => Distance::Gameplay,
        "macro"    => Distance::Macro,
        _ => return None,
    })
}

fn parse_args() -> Args {
    let (mut model, mut out) = (None, None);
    let mut size = (1024u32, 1024u32);
    let mut scale = 1.0f32;
    let mut lighting = vec![Lighting::Studio];
    let mut channel = vec![Channel::Beauty];
    let mut angles = 4u32;
    let mut distance = vec![Distance::Full, Distance::Gameplay, Distance::Macro];
    let mut reference = false;

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out"       => out = it.next(),
            "--size"      => size = it.next().as_deref().and_then(parse_size).unwrap_or_else(|| usage("--size needs WxH")),
            "--scale"     => scale = it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage("--scale needs a float")),
            "--lighting"  => lighting = it.next().as_deref().and_then(|s| parse_list(s, parse_lighting)).unwrap_or_else(|| usage("--lighting needs studio,furnace,raking,dusk")),
            "--channel"   => channel = it.next().as_deref().and_then(|s| parse_list(s, parse_channel)).unwrap_or_else(|| usage("--channel needs beauty,albedo,rough,metal,normal,ao,clay")),
            "--angles"    => angles = it.next().and_then(|s| s.parse().ok()).filter(|&n| n > 0).unwrap_or_else(|| usage("--angles needs a positive integer")),
            "--distance"  => distance = it.next().as_deref().and_then(|s| parse_list(s, parse_distance)).unwrap_or_else(|| usage("--distance needs full,gameplay,macro")),
            "--reference" => reference = true,
            _ if a.starts_with("--") => usage(&format!("unknown flag {a}")),
            _ => model = Some(a),
        }
    }
    let model = model.unwrap_or_else(|| usage("required: <glb|gltf>"));
    let out = out.unwrap_or_else(|| "target/asset-inspect".to_string());
    Args { model, out, size, scale, lighting, channel, angles, distance, reference }
}

/// Load a glTF, collapse any skeleton to bind pose (turntable.rs's tolerant
/// pattern — this bin only ever wants one still frame), and scale every
/// vertex position uniformly. Reloaded per frame: `render_mesh` consumes
/// `MeshData`.
fn load_scaled(path: &str, scale: f32) -> Vec<PrimitiveData> {
    let mut data = load_gltf_data(path).unwrap_or_else(|e| die(format!("{path}: {e}")));
    let pose: Vec<LocalTransform> = data.skeleton.as_ref()
        .map(|s| s.joints.iter().map(|j| j.rest).collect())
        .unwrap_or_default();
    review::skin_to_pose(&mut data, &pose);
    for prim in &mut data.primitives {
        for v in &mut prim.vertices {
            v.position = (Vec3::from_array(v.position) * scale).to_array();
        }
    }
    data.primitives
}

/// `clay` is a CPU-side material override, not a debug channel: every image
/// slot clears to `None` so the mesh store binds its neutral 1x1 defaults
/// (store.rs:121-125 — white albedo/AO/MR, flat normal), then the primitive
/// renders through the ordinary Beauty path.
fn clay_override(prims: &mut [PrimitiveData]) {
    for p in prims {
        p.material = MaterialData {
            base_color_factor: [0.5, 0.5, 0.5, 1.0],
            roughness_factor:  0.8,
            metallic_factor:   0.0,
            ..Default::default()
        };
    }
}

/// Direction pointing toward a light source at `azimuth` (radians, matching
/// `Camera::recompute_eye`'s XZ convention: 0 = +X, increasing toward +Z)
/// and `elevation` above horizontal — the same "points toward the source"
/// convention as `SUN_DIR`/`TestLight::direction`.
fn light_dir(azimuth: f32, elevation: f32) -> Vec3 {
    Vec3::new(azimuth.cos() * elevation.cos(), elevation.sin(), azimuth.sin() * elevation.cos())
}

/// Fixed key-light azimuth for `studio`/`furnace` — arbitrary (their
/// neutrality doesn't depend on which side the key comes from) but held
/// constant so a sweep's frames only vary by camera yaw.
const KEY_AZIMUTH: f32 = std::f32::consts::FRAC_PI_4;

fn set_light_for(r: &mut OffscreenRenderer, lighting: Lighting, camera_yaw: f32) {
    match lighting {
        Lighting::Studio => r.set_light(TestLight {
            direction: light_dir(KEY_AZIMUTH, 60.0_f32.to_radians()),
            color:     Vec3::ONE,
            ambient:   1.0,
        }),
        Lighting::Furnace => r.set_light(TestLight {
            direction: light_dir(KEY_AZIMUTH, 60.0_f32.to_radians()),
            color:     Vec3::ZERO,
            ambient:   1.0,
        }),
        // Camera-relative: a fixed world-space rake only rakes at some yaws.
        Lighting::Raking => r.set_light(TestLight {
            direction: light_dir(camera_yaw + std::f32::consts::FRAC_PI_2, 8.0_f32.to_radians()),
            color:     Vec3::splat(2.0),
            ambient:   1.0,
        }),
        Lighting::Dusk => r.set_light(TestLight { direction: SUN_DIR, color: SUN_COLOR, ambient: 1.0 }),
    }
}

/// One-time-per-lighting-group setup: environment, bloom, fog. `start_fog`
/// is the "start" zone's authored fog (`dusk` is the ship gate — it wants
/// the real destination atmosphere); every other preset renders fog-free so
/// material/debug reads aren't hazed.
fn setup_environment(r: &mut OffscreenRenderer, lighting: Lighting, start_fog: (Vec3, f32, f32, f32)) {
    match lighting {
        Lighting::Studio  => r.set_uniform_environment([0.6; 3]),
        Lighting::Furnace => r.set_uniform_environment([1.0; 3]),
        Lighting::Raking  => r.set_uniform_environment([0.05; 3]),
        Lighting::Dusk => {
            if let Err(e) = r.load_environment_hdr(HDRI_DUSK) {
                die(format!("HDRI {HDRI_DUSK}: {e}"));
            }
        }
    }
    // The 1.0 uniform-radiance background would otherwise bloom over the
    // silhouette and defeat furnace's energy check.
    r.set_bloom_intensity(if lighting == Lighting::Furnace { 0.0 } else { DEFAULT_BLOOM });
    let (color, density, height, falloff) = if lighting == Lighting::Dusk {
        start_fog
    } else {
        (Vec3::ZERO, 0.0, 0.0, 0.0)
    };
    r.set_fog(color, density);
    r.set_fog_height(height, falloff);
}

/// Aim the camera `dist` metres from `prims`' near surface along `yaw`, eye
/// at height `eye_y`. Mirrors zone_review.rs's `render_close`: aiming at the
/// raw AABB centre puts the camera inside solid geometry for large props
/// (chapel_arch is 136 m², rock_face_01 spans tens of metres at its shipped
/// scale), so the near-surface projection along the azimuth is used instead.
fn aim_close(r: &mut OffscreenRenderer, prims: &[PrimitiveData], min: Vec3, max: Vec3, yaw: f32, dist: f32, eye_y: f32) {
    let center = Vec3::new((min.x + max.x) * 0.5, 0.0, (min.z + max.z) * 0.5);
    let az = Vec3::new(yaw.cos(), 0.0, yaw.sin());
    let near_extent = prims.iter().flat_map(|p| p.vertices.iter())
        .map(|v| (Vec3::from_array(v.position) - center).dot(az))
        .fold(0.0f32, f32::max);
    let aim = center + az * near_extent;

    let target_y = eye_y.clamp(min.y + 0.15, (max.y - 0.15).max(min.y + 0.15));
    let target = Vec3::new(aim.x, target_y, aim.z);

    let dy = eye_y - target_y;
    let horiz = (dist * dist - dy * dy).max(0.25).sqrt();
    let eye = Vec3::new(target.x + az.x * horiz, eye_y, target.z + az.z * horiz);
    r.set_camera_lookat(eye, target);
}

fn frame_camera(r: &mut OffscreenRenderer, distance: Distance, prims: &[PrimitiveData], min: Vec3, max: Vec3, yaw: f32) {
    match distance {
        Distance::Full     => r.set_camera_turntable(min, max, yaw),
        Distance::Gameplay => aim_close(r, prims, min, max, yaw, CLOSE_DISTANCE, min.y + EYE_HEIGHT),
        Distance::Macro    => aim_close(r, prims, min, max, yaw, MACRO_DISTANCE, (min.y + max.y) * 0.5),
    }
}

/// Render one frame of `model` at `scale`, framed to the precomputed
/// `(min, max)` AABB. `min.y` also seats the calibration ground quad, so a
/// caller's own AABB (not a freshly reloaded one) keeps that quad and the
/// camera's framing in agreement.
#[allow(clippy::too_many_arguments)]
fn shoot(
    r: &mut OffscreenRenderer, model: &str, scale: f32, min: Vec3, max: Vec3,
    channel: Channel, distance: Distance, yaw: f32, w: u32, h: u32, clear: wgpu::Color,
) -> RgbaImage {
    let mut prims = load_scaled(model, scale);
    if channel == Channel::Clay {
        clay_override(&mut prims);
    }
    frame_camera(r, distance, &prims, min, max, yaw);
    prims.push(review::ground_quad(min.y, GROUND_EXTENT));
    let target = r.target(w, h);
    r.render_mesh(&target, MeshData { primitives: prims, skeleton: None, clips: Vec::new() }, clear);
    let pixels = r.read(&target);
    RgbaImage::from_raw(w, h, pixels).expect("readback size matches WxH")
}

fn save(img: &RgbaImage, path: &Path) {
    if let Err(e) = img.save(path) {
        die(format!("cannot write {}: {e}", path.display()));
    }
}

/// Downscale so the long edge is at most `MAX_SHEET_EDGE` — the sheet is
/// only a differencing index; the full-resolution PNGs beside it are the
/// evidence.
fn cap_sheet(sheet: RgbaImage) -> RgbaImage {
    let (w, h) = sheet.dimensions();
    let long = w.max(h);
    if long <= MAX_SHEET_EDGE {
        return sheet;
    }
    let scale = MAX_SHEET_EDGE as f32 / long as f32;
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    image::imageops::resize(&sheet, nw, nh, image::imageops::FilterType::Triangle)
}

fn main() {
    let args = parse_args();
    let (w, h) = args.size;

    let Some(mut r) = OffscreenRenderer::new(w as f32 / h as f32) else {
        eprintln!("asset_inspect: no GPU adapter available");
        exit(1);
    };

    let out = Path::new(&args.out);
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("asset_inspect: cannot create {}: {e}", args.out);
        exit(1);
    }

    let (min, max) = review::aabb(&load_scaled(&args.model, args.scale));
    let ref_bounds = args.reference
        .then(|| review::aabb(&load_scaled(REFERENCE_MODEL, REFERENCE_SCALE)));

    let def = load_zones(ZONES_PATH);
    let start = def.zones.iter().find(|z| z.name == "start")
        .unwrap_or_else(|| die("zones.ron has no \"start\" zone".to_string()));
    let start_fog = (
        start.visuals.fog_color, start.visuals.fog_density,
        start.visuals.fog_height, start.visuals.fog_height_falloff,
    );

    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut total_frames = 0usize;

    for &lighting in &args.lighting {
        setup_environment(&mut r, lighting, start_fog);
        for &channel in &args.channel {
            let dir = out.join(format!("{}_{}", lighting.name(), channel.name()));
            if let Err(e) = std::fs::create_dir_all(&dir) {
                die(format!("cannot create {}: {e}", dir.display()));
            }
            r.set_debug_channel(channel.debug_channel());
            let (draw_sky, clear) = if channel == Channel::Beauty {
                (lighting.draw_sky(), lighting.clear_color())
            } else {
                (false, wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 })
            };
            r.draw_sky = draw_sky;

            let mut sheet_frames: Vec<RgbaImage> = Vec::new();
            for &distance in &args.distance {
                for angle in 0..args.angles {
                    let yaw = std::f32::consts::TAU * angle as f32 / args.angles as f32;
                    set_light_for(&mut r, lighting, yaw);

                    let img = shoot(&mut r, &args.model, args.scale, min, max, channel, distance, yaw, w, h, clear);
                    save(&img, &dir.join(format!("{}_{angle:02}.png", distance.name())));
                    sheet_frames.push(img);
                    total_frames += 1;

                    if let Some((rmin, rmax)) = ref_bounds {
                        let rimg = shoot(&mut r, REFERENCE_MODEL, REFERENCE_SCALE, rmin, rmax, channel, distance, yaw, w, h, clear);
                        save(&rimg, &dir.join(format!("ref_{}_{angle:02}.png", distance.name())));
                        sheet_frames.push(rimg);
                        total_frames += 1;
                    }
                }
            }

            let cell = (w.min(SHEET_TILE), h.min(SHEET_TILE));
            let sheet = cap_sheet(review::contact_sheet(&sheet_frames, cell));
            save(&sheet, &out.join(format!("sheet_{}_{}.png", lighting.name(), channel.name())));
            dirs.push(dir);
        }
    }

    println!("asset_inspect: wrote {total_frames} frames + {} sheets under {}", dirs.len(), args.out);
    println!(
        "asset_inspect: sheets are an index only (tiles capped at {SHEET_TILE}px, long edge near {MAX_SHEET_EDGE}px) — \
         judge gameplay/macro frames individually at full {w}x{h} resolution, from:"
    );
    for dir in &dirs {
        println!("  {}", dir.display());
    }
}
