// Frustum/AABB culling math, pure and GPU-independent: `Aabb`/`Frustum`
// plus `classify`, which decides whether a mesh instance's world-space bound
// is visible from the camera, the shadow light, both, or neither. This step
// only adds the math and captures mesh bounds at upload — no draw-path
// behavior changes yet.

use glam::{Mat4, Vec3, Vec4};

/// Axis-aligned bounding box, defined by its min/max corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn union(self, other: Aabb) -> Aabb {
        Aabb { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) / 2.0
    }

    fn half_extents(&self) -> Vec3 {
        (self.max - self.min) / 2.0
    }

    /// Scales the half-extents by `factor` about the box's center.
    pub fn inflated(&self, factor: f32) -> Aabb {
        let center = self.center();
        let he = self.half_extents() * factor;
        Aabb { min: center - he, max: center + he }
    }

    /// World-space bound of this AABB under `m`, via the Arvo abs-matrix
    /// method — exact for the transformed box, not just its center.
    pub fn transformed(&self, m: &Mat4) -> Aabb {
        let center = m.transform_point3(self.center());
        let he = self.half_extents();
        let world_he = m.x_axis.truncate().abs() * he.x
            + m.y_axis.truncate().abs() * he.y
            + m.z_axis.truncate().abs() * he.z;
        Aabb { min: center - world_he, max: center + world_he }
    }
}

/// A camera or light's six clip-space half-spaces, derived from a
/// view-projection matrix.
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Gribb-Hartmann plane extraction for wgpu's clip z ∈ [0,1] convention.
    pub fn from_view_proj(m: Mat4) -> Self {
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        Frustum { planes: [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2] }
    }

    /// p-vertex test: conservative but never a false negative — an AABB that
    /// overlaps the frustum even partially tests true.
    pub fn intersects(&self, aabb: &Aabb) -> bool {
        for plane in &self.planes {
            let p = Vec3::new(
                if plane.x >= 0.0 { aabb.max.x } else { aabb.min.x },
                if plane.y >= 0.0 { aabb.max.y } else { aabb.min.y },
                if plane.z >= 0.0 { aabb.max.z } else { aabb.min.z },
            );
            if plane.truncate().dot(p) + plane.w < 0.0 {
                return false;
            }
        }
        true
    }
}

/// Which of two frustums (camera, shadow) an AABB is visible from.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Visibility {
    Both,
    CamOnly,
    ShadowOnly,
}

/// `None` when `world_aabb` intersects neither frustum.
pub fn classify(world_aabb: &Aabb, camera: &Frustum, shadow: &Frustum) -> Option<Visibility> {
    match (camera.intersects(world_aabb), shadow.intersects(world_aabb)) {
        (true, true) => Some(Visibility::Both),
        (true, false) => Some(Visibility::CamOnly),
        (false, true) => Some(Visibility::ShadowOnly),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perspective_frustum_from_camera_classifies_expected_points() {
        let camera = crate::camera::Camera::new(16.0 / 9.0);
        let vp = camera.build_view_projection_matrix();
        let frustum = Frustum::from_view_proj(vp);
        let eye = camera.eye();
        let target = camera.target;

        // Unit cube at the camera target: center of view, well inside near/far.
        let at_target = Aabb { min: target - Vec3::splat(0.5), max: target + Vec3::splat(0.5) };
        assert!(frustum.intersects(&at_target), "cube at target must be visible");

        // 30 units behind the eye, continuing away from the target: behind the near plane.
        let eye_dir = (eye - target).normalize();
        let behind_eye = eye + eye_dir * 30.0;
        let behind = Aabb { min: behind_eye - Vec3::splat(0.5), max: behind_eye + Vec3::splat(0.5) };
        assert!(!frustum.intersects(&behind), "point behind the eye must be culled");

        // 500 units past the target along the view direction: beyond zfar=400.
        let forward = (target - eye).normalize();
        let past_far = target + forward * 500.0;
        let far = Aabb { min: past_far - Vec3::splat(0.5), max: past_far + Vec3::splat(0.5) };
        assert!(!frustum.intersects(&far), "point past zfar must be culled");

        // A box centered exactly on the right clip plane at mid-depth: straddles it.
        let inv_vp = vp.inverse();
        let boundary = inv_vp.project_point3(Vec3::new(1.0, 0.0, 0.5));
        let straddling = Aabb { min: boundary - Vec3::splat(0.5), max: boundary + Vec3::splat(0.5) };
        assert!(frustum.intersects(&straddling), "box straddling a side plane must be accepted");
    }

    #[test]
    fn ortho_frustum_from_shadow_fit_classifies_expected_points() {
        let vp = crate::shadow::fit_light_vp(Vec3::ZERO, Vec3::new(-1.0, 2.0, -1.0));
        let frustum = Frustum::from_view_proj(vp);

        let at_origin = Aabb { min: Vec3::splat(-1.0), max: Vec3::splat(1.0) };
        assert!(frustum.intersects(&at_origin), "cube at origin must be inside the fitted shadow volume");

        let far = Vec3::new(500.0, 0.0, 0.0);
        let outside = Aabb { min: far - Vec3::splat(80.0), max: far + Vec3::splat(80.0) };
        assert!(!frustum.intersects(&outside), "cube far outside the fitted shadow volume must be culled");
    }

    #[test]
    fn transformed_aabb_matches_translate_rotate_scale() {
        let local = Aabb { min: Vec3::splat(-0.5), max: Vec3::splat(0.5) };
        let m = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 3.0),
            glam::Quat::from_rotation_y(90f32.to_radians()),
            Vec3::new(5.0, 0.0, 0.0),
        );
        let world = local.transformed(&m);
        assert!(world.min.abs_diff_eq(Vec3::new(3.5, -0.5, -1.0), 1e-4), "min: {:?}", world.min);
        assert!(world.max.abs_diff_eq(Vec3::new(6.5, 0.5, 1.0), 1e-4), "max: {:?}", world.max);
    }

    #[test]
    fn classify_covers_both_camonly_shadowonly_and_none() {
        // Two ortho frustums (no view transform, so world-axis-aligned). The
        // RH forward=-Z convention means orthographic_rh(.., near, far)'s
        // valid z range is [-far,-near]: camera covers [-10,0], shadow
        // covers [-30,-5], overlapping in [-10,-5] — lets one AABB land in
        // each region.
        let camera = Frustum::from_view_proj(Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.0, 10.0));
        let shadow = Frustum::from_view_proj(Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 5.0, 30.0));

        let both = Aabb { min: Vec3::new(-0.1, -0.1, -7.5), max: Vec3::new(0.1, 0.1, -6.5) };
        assert_eq!(classify(&both, &camera, &shadow), Some(Visibility::Both));

        let cam_only = Aabb { min: Vec3::new(-0.1, -0.1, -2.5), max: Vec3::new(0.1, 0.1, -1.5) };
        assert_eq!(classify(&cam_only, &camera, &shadow), Some(Visibility::CamOnly));

        let shadow_only = Aabb { min: Vec3::new(-0.1, -0.1, -20.5), max: Vec3::new(0.1, 0.1, -19.5) };
        assert_eq!(classify(&shadow_only, &camera, &shadow), Some(Visibility::ShadowOnly));

        let none = Aabb { min: Vec3::new(4.9, 4.9, 4.9), max: Vec3::new(5.1, 5.1, 5.1) };
        assert_eq!(classify(&none, &camera, &shadow), None);
    }
}
