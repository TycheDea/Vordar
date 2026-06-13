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
use engine_core::components::{CollisionShape, Hitbox, Solid, Transform};
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
        let mut corrections: HashMap<Entity, Vec3> = HashMap::new();
        for &(a, b) in &active.0 {
            // Both must be Solid to separate.
            let a_solid = world.get::<&Solid>(a).is_ok();
            let b_solid = world.get::<&Solid>(b).is_ok();
            if !a_solid || !b_solid { continue; }

            let pos_a   = world.get::<&Transform>(a).ok().map(|t| t.position);
            let shape_a = world.get::<&Hitbox>(a).ok().map(|h| h.shape.clone());
            let pos_b   = world.get::<&Transform>(b).ok().map(|t| t.position);
            let shape_b = world.get::<&Hitbox>(b).ok().map(|h| h.shape.clone());

            if let (Some(pa), Some(sa), Some(pb), Some(sb)) = (pos_a, shape_a, pos_b, shape_b) {
                if let Some(correction) = mtv(&pa, &sa, &pb, &sb) {
                    *corrections.entry(a).or_insert(Vec3::ZERO) += correction;
                    *corrections.entry(b).or_insert(Vec3::ZERO) -= correction;
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
