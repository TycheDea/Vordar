// SeparationSystem — pushes overlapping Solid entities apart each frame.
//
// Uses the minimum translation vector (MTV): the shortest axis along which the
// two shapes can be separated. Each entity receives half the correction so they
// meet at the contact surface rather than one being pushed through the other.
//
// All corrections are computed from a consistent position snapshot and applied
// in one pass afterwards. ActivePairs is a HashSet whose iteration order varies
// frame to frame — resolving pairs sequentially against live positions makes
// clustered entities ping-pong (A pushed out of B into C, then back), which
// reads as flicker. Damping + slop keep the remaining correction calm while
// AI steering re-penetrates every step.
//
// Runs Phase::CollisionResolve, First — before damage and death so positions
// are corrected before any downstream systems read them.

use engine_app::scheduler::System;
use engine_core::components::{Anchored, CollisionShape, Hitbox, Solid, Transform};
use engine_core::traits::Resources;
use engine_core::World;
use engine_physics::narrowphase::ActivePairs;
use glam::Vec3;
use hecs::Entity;
use std::collections::HashMap;

/// Fraction of the accumulated MTV applied per fixed step. Full correction is
/// stiff against movement that re-penetrates every step; 0.8 converges within
/// a couple of steps without oscillating.
const CORRECTION_PERCENT: f32 = 0.8;

/// Penetration depth tolerated without correction — sub-visible overlaps are
/// not worth re-resolving every step (that constant micro-push is jitter).
const SLOP: f32 = 0.01;

pub struct SeparationSystem;

impl System for SeparationSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        // resources and world are separate borrows — hold ActivePairs while calling world.get.
        let active = resources.get::<ActivePairs>().expect("ActivePairs not in resources");

        // Pass 1: accumulate corrections from a consistent snapshot (order-independent).
        // One query view for all pair lookups — a single borrow acquisition
        // instead of six world.get calls (and two shape clones) per pair.
        let mut corrections: HashMap<Entity, Vec3> = HashMap::new();
        {
            let mut query =
                world.query::<(&Transform, &Hitbox, hecs::Satisfies<&Solid>, hecs::Satisfies<&Anchored>)>();
            let view = query.view();
            for &(a, b) in &active.0 {
                let (Some((t_a, h_a, a_solid, a_anchored)), Some((t_b, h_b, b_solid, b_anchored))) =
                    (view.get(a), view.get(b))
                else {
                    continue;
                };
                // Both must be Solid to separate; two Anchored never move.
                if !a_solid || !b_solid || (a_anchored && b_anchored) { continue; }

                if let Some(correction) = mtv(&t_a.position, &h_a.shape, &t_b.position, &h_b.shape) {
                    // An Anchored side yields nothing — the other side takes
                    // the WHOLE separation (mtv returns each side's half).
                    if a_anchored {
                        *corrections.entry(b).or_insert(Vec3::ZERO) -= correction * 2.0;
                    } else if b_anchored {
                        *corrections.entry(a).or_insert(Vec3::ZERO) += correction * 2.0;
                    } else {
                        *corrections.entry(a).or_insert(Vec3::ZERO) += correction;
                        *corrections.entry(b).or_insert(Vec3::ZERO) -= correction;
                    }
                }
            }
        }

        // Pass 2: apply damped corrections once per entity.
        for (entity, correction) in corrections {
            if let Ok(mut t) = world.get::<&mut Transform>(entity) {
                t.position += correction * CORRECTION_PERCENT;
            }
        }
    }
}

/// Returns the vector to add to A (and subtract from B) to separate them.
/// Returns None if the shapes are not actually overlapping.
fn mtv(pa: &Vec3, sa: &CollisionShape, pb: &Vec3, sb: &CollisionShape) -> Option<Vec3> {
    match (sa, sb) {
        (CollisionShape::Aabb { half_extents: ha }, CollisionShape::Aabb { half_extents: hb }) => {
            let overlap_x = (ha.x + hb.x) - (pa.x - pb.x).abs() - SLOP;
            let overlap_z = (ha.z + hb.z) - (pa.z - pb.z).abs() - SLOP;

            if overlap_x <= 0.0 || overlap_z <= 0.0 { return None; }

            // Push apart on the axis of least penetration.
            let half = if overlap_x < overlap_z {
                let sign = if pa.x >= pb.x { 1.0 } else { -1.0 };
                Vec3::new(sign * overlap_x * 0.5, 0.0, 0.0)
            } else {
                let sign = if pa.z >= pb.z { 1.0 } else { -1.0 };
                Vec3::new(0.0, 0.0, sign * overlap_z * 0.5)
            };
            Some(half)
        }
        (CollisionShape::Sphere { radius: ra }, CollisionShape::Sphere { radius: rb }) => {
            let diff  = *pa - *pb;
            let dist  = diff.length();
            let overlap = ra + rb - dist - SLOP;
            if overlap <= 0.0 { return None; }
            let normal = if dist > 1e-4 { diff / dist } else { Vec3::X };
            Some(normal * overlap * 0.5)
        }
        // Mixed: treat sphere as bounding AABB for separation.
        (CollisionShape::Sphere { radius }, CollisionShape::Aabb { half_extents }) => {
            let ha = Vec3::splat(*radius);
            mtv(pa, &CollisionShape::Aabb { half_extents: ha }, pb, &CollisionShape::Aabb { half_extents: *half_extents })
        }
        (CollisionShape::Aabb { half_extents }, CollisionShape::Sphere { radius }) => {
            let hb = Vec3::splat(*radius);
            mtv(pa, &CollisionShape::Aabb { half_extents: *half_extents }, pb, &CollisionShape::Aabb { half_extents: hb })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_physics::narrowphase::ActivePairs;

    fn solid(world: &mut World, x: f32) -> Entity {
        world.spawn((
            Transform::new(Vec3::new(x, 0.0, 0.0)),
            Hitbox { shape: CollisionShape::Aabb { half_extents: Vec3::splat(0.5) } },
            Solid,
        ))
    }

    fn run_pair(world: &mut World, a: Entity, b: Entity) {
        let mut resources = Resources::new();
        let mut pairs = ActivePairs::new();
        pairs.0.insert((a, b));
        resources.insert(pairs);
        SeparationSystem.run(world, &mut resources, 1.0 / 60.0);
    }

    #[test]
    fn anchored_side_never_yields() {
        let mut world = World::new();
        let wall = solid(&mut world, 0.0);
        world.insert_one(wall, Anchored).unwrap();
        let walker = solid(&mut world, 0.8); // 0.2 overlap on x

        run_pair(&mut world, wall, walker);
        assert_eq!(world.get::<&Transform>(wall).unwrap().position.x, 0.0, "the wall must not move");
        let moved = world.get::<&Transform>(walker).unwrap().position.x;
        assert!(moved > 0.8, "the walker takes the whole correction, got {moved}");
    }

    #[test]
    fn two_anchored_never_push_each_other() {
        let mut world = World::new();
        let a = solid(&mut world, 0.0);
        let b = solid(&mut world, 0.6);
        world.insert_one(a, Anchored).unwrap();
        world.insert_one(b, Anchored).unwrap();

        run_pair(&mut world, a, b);
        assert_eq!(world.get::<&Transform>(a).unwrap().position.x, 0.0);
        assert_eq!(world.get::<&Transform>(b).unwrap().position.x, 0.6);
    }
}
