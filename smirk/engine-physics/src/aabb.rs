use glam::Vec3;

pub struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    pub fn new(center: Vec3, half_extents: Vec3) -> Self {
        Self { min: center - half_extents, max: center + half_extents }
    }

    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.min.x <= other.max.x && other.min.x <= self.max.x
            && self.min.y <= other.max.y && other.min.y <= self.max.y
            && self.min.z <= other.max.z && other.min.z <= self.max.z
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
        // <= preserved on each axis — touching faces count as overlapping
        let a = Aabb::new(Vec3::ZERO,                Vec3::splat(0.5));
        let b = Aabb::new(Vec3::new(1.0, 0.0, 0.0), Vec3::splat(0.5));
        assert!(a.overlaps(&b));
    }
}
