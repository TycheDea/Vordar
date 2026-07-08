// Procedural pose animation — the smallest thing that makes composed bodies
// feel alive: a sine "breathing" bob on the torso (base shape 0) and an
// out-and-back swing of the class's weapon shape on a cast. Client-only
// cosmetics at Phase::RenderSync (real frame delta), driven by data authored
// per class (PoseParams in content/classes/*.ron). Not a skeletal or
// keyframe system — sub-shape offsets/rotations nudged by pure functions.

use engine_app::scheduler::System;
use engine_core::components::ShapeGroup;
use engine_core::traits::Resources;
use engine_core::World;
use glam::{Quat, Vec3};
use hecs::Entity;

/// Seconds an out-and-back cast swing takes.
const SWING_SECS: f32 = 0.25;
/// Peak swing tilt (radians around local X), scaled by the envelope.
const SWING_TILT: f32 = 0.9;

/// Runtime pose state, inserted by BodyComposeSystem next to the composed
/// ShapeGroup. Never authored in RON.
pub struct PoseRig {
    pub bob_amplitude: f32,
    pub bob_speed: f32,
    /// Absolute ShapeGroup index of the cast-swing shape.
    pub swing_index: Option<usize>,
    /// Peak offset delta of the swing (entity-local).
    pub swing_arc: Vec3,
    /// Torso rest height (restored around the bob each frame).
    pub torso_rest_y: f32,
    /// Swing shape's rest offset/rotation (restored when the swing ends).
    pub swing_rest: Vec3,
    pub swing_rest_rotation: Quat,
    /// Sine phase, radians.
    pub phase: f32,
    /// Progress of an active swing, 0..1; None = at rest.
    pub swing_t: Option<f32>,
}

/// Start the cast swing on `entity`'s rig, if it has one. Called by the cast
/// systems when the local player commits a cast (own player only — remote
/// casts don't replicate a caster today).
pub fn trigger_swing(world: &World, entity: Entity) {
    if let Ok(mut rig) = world.get::<&mut PoseRig>(entity) {
        rig.swing_t = Some(0.0);
    }
}

/// Out-and-back envelope: 0 → 1 at halfway → 0.
pub fn swing_envelope(t: f32) -> f32 {
    (1.0 - (2.0 * t.clamp(0.0, 1.0) - 1.0).abs()).max(0.0)
}

pub struct PoseAnimationSystem;

impl System for PoseAnimationSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, delta: f32) {
        for (_, (rig, group)) in world.query::<(Entity, (&mut PoseRig, &mut ShapeGroup))>().iter() {
            // Idle breathing on the torso.
            if rig.bob_amplitude > 0.0 {
                rig.phase += delta * rig.bob_speed;
                if rig.phase > std::f32::consts::TAU {
                    rig.phase -= std::f32::consts::TAU;
                }
                if let Some(torso) = group.shapes.get_mut(0) {
                    torso.offset.y = rig.torso_rest_y + rig.bob_amplitude * rig.phase.sin();
                }
            }
            // Cast swing on the weapon shape.
            let Some(index) = rig.swing_index else { continue };
            let Some(t) = rig.swing_t else { continue };
            let Some(shape) = group.shapes.get_mut(index) else { continue };
            let t = t + delta / SWING_SECS;
            if t >= 1.0 {
                rig.swing_t = None;
                shape.offset = rig.swing_rest;
                shape.rotation = rig.swing_rest_rotation;
            } else {
                rig.swing_t = Some(t);
                let k = swing_envelope(t);
                shape.offset = rig.swing_rest + rig.swing_arc * k;
                shape.rotation = rig.swing_rest_rotation * Quat::from_rotation_x(-SWING_TILT * k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_is_out_and_back() {
        assert_eq!(swing_envelope(0.0), 0.0);
        assert_eq!(swing_envelope(0.5), 1.0);
        assert_eq!(swing_envelope(1.0), 0.0);
        assert!(swing_envelope(0.25) > 0.0 && swing_envelope(0.25) < 1.0);
    }

    #[test]
    fn bob_moves_torso_and_swing_restores_rest() {
        let mut world = World::new();
        let mut resources = Resources::new();
        let rest = Vec3::new(0.4, 0.1, 0.0);
        let shapes = vec![
            engine_core::components::SubShape {
                shape: Default::default(),
                offset: Vec3::new(0.0, 0.05, 0.0),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                color: Vec3::ONE,
            },
            engine_core::components::SubShape {
                shape: Default::default(),
                offset: rest,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
                color: Vec3::ONE,
            },
        ];
        let entity = world.spawn((
            ShapeGroup { shapes },
            PoseRig {
                bob_amplitude: 0.05,
                bob_speed: 2.0,
                swing_index: Some(1),
                swing_arc: Vec3::new(0.1, 0.3, 0.4),
                torso_rest_y: 0.05,
                swing_rest: rest,
                swing_rest_rotation: Quat::IDENTITY,
                phase: 0.0,
                swing_t: None,
            },
        ));

        let mut sys = PoseAnimationSystem;
        sys.run(&mut world, &mut resources, 0.1);
        let torso_y = world.get::<&ShapeGroup>(entity).unwrap().shapes[0].offset.y;
        assert!((torso_y - 0.05).abs() > 1e-4, "torso must breathe");

        trigger_swing(&world, entity);
        sys.run(&mut world, &mut resources, 0.1); // mid-swing
        let mid = world.get::<&ShapeGroup>(entity).unwrap().shapes[1].offset;
        assert!((mid - rest).length() > 1e-4, "weapon must move during the swing");

        sys.run(&mut world, &mut resources, 1.0); // completes the swing
        let done = world.get::<&ShapeGroup>(entity).unwrap();
        assert_eq!(done.shapes[1].offset, rest, "swing restores the rest offset");
        assert!(world.get::<&PoseRig>(entity).unwrap().swing_t.is_none());
    }
}
