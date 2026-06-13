use glam::Vec3;
use parry3d::bounding_volume::{Aabb as ParryAabb, BoundingVolume};
use parry3d::math::Vec3 as PVec3;

// Thin wrapper over parry3d's Aabb. Keeps the center+half_extents API used by game code
// while delegating the actual intersection test to parry3d. parry3d types (Point, nalgebra)
// stay confined to this file and never leak into engine-core or game.
pub struct Aabb(ParryAabb);

impl Aabb {
    pub fn new(center: Vec3, half_extents: Vec3) -> Self {
        // parry3d 0.26 uses its own glam re-export (0.30.x), our workspace uses glam 0.32.
        // Bridge by constructing from raw f32 components.
        let mins = PVec3::new(center.x - half_extents.x, center.y - half_extents.y, center.z - half_extents.z);
        let maxs = PVec3::new(center.x + half_extents.x, center.y + half_extents.y, center.z + half_extents.z);
        Self(ParryAabb::new(mins, maxs))
    }

    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.0.intersects(&other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_boxes() {
        let a = Aabb::new(Vec3::ZERO,                Vec3::splat(0.5));
        let b = Aabb::new(Vec3::new(0.5, 0.0, 0.0), Vec3::splat(0.5));
        assert!(a.overlaps(&b));
    }

    #[test]
    fn separated_boxes() {
        let a = Aabb::new(Vec3::ZERO,                Vec3::splat(0.5));
        let b = Aabb::new(Vec3::new(5.0, 0.0, 0.0), Vec3::splat(0.5));
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn touching_edge_overlapping() {
        // parry3d uses <= for intersection — touching faces count as overlapping
        let a = Aabb::new(Vec3::ZERO,                Vec3::splat(0.5));
        let b = Aabb::new(Vec3::new(1.0, 0.0, 0.0), Vec3::splat(0.5));
        assert!(a.overlaps(&b));
    }
}
