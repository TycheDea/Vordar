// NarrowphaseSystem — shape-accurate overlap tests on broadphase candidates.
//
// Reads CandidatePairs (from BroadphaseSystem), tests each pair with the
// appropriate shape test, then diffs against the previous frame's ActivePairs:
//   - New overlap  → emit CollisionStarted
//   - Lost overlap → emit CollisionEnded
//
// Shape dispatch:
//   Aabb   vs Aabb   — parry3d Aabb wrapper
//   Sphere vs Sphere — distance² < (r_a + r_b)²
//   Mixed  pair      — treat sphere as its bounding AABB (conservative)

use crate::aabb::Aabb;
use crate::broadphase::CandidatePairs;
use engine_app::events::{CollisionEnded, CollisionStarted, EventBus};
use engine_app::scheduler::System;
use engine_core::components::{CollisionShape, Hitbox, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;
use hecs::Entity;
use std::collections::HashSet;

/// Entity pairs that overlapped last frame. Persists across frames to detect
/// transition events (started / ended).
pub struct ActivePairs(pub HashSet<(Entity, Entity)>);

impl ActivePairs {
    pub fn new() -> Self { Self(HashSet::new()) }
}

pub struct NarrowphaseSystem {
    overlapping_buf: HashSet<(Entity, Entity)>,
}

impl NarrowphaseSystem {
    pub fn new() -> Self { Self { overlapping_buf: HashSet::new() } }
}

impl System for NarrowphaseSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // Take the vec to avoid cloning; restore it at the end so broadphase reuses capacity.
        let candidates = std::mem::take(
            &mut resources.get_mut::<CandidatePairs>()
                .expect("CandidatePairs not in resources")
                .0
        );

        // Reuse scratch set — clear preserves heap allocation.
        self.overlapping_buf.clear();
        {
            // One query view for all pair lookups: a single borrow acquisition
            // instead of per-entity world.get calls (and no shape clones).
            let mut query = world.query::<(&Transform, &Hitbox)>();
            let view = query.view();
            for (a, b) in &candidates {
                let (Some((t_a, h_a)), Some((t_b, h_b))) = (view.get(*a), view.get(*b)) else {
                    continue;
                };
                if shapes_overlap(t_a.position, &h_a.shape, t_b.position, &h_b.shape) {
                    self.overlapping_buf.insert((*a, *b));
                }
            }
        }

        // Restore candidates vec (capacity intact) for broadphase to refill next frame.
        resources.get_mut::<CandidatePairs>()
            .expect("CandidatePairs not in resources")
            .0 = candidates;

        // Diff against previous frame — compute started/ended while active is borrowed.
        let started: Vec<(Entity, Entity)>;
        let ended:   Vec<(Entity, Entity)>;
        {
            let active = resources
                .get_mut::<ActivePairs>()
                .expect("ActivePairs not in resources");

            started = self.overlapping_buf.iter().filter(|p| !active.0.contains(p)).copied().collect();
            ended   = active.0.iter().filter(|p| !self.overlapping_buf.contains(p)).copied().collect();

            for pair in &ended   { active.0.remove(pair);  }
            for pair in &started { active.0.insert(*pair); }
        } // active borrow ends

        // Emit events.
        let bus = resources
            .get_mut::<EventBus>()
            .expect("EventBus not in resources");

        for (a, b) in started { bus.emit(CollisionStarted { a, b }); }
        for (a, b) in ended   { bus.emit(CollisionEnded   { a, b }); }
    }
}

fn shapes_overlap(
    pos_a: Vec3, shape_a: &CollisionShape,
    pos_b: Vec3, shape_b: &CollisionShape,
) -> bool {
    match (shape_a, shape_b) {
        (CollisionShape::Aabb   { half_extents: he_a },
         CollisionShape::Aabb   { half_extents: he_b }) => {
            Aabb::new(pos_a, *he_a).overlaps(&Aabb::new(pos_b, *he_b))
        }
        (CollisionShape::Sphere { radius: r_a },
         CollisionShape::Sphere { radius: r_b }) => {
            pos_a.distance_squared(pos_b) < (r_a + r_b) * (r_a + r_b)
        }
        (CollisionShape::Sphere { radius }, CollisionShape::Aabb { half_extents }) => {
            Aabb::new(pos_a, Vec3::splat(*radius)).overlaps(&Aabb::new(pos_b, *half_extents))
        }
        (CollisionShape::Aabb { half_extents }, CollisionShape::Sphere { radius }) => {
            Aabb::new(pos_a, *half_extents).overlaps(&Aabb::new(pos_b, Vec3::splat(*radius)))
        }
    }
}
