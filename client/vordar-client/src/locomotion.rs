// Locomotion + facing — the client-side glue that makes a skinned RenderMesh
// move like a professional animator authored it: a speed-driven idle/walk/run
// state machine (each transition crossfaded by the engine's AnimationPlayer),
// an attack/death one-shot layer, and eased turning toward the movement
// heading. All cosmetic and client-owned — the sim never reads Transform.rotation
// (see net.rs), so facing here changes nothing about gameplay.
//
// A character opts in by carrying `LocomotionClips` (which clip is which, +
// speed thresholds) and `AnimController` (runtime latch). The engine attaches
// the `AnimationPlayer` itself. Clip names that don't exist in the asset fall
// back to its first clip, so a bad mapping degrades to "always idle", never a
// crash.

use engine_app::scheduler::System;
use engine_core::components::{AnimationPlayer, Transform, Velocity};
use engine_core::traits::Resources;
use engine_core::World;
use glam::{Quat, Vec2};
use hecs::Entity;
use std::f32::consts::{PI, TAU};

/// Which clips this character uses for each locomotion state, and the speed
/// thresholds between them (world units/sec).
#[derive(Clone)]
pub struct LocomotionClips {
    pub idle:   String,
    pub walk:   String,
    pub run:    String,
    pub attack: String,
    pub death:  String,
    /// Hit-react flinch; empty when the rig lacks one.
    pub hit:    String,
    /// Move faster than this → at least walk.
    pub walk_speed: f32,
    /// Move faster than this → run.
    pub run_speed:  f32,
    /// How long the attack one-shot latches before locomotion resumes.
    pub attack_secs: f32,
    /// Radians added to the facing yaw, per asset — glTF models disagree on
    /// which axis is "forward". 0 assumes the −Z convention; π flips a model
    /// authored facing +Z.
    pub forward_offset: f32,
}

impl Default for LocomotionClips {
    fn default() -> Self {
        Self {
            idle:  "Idle".into(),
            walk:  "Walk".into(),
            run:   "Run".into(),
            attack:"Attack".into(),
            death: "Death".into(),
            hit:   String::new(),
            walk_speed: 0.1,
            run_speed:  4.0,
            attack_secs: 0.6,
            forward_offset: 0.0,
        }
    }
}

/// Runtime latch travelling with `LocomotionClips`. `oneshot` counts down an
/// in-progress attack; `dead` holds the death pose permanently.
#[derive(Clone, Default)]
pub struct AnimController {
    pub oneshot: f32,
    pub dead:    bool,
}

/// Estimated velocity for entities the local sim doesn't move (remote,
/// snapshot-lerped players) — derived from snapshot position deltas in
/// net.rs. Locomotion/facing fall back to it when the sim `Velocity` is
/// absent or zero, so remote characters animate too. Client-only, ≤ one
/// snapshot interval stale.
#[derive(Clone, Copy, Default)]
pub struct NetMotion {
    pub velocity: glam::Vec3,
}

/// The velocity locomotion/facing should animate from: the sim's when it is
/// actually moving the entity (local/predicted player), else the
/// snapshot-derived estimate.
fn effective_velocity(sim: Option<glam::Vec3>, net: Option<glam::Vec3>) -> glam::Vec3 {
    sim.filter(|v| v.length_squared() > 1e-6)
        .or(net)
        .unwrap_or(glam::Vec3::ZERO)
}

/// Crossfade durations (seconds) and turn rate (rad/s).
const LOCO_BLEND:   f32 = 0.18;
const ATTACK_BLEND: f32 = 0.08;
const DEATH_BLEND:  f32 = 0.20;
const TURN_RATE:    f32 = 12.0;
/// Below this speed the character keeps its current heading (no snap-to-zero).
const MOVE_EPS: f32 = 0.05;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocoState {
    Idle,
    Walk,
    Run,
}

/// Pure speed → state selection. Thresholds are inclusive at the boundary.
pub fn desired_state(speed: f32, walk_speed: f32, run_speed: f32) -> LocoState {
    if speed >= run_speed {
        LocoState::Run
    } else if speed >= walk_speed {
        LocoState::Walk
    } else {
        LocoState::Idle
    }
}

/// Yaw (radians about +Y) that points a glTF model's local forward (−Z) along
/// the XZ heading `(hx, hz)`. Consistent with `Quat::from_rotation_y`.
pub fn heading_yaw(hx: f32, hz: f32) -> f32 {
    // from_rotation_y(θ)·(0,0,−1) = (−sinθ, 0, −cosθ); solve for the heading.
    (-hx).atan2(-hz)
}

/// Step `current` yaw toward `target` by at most `max_step`, taking the short
/// way around. Never overshoots.
pub fn turn_toward_yaw(current: f32, target: f32, max_step: f32) -> f32 {
    let mut delta = (target - current).rem_euclid(TAU);
    if delta > PI {
        delta -= TAU;
    }
    current + delta.clamp(-max_step, max_step)
}

/// Begin the attack one-shot on `entity` (analogue of `pose::trigger_swing`).
/// No-op if the entity isn't an animated character or is already dead.
pub fn trigger_attack(world: &World, entity: Entity) {
    trigger_attack_clip(world, entity, None, None);
}

/// Begin an attack one-shot with a specific clip (per-ability animations).
/// `clip` = None falls back to the race's default attack clip; `secs` = None
/// uses the default latch. Unknown clip names degrade to the asset's first
/// clip (the engine's fallback), never a crash.
pub fn trigger_attack_clip(world: &World, entity: Entity, clip: Option<&str>, secs: Option<f32>) {
    let (attack, latch) = match world.get::<&LocomotionClips>(entity) {
        Ok(c) => (
            clip.filter(|s| !s.is_empty()).unwrap_or(&c.attack).to_owned(),
            secs.unwrap_or(c.attack_secs),
        ),
        Err(_) => return,
    };
    if let Ok(mut ctrl) = world.get::<&mut AnimController>(entity) {
        if ctrl.dead {
            return;
        }
        ctrl.oneshot = latch;
    } else {
        return;
    }
    if let Ok(mut player) = world.get::<&mut AnimationPlayer>(entity) {
        player.transition_to(&attack, false, ATTACK_BLEND);
    }
}

/// Latch the death pose on `entity` — plays the death clip once and holds the
/// last frame; locomotion never resumes.
pub fn trigger_death(world: &World, entity: Entity) {
    if let Ok(mut ctrl) = world.get::<&mut AnimController>(entity) {
        ctrl.dead = true;
    }
}

/// Turns each animated character to face its movement heading (eased slerp).
/// Runs before the mesh sync so the rotation is current the same frame.
pub struct FacingSystem;

impl System for FacingSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, delta: f32) {
        let max_step = TURN_RATE * delta;
        for (transform, clips, vel, net) in
            world.query::<(&mut Transform, &LocomotionClips, Option<&Velocity>, Option<&NetMotion>)>().iter()
        {
            let v = effective_velocity(vel.map(|v| v.linear), net.map(|n| n.velocity));
            let heading = Vec2::new(v.x, v.z);
            if heading.length() < MOVE_EPS {
                continue; // standing still: keep the current heading
            }
            let target = heading_yaw(heading.x, heading.y) + clips.forward_offset;
            let current = transform.rotation.to_euler(glam::EulerRot::YXZ).0;
            let yaw = turn_toward_yaw(current, target, max_step);
            transform.rotation = Quat::from_rotation_y(yaw);
        }
    }
}

/// Drives each character's AnimationPlayer from its state: dead → death;
/// mid-attack → hold the attack clip; else speed → idle/walk/run.
pub struct LocomotionSystem;

impl System for LocomotionSystem {
    fn run(&mut self, world: &mut World, _resources: &mut Resources, delta: f32) {
        for (_entity, clips, player, ctrl, vel, net) in world
            .query::<(Entity, &LocomotionClips, Option<&mut AnimationPlayer>, &mut AnimController, Option<&Velocity>, Option<&NetMotion>)>()
            .iter()
        {
            let Some(player) = player else { continue }; // engine attaches next frame

            if ctrl.dead {
                player.transition_to(&clips.death, false, DEATH_BLEND);
                continue;
            }
            if ctrl.oneshot > 0.0 {
                ctrl.oneshot -= delta; // attack in progress — don't override it
                continue;
            }

            let v = effective_velocity(vel.map(|v| v.linear), net.map(|n| n.velocity));
            let speed = Vec2::new(v.x, v.z).length();
            let clip = match desired_state(speed, clips.walk_speed, clips.run_speed) {
                LocoState::Idle => &clips.idle,
                LocoState::Walk => &clips.walk,
                LocoState::Run  => &clips.run,
            };
            player.transition_to(clip, true, LOCO_BLEND);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_thresholds() {
        assert_eq!(desired_state(0.0, 0.1, 4.0), LocoState::Idle);
        assert_eq!(desired_state(0.05, 0.1, 4.0), LocoState::Idle);
        assert_eq!(desired_state(1.0, 0.1, 4.0), LocoState::Walk);
        assert_eq!(desired_state(3.99, 0.1, 4.0), LocoState::Walk);
        assert_eq!(desired_state(4.0, 0.1, 4.0), LocoState::Run);
        assert_eq!(desired_state(9.0, 0.1, 4.0), LocoState::Run);
    }

    #[test]
    fn heading_yaw_points_local_forward() {
        // Rotating (0,0,-1) by the returned yaw must land on the heading.
        for (hx, hz) in [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.7, 0.7)] {
            let yaw = heading_yaw(hx, hz);
            let fwd = Quat::from_rotation_y(yaw) * glam::Vec3::NEG_Z;
            let want = glam::Vec3::new(hx, 0.0, hz).normalize();
            assert!(fwd.abs_diff_eq(want, 1e-5), "yaw {yaw} gave {fwd}, want {want}");
        }
    }

    #[test]
    fn turn_takes_short_way_and_clamps() {
        // Small target within reach: snaps exactly.
        assert!((turn_toward_yaw(0.0, 0.3, 1.0) - 0.3).abs() < 1e-6);
        // Far target: clamped to max_step.
        assert!((turn_toward_yaw(0.0, 3.0, 0.1) - 0.1).abs() < 1e-6);
        // Wrap-around: target just below 0 (= TAU-ε) turns negative, not +TAU.
        let out = turn_toward_yaw(0.0, TAU - 0.1, 1.0);
        assert!((out - (-0.1)).abs() < 1e-5, "got {out}");
    }

    /// DIAGNOSTIC: the two RenderSync systems on a moving character — does
    /// FacingSystem rotate it toward its heading, and does LocomotionSystem pick
    /// the run clip? Pure CPU, no GPU/attach timing.
    #[test]
    fn facing_and_locomotion_drive_a_moving_character() {
        use engine_core::components::{AnimationPlayer, Transform, Velocity};
        let mut world = World::new();
        let mut res = Resources::new();
        let clips = LocomotionClips {
            idle: "Idle".into(), walk: "Walking_A".into(), run: "Running_A".into(),
            attack: String::new(), death: String::new(), hit: String::new(),
            walk_speed: 0.1, run_speed: 3.0, attack_secs: 0.6, forward_offset: 0.0,
        };
        let e = world.spawn((
            Transform::default(),
            Velocity { linear: glam::Vec3::new(6.0, 0.0, 0.0) }, // running +X
            clips,
            AnimController::default(),
            AnimationPlayer::default(),
        ));

        let (mut facing, mut loco) = (FacingSystem, LocomotionSystem);
        for _ in 0..40 {
            facing.run(&mut world, &mut res, 0.016);
            loco.run(&mut world, &mut res, 0.016);
        }

        let fwd = world.get::<&Transform>(e).unwrap().rotation * glam::Vec3::NEG_Z;
        assert!(fwd.x > 0.9, "should face its +X heading, forward = {fwd}");
        assert_eq!(world.get::<&AnimationPlayer>(e).unwrap().clip, "Running_A", "6 u/s → run");
    }

    /// A remote (snapshot-lerped) character has no sim Velocity — NetMotion
    /// alone must reach the run clip and face the heading, or networked
    /// players glide around frozen in idle.
    #[test]
    fn net_motion_alone_drives_run_and_facing() {
        use engine_core::components::{AnimationPlayer, Transform};
        let mut world = World::new();
        let mut res = Resources::new();
        let clips = LocomotionClips {
            idle: "Idle".into(), walk: "Walking_A".into(), run: "Running_A".into(),
            walk_speed: 0.1, run_speed: 3.0, ..Default::default()
        };
        let e = world.spawn((
            Transform::default(),
            NetMotion { velocity: glam::Vec3::new(6.0, 0.0, 0.0) },
            clips,
            AnimController::default(),
            AnimationPlayer::default(),
        ));

        let (mut facing, mut loco) = (FacingSystem, LocomotionSystem);
        for _ in 0..40 {
            facing.run(&mut world, &mut res, 0.016);
            loco.run(&mut world, &mut res, 0.016);
        }

        let fwd = world.get::<&Transform>(e).unwrap().rotation * glam::Vec3::NEG_Z;
        assert!(fwd.x > 0.9, "should face its +X heading, forward = {fwd}");
        assert_eq!(world.get::<&AnimationPlayer>(e).unwrap().clip, "Running_A");
    }

    #[test]
    fn trigger_attack_clip_plays_named_clip_and_latches() {
        use engine_core::components::AnimationPlayer;
        let mut world = World::new();
        let e = world.spawn((
            LocomotionClips { attack: "Default_Attack".into(), ..Default::default() },
            AnimController::default(),
            AnimationPlayer::default(),
        ));

        trigger_attack_clip(&world, e, Some("Spellcast_Shoot"), Some(0.4));
        assert_eq!(world.get::<&AnimationPlayer>(e).unwrap().clip, "Spellcast_Shoot");
        assert!((world.get::<&AnimController>(e).unwrap().oneshot - 0.4).abs() < 1e-6);

        // None falls back to the race's default attack clip + default latch.
        world.get::<&mut AnimController>(e).unwrap().oneshot = 0.0;
        trigger_attack_clip(&world, e, None, None);
        assert_eq!(world.get::<&AnimationPlayer>(e).unwrap().clip, "Default_Attack");
        let clips_secs = world.get::<&LocomotionClips>(e).unwrap().attack_secs;
        assert!((world.get::<&AnimController>(e).unwrap().oneshot - clips_secs).abs() < 1e-6);
    }

    #[test]
    fn turn_never_overshoots_over_many_steps() {
        let target = 2.5;
        let mut yaw = 0.0;
        for _ in 0..100 {
            yaw = turn_toward_yaw(yaw, target, 0.1);
        }
        assert!((yaw - target).abs() < 1e-4, "converged to {yaw}");
    }
}
