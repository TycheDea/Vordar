// MovementSystem — integrates Velocity into Transform each fixed tick, then
// clamps XZ inside the playable radius.

use engine_app::scheduler::System;
use engine_core::components::{Transform, Velocity};
use engine_core::traits::Resources;
use engine_core::World;
use glam::{Vec2, Vec3};

/// The playable world radius, as a resource so a future zone with a
/// different shape is content, not a code change (ZoneDef may later carry
/// it). The zone ground mesh is flat (walkable) only inside r < 70 and rolls
/// into visual-only hills beyond it while the sim stays on the flat y = 0
/// plane (client ground.rs, FLAT_RADIUS) — anything that wanders past the
/// flat area renders half-buried in scenery. Clamping keeps every mover on
/// ground that actually exists; the default (65.0) matches every zone
/// shipped today.
#[derive(Clone, Copy)]
pub struct PlayRadius(pub f32);

impl Default for PlayRadius {
    fn default() -> Self {
        PlayRadius(65.0)
    }
}

/// One tick of movement: integrate `velocity` over `dt` from `pos`, then
/// clamp XZ inside `bound`. MovementSystem, the client's reconciliation
/// replay, and the server's mechanic rewind (its inverse, negating velocity)
/// all share this step so the three never disagree at the world boundary
/// (DESIGN.md §6 determinism).
pub fn step(pos: Vec3, velocity: Vec3, dt: f32, bound: f32) -> Vec3 {
    let mut next = pos + velocity * dt;
    let r = Vec2::new(next.x, next.z);
    if r.length_squared() > bound * bound {
        let clamped = r.normalize() * bound;
        next.x = clamped.x;
        next.z = clamped.y;
    }
    next
}

pub struct MovementSystem;

impl System for MovementSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let bound = resources.get::<PlayRadius>().copied().unwrap_or_default().0;
        for (transform, velocity) in world.query::<(&mut Transform, &Velocity)>().iter() {
            transform.position = step(transform.position, velocity.linear, delta, bound);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_inside_the_play_radius_is_untouched() {
        let mut world = World::new();
        let mut resources = Resources::new();
        let e = world.spawn((
            Transform { position: Vec3::new(10.0, 0.0, -4.0), ..Default::default() },
            Velocity { linear: Vec3::new(2.0, 0.0, 0.0) },
        ));
        MovementSystem.run(&mut world, &mut resources, 0.5);
        let p = world.get::<&Transform>(e).unwrap().position;
        assert!(p.abs_diff_eq(Vec3::new(11.0, 0.0, -4.0), 1e-6));
    }

    /// A mover past the flat ground radius (e.g. a stale persisted position
    /// out in the scenic hills) is pulled back to the boundary on the first
    /// tick — the "logs in half-buried in a hill" fix.
    #[test]
    fn position_beyond_the_play_radius_is_clamped_to_the_boundary() {
        let mut world = World::new();
        let mut resources = Resources::new();
        let e = world.spawn((
            Transform { position: Vec3::new(-8.3, 0.0, 105.0), ..Default::default() },
            Velocity { linear: Vec3::ZERO },
        ));
        MovementSystem.run(&mut world, &mut resources, 1.0 / 60.0);
        let p = world.get::<&Transform>(e).unwrap().position;
        let r = glam::Vec2::new(p.x, p.z).length();
        assert!((r - PlayRadius::default().0).abs() < 1e-4, "clamped to the boundary, r = {r}");
        // Direction preserved (pulled straight back, no teleport sideways).
        let dir = glam::Vec2::new(p.x, p.z).normalize();
        let want = glam::Vec2::new(-8.3, 105.0).normalize();
        assert!(dir.abs_diff_eq(want, 1e-5));
        assert_eq!(p.y, 0.0);
    }
}
