// Turntable render tool (A0.12): render a glb under the standard HDRI
// environment plus a ground quad, writing one PNG per frame and a stitched
// contact sheet — an eyeball check on asset quality without launching the
// game. Two modes: `--angles N` spins the camera around the bind pose;
// `--clip <name>` holds a fixed 3/4 camera and renders one frame per sampled
// clip time. Either way the pose is skinned on the CPU so the static mesh
// path draws it.

use engine_renderer::anim::{joint_matrices, sample_pose, AnimationClip, LocalTransform};
use engine_renderer::mesh::{load_gltf_data, MaterialData, MeshData, PrimitiveData};
use engine_renderer::offscreen::OffscreenRenderer;
use engine_renderer::MeshVertex;
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
    skin_to_pose(&mut data, &pose);
    let (min, max) = aabb(&data);
    Ok((data, min, max))
}

/// World-space bounds over every vertex of the mesh.
fn aabb(data: &MeshData) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for v in data.primitives.iter().flat_map(|p| p.vertices.iter()) {
        let pos = Vec3::from_array(v.position);
        min = min.min(pos);
        max = max.max(pos);
    }
    (min, max)
}

/// Replace every skinned vertex with the pose-weighted sum over its joints,
/// then drop the skeleton so the static mesh path draws it. A no-op for
/// meshes that carry no skin.
fn skin_to_pose(data: &mut MeshData, pose: &[LocalTransform]) {
    let Some(skel) = data.skeleton.as_ref() else { return };
    let mats = joint_matrices(skel, pose);
    for prim in &mut data.primitives {
        let Some(skin) = prim.skin.take() else { continue };
        for (v, s) in prim.vertices.iter_mut().zip(&skin) {
            let pos = Vec3::from_array(v.position);
            let nrm = Vec3::from_array(v.normal);
            let tan = Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2]);
            let (mut p, mut n, mut t) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
            for (&j, &w) in s.joints.iter().zip(&s.weights) {
                if w > 0.0 {
                    let m = mats[j as usize];
                    p += m.transform_point3(pos) * w;
                    n += m.transform_vector3(nrm) * w;
                    t += m.transform_vector3(tan) * w;
                }
            }
            v.position = p.to_array();
            v.normal = n.normalize_or_zero().to_array();
            let tn = t.normalize_or_zero();
            v.tangent = [tn.x, tn.y, tn.z, v.tangent[3]];
        }
    }
    data.skeleton = None;
    data.clips = Vec::new();
}

/// A grey ground quad at `y`, spanning ±GROUND_EXTENT, normal +Y.
fn ground_quad(y: f32) -> PrimitiveData {
    let e = GROUND_EXTENT;
    let v = |x: f32, z: f32, u: f32, w: f32| MeshVertex {
        position: [x, y, z],
        normal:   [0.0, 1.0, 0.0],
        uv:       [u, w],
        tangent:  [1.0, 0.0, 0.0, 1.0],
    };
    PrimitiveData {
        vertices: vec![v(-e, -e, 0.0, 0.0), v(e, -e, 1.0, 0.0), v(e, e, 1.0, 1.0), v(-e, e, 0.0, 1.0)],
        indices:  vec![0, 2, 1, 0, 3, 2],
        material: MaterialData {
            base_color_factor: [0.5, 0.5, 0.5, 1.0],
            roughness_factor:  0.9,
            metallic_factor:   0.0,
            ..Default::default()
        },
        skin: None,
    }
}

/// Lay the frames out in a near-square grid, one cell per frame.
fn contact_sheet(frames: &[RgbaImage], w: u32, h: u32) -> RgbaImage {
    let n = frames.len() as u32;
    let cols = (n as f64).sqrt().ceil() as u32;
    let rows = n.div_ceil(cols);
    let mut sheet = RgbaImage::from_pixel(cols * w, rows * h, image::Rgba([0, 0, 0, 255]));
    for (i, frame) in frames.iter().enumerate() {
        let x = (i as u32 % cols) * w;
        let y = (i as u32 / cols) * h;
        image::imageops::replace(&mut sheet, frame, x as i64, y as i64);
    }
    sheet
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
        scene.primitives.push(ground_quad(fmin.y));
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

    let sheet = contact_sheet(&frames, w, h);
    let sheet_path = out.join("contact_sheet.png");
    if let Err(e) = sheet.save(&sheet_path) {
        eprintln!("turntable: cannot write {}: {e}", sheet_path.display());
        exit(1);
    }
    println!("turntable: wrote {} frames + contact sheet to {}", frames.len(), args.out);
}
