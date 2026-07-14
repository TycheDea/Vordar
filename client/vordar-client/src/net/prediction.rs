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
/// Last-3 redundancy depth for `ClientMsg::MoveIntents` (protocol v15,
/// networking rework 3 finding 5): this tick's entry plus the two previous,
/// sent via datagram every Input tick — a single lost datagram is fully
/// recovered by the next tick's batch.
pub(super) const MOVE_RING_LEN: usize = 3;

/// An intent sent to the server but not yet covered by `last_processed_seq`.
/// Replayed on top of each snapshot of our own player. `leap` mirrors a
/// LeapImpulse active on the entity when this tick's intent was recorded —
/// replay reproduces the dash's straight-line displacement instead of
/// dead-reckoning plain WASD movement through it (networking audit
/// 2026-07-11, finding 11).
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
        (replay_position(server_pos, speed, state.pending.iter()), still_reconciling_a_dash)
    };
    // Collision response isn't replayed (finding 11 — full collision-in-replay
    // is rework-scale, `reworks-networking-2026-07-11.md` finding 7): mid-dash
    // the free-flight `replayed` position and a wall-clamped real one can
    // differ for reasons that aren't mispredictions, so corrections stay
    // suppressed until the server has caught up on the whole dash instead of
    // tugging every snapshot.
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
/// authoritative position — the same movement rule the simulation runs,
/// including a leap override where one was active (finding 11) — collision
/// response is the one part of the shared rule still unreplayed (rework-scale,
/// `reworks-networking-2026-07-11.md` finding 7).
fn replay_position<'a>(
    server_pos: Vec3,
    speed: f32,
    pending: impl Iterator<Item = &'a PendingIntent>,
) -> Vec3 {
    pending.fold(server_pos, |pos, p| {
        let velocity = p.leap.unwrap_or_else(|| movement_velocity(p.dir, speed));
        pos + velocity * p.dt
    })
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

/// Inserts the client-predicted LeapImpulse for a dash cast and retags this
/// tick's already-recorded PendingIntent (NetSendInputSystem runs earlier in
/// the same Input phase, before the dash existed) so replay reproduces the
/// dash from its very first tick too, not just the ticks after — networking
/// audit 2026-07-11, finding 11.
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
                // Rides the unreliable datagram lane with last-3 redundancy
                // (protocol v15, networking rework 3 finding 5): a single
                // lost datagram is fully recovered by the next tick's batch.
                let intents: Vec<MoveIntentEntry> = state.move_ring.iter().cloned().collect();
                client.send_datagram(encode(&ClientMsg::MoveIntents { intents }));
            }

            let entity = if state.predict { state.own_entity() } else { None };
            if let Some(entity) = entity {
                // A LeapImpulse already on the entity when this tick's intent
                // is recorded means the Update-phase LeapSystem (later this
                // same tick) will override this tick's velocity too — mirror
                // that into the pending record so replay reconstructs the
                // dash instead of dead-reckoning plain movement (networking
                // audit 2026-07-11, finding 11). A dash that starts THIS tick
                // is retagged onto this same entry by `start_predicted_leap`,
                // called later in this same Input phase.
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

    const DT: f32 = 1.0 / 60.0;

    fn intent(seq: u32, dir: Vec2) -> PendingIntent {
        PendingIntent { seq, dir, dt: DT, leap: None }
    }

    #[test]
    fn replay_applies_unacked_intents() {
        let pending = vec![intent(1, Vec2::new(1.0, 0.0)), intent(2, Vec2::new(1.0, 0.0))];
        let pos = replay_position(Vec3::ZERO, 6.0, pending.iter());
        assert!((pos.x - 2.0 * 6.0 * DT).abs() < 1e-6);
        assert_eq!(pos.y, 0.0);
        assert_eq!(pos.z, 0.0);
    }

    #[test]
    fn replay_normalizes_direction_like_the_simulation() {
        // An over-unit dir must move exactly as fast as a unit dir.
        let cheat = vec![intent(1, Vec2::new(30.0, 40.0))];
        let fair = vec![intent(1, Vec2::new(0.6, 0.8))];
        let a = replay_position(Vec3::ZERO, 6.0, cheat.iter());
        let b = replay_position(Vec3::ZERO, 6.0, fair.iter());
        assert!((a - b).length() < 1e-6);
    }

    /// Networking audit 2026-07-11, finding 11: at 150 ms RTT an Onslaught
    /// (cast_secs 0.4 s, content/classes/ravager.ron) replays ~9 unacked
    /// intents while the dash is in flight. Folding plain WASD movement
    /// through them (the pre-fix behaviour, `leap: None` throughout) misses
    /// the dash's real displacement by more than SNAP_DISTANCE — exactly the
    /// mid-dash teleport the finding describes. Leap-aware replay
    /// (`leap: Some(velocity)`) must instead land exactly where the dash
    /// actually went.
    #[test]
    fn replay_reconstructs_a_dash_leap_instead_of_dead_reckoning_wasd() {
        let dash_velocity = Vec3::new(30.0, 0.0, 0.0); // 12 units over a 0.4 s cast
        let ticks: u32 = 9;
        // `dir` is deliberately non-zero and irrelevant: a LeapImpulse
        // overrides velocity outright, so replay must ignore `dir` too.
        let leaping: Vec<PendingIntent> = (1..=ticks)
            .map(|seq| PendingIntent { seq, dir: Vec2::new(0.0, 1.0), dt: DT, leap: Some(dash_velocity) })
            .collect();
        let dashed = replay_position(Vec3::ZERO, 6.0, leaping.iter());
        let expected = dash_velocity * DT * ticks as f32;
        assert!((dashed - expected).length() < 1e-4, "leap-aware replay must follow the dash exactly: {dashed:?}");

        let plain: Vec<PendingIntent> = (1..=ticks)
            .map(|seq| PendingIntent { seq, dir: Vec2::new(0.0, 1.0), dt: DT, leap: None })
            .collect();
        let dead_reckoned = replay_position(Vec3::ZERO, 6.0, plain.iter());
        assert!(
            (dashed - dead_reckoned).length() > SNAP_DISTANCE,
            "dead-reckoned WASD must diverge from the real dash past SNAP_DISTANCE, got {:.2}",
            (dashed - dead_reckoned).length()
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
