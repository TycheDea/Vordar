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
//   Mixed  pair      — exact closest-point-on-AABB sphere test

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

impl Default for ActivePairs {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivePairs {
    pub fn new() -> Self { Self(HashSet::new()) }
}

pub struct NarrowphaseSystem {
    overlapping_buf: HashSet<(Entity, Entity)>,
}

impl Default for NarrowphaseSystem {
    fn default() -> Self {
        Self::new()
    }
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
        let mut started: Vec<(Entity, Entity)>;
        let mut ended:   Vec<(Entity, Entity)>;
        {
            let active = resources
                .get_mut::<ActivePairs>()
                .expect("ActivePairs not in resources");

            started = self.overlapping_buf.iter().filter(|p| !active.0.contains(p)).copied().collect();
            ended   = active.0.iter().filter(|p| !self.overlapping_buf.contains(p)).copied().collect();

            for pair in &ended   { active.0.remove(pair);  }
            for pair in &started { active.0.insert(*pair); }
        } // active borrow ends

        // HashSet iteration order is run-varying; sort by canonical pair id so
        // event order (and anything resolving "first contact wins" from it) is
        // deterministic run to run.
        started.sort_unstable();
        ended.sort_unstable();

        // Emit events.
        let bus = resources
            .get_mut::<EventBus>()
            .expect("EventBus not in resources");

        for (a, b) in started { bus.emit(CollisionStarted { a, b }); }
        for (a, b) in ended   { bus.emit(CollisionEnded   { a, b }); }
    }
}

/// The `ActivePairs` membership test: any code outside this module that
/// needs to know whether a pair would be considered in contact (e.g.
/// vordar-game's `anchored_push`, predicting a static collision without
/// running the full physics pipeline) must gate through this function so
/// prediction and the live narrowphase never disagree on what counts as a
/// contact.
pub fn shapes_overlap(
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
            sphere_aabb_overlap(pos_a, *radius, pos_b, *half_extents)
        }
        (CollisionShape::Aabb { half_extents }, CollisionShape::Sphere { radius }) => {
            sphere_aabb_overlap(pos_b, *radius, pos_a, *half_extents)
        }
    }
}

/// Exact sphere-vs-AABB test: clamp the sphere center into the box to get the
/// closest point, then compare squared distance to the radius. The bounding-box
/// approximation this replaces reports false positives on diagonal near-misses
/// (corner region up to sqrt(2)x the true radius).
fn sphere_aabb_overlap(sphere_pos: Vec3, radius: f32, aabb_pos: Vec3, half_extents: Vec3) -> bool {
    let closest = Vec3::new(
        (sphere_pos.x - aabb_pos.x).clamp(-half_extents.x, half_extents.x) + aabb_pos.x,
        (sphere_pos.y - aabb_pos.y).clamp(-half_extents.y, half_extents.y) + aabb_pos.y,
        (sphere_pos.z - aabb_pos.z).clamp(-half_extents.z, half_extents.z) + aabb_pos.z,
    );
    sphere_pos.distance_squared(closest) < radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    // Three mutually overlapping entities spawned in ascending Entity-id order:
    // center < victim_a < victim_b. Candidate pairs are fed in reverse order to
    // rule out "it happened to come out sorted" as an explanation for a pass.
    fn overlapping_world() -> (World, Entity, Entity, Entity) {
        let mut world = World::new();
        let center = world.spawn((
            Transform { position: Vec3::ZERO, ..Default::default() },
            Hitbox { shape: CollisionShape::Sphere { radius: 1.0 } },
        ));
        let victim_a = world.spawn((
            Transform { position: Vec3::new(0.2, 0.0, 0.0), ..Default::default() },
            Hitbox { shape: CollisionShape::Sphere { radius: 1.0 } },
        ));
        let victim_b = world.spawn((
            Transform { position: Vec3::new(-0.2, 0.0, 0.0), ..Default::default() },
            Hitbox { shape: CollisionShape::Sphere { radius: 1.0 } },
        ));
        (world, center, victim_a, victim_b)
    }

    // Two victims start overlapping the same entity on the same tick. The
    // emitted CollisionStarted order must be the canonical (sorted) pair
    // order on every run — not whatever order a fresh HashSet's hasher seed
    // happens to iterate in. Repeated with fresh System/ActivePairs instances
    // (each gets a different SipHash seed) so a flaky pre-fix ordering can't
    // hide behind one lucky seed.
    // A sphere and an AABB placed on a shared diagonal where the true
    // closest-point distance exceeds the radius (no overlap) but the
    // conservative bounding-box test (sphere treated as its own AABB) would
    // report overlap on both axes independently. Guards against regressing
    // to the box approximation.
    #[test]
    fn sphere_aabb_diagonal_near_miss_does_not_collide() {
        let sphere = CollisionShape::Sphere { radius: 1.0 };
        let cube = CollisionShape::Aabb { half_extents: Vec3::splat(0.5) };
        let sphere_pos = Vec3::new(1.3, 0.0, 1.3);
        let cube_pos = Vec3::ZERO;

        assert!(!shapes_overlap(sphere_pos, &sphere, cube_pos, &cube));
        assert!(!shapes_overlap(cube_pos, &cube, sphere_pos, &sphere));
    }

    #[test]
    fn collision_started_order_is_canonical_every_run() {
        for _ in 0..20 {
            let (mut world, center, victim_a, victim_b) = overlapping_world();
            let pair_a = (center, victim_a);
            let pair_b = (center, victim_b);
            let mut expected = [pair_a, pair_b];
            expected.sort();

            let mut resources = Resources::new();
            resources.insert(CandidatePairs(vec![pair_b, pair_a]));
            resources.insert(ActivePairs::new());
            resources.insert(EventBus::new());

            let mut system = NarrowphaseSystem::new();
            system.run(&mut world, &mut resources, 1.0 / 60.0);

            let bus = resources.get::<EventBus>().expect("EventBus not in resources");
            let started: Vec<(Entity, Entity)> =
                bus.read::<CollisionStarted>().map(|e| (e.a, e.b)).collect();
            assert_eq!(started, expected.to_vec());
        }
    }
}
