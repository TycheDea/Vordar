// Turntable render tool (A0.12): render a glb under the standard HDRI
// environment plus a ground quad, writing one PNG per frame and a stitched
// contact sheet — an eyeball check on asset quality without launching the
// game. Two modes: `--angles N` spins the camera around the bind pose;
// `--clip <name>` holds a fixed 3/4 camera and renders one frame per sampled
// clip time. Either way the pose is skinned on the CPU so the static mesh
// path draws it.

use engine_renderer::anim::{sample_pose, AnimationClip};
use engine_renderer::mesh::{load_gltf_data, MeshData};
use engine_renderer::offscreen::OffscreenRenderer;
use engine_renderer::review;
use glam::Vec3;
use image::RgbaImage;
use std::path::Path;
use std::process::exit;

const HDRI: &str = "content/textures/env/castilian_plateau_dusk_2k.hdr";
const GROUND_EXTENT: f32 = 40.0;
/// Fixed camera yaw for `--clip` frames: the classic 3/4 view.
const THREE_QUARTER_YAW: f32 = std::f32::consts::TAU / 8.0;

struct Args {
    glb:  String,
    out:  String,
    size: (u32, u32),
    hdri: String,
    mode: Mode,
}

enum Mode {
    /// Spin the camera: one frame per evenly spaced yaw angle, bind pose.
    Static { angles: u32 },
    /// Fixed 3/4 camera: one frame per clip sample time (seconds). `times`
    /// None → 5 evenly spaced samples across the clip's duration.
    Clip { name: String, times: Option<Vec<f32>> },
}

fn usage(msg: &str) -> ! {
    eprintln!("turntable: {msg}");
    eprintln!(
        "usage: turntable <glb> --out <dir> --size WxH \
         (--angles N | --clip <name> [--times t0,t1,..]) [--hdri <path>]"
    );
    exit(2);
}

fn die(e: String) -> ! {
    eprintln!("turntable: {e}");
    exit(1);
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn parse_times(s: &str) -> Option<Vec<f32>> {
    let times: Option<Vec<f32>> = s.split(',').map(|t| t.parse().ok()).collect();
    times.filter(|t| !t.is_empty())
}

fn parse_args() -> Args {
    let (mut glb, mut out, mut angles, mut size, mut hdri) = (None, None, None, None, None);
    let (mut clip, mut times) = (None, None);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out"    => out = it.next(),
            "--angles" => angles = it.next().and_then(|s| s.parse().ok()),
            "--size"   => size = it.next().as_deref().and_then(parse_size),
            "--hdri"   => hdri = it.next(),
            "--clip"   => clip = it.next(),
            "--times"  => times = it.next().as_deref().and_then(parse_times),
            _ if a.starts_with("--") => usage(&format!("unknown flag {a}")),
            _ => glb = Some(a),
        }
    }
    let mode = match (clip, angles) {
        (Some(name), None) => Mode::Clip { name, times },
        (None, Some(angles)) if angles > 0 && times.is_none() => Mode::Static { angles },
        _ => usage("exactly one of --angles N / --clip <name> (--times goes with --clip)"),
    };
    match (glb, out, size) {
        (Some(glb), Some(out), Some(size)) => {
            Args { glb, out, size, hdri: hdri.unwrap_or_else(|| HDRI.to_string()), mode }
        }
        _ => usage("required: <glb> --out <dir> --size WxH"),
    }
}

/// The named clip, or an error listing what the asset actually carries.
fn find_clip<'d>(data: &'d MeshData, name: &str) -> Result<&'d AnimationClip, String> {
    data.clips.iter().find(|c| c.name == name).ok_or_else(|| {
        let have: Vec<&str> = data.clips.iter().map(|c| c.name.as_str()).collect();
        format!("no clip {name:?} (asset clips: [{}])", have.join(", "))
    })
}

/// Skin the loaded mesh to a pose — the named clip sampled at `t` when
/// `clip_time` is `Some((name, t))`, the bind pose otherwise — and return it
/// with its world-space AABB for camera framing and ground placement.
/// Consumes `data` because `render_mesh` consumes MeshData, so callers reload
/// per frame.
fn pose_scene(
    mut data:  MeshData,
    clip_time: Option<(&str, f32)>,
) -> Result<(MeshData, Vec3, Vec3), String> {
    let pose = match (clip_time, data.skeleton.as_ref()) {
        (Some((name, _)), None) => return Err(format!("--clip {name}: mesh has no skeleton")),
        (Some((name, t)), Some(skel)) => sample_pose(skel, find_clip(&data, name)?, t),
        (None, Some(skel)) => skel.joints.iter().map(|j| j.rest).collect(),
        (None, None) => Vec::new(),
    };
    review::skin_to_pose(&mut data, &pose);
    let (min, max) = review::aabb(&data.primitives);
    Ok((data, min, max))
}

fn main() {
    let args = parse_args();
    let (w, h) = args.size;

    let Some(mut r) = OffscreenRenderer::new(w as f32 / h as f32) else {
        eprintln!("turntable: no GPU adapter available");
        exit(1);
    };
    if let Err(e) = r.load_environment_hdr(&args.hdri) {
        eprintln!("turntable: failed to load HDRI {}: {e}", args.hdri);
        exit(1);
    }
    r.draw_sky = true;

    let out = Path::new(&args.out);
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("turntable: cannot create {}: {e}", args.out);
        exit(1);
    }

    // One (clip sample time, camera yaw) per frame. Clip mode also fixes the
    // framing bounds to the bind-pose AABB so the camera and ground hold
    // still while the pose changes between frames.
    let (plan, fixed_bounds, clip_name): (Vec<(Option<f32>, f32)>, _, _) = match args.mode {
        Mode::Static { angles } => (
            (0..angles)
                .map(|i| (None, std::f32::consts::TAU * i as f32 / angles as f32))
                .collect(),
            None,
            None,
        ),
        Mode::Clip { name, times } => {
            let data = load_gltf_data(&args.glb).unwrap_or_else(|e| die(e));
            let duration = find_clip(&data, &name).unwrap_or_else(|e| die(e)).duration;
            // i·d/5, i<5: even spacing that skips t=d, which a looping clip
            // renders identical to t=0.
            let times = times
                .unwrap_or_else(|| (0..5).map(|i| duration * i as f32 / 5.0).collect());
            let (_, min, max) = pose_scene(data, None).unwrap_or_else(|e| die(e));
            (
                times.into_iter().map(|t| (Some(t), THREE_QUARTER_YAW)).collect(),
                Some((min, max)),
                Some(name),
            )
        }
    };

    let mut frames: Vec<RgbaImage> = Vec::with_capacity(plan.len());
    for (i, &(time, yaw)) in plan.iter().enumerate() {
        let data = load_gltf_data(&args.glb).unwrap_or_else(|e| die(e));
        let clip_time = clip_name.as_deref().zip(time);
        let (mut scene, min, max) = pose_scene(data, clip_time).unwrap_or_else(|e| die(e));
        let (fmin, fmax) = fixed_bounds.unwrap_or((min, max));
        scene.primitives.push(review::ground_quad(fmin.y, GROUND_EXTENT));
        r.set_camera_turntable(fmin, fmax, yaw);
        let target = r.target(w, h);
        r.render_mesh(&target, scene, wgpu::Color::BLACK);
        let pixels = r.read(&target);
        let img = RgbaImage::from_raw(w, h, pixels).expect("readback size matches WxH");
        let path = out.join(format!("frame_{i:02}.png"));
        if let Err(e) = img.save(&path) {
            eprintln!("turntable: cannot write {}: {e}", path.display());
            exit(1);
        }
        frames.push(img);
    }

    let sheet = review::contact_sheet(&frames, (w, h));
    let sheet_path = out.join("contact_sheet.png");
    if let Err(e) = sheet.save(&sheet_path) {
        eprintln!("turntable: cannot write {}: {e}", sheet_path.display());
        exit(1);
    }
    println!("turntable: wrote {} frames + contact sheet to {}", frames.len(), args.out);
}
