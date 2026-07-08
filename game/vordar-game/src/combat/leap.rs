// Leap — the gap-closer's movement half. A LeapImpulse overrides the
// entity's velocity for `remaining` seconds; the arrival AOE is an ordinary
// Mechanic scheduled at the same instant (net_plugin's Leap cast arm derives
// both from the same cast time, so they complete together without either
// referencing the other).
//
// Deliberately clock-free (a seconds countdown, like Projectile.ttl and
// Enemy.cooldown_left): this system runs in the client's predicted/replayed
// path, where DESIGN.md §6 bans wall-clock reads.

use engine_app::scheduler::System;
use engine_core::components::Velocity;
use engine_core::traits::Resources;
use engine_core::World;
use glam::Vec3;
use hecs::Entity;

/// Overrides Velocity while active; removed when the countdown ends.
/// Code-inserted only (the cast handler / predicting client), never RON.
pub struct LeapImpulse {
    pub velocity: Vec3,
    /// Seconds of dash left.
    pub remaining: f32,
}

/// The velocity that departs `from` and arrives exactly at `to` after
/// `cast_secs` — shared by the server's cast handler and the predicting
/// client so both integrate the identical dash (DESIGN.md §6 determinism).
pub fn leap_velocity(from: Vec3, to: Vec3, cast_secs: f32) -> Vec3 {
    let mut delta = to - from;
    delta.y = 0.0;
    if cast_secs <= 0.0 {
        return Vec3::ZERO;
    }
    delta / cast_secs
}

/// Applies LeapImpulse over player input: runs between PlayerMovementSystem
/// (Update/First, sets velocity from intent) and MovementSystem (Update/Last,
/// integrates), so the dash wins for its duration without either of those
/// systems knowing leaps exist.
pub struct LeapSystem;

impl System for LeapSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, delta: f32) {
        let mut done: Vec<Entity> = Vec::new();
        for (entity, (velocity, leap)) in world.query::<(Entity, (&mut Velocity, &mut LeapImpulse))>().iter() {
            velocity.linear = leap.velocity;
            leap.remaining -= delta;
            if leap.remaining <= 0.0 {
                done.push(entity);
            }
        }
        for entity in done {
            let _ = world.remove_one::<LeapImpulse>(entity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    #[test]
    fn leap_velocity_arrives_exactly() {
        let from = Vec3::new(1.0, 0.0, 2.0);
        let to = Vec3::new(7.0, 0.0, -4.0);
        let cast_secs = 0.4; // 24 ticks at 60 Hz
        let v = leap_velocity(from, to, cast_secs);
        let arrived = from + v * cast_secs;
        assert!((arrived - to).length() < 1e-4, "must land on the target: {arrived:?}");
    }

    #[test]
    fn leap_velocity_ignores_height_and_zero_cast() {
        let v = leap_velocity(Vec3::ZERO, Vec3::new(0.0, 5.0, 3.0), 0.5);
        assert_eq!(v.y, 0.0, "dashes stay on the ground plane");
        assert_eq!(leap_velocity(Vec3::ZERO, Vec3::X, 0.0), Vec3::ZERO);
    }

    #[test]
    fn impulse_overrides_velocity_then_expires() {
        let mut world = World::new();
        let mut resources = Resources::new();
        let dash = Vec3::new(10.0, 0.0, 0.0);
        // 2.5 ticks: expires mid-third-run regardless of f32 rounding.
        let entity = world.spawn((
            Velocity { linear: Vec3::new(-1.0, 0.0, 0.0) },
            LeapImpulse { velocity: dash, remaining: 2.5 * DT },
        ));

        let mut sys = LeapSystem;
        for _ in 0..2 {
            assert!(world.get::<&LeapImpulse>(entity).is_ok(), "still dashing");
            sys.run(&mut world, &mut resources, DT);
            assert_eq!(world.get::<&Velocity>(entity).unwrap().linear, dash);
        }
        sys.run(&mut world, &mut resources, DT);
        assert_eq!(world.get::<&Velocity>(entity).unwrap().linear, dash, "the expiry tick still dashes");
        assert!(world.get::<&LeapImpulse>(entity).is_err(), "impulse removed at zero");
    }
}
