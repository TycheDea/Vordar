// Framing/scene/contact-sheet helpers shared by the offscreen review bins
// (turntable, gear_render, render_material, zone_review) — split out because
// a bin cannot reach a sibling bin.

use crate::anim::{joint_matrices, LocalTransform};
use crate::mesh::{MaterialData, MeshData, PrimitiveData};
use crate::mesh_pipeline::MeshVertex;
use glam::Vec3;
use image::RgbaImage;

/// World-space bounds over every vertex across `prims`.
pub fn aabb(prims: &[PrimitiveData]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for v in prims.iter().flat_map(|p| p.vertices.iter()) {
        let pos = Vec3::from_array(v.position);
        min = min.min(pos);
        max = max.max(pos);
    }
    (min, max)
}

/// A grey ground quad at `y`, spanning ±`extent`, normal +Y.
pub fn ground_quad(y: f32, extent: f32) -> PrimitiveData {
    let e = extent;
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

/// Replace every skinned vertex with the pose-weighted sum over its joints,
/// then drop the skeleton so the static mesh path draws it. A no-op for
/// meshes that carry no skin.
pub fn skin_to_pose(data: &mut MeshData, pose: &[LocalTransform]) {
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

/// Lay `frames` out in a near-square grid, one cell per frame — a frame is
/// resampled to `cell` only when its own size doesn't already match it.
pub fn contact_sheet(frames: &[RgbaImage], cell: (u32, u32)) -> RgbaImage {
    let (cw, ch) = cell;
    let n = frames.len() as u32;
    let cols = (n as f64).sqrt().ceil() as u32;
    let rows = n.div_ceil(cols);
    let mut sheet = RgbaImage::from_pixel(cols * cw, rows * ch, image::Rgba([0, 0, 0, 255]));
    for (i, frame) in frames.iter().enumerate() {
        let x = (i as u32 % cols) * cw;
        let y = (i as u32 / cols) * ch;
        if frame.dimensions() == cell {
            image::imageops::replace(&mut sheet, frame, x as i64, y as i64);
        } else {
            let thumb = image::imageops::resize(frame, cw, ch, image::imageops::FilterType::Triangle);
            image::imageops::replace(&mut sheet, &thumb, x as i64, y as i64);
        }
    }
    sheet
}
