// Turntable render tool (A0.12): render N yaw angles of a glb under the
// standard HDRI environment plus a ground quad, writing one PNG per frame and
// a stitched contact sheet — an eyeball check on asset quality without
// launching the game. Skinned meshes are flattened to bind pose on the CPU so
// the static mesh path draws them (animated turntables are a later phase).

use engine_renderer::anim::{joint_matrices, LocalTransform};
use engine_renderer::mesh::{load_gltf_data, MaterialData, MeshData, PrimitiveData};
use engine_renderer::offscreen::OffscreenRenderer;
use engine_renderer::MeshVertex;
use glam::Vec3;
use image::RgbaImage;
use std::path::Path;
use std::process::exit;

const HDRI: &str = "content/textures/env/evening_road_01_puresky_2k.hdr";
const GROUND_EXTENT: f32 = 40.0;

struct Args {
    glb:    String,
    out:    String,
    angles: u32,
    size:   (u32, u32),
}

fn usage(msg: &str) -> ! {
    eprintln!("turntable: {msg}");
    eprintln!("usage: turntable <glb> --out <dir> --angles N --size WxH");
    exit(2);
}

fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn parse_args() -> Args {
    let (mut glb, mut out, mut angles, mut size) = (None, None, None, None);
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out"    => out = it.next(),
            "--angles" => angles = it.next().and_then(|s| s.parse().ok()),
            "--size"   => size = it.next().as_deref().and_then(parse_size),
            _ if a.starts_with("--") => usage(&format!("unknown flag {a}")),
            _ => glb = Some(a),
        }
    }
    match (glb, out, angles, size) {
        (Some(glb), Some(out), Some(angles), Some(size)) if angles > 0 => {
            Args { glb, out, angles, size }
        }
        _ => usage("required: <glb> --out <dir> --angles N --size WxH"),
    }
}

/// Load the glb, flatten any skin to bind pose, and add a ground quad at the
/// asset's floor. Returns the asset's world-space AABB (before the ground) for
/// camera framing. Rebuilt per frame because `render_mesh` consumes MeshData.
fn build_scene(glb: &str) -> Result<(MeshData, Vec3, Vec3), String> {
    let mut data = load_gltf_data(glb)?;
    flatten_to_bind_pose(&mut data);
    let (min, max) = aabb(&data);
    data.primitives.push(ground_quad(min.y));
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

/// Replace every skinned vertex with the bind-pose-weighted sum over its
/// joints, then drop the skeleton so the static mesh path draws it. A no-op
/// for meshes that carry no skin.
fn flatten_to_bind_pose(data: &mut MeshData) {
    let Some(skel) = data.skeleton.as_ref() else { return };
    let rest: Vec<LocalTransform> = skel.joints.iter().map(|j| j.rest).collect();
    let mats = joint_matrices(skel, &rest);
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
    if let Err(e) = r.load_environment_hdr(HDRI) {
        eprintln!("turntable: failed to load HDRI {HDRI}: {e}");
        exit(1);
    }
    r.draw_sky = true;

    let out = Path::new(&args.out);
    if let Err(e) = std::fs::create_dir_all(out) {
        eprintln!("turntable: cannot create {}: {e}", args.out);
        exit(1);
    }

    let mut frames: Vec<RgbaImage> = Vec::with_capacity(args.angles as usize);
    for i in 0..args.angles {
        let (scene, min, max) = build_scene(&args.glb).unwrap_or_else(|e| {
            eprintln!("turntable: {e}");
            exit(1);
        });
        let yaw = std::f32::consts::TAU * i as f32 / args.angles as f32;
        r.set_camera_turntable(min, max, yaw);
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
    println!("turntable: wrote {} frames + contact sheet to {}", args.angles, args.out);
}
