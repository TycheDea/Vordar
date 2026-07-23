// Golden-frame visual regression harness.
//
// Thresholded mean-FLIP perceptual comparison against checked-in goldens — a
// tolerance band, not exact-pixel equality, so sub-threshold driver variance
// can't flake it (deliberately distinct from offscreen.rs's analytic-only
// discipline). Goldens are canonical for this workstation's GPU adapter;
// regeneration ONLY via UPDATE_GOLDENS=1 by the user or an explicitly
// instructed session — automated workers must never regenerate.

use engine_renderer::anim::{joint_matrices, sample_pose, LocalTransform};
use engine_renderer::instance::SdfInstance;
use engine_renderer::mesh::{load_gltf_data, MaterialData, MeshData, PrimitiveData};
use engine_renderer::offscreen::{OffscreenRenderer, TestLight, TestPointLight};
use engine_renderer::MeshVertex;
use glam::{Mat4, Quat, Vec3};
use std::path::{Path, PathBuf};

const W: u32 = 512;
const H: u32 = 512;

// Calibration task tightens these next — do not tighten from this harness.
const SDF_COMPOSITE_THRESHOLD: f32 = 0.05;
const HELMET_THRESHOLD:        f32 = 0.05;
const HUMAN_THRESHOLD:         f32 = 0.05;

fn renderer_or_skip() -> Option<OffscreenRenderer> {
    let r = OffscreenRenderer::new(W as f32 / H as f32);
    if r.is_none() {
        eprintln!("SKIP: no GPU adapter available — golden tests need one");
    }
    r
}

// ── Compare/update helper ────────────────────────────────────────────────────

fn golden_path(scene: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens").join(format!("{scene}.png"))
}

/// Drop alpha: FLIP compares RGB only.
fn to_rgb8(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect()
}

/// Render vs. checked-in golden, thresholded on mean FLIP error. `UPDATE_GOLDENS=1`
/// (over)writes the golden and returns instead of comparing — the same path bootstraps
/// a golden that doesn't exist yet. A missing golden without the env var is a hard
/// failure, never a silent pass.
fn compare_to_golden(scene: &str, pixels: &[u8], width: u32, height: u32, threshold: f32) {
    let path = golden_path(scene);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create tests/goldens");
        image::RgbaImage::from_raw(width, height, pixels.to_vec())
            .expect("readback size matches WxH")
            .save(&path)
            .unwrap_or_else(|e| panic!("writing golden {}: {e}", path.display()));
        return;
    }

    let Ok(golden_img) = image::open(&path) else {
        panic!("{scene}: no golden at {} — run with UPDATE_GOLDENS=1 to bootstrap it", path.display());
    };
    let golden = golden_img.into_rgba8();
    assert_eq!(
        (golden.width(), golden.height()), (width, height),
        "{scene}: golden size mismatch"
    );

    let reference = nv_flip::FlipImageRgb8::with_data(width, height, &to_rgb8(&golden));
    let test      = nv_flip::FlipImageRgb8::with_data(width, height, &to_rgb8(pixels));
    let error_map = nv_flip::flip(reference, test, nv_flip::DEFAULT_PIXELS_PER_DEGREE);
    let mean      = nv_flip::FlipPool::from_image(&error_map).mean();

    if mean > threshold {
        let diff_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-diffs").join(scene);
        std::fs::create_dir_all(&diff_dir).expect("create golden-diffs dir");
        image::RgbaImage::from_raw(width, height, pixels.to_vec())
            .expect("readback size matches WxH")
            .save(diff_dir.join("actual.png"))
            .expect("write actual.png");
        let visualized = error_map.apply_color_lut(&nv_flip::magma_lut());
        image::RgbImage::from_raw(width, height, visualized.to_vec())
            .expect("FLIP map size matches WxH")
            .save(diff_dir.join("flip.png"))
            .expect("write flip.png");
        panic!("{scene}: mean FLIP {mean} > threshold {threshold}");
    }
}

// ── Shared scene helpers ─────────────────────────────────────────────────────

fn sdf_box(scale: Vec3, position: Vec3, color: [f32; 3]) -> SdfInstance {
    SdfInstance {
        model: Mat4::from_scale_rotation_translation(scale, Quat::IDENTITY, position).to_cols_array_2d(),
        color,
        shape_type: 0, // cube
        shape_params: [0.0; 4],
    }
}

const THREE_QUARTER_YAW: f32 = std::f32::consts::TAU / 8.0;
const GROUND_EXTENT: f32 = 40.0;

/// World-space bounds over every vertex of the mesh — camera framing input.
fn mesh_aabb(data: &MeshData) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for v in data.primitives.iter().flat_map(|p| p.vertices.iter()) {
        let pos = Vec3::from_array(v.position);
        min = min.min(pos);
        max = max.max(pos);
    }
    (min, max)
}

/// A grey ground quad at `y`, spanning ±GROUND_EXTENT, normal +Y — mirrors the
/// turntable bin's framing so helmet/human goldens match its contact sheets.
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

// ── Scene 1: procedural SDF composite ───────────────────────────────────────

/// Shadow caster + ground-contact box + HDR-bright box under a sun + point
/// light: one frame exercising cast shadows, SSAO contact darkening, and
/// bloom, with zero content-fixture dependencies.
#[test]
fn golden_sdf_composite() {
    let Some(mut r) = renderer_or_skip() else { return };

    let ground        = sdf_box(Vec3::new(60.0, 1.0, 60.0), Vec3::new(0.0, -0.5, 0.0), [0.8, 0.8, 0.8]);
    let shadow_caster = sdf_box(Vec3::splat(5.0), Vec3::new(-2.0, 6.0, -2.0), [0.85, 0.25, 0.2]);
    let ao_box        = sdf_box(Vec3::new(6.0, 3.0, 6.0), Vec3::new(6.0, 1.5, 4.0), [0.75, 0.75, 0.8]);
    let bloom_box     = sdf_box(Vec3::splat(2.0), Vec3::new(-8.0, 1.0, 8.0), [4.0, 3.2, 0.0]);

    r.set_light(TestLight {
        direction: Vec3::new(-1.0, 1.3, -0.6),
        color:     Vec3::new(1.0, 0.95, 0.85),
        ambient:   0.5,
    });
    r.set_point_lights(&[TestPointLight {
        position:  Vec3::new(6.0, 4.0, 9.0),
        color:     Vec3::new(0.3, 0.6, 1.0),
        intensity: 40.0,
        radius:    25.0,
    }]);
    r.set_ssao(true);

    let target = r.target(W, H);
    r.render_sdf(&target, &[ground, shadow_caster, ao_box, bloom_box], wgpu::Color::BLACK);
    let pixels = r.read(&target);

    compare_to_golden("golden_sdf_composite", &pixels, W, H, SDF_COMPOSITE_THRESHOLD);
}

// ── Scene 2: DamagedHelmet under a baked HDRI ───────────────────────────────

#[test]
fn golden_helmet_ibl() {
    let helmet = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/source/test/DamagedHelmet.glb");
    let hdri   = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/textures/env/castilian_plateau_dusk_2k.hdr");
    if !Path::new(helmet).exists() || !Path::new(hdri).exists() {
        eprintln!("SKIP: DamagedHelmet/HDRI fixtures missing");
        return;
    }
    let Some(mut r) = renderer_or_skip() else { return };

    r.load_environment_hdr(hdri).expect("HDRI decodes");
    r.draw_sky = true;

    let mut data = load_gltf_data(helmet).expect("helmet parses");
    let (min, max) = mesh_aabb(&data);
    data.primitives.push(ground_quad(min.y));
    r.set_camera_turntable(min, max, THREE_QUARTER_YAW);

    let target = r.target(W, H);
    r.render_mesh(&target, data, wgpu::Color::BLACK);
    let pixels = r.read(&target);

    compare_to_golden("golden_helmet_ibl", &pixels, W, H, HELMET_THRESHOLD);
}

// ── Scene 3: CPU-skinned human at a fixed clip time ─────────────────────────

/// Replace every skinned vertex with the pose-weighted sum over its joints,
/// then drop the skeleton so the static mesh path draws it — same as
/// turntable.rs's `skin_to_pose`, duplicated here since integration tests
/// can't reach a sibling bin target.
fn skin_to_pose(data: &mut MeshData, pose: &[LocalTransform]) {
    let mats = joint_matrices(data.skeleton.as_ref().expect("human is skinned"), pose);
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

#[test]
fn golden_skinned_human() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/models/human.glb");
    if !Path::new(path).exists() {
        eprintln!("SKIP: human.glb fixture missing");
        return;
    }
    let Some(mut r) = renderer_or_skip() else { return };

    let mut data = load_gltf_data(path).expect("human parses");
    let clip = data.clips.iter().find(|c| c.name == "walk").expect("walk clip present");
    let pose = sample_pose(data.skeleton.as_ref().expect("human is skinned"), clip, clip.duration * 0.4);
    skin_to_pose(&mut data, &pose);

    let (min, max) = mesh_aabb(&data);
    data.primitives.push(ground_quad(min.y));
    r.set_camera_turntable(min, max, THREE_QUARTER_YAW);

    let target = r.target(W, H);
    r.render_mesh(&target, data, wgpu::Color::BLACK);
    let pixels = r.read(&target);

    compare_to_golden("golden_skinned_human", &pixels, W, H, HUMAN_THRESHOLD);
}
