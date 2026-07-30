// render_material: render a ground texture set under the standard HDRI across
// N camera-yaw angles for vision review, writing per-frame PNGs and a stitched
// contact sheet. Spinning the view varies the HDRI's apparent lighting angle,
// so a broken or inverted normal map shows as highlights sliding the wrong way
// between frames.

use engine_renderer::offscreen::OffscreenRenderer;
use engine_renderer::review;
use image::RgbaImage;
use std::path::Path;
use std::process::exit;
use vordar_client::ground::{generate_ground, load_ground_material};

const HDRI: &str = "content/textures/env/castilian_plateau_overcast_2k.hdr";
const GROUND_SIZE: f32 = 40.0;
const GROUND_TILE: f32 = 7.0;

struct Args {
    dir:    String,
    out:    String,
    angles: u32,
    size:   (u32, u32),
    hdri:   String,
}

fn usage(msg: &str) -> ! {
    eprintln!("render_material: {msg}");
    eprintln!("usage: render_material <texture-dir> --out <dir> [--angles N] [--size WxH] [--hdri <path>]");
    exit(2);
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn parse_args() -> Args {
    let (mut dir, mut out, mut angles, mut size, mut hdri) = (None, None, 4u32, (512u32, 512u32), None);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out"    => out = it.next(),
            "--angles" => angles = it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage("--angles needs a positive integer")),
            "--size"   => size = it.next().as_deref().and_then(parse_size).unwrap_or_else(|| usage("--size needs WxH")),
            "--hdri"   => hdri = it.next(),
            _ if a.starts_with("--") => usage(&format!("unknown flag {a}")),
            _ => dir = Some(a),
        }
    }
    match (dir, out) {
        (Some(dir), Some(out)) if angles > 0 => {
            Args { dir, out, angles, size, hdri: hdri.unwrap_or_else(|| HDRI.to_string()) }
        }
        _ => usage("required: <texture-dir> --out <dir>"),
    }
}

fn main() {
    let args = parse_args();
    let (w, h) = args.size;

    let Some(mut r) = OffscreenRenderer::new(w as f32 / h as f32) else {
        eprintln!("render_material: no GPU adapter available");
        exit(1);
    };
    if !r.gpu.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
        eprintln!("render_material: adapter lacks TEXTURE_COMPRESSION_BC (ground sets ship BC7 .dds)");
        exit(1);
    }
    if let Err(e) = r.load_environment_hdr(&args.hdri) {
        eprintln!("render_material: failed to load HDRI {}: {e}", args.hdri);
        exit(1);
    }
    r.draw_sky = true;

    let out = Path::new(&args.out);
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("render_material: cannot create {}: {e}", args.out);
        exit(1);
    }

    let mut frames: Vec<RgbaImage> = Vec::with_capacity(args.angles as usize);
    for i in 0..args.angles {
        // Rebuilt per frame: generate_ground consumes the material, render_mesh
        // consumes the mesh.
        let material = load_ground_material(&args.dir).unwrap_or_else(|e| {
            eprintln!("render_material: {e}");
            exit(1);
        });
        let ground = generate_ground(GROUND_SIZE, GROUND_TILE, material);
        let yaw = std::f32::consts::TAU * i as f32 / args.angles as f32;
        r.set_camera_yaw(yaw);
        let target = r.target(w, h);
        r.render_mesh(&target, ground, wgpu::Color::BLACK);
        let pixels = r.read(&target);
        let img = RgbaImage::from_raw(w, h, pixels).expect("readback size matches WxH");
        let path = out.join(format!("frame_{i:02}.png"));
        if let Err(e) = img.save(&path) {
            eprintln!("render_material: cannot write {}: {e}", path.display());
            exit(1);
        }
        frames.push(img);
    }

    let sheet = review::contact_sheet(&frames, (w, h));
    let sheet_path = out.join("contact_sheet.png");
    if let Err(e) = sheet.save(&sheet_path) {
        eprintln!("render_material: cannot write {}: {e}", sheet_path.display());
        exit(1);
    }
    println!("render_material: wrote {} frames + contact sheet to {}", args.angles, args.out);
}
