// gear_render: turntable a skinned character glb with the procedural
// sword+shield from weapons.rs riding its hand sockets — a vision-review
// render for WeaponAttachment::local grip tuning, mirroring turntable.rs's
// shape one crate over (its skin_to_pose is duplicated here for the same
// reason golden.rs duplicates it: a bin can't reach a sibling crate's bin).

use engine_renderer::anim::{global_transforms, joint_matrices, sample_pose, LocalTransform};
use engine_renderer::mesh::{load_gltf_data, MaterialData, MeshData, PrimitiveData};
use engine_renderer::offscreen::OffscreenRenderer;
use engine_renderer::MeshVertex;
use glam::{Mat4, Vec3};
use image::RgbaImage;
use std::path::Path;
use std::process::exit;
use vordar_client::weapons::{shield_grip_local, shield_mesh, sword_grip_local, sword_mesh};

const HDRI: &str = "content/textures/env/castilian_plateau_dusk_2k.hdr";
const GROUND_EXTENT: f32 = 40.0;
const THREE_QUARTER_YAW: f32 = std::f32::consts::TAU / 8.0;

struct Args {
    glb:   String,
    out:   String,
    size:  (u32, u32),
    hdri:  String,
    clip:  Option<String>,
    time:  f32,
    angles: u32,
}

fn usage(msg: &str) -> ! {
    eprintln!("gear_render: {msg}");
    eprintln!(
        "usage: gear_render <glb> --out <dir> --size WxH [--angles N] \
         [--clip <name>] [--time t] [--hdri <path>]"
    );
    exit(2);
}

fn die(e: String) -> ! {
    eprintln!("gear_render: {e}");
    exit(1);
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn parse_args() -> Args {
    let (mut glb, mut out, mut size, mut hdri) = (None, None, None, None);
    let (mut clip, mut time, mut angles) = (None, 0.0f32, 4u32);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out"    => out = it.next(),
            "--size"   => size = it.next().as_deref().and_then(parse_size),
            "--hdri"   => hdri = it.next(),
            "--clip"   => clip = it.next(),
            "--time"   => time = it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage("--time needs a float")),
            "--angles" => angles = it.next().and_then(|s| s.parse().ok()).unwrap_or_else(|| usage("--angles needs a positive integer")),
            _ if a.starts_with("--") => usage(&format!("unknown flag {a}")),
            _ => glb = Some(a),
        }
    }
    match (glb, out, size) {
        (Some(glb), Some(out), Some(size)) => {
            Args { glb, out, size, hdri: hdri.unwrap_or_else(|| HDRI.to_string()), clip, time, angles }
        }
        _ => usage("required: <glb> --out <dir> --size WxH"),
    }
}

/// Replace every skinned vertex with the pose-weighted sum over its joints,
/// then drop the skeleton so the static mesh path draws it.
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

/// Rigidly place a static (unskinned) weapon mesh's primitives into world
/// space, matching WeaponAttachSystem's socket-follow: rotation + translation
/// only, scale dropped.
fn place_rigid(mesh: MeshData, world: Mat4) -> Vec<PrimitiveData> {
    let (_, rotation, translation) = world.to_scale_rotation_translation();
    let placement = Mat4::from_rotation_translation(rotation, translation);
    mesh.primitives
        .into_iter()
        .map(|mut prim| {
            for v in &mut prim.vertices {
                let pos = Vec3::from_array(v.position);
                let nrm = Vec3::from_array(v.normal);
                let tan = Vec3::new(v.tangent[0], v.tangent[1], v.tangent[2]);
                v.position = placement.transform_point3(pos).to_array();
                v.normal = placement.transform_vector3(nrm).normalize_or_zero().to_array();
                let tn = placement.transform_vector3(tan).normalize_or_zero();
                v.tangent = [tn.x, tn.y, tn.z, v.tangent[3]];
            }
            prim
        })
        .collect()
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

/// A grey ground quad at `y`, spanning ±GROUND_EXTENT, normal +Y.
fn ground_quad(y: f32) -> PrimitiveData {
    let e = GROUND_EXTENT;
    let v = |x: f32, z: f32, u: f32, w: f32| MeshVertex {
        position: [x, y, z], normal: [0.0, 1.0, 0.0], uv: [u, w], tangent: [1.0, 0.0, 0.0, 1.0],
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

/// Character + sword + shield at one posed instant, world-space, gear glued
/// to the handslot sockets exactly as WeaponAttachSystem would.
fn build_scene(glb: &str, clip_time: Option<(&str, f32)>) -> MeshData {
    let mut data = load_gltf_data(glb).unwrap_or_else(|e| die(e));
    let skel = data.skeleton.as_ref().unwrap_or_else(|| die("mesh has no skeleton".into()));
    let pose = match clip_time {
        Some((name, t)) => {
            let clip = data.clips.iter().find(|c| c.name == name).unwrap_or_else(|| {
                let have: Vec<&str> = data.clips.iter().map(|c| c.name.as_str()).collect();
                die(format!("no clip {name:?} (asset clips: [{}])", have.join(", ")))
            });
            sample_pose(skel, clip, t)
        }
        None => skel.joints.iter().map(|j| j.rest).collect(),
    };
    let globals = global_transforms(skel, &pose);
    let socket = |bone: &str| {
        let j = skel.joints.iter().position(|jt| jt.name == bone).unwrap_or_else(|| {
            die(format!("skeleton has no {bone:?} joint"))
        });
        globals[j]
    };
    let sword_world = socket("handslot.r") * sword_grip_local();
    let shield_world = socket("handslot.l") * shield_grip_local();

    skin_to_pose(&mut data, &pose);
    data.primitives.extend(place_rigid(sword_mesh(), sword_world));
    data.primitives.extend(place_rigid(shield_mesh(), shield_world));
    data
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
        eprintln!("gear_render: no GPU adapter available");
        exit(1);
    };
    if let Err(e) = r.load_environment_hdr(&args.hdri) {
        eprintln!("gear_render: failed to load HDRI {}: {e}", args.hdri);
        exit(1);
    }
    r.draw_sky = true;

    let out = Path::new(&args.out);
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("gear_render: cannot create {}: {e}", args.out);
        exit(1);
    }

    let clip_time = args.clip.as_deref().map(|name| (name, args.time));
    let (fmin, fmax) = aabb(&build_scene(&args.glb, clip_time));

    let mut frames: Vec<RgbaImage> = Vec::with_capacity(args.angles as usize);
    for i in 0..args.angles {
        let yaw = if args.angles == 1 {
            THREE_QUARTER_YAW
        } else {
            std::f32::consts::TAU * i as f32 / args.angles as f32
        };
        let mut scene = build_scene(&args.glb, clip_time);
        scene.primitives.push(ground_quad(fmin.y));
        r.set_camera_turntable(fmin, fmax, yaw);
        let target = r.target(w, h);
        r.render_mesh(&target, scene, wgpu::Color::BLACK);
        let pixels = r.read(&target);
        let img = RgbaImage::from_raw(w, h, pixels).expect("readback size matches WxH");
        let path = out.join(format!("frame_{i:02}.png"));
        if let Err(e) = img.save(&path) {
            eprintln!("gear_render: cannot write {}: {e}", path.display());
            exit(1);
        }
        frames.push(img);
    }

    let sheet = contact_sheet(&frames, w, h);
    let sheet_path = out.join("contact_sheet.png");
    if let Err(e) = sheet.save(&sheet_path) {
        eprintln!("gear_render: cannot write {}: {e}", sheet_path.display());
        exit(1);
    }
    println!("gear_render: wrote {} frames + contact sheet to {}", frames.len(), args.out);
}
