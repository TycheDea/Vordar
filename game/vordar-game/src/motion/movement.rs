// MovementSystem — integrates Velocity into Transform each fixed tick, then
// clamps XZ inside the playable radius.

use engine_app::scheduler::System;
use engine_core::components::{Transform, Velocity};
use engine_core::traits::Resources;
use engine_core::World;

/// The playable world radius. The zone ground mesh is flat (walkable) only
/// inside r < 70 and rolls into visual-only hills beyond it while the sim
/// stays on the flat y = 0 plane (client ground.rs, FLAT_RADIUS) — anything
/// that wanders past the flat area renders half-buried in scenery. Clamping
/// here keeps every mover on ground that actually exists; runs in the shared
/// sim so server and client prediction agree.
pub const PLAY_RADIUS: f32 = 65.0;

pub struct MovementSystem;

impl System for MovementSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, delta: f32) {
        for (transform, velocity) in world.query::<(&mut Transform, &Velocity)>().iter() {
            transform.position += velocity.linear * delta;
            let r = glam::Vec2::new(transform.position.x, transform.position.z);
            if r.length_squared() > PLAY_RADIUS * PLAY_RADIUS {
                let clamped = r.normalize() * PLAY_RADIUS;
                transform.position.x = clamped.x;
                transform.position.z = clamped.y;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

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
        assert!((r - PLAY_RADIUS).abs() < 1e-4, "clamped to the boundary, r = {r}");
        // Direction preserved (pulled straight back, no teleport sideways).
        let dir = glam::Vec2::new(p.x, p.z).normalize();
        let want = glam::Vec2::new(-8.3, 105.0).normalize();
        assert!(dir.abs_diff_eq(want, 1e-5));
        assert_eq!(p.y, 0.0);
    }
}
