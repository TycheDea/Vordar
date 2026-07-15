// Predict-and-reconcile: send our movement intent and emit it locally each
// Input tick so the shared simulation moves immediately; each snapshot
// rebases our own player onto the server's authoritative position and
// replays every intent the server hasn't processed yet. Trust/smooth/snap
// bands decide whether the reconciliation error is ignored, blended out
// over time, or snapped outright. NetSendInputSystem runs in Phase::Input
// (after NetReceiveSystem); NetCorrectionSystem runs in Phase::Update,
// SystemOrder::Last.

use super::*;

/// Reconciliation error below this is ignored — local prediction is trusted
/// outright. Tick-phase jitter between client and server lives in this band
/// (~1–2 sim ticks of movement); correcting it every snapshot reads as shaking.
const TRUST_DISTANCE: f32 = 0.3;
/// Mispredictions larger than this snap to the reconciled position (forced
/// corrections, teleports); between TRUST and SNAP the error is folded into
/// the predicted position gradually by NetCorrectionSystem.
pub(super) const SNAP_DISTANCE: f32 = 1.0;
/// Half-life of an outstanding correction — time until half the error has
/// been folded in. Short enough to converge in a few hundred ms, long enough
/// that the per-tick nudge stays below normal movement speed.
const CORRECTION_HALF_LIFE: f32 = 0.15;
/// Safety bound on unacknowledged intents (~4 s at 60 Hz). Hitting it means
/// the server stopped acking; predicting further is pointless.
const MAX_PENDING_INTENTS: usize = 240;
/// Last-3 redundancy depth for `ClientMsg::MoveIntents`: this tick's entry
/// plus the two previous, sent via datagram every Input tick — a single lost
/// datagram is fully recovered by the next tick's batch.
pub(super) const MOVE_RING_LEN: usize = 3;

/// An intent sent to the server but not yet covered by `last_processed_seq`.
/// Replayed on top of each snapshot of our own player. `leap` mirrors a
/// LeapImpulse active on the entity when this tick's intent was recorded —
/// replay reproduces the dash's straight-line displacement instead of
/// dead-reckoning plain WASD movement through it.
pub(super) struct PendingIntent {
    pub(super) seq: u32,
    pub(super) dir: Vec2,
    pub(super) dt: f32,
    pub(super) leap: Option<Vec3>,
}

/// Rewind + replay: rebase our player onto the server's authoritative position
/// and re-apply every intent the server hasn't processed yet. The result is
/// where we SHOULD be — but movement stays optimistic: errors inside the trust
/// band are ignored, mid-size drift is handed to NetCorrectionSystem to blend
/// out, only real mispredictions (server-side corrections) snap.
pub(super) fn reconcile_own(
    world: &mut World,
    resources: &mut Resources,
    entity: Entity,
    server_pos: Vec3,
    last_processed_seq: u32,
) {
    let speed = world.get::<&Player>(entity).map(|p| p.speed).unwrap_or(0.0);
    let bound = resources.get::<PlayRadius>().copied().unwrap_or_default().0;
    let own_shape = world.get::<&Hitbox>(entity).map(|h| h.shape.clone()).ok();
    // Defensive only: the player prefab always carries a Hitbox. Without one
    // there's no shape to test against statics, so the replay falls back to
    // free-flight instead of pushing an unknown shape around.
    let statics: Vec<(Vec3, CollisionShape)> =
        if own_shape.is_some() { collect_solid_anchored_statics(world) } else { Vec::new() };
    let shape = own_shape.unwrap_or(CollisionShape::Aabb { half_extents: Vec3::ZERO });
    let (replayed, still_reconciling_a_dash) = {
        let state = resources.get_mut::<NetClientState>().unwrap();
        state.pending.retain(|p| p.seq > last_processed_seq);
        // Not just "is the local LeapImpulse still active": the server mirrors
        // the same cast only after its own one-way network delay, so its copy
        // of the dash finishes strictly later than the local one, and the
        // MoveIntent queue it drains at one-per-tick can lag further behind
        // still. Any unacked intent recorded during the dash means the
        // server hasn't caught up on the dash yet.
        let still_reconciling_a_dash = state.pending.iter().any(|p| p.leap.is_some());
        (replay_position(server_pos, speed, state.pending.iter(), bound, &shape, &statics), still_reconciling_a_dash)
    };
    // Collision is part of the replay (predict_step pushes out of statics
    // exactly as SeparationSystem does), so wall contact isn't a source of
    // mismatch here. What forces suppression during a dash is network
    // timing: the server's dash mirror finishes strictly later than the
    // local one, so corrections stay off until the server has caught up on
    // the whole dash.
    if still_reconciling_a_dash {
        return;
    }
    let Ok(mut transform) = world.get::<&mut Transform>(entity) else { return };
    let error = replayed - transform.position;
    let correction = match classify_error(error) {
        Correction::Trust => Vec3::ZERO,
        Correction::Smooth => {
            log::debug!("prediction drift: {:.3} units", error.length());
            error
        }
        Correction::Snap => {
            log::debug!("prediction snap: {:.2} units off", error.length());
            transform.position = replayed;
            Vec3::ZERO
        }
    };
    drop(transform);
    resources.get_mut::<NetClientState>().unwrap().correction = correction;
}

/// Position after replaying pending intents on top of the server's
/// authoritative position — the same movement + static-collision rule the
/// simulation runs (`vordar_game::motion::predict_step`), including a leap
/// override where one was active, the world-boundary clamp, and the push out
/// of anchored statics the server's SeparationSystem applies.
fn replay_position<'a>(
    server_pos: Vec3,
    speed: f32,
    pending: impl Iterator<Item = &'a PendingIntent>,
    bound: f32,
    shape: &CollisionShape,
    statics: &[(Vec3, CollisionShape)],
) -> Vec3 {
    pending.fold(server_pos, |pos, p| {
        let velocity = p.leap.unwrap_or_else(|| movement_velocity(p.dir, speed));
        predict_step(pos, velocity, p.dt, bound, shape, statics)
    })
}

/// Solid + Anchored statics in the world, as `(position, shape)` pairs —
/// what both `reconcile_own`'s replay and `PredictedStaticCollisionSystem`
/// push the own player out of.
fn collect_solid_anchored_statics(world: &World) -> Vec<(Vec3, CollisionShape)> {
    world
        .query::<(&Transform, &Hitbox, hecs::Satisfies<&Solid>, hecs::Satisfies<&Anchored>)>()
        .iter()
        .filter_map(|(t, h, solid, anchored)| (solid && anchored).then(|| (t.position, h.shape.clone())))
        .collect()
}

/// What to do about a reconciliation error.
#[derive(Debug, PartialEq)]
enum Correction {
    /// Inside the trust band — keep the predicted position untouched.
    Trust,
    /// Real drift — blend it out over time.
    Smooth,
    /// Way off — hard snap.
    Snap,
}

fn classify_error(error: Vec3) -> Correction {
    let d2 = error.length_squared();
    if d2 < TRUST_DISTANCE * TRUST_DISTANCE {
        Correction::Trust
    } else if d2 > SNAP_DISTANCE * SNAP_DISTANCE {
        Correction::Snap
    } else {
        Correction::Smooth
    }
}

/// Portion of the outstanding correction to apply after `dt` (exponential decay).
fn correction_step(correction: Vec3, dt: f32) -> Vec3 {
    correction * (1.0 - (-(std::f32::consts::LN_2 / CORRECTION_HALF_LIFE) * dt).exp())
}

/// Folds the outstanding reconciliation error into the predicted position a
/// little each fixed Update tick. Corrections applied here are rendered as
/// interpolated motion like any other movement; applying them where they are
/// detected (Phase::Input) pops instead, because SaveTransformSystem captures
/// PreviousTransform afterward and the offset is never interpolated.
pub(super) struct NetCorrectionSystem;

impl System for NetCorrectionSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let (entity, step) = {
            let state = resources.get_mut::<NetClientState>().unwrap();
            if state.correction.length_squared() < 1e-8 {
                return;
            }
            let Some(entity) = state.own_entity() else { return };
            let step = correction_step(state.correction, delta);
            state.correction -= step;
            (entity, step)
        };
        if let Ok(mut transform) = world.get::<&mut Transform>(entity) {
            transform.position += step;
        }
    }
}

/// Pushes the own predicted player out of anchored statics every Update tick
/// — the same `anchored_push` rule the server's SeparationSystem and
/// `reconcile_own`'s replay both apply — so the locally displayed position
/// obeys walls tick-by-tick instead of free-flighting until the next
/// snapshot's replay forces a Snap-class correction. Runs after
/// NetCorrectionSystem (net/mod.rs registration) so it acts on this tick's
/// fully corrected position.
pub(super) struct PredictedStaticCollisionSystem;

impl System for PredictedStaticCollisionSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let Some(entity) = resources.get::<NetClientState>().and_then(|s| s.own_entity()) else { return };
        let Some(shape) = world.get::<&Hitbox>(entity).map(|h| h.shape.clone()).ok() else { return };
        let statics = collect_solid_anchored_statics(world);
        let Ok(mut transform) = world.get::<&mut Transform>(entity) else { return };
        let push = anchored_push(transform.position, &shape, &statics);
        transform.position += push;
    }
}

/// Inserts the client-predicted LeapImpulse for a dash cast and retags this
/// tick's already-recorded PendingIntent (NetSendInputSystem runs earlier in
/// the same Input phase, before the dash existed) so replay reproduces the
/// dash from its very first tick too, not just the ticks after.
pub(crate) fn start_predicted_leap(world: &mut World, resources: &mut Resources, entity: Entity, velocity: Vec3, cast_secs: f32) {
    let _ = world.insert_one(entity, vordar_game::combat::LeapImpulse { velocity, remaining: cast_secs });
    if let Some(state) = resources.get_mut::<NetClientState>() {
        if let Some(pending) = state.pending.back_mut() {
            pending.leap = Some(velocity);
        }
    }
}

/// Sends our movement intent each Input tick, stamped with synced server time.
/// Nothing is sent until the clock sync has at least one sample. When
/// predicting, the intent is also emitted locally for the shared movement
/// system and remembered for reconciliation replay.
pub(super) struct NetSendInputSystem;

impl System for NetSendInputSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let dir = read_move_dir(resources);
        let predicted_entity = {
            let state = resources.get_mut::<NetClientState>().unwrap();
            let Some(t_server_micros) = state.client.as_ref().and_then(|c| c.server_now_micros()) else {
                return;
            };
            state.seq += 1;
            state.move_ring.push_back(MoveIntentEntry { seq: state.seq, t_server_micros, dir });
            if state.move_ring.len() > MOVE_RING_LEN {
                state.move_ring.pop_front();
            }
            if let Some(client) = &state.client {
                // Rides the unreliable datagram lane with last-3 redundancy:
                // a single lost datagram is fully recovered by the next
                // tick's batch.
                let intents: Vec<MoveIntentEntry> = state.move_ring.iter().cloned().collect();
                client.send_datagram(encode(&ClientMsg::MoveIntents { intents }));
            }

            let entity = if state.predict { state.own_entity() } else { None };
            if let Some(entity) = entity {
                // A LeapImpulse already on the entity when this tick's intent
                // is recorded means the Update-phase LeapSystem (later this
                // same tick) will override this tick's velocity too — mirror
                // that into the pending record so replay reconstructs the
                // dash instead of dead-reckoning plain movement. A dash that
                // starts THIS tick is retagged onto this same entry by
                // `start_predicted_leap`, called later in this same Input
                // phase.
                let leap = world.get::<&vordar_game::combat::leap::LeapImpulse>(entity).ok().map(|l| l.velocity);
                state.pending.push_back(PendingIntent { seq: state.seq, dir, dt: delta, leap });
                if state.pending.len() > MAX_PENDING_INTENTS {
                    state.pending.pop_front();
                }
            }
            entity
        };
        if let Some(entity) = predicted_entity {
            let bus = resources.get_mut::<EventBus>().expect("EventBus not in resources");
            bus.emit(MoveIntent { entity, dir });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::components::Velocity;

    const DT: f32 = 1.0 / 60.0;

    fn intent(seq: u32, dir: Vec2) -> PendingIntent {
        PendingIntent { seq, dir, dt: DT, leap: None }
    }

    fn walker_shape() -> CollisionShape {
        CollisionShape::Aabb { half_extents: Vec3::splat(0.5) }
    }

    fn wall_shape() -> CollisionShape {
        CollisionShape::Aabb { half_extents: Vec3::new(1.6, 0.9, 1.3) } // the cottage's shape (content/chapters/chapter02/prefabs/cottage.ron)
    }

    #[test]
    fn replay_applies_unacked_intents() {
        let pending = vec![intent(1, Vec2::new(1.0, 0.0)), intent(2, Vec2::new(1.0, 0.0))];
        let pos = replay_position(Vec3::ZERO, 6.0, pending.iter(), PlayRadius::default().0, &walker_shape(), &[]);
        assert!((pos.x - 2.0 * 6.0 * DT).abs() < 1e-6);
        assert_eq!(pos.y, 0.0);
        assert_eq!(pos.z, 0.0);
    }

    #[test]
    fn replay_normalizes_direction_like_the_simulation() {
        // An over-unit dir must move exactly as fast as a unit dir.
        let cheat = vec![intent(1, Vec2::new(30.0, 40.0))];
        let fair = vec![intent(1, Vec2::new(0.6, 0.8))];
        let a = replay_position(Vec3::ZERO, 6.0, cheat.iter(), PlayRadius::default().0, &walker_shape(), &[]);
        let b = replay_position(Vec3::ZERO, 6.0, fair.iter(), PlayRadius::default().0, &walker_shape(), &[]);
        assert!((a - b).length() < 1e-6);
    }

    /// At 150 ms RTT an Onslaught (cast_secs 0.4 s, content/classes/ravager.ron)
    /// replays ~9 unacked intents while the dash is in flight. Folding plain
    /// WASD movement through them (`leap: None` throughout) misses the
    /// dash's real displacement by more than SNAP_DISTANCE — a mid-dash
    /// teleport. Leap-aware replay (`leap: Some(velocity)`) must instead
    /// land exactly where the dash actually went.
    #[test]
    fn replay_reconstructs_a_dash_leap_instead_of_dead_reckoning_wasd() {
        let dash_velocity = Vec3::new(30.0, 0.0, 0.0); // 12 units over a 0.4 s cast
        let ticks: u32 = 9;
        // `dir` is deliberately non-zero and irrelevant: a LeapImpulse
        // overrides velocity outright, so replay must ignore `dir` too.
        let leaping: Vec<PendingIntent> = (1..=ticks)
            .map(|seq| PendingIntent { seq, dir: Vec2::new(0.0, 1.0), dt: DT, leap: Some(dash_velocity) })
            .collect();
        let dashed = replay_position(Vec3::ZERO, 6.0, leaping.iter(), PlayRadius::default().0, &walker_shape(), &[]);
        let expected = dash_velocity * DT * ticks as f32;
        assert!((dashed - expected).length() < 1e-4, "leap-aware replay must follow the dash exactly: {dashed:?}");

        let plain: Vec<PendingIntent> = (1..=ticks)
            .map(|seq| PendingIntent { seq, dir: Vec2::new(0.0, 1.0), dt: DT, leap: None })
            .collect();
        let dead_reckoned = replay_position(Vec3::ZERO, 6.0, plain.iter(), PlayRadius::default().0, &walker_shape(), &[]);
        assert!(
            (dashed - dead_reckoned).length() > SNAP_DISTANCE,
            "dead-reckoned WASD must diverge from the real dash past SNAP_DISTANCE, got {:.2}",
            (dashed - dead_reckoned).length()
        );
    }

    /// A player run straight into the world edge for long enough to hit the
    /// boundary: replaying pending intents must land exactly where the live
    /// `MovementSystem` does over the same ticks, not merely inside the
    /// Smooth-correction band (the pre-fix free-flight fold overran the
    /// clamp every tick past the boundary and drifted, tugging the player at
    /// the edge for no misprediction).
    #[test]
    fn replay_into_the_boundary_matches_the_live_system_exactly() {
        let bound = PlayRadius::default().0;
        let speed = 6.0;
        let dir = Vec2::new(1.0, 0.0);
        let velocity = movement_velocity(dir, speed);
        let start = Vec3::new(bound - 0.5, 0.0, 0.0);
        let ticks: u32 = 30;

        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(PlayRadius(bound));
        let e = world.spawn((
            Transform { position: start, ..Default::default() },
            Velocity { linear: velocity },
        ));
        for _ in 0..ticks {
            MovementSystem.run(&mut world, &mut resources, DT);
        }
        let live = world.get::<&Transform>(e).unwrap().position;

        let pending: Vec<PendingIntent> = (1..=ticks).map(|seq| intent(seq, dir)).collect();
        let replayed = replay_position(start, speed, pending.iter(), bound, &walker_shape(), &[]);

        assert_eq!(replayed, live, "replay must land exactly where the live system does at the boundary");
        assert!(
            classify_error(replayed - live) == Correction::Trust,
            "boundary replay must not fall into the Smooth-correction band"
        );
    }

    /// A player already pressed against a wall (the live-pipeline
    /// wall-equilibrium position, computed by folding `predict_step` the same
    /// way `vordar_game::motion`'s equivalence tests do) who keeps sending +X
    /// intents into it: the replay must fold the same static push the server
    /// applies and stay put, not free-flight through the wall and snap.
    #[test]
    fn reconcile_against_a_wall_stays_in_the_trust_band() {
        let wall_pos = Vec3::new(3.0, 0.0, 0.0);
        let bound = PlayRadius::default().0;
        let speed = 6.0;
        let velocity = movement_velocity(Vec2::X, speed);
        let statics = [(wall_pos, wall_shape())];

        let mut server_pos = Vec3::ZERO;
        for _ in 0..60 {
            server_pos = predict_step(server_pos, velocity, DT, bound, &walker_shape(), &statics);
        }

        let mut world = World::new();
        let mut resources = Resources::new();
        resources.insert(PlayRadius(bound));
        let entity = world.spawn((
            Transform::new(server_pos),
            Player { speed },
            Hitbox { shape: walker_shape() },
            Solid,
        ));
        let wall = world.spawn((Transform::new(wall_pos), Hitbox { shape: wall_shape() }, Solid));
        world.insert_one(wall, Anchored).unwrap();

        let mut state =
            NetClientState::new(None, "127.0.0.1:9".parse().unwrap(), "unit-test".into(), [0u8; 32], true, Duration::ZERO);
        state.own_id = Some(1);
        state.entities.insert(1, entity);
        state.pending = (1..=30u32).map(|seq| intent(seq, Vec2::X)).collect();
        resources.insert(state);

        let before = world.get::<&Transform>(entity).unwrap().position;
        reconcile_own(&mut world, &mut resources, entity, server_pos, 0);
        let after = world.get::<&Transform>(entity).unwrap().position;

        assert!(
            (after - before).length() < TRUST_DISTANCE,
            "replay against a wall must stay in the Trust band, moved {:.2} units",
            (after - before).length()
        );
    }

    #[test]
    fn error_classification_bands() {
        // Optimistic movement: jitter-scale disagreement never tugs the player.
        assert_eq!(classify_error(Vec3::new(0.2, 0.0, 0.0)), Correction::Trust);
        assert_eq!(classify_error(Vec3::new(0.5, 0.0, 0.0)), Correction::Smooth);
        assert_eq!(classify_error(Vec3::new(2.0, 0.0, 0.0)), Correction::Snap);
    }

    #[test]
    fn correction_decays_smoothly_to_zero() {
        let mut remaining = Vec3::new(0.9, 0.0, 0.0);
        let mut largest_step = 0.0f32;
        for _ in 0..120 {
            let step = correction_step(remaining, DT);
            largest_step = largest_step.max(step.length());
            remaining -= step;
        }
        assert!(remaining.length() < 1e-3, "did not converge: {remaining}");
        // Every nudge stays below one tick of run-speed movement — corrections
        // must read as motion, not teleports.
        assert!(largest_step < 6.0 * DT, "step too large: {largest_step}");
    }
}
