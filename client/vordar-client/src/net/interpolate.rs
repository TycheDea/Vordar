// Fixed-delay playback for replicated (non-predicted) entities: each remote
// entity's NetBuffer sample ring is rendered a fixed INTERP_DELAY_TICKS
// behind the newest received snapshot tick via a slewed playback cursor
// (absorbing jitter without freezing or warbling), with capped extrapolation
// bridging short gaps in arrivals and holding terminally through a sustained
// stall. Runs in Phase::Update, SystemOrder::First.

use super::*;

/// Cap on `NetBuffer`'s sample ring — bounded even if no consumer runs (e.g.
/// a criterion bench loop), so memory stays flat regardless of how long a
/// connection lives.
const NET_BUFFER_CAP: usize = 16;

/// Playback runs this many ticks behind the newest received `Snapshot.tick`
/// — 2 snapshot intervals (200 ms). Chosen so a *single* lost/late snapshot
/// datagram (the common case on an unreliable datagram lane) stays entirely
/// inside interpolation; only 2+ consecutive losses dip into extrapolation.
const INTERP_DELAY_TICKS: f64 = 2.0 * (TICK_HZ / SNAPSHOT_HZ) as f64;

/// Bound on how far the playback cursor's per-tick advance may deviate from
/// the nominal `delta * TICK_HZ`, as a fraction of that nominal advance — the
/// slew that keeps the cursor tracking `latest_state_tick -
/// INTERP_DELAY_TICKS` always reads as a smooth change of pace, never a pop.
const MAX_SLEW_FRACTION: f64 = 0.10;

/// Forward divergence (in ticks) beyond which the playback cursor gives up
/// slewing and hard-snaps to the target delay instead — a reconnect or a
/// stall long enough that smooth catch-up would take too long to be worth
/// it. One-sided: the cursor never moves backward within a session, so
/// backward divergence is never resynced this way — it's bounded instead by
/// the horizon clamp in `advance_playback` at `INTERP_DELAY_TICKS +
/// EXTRAP_CAP_TICKS` ticks behind `latest_state_tick`. A genuine reconnect
/// resets `playback` to `None` (see `lifecycle.rs`) rather than relying on
/// this constant to snap backward.
const RESYNC_TICKS: f64 = 30.0;

/// Cap on capped extrapolation past an entity's newest buffered sample, in
/// ticks (250 ms) — matches the loss-probe gate (BASELINE.md's post-datagram
/// probe shows max gaps ~300 ms at 5 % loss, i.e. two consecutive losses).
/// Past this the entity holds at the capped point instead of continuing to
/// dead-reckon indefinitely.
const EXTRAP_CAP_TICKS: f64 = 15.0;

/// Estimated velocity for entities the local sim doesn't move (remote,
/// snapshot-lerped players) — derived from snapshot position deltas by
/// `NetInterpolateSystem`. Locomotion/facing fall back to it when the sim
/// `Velocity` is absent or zero, so remote characters animate too.
/// Client-only, ≤ one snapshot interval stale.
#[derive(Clone, Copy, Default)]
pub struct NetMotion {
    pub velocity: Vec3,
}

/// Tick-indexed position history for a replicated (non-predicted) entity —
/// component on every replicated entity except a predicted own player.
/// `NetInterpolateSystem` renders `Transform.position` a fixed
/// `INTERP_DELAY_TICKS` behind the newest sample by interpolating the
/// bracketing pair; `apply_aoi_delta` seeds this on AOI entry and
/// `apply_states` pushes into it afterward. Samples always arrive in
/// strictly increasing tick order (the tick guard in `apply_states` sees to
/// that), so insertion is a plain push; capped at `NET_BUFFER_CAP` so it
/// stays memory-flat even if nothing ever consumes it.
pub(super) struct NetBuffer {
    pub(super) samples: VecDeque<(u64, Vec3)>,
}

impl NetBuffer {
    /// A freshly entered entity's buffer: one sample, so playback holds at
    /// the entry position until the first real snapshot sample brackets it.
    pub(super) fn seeded(tick: u64, pos: Vec3) -> Self {
        let mut samples = VecDeque::with_capacity(NET_BUFFER_CAP);
        samples.push_back((tick, pos));
        Self { samples }
    }

    /// Pushes a new sample, skipping it if `tick` would not keep the ring
    /// strictly increasing (guards both an out-of-order caller and the
    /// dry-recovery synthetic sample).
    pub(super) fn push(&mut self, tick: u64, pos: Vec3) {
        if let Some(&(back_tick, _)) = self.samples.back() {
            if tick <= back_tick {
                return;
            }
        }
        if self.samples.len() >= NET_BUFFER_CAP {
            self.samples.pop_front();
        }
        self.samples.push_back((tick, pos));
    }
}

/// Renders every replicated entity a fixed `INTERP_DELAY_TICKS` behind the
/// newest received snapshot tick by interpolating its `NetBuffer` sample
/// ring, instead of restarting a one-interval lerp from wherever the entity
/// is currently displayed, which reads as speed warble on every late
/// arrival. Also writes `NetMotion` with the active segment's velocity —
/// zero while holding at the first sample or capped past the newest, the
/// extrapolation velocity in between.
pub(super) struct NetInterpolateSystem;

impl System for NetInterpolateSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, delta: f32) {
        let cursor = {
            let state = resources.get_mut::<NetClientState>().unwrap();
            let cursor = advance_playback(state.playback, state.latest_state_tick, delta);
            state.playback = Some(cursor);
            cursor
        };

        // Collected inside the view borrow, inserted after it: hecs's query
        // borrow must be released before the world can be mutated again.
        let mut net_motions: Vec<(Entity, Vec3)> = Vec::new();
        for (entity, buffer, transform) in world.query::<(Entity, &NetBuffer, &mut Transform)>().iter() {
            let (pos, velocity) = sample_buffer(buffer, cursor);
            transform.position = pos;
            net_motions.push((entity, velocity));
        }
        for (entity, velocity) in net_motions {
            let _ = world.insert_one(entity, NetMotion { velocity });
        }
    }
}

/// One Update tick's worth of playback-cursor advance: nominally `delta *
/// TICK_HZ` ticks, slewed toward `latest_state_tick as f64 -
/// INTERP_DELAY_TICKS` within `±MAX_SLEW_FRACTION` of that nominal advance so
/// catching up never pops — except `playback == None` (never driven) or a
/// forward divergence past `RESYNC_TICKS`, which snaps to the target instead
/// of slewing toward it. The result is clamped to `latest_state_tick as f64 +
/// EXTRAP_CAP_TICKS`: advancing the cursor past that horizon changes no
/// rendered position (`sample_buffer` already holds at the capped point), so
/// a sustained stall is a terminal capped hold rather than a cursor that
/// keeps running ahead and periodically snapping backward.
fn advance_playback(playback: Option<f64>, latest_state_tick: u64, delta: f32) -> f64 {
    let target = latest_state_tick as f64 - INTERP_DELAY_TICKS;
    let Some(prev) = playback else { return target };
    let error = target - prev;
    if error > RESYNC_TICKS {
        return target;
    }
    let nominal = delta as f64 * TICK_HZ as f64;
    let max_correction = nominal * MAX_SLEW_FRACTION;
    (prev + nominal + error.clamp(-max_correction, max_correction)).min(latest_state_tick as f64 + EXTRAP_CAP_TICKS)
}

/// Position and velocity at fractional server `tick` position `cursor`
/// inside `buffer`'s sample ring: holds at the first sample when `cursor` is
/// before it; past the newest sample it extrapolates at the last segment's
/// velocity for up to `EXTRAP_CAP_TICKS` (so a run of 2+ consecutive lost
/// snapshot datagrams bridges instead of freezing the entity), then holds at
/// the capped point; otherwise it linearly interpolates the bracketing pair.
/// Velocity is that segment's slope, zero while holding at the first sample
/// or capped past the newest.
fn sample_buffer(buffer: &NetBuffer, cursor: f64) -> (Vec3, Vec3) {
    let samples = &buffer.samples;
    let Some(&(first_tick, first_pos)) = samples.front() else {
        return (Vec3::ZERO, Vec3::ZERO); // never seeded — nothing to render yet
    };
    if cursor <= first_tick as f64 {
        return (first_pos, Vec3::ZERO);
    }
    let &(last_tick, last_pos) = samples.back().unwrap();
    if cursor >= last_tick as f64 {
        // Velocity of the last two samples (zero if the buffer holds only
        // one) drives capped extrapolation past the newest sample.
        let velocity = samples
            .len()
            .checked_sub(2)
            .and_then(|i| samples.get(i))
            .map_or(Vec3::ZERO, |&(prev_tick, prev_pos)| {
                (last_pos - prev_pos) / ((last_tick - prev_tick) as f32 / TICK_HZ)
            });
        let extrap_ticks = (cursor - last_tick as f64).min(EXTRAP_CAP_TICKS);
        let pos = last_pos + velocity * (extrap_ticks as f32 / TICK_HZ);
        let capped = extrap_ticks >= EXTRAP_CAP_TICKS;
        return (pos, if capped { Vec3::ZERO } else { velocity });
    }
    for (a, b) in samples.iter().zip(samples.iter().skip(1)) {
        if cursor <= b.0 as f64 {
            let span = (b.0 - a.0) as f64;
            let t = ((cursor - a.0 as f64) / span) as f32;
            let velocity = (b.1 - a.1) / (span as f32 / TICK_HZ);
            return (a.1.lerp(b.1, t), velocity);
        }
    }
    (last_pos, Vec3::ZERO) // unreachable: cursor is bounded by the checks above
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::apply::apply_states;
    use vordar_protocol::WirePos;

    const DT: f32 = 1.0 / 60.0;

    /// A late or jittered snapshot arrival must never freeze the entity nor
    /// make it "catch up" at compressed speed (jitter → speed warble) — this
    /// drives the real receive path (`apply_states`) and the real render
    /// system directly, one Update tick (`delta = 1/60`) per loop iteration,
    /// no network, no sleeps. A remote entity moves +X at a steady 6 u/s;
    /// the server samples it every 6
    /// ticks (100 ms, `SNAPSHOT_HZ`) at `pos.x = tick / 60 * 6.0`, but each
    /// sample's arrival is jittered by a deterministic pattern in [-2, +2]
    /// ticks (including a late-by-2 arrival) relative to its nominal 6k
    /// arrival tick. After a 30-tick warmup the render step every tick must
    /// stay within [0.5, 1.5] × the nominal per-tick displacement, and total
    /// displacement over the window must track the true speed within 5 %.
    #[test]
    fn fixed_delay_playback_rides_through_jittered_arrivals() {
        const SPEED: f32 = 6.0;
        const CADENCE_TICKS: u64 = 6; // SNAPSHOT_HZ = 10 Hz at TICK_HZ = 60 Hz
        const WARMUP_TICKS: u32 = 30;
        const WINDOW_TICKS: u32 = 180;
        // Deterministic jitter pattern in [-2, +2], includes a late-by-2 arrival.
        const JITTER: [i64; 6] = [0, 2, -2, 1, -1, 0];

        let mut world = World::new();
        let mut resources = Resources::new();

        let remote = world.spawn((Transform::new(Vec3::ZERO), NetBuffer::seeded(0, Vec3::ZERO)));
        let mut entities = HashMap::new();
        entities.insert(1u32, remote);

        let mut state =
            NetClientState::new(None, "127.0.0.1:9".parse().unwrap(), "unit-test".into(), [0u8; 32], false, Duration::ZERO);
        state.entities = entities;
        resources.insert(state);

        let mut render_sys = NetInterpolateSystem;
        let mut next_k: u64 = 1;
        let total_ticks = WARMUP_TICKS + WINDOW_TICKS;
        let mut window_positions: Vec<Vec3> = Vec::new();

        for client_tick in 0..total_ticks {
            // Deliver every sample whose jittered arrival tick is due now.
            loop {
                let server_tick = CADENCE_TICKS * next_k;
                let jitter = JITTER[(next_k as usize - 1) % JITTER.len()];
                let arrival_tick = (server_tick as i64 + jitter).max(0) as u64;
                if arrival_tick != client_tick as u64 {
                    break;
                }
                let pos = Vec3::new(server_tick as f32 / 60.0 * SPEED, 0.0, 0.0);
                apply_states(&mut world, &mut resources, server_tick, 0, vec![EntityPos { id: 1, pos: WirePos(pos), hp: None }]);
                next_k += 1;
            }
            render_sys.run(&mut world, &mut resources, DT);
            if client_tick >= WARMUP_TICKS {
                window_positions.push(world.get::<&Transform>(remote).unwrap().position);
            }
        }

        let nominal_step = SPEED * DT;
        let mut max_step = 0.0f32;
        let mut min_step = f32::MAX;
        for pair in window_positions.windows(2) {
            let step = (pair[1] - pair[0]).length();
            max_step = max_step.max(step);
            min_step = min_step.min(step);
        }
        assert!(
            min_step >= 0.5 * nominal_step,
            "a step froze or shrank too far below nominal: min_step={min_step:.4}, nominal={nominal_step:.4}"
        );
        assert!(
            max_step <= 1.5 * nominal_step,
            "a step warbled too far above nominal: max_step={max_step:.4}, nominal={nominal_step:.4}"
        );

        let displacement = (*window_positions.last().unwrap() - *window_positions.first().unwrap()).x;
        let window_secs = (window_positions.len() - 1) as f32 * DT;
        let expected = SPEED * window_secs;
        assert!(
            (displacement - expected).abs() <= 0.05 * expected,
            "total displacement drifted from true speed: got {displacement:.3}, expected {expected:.3}"
        );
    }

    /// A run of 2+ consecutive lost snapshot datagrams must not freeze the
    /// entity (extrapolation bridges it), and the eventual real sample must
    /// resume playback without a pop. Same deterministic harness as
    /// `fixed_delay_playback_rides_through_jittered_arrivals` — drives
    /// `apply_states` / `NetInterpolateSystem` directly, one Update tick
    /// (`delta = 1/60`) per loop iteration, no network, no sleeps. A remote
    /// entity moves +X at 6 u/s; samples at server ticks 6 and 12 arrive at
    /// their natural client ticks, ticks 18 and 24 are never delivered, tick
    /// 30 arrives at its natural client tick, and nothing more is ever
    /// delivered after that (the buffer runs permanently dry).
    ///
    /// Once no more real samples arrive, the playback cursor's horizon clamp
    /// holds it at `EXTRAP_CAP_TICKS + INTERP_DELAY_TICKS` (27) past tick
    /// 30's sample — a terminal capped hold, so the held window asserted
    /// below stays bit-identical all the way to `TOTAL_TICKS` (120) with no
    /// backward step anywhere in the run.
    #[test]
    fn extrapolation_bridges_lost_snapshots_then_caps() {
        const SPEED: f32 = 6.0;
        // Deliveries: server ticks 6 and 12 land on time; 18 and 24 are
        // simply never sent; 30 lands on time; nothing after.
        const DELIVERIES: [u64; 3] = [6, 12, 30];
        // Runs well past the horizon clamp engaging (tick ~57) to observe
        // the terminal capped hold sustained for the rest of the run.
        const TOTAL_TICKS: usize = 120;

        let pos_at = |tick: u64| Vec3::new(tick as f32 / 60.0 * SPEED, 0.0, 0.0);

        let mut world = World::new();
        let mut resources = Resources::new();

        let remote = world.spawn((Transform::new(Vec3::ZERO), NetBuffer::seeded(0, Vec3::ZERO)));
        let mut entities = HashMap::new();
        entities.insert(1u32, remote);

        let mut state =
            NetClientState::new(None, "127.0.0.1:9".parse().unwrap(), "unit-test".into(), [0u8; 32], false, Duration::ZERO);
        state.entities = entities;
        resources.insert(state);

        let mut render_sys = NetInterpolateSystem;
        let mut positions: Vec<Vec3> = Vec::with_capacity(TOTAL_TICKS);
        let mut motions: Vec<Vec3> = Vec::with_capacity(TOTAL_TICKS);
        for client_tick in 0u64..TOTAL_TICKS as u64 {
            if DELIVERIES.contains(&client_tick) {
                apply_states(
                    &mut world,
                    &mut resources,
                    client_tick,
                    0,
                    vec![EntityPos { id: 1, pos: WirePos(pos_at(client_tick)), hp: None }],
                );
            }
            render_sys.run(&mut world, &mut resources, DT);
            positions.push(world.get::<&Transform>(remote).unwrap().position);
            motions.push(
                world.get::<&NetMotion>(remote).map(|m| m.velocity).unwrap_or(Vec3::ZERO),
            );
        }

        let nominal_step = SPEED * DT;

        // (a) After tick 12's sample, the entity keeps advancing right
        // through the dry window (18/24 never arrive) instead of freezing
        // once the cursor passes tick 12's sample around client tick 26.
        for tick in 13..30usize {
            let step = (positions[tick] - positions[tick - 1]).x;
            assert!(
                step >= 0.5 * nominal_step && step <= 1.5 * nominal_step,
                "tick {tick}: step {step:.4} outside [{:.4},{:.4}] during the dry window",
                0.5 * nominal_step,
                1.5 * nominal_step
            );
            assert!(motions[tick].x > 0.0, "tick {tick}: NetMotion must stay non-zero while bridging the dry window");
        }

        // (b) No pop across tick 30's arrival — fails without the
        // dry-recovery synthetic sample splicing continuity into the jump
        // from the extrapolated position to the freshly-pushed real one.
        let arrival_step = (positions[30] - positions[29]).x;
        assert!(
            arrival_step.abs() < 2.0 * nominal_step,
            "tick 30 arrival popped: step {arrival_step:.4}, bound {:.4}",
            2.0 * nominal_step
        );

        // (c) Capped extrapolation: position never advances more than
        // EXTRAP_CAP_TICKS worth of motion past tick 30's sample position
        // (small float tolerance).
        let cap_bound = pos_at(30).x + (EXTRAP_CAP_TICKS as f32 / TICK_HZ) * SPEED + 0.01;
        let max_pos = positions.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!(max_pos <= cap_bound, "extrapolation exceeded its cap: max position {max_pos:.4}, bound {cap_bound:.4}");

        // Bit-identical hold once capped — the terminal state for the rest
        // of the run, so the last 3 ticks hold exactly.
        let held = &positions[TOTAL_TICKS - 3..];
        assert!(held[0] == held[1] && held[1] == held[2], "capped position must hold bit-identical, got {held:?}");
        let held_motion = &motions[TOTAL_TICKS - 3..];
        assert!(
            held_motion.iter().all(|m| m.length_squared() == 0.0),
            "NetMotion must be exactly zero once capped, got {held_motion:?}"
        );

        // (d) The capped hold is terminal: the playback cursor never moves
        // backward, so no tick may render an earlier position than the last.
        for t in 1..TOTAL_TICKS {
            let step = (positions[t] - positions[t - 1]).x;
            assert!(step >= -1e-6, "tick {t}: position stepped backward by {step:.4} during the stall");
        }
    }
}
