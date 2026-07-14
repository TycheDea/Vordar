# Plan: No jitter buffer or extrapolation — remote entities freeze at every late snapshot — 2026-07-14

Source: docs/reviews/networking/reworks-networking-2026-07-11.md finding 4.

## Ideal end state

Remote entities render a short, fixed distance in the past — a playback cursor
in server-tick units runs ~200 ms (two snapshot intervals) behind the newest
received `Snapshot.tick`, interpolating each entity across a small tick-indexed
sample buffer instead of restarting a one-interval lerp from the displayed
position on every arrival. A late or lost datagram (now the common case:
snapshots ride lossy datagrams since rework 3) is absorbed by the buffer's
slack; when a run of losses drains it, motion continues by capped extrapolation
(≤ 250 ms) from the buffer's own velocity, then holds. Jitter no longer
converts into speed warble, single losses no longer freeze anything, and a
headless smoothness probe (loss.rs-style) asserts the rendered path of a
moving remote stays continuous under WAN impairment — recorded in BASELINE.md
as a regression gate.

## Design decisions

- **Playback cursor is a local 60 Hz tick clock slewed to the newest snapshot
  tick — not an absolute synced-server-time mapping.** The finding's Ideal says
  "clocked off synced server time", but nothing on the wire maps `tick` to
  server micros: `Snapshot.tick` is `NetServerState.tick` (a PostUpdate counter,
  net_plugin.rs:1179) and no message carries its epoch. The two honest routes to
  an absolute mapping — adding a server timestamp to `Snapshot` (protocol bump,
  +~7 B every 100 ms) or a client-side epoch min-filter estimator — both buy
  nothing over a cursor that advances `delta * TICK_HZ` per fixed Update tick
  and is gently slewed toward `latest_state_tick - INTERP_DELAY_TICKS`: the
  effect is the identical fixed delay in the server-tick timebase, with no wire
  change, no coupling to clock-sync convergence/transients, and automatic
  tracking of any server tick-rate drift. Rejected: both absolute-mapping
  variants. The slew is bounded (playback rate stays within ±10 % of 1.0) so a
  drifting cursor reads as motion, never a pop; a divergence beyond
  `RESYNC_TICKS` (30 ticks / 500 ms — reconnect, long stall) hard-snaps.
- **The tick timebase becomes protocol-meaningful.** The client needs to know
  ticks advance at 60 Hz. That constant moves to where the tick stamp is
  defined: `vordar-protocol` gains `pub const TICK_HZ: f32 = 60.0` (documented
  as the rate `AoiDelta.tick`/`Snapshot.tick` advance at), and the server's
  private `POST_HZ` (net_plugin.rs:72) is defined from it. Pure semantics
  documentation of an existing wire field — **no wire change, no
  PROTOCOL_VERSION bump anywhere in this rework.**
- **Interpolation delay = 2 snapshot intervals (12 ticks, 200 ms):**
  `INTERP_DELAY_TICKS = 2.0 * (TICK_HZ / SNAPSHOT_HZ) as f64`. The finding
  allows 1.5–2; 2 is chosen because snapshots are lossy by design since rework
  3 (a lost datagram is never retransmitted, 1–5 % loss is the normal envelope),
  and at 2 intervals a *single* loss stays entirely inside interpolation —
  extrapolation engages only on 2+ consecutive losses (rare; BASELINE.md's
  post-datagram probe shows max gaps ~300 ms at 5 % loss). At 1.5 intervals
  every single loss would dip into extrapolation. Cost: 50 ms extra remote-view
  delay, invisible in this game (no hitscan target-lock; hit resolution is
  server-side against intent timestamps).
- **Per-entity sample ring, not a central tick-indexed snapshot map.** Each
  buffered entity carries `NetBuffer { samples: VecDeque<(u64 /*tick*/, Vec3)> }`
  (cap 16, pop_front on overflow — bounded even if no consumer runs, which
  keeps the criterion bench loop memory-flat). Per-entity is the right shape
  because sampling cadence IS per-entity: the server's `MAX_SNAPSHOT_STATES=64`
  round-robin (net_plugin.rs:60-68) refreshes non-nearest entities only every
  few snapshots, so entities legitimately have different tick gaps and the
  interpolation must span each entity's own gaps. A central map would still
  need per-entity presence tracking on top. `NetBuffer` replaces `NetLerp`
  outright — no parallel path, no transition flag.
- **The strict latest-wins tick guard stays exactly as it is.** `apply_states`
  keeps dropping any `Snapshot` whose tick is not strictly newer than
  `latest_state_tick` before reading any field (net.rs:610-617) — a reordered
  stale datagram is NOT inserted into buffers. Rationale: the realistic reorder
  window (jitter ≤ 40 ms in the WAN profiles) is well under the 100 ms cadence,
  so cross-cadence reordering is rare; the guard is what keeps the ack, hp, and
  reconciliation sound, and interpolation spans the resulting one-sample gap
  anyway. Rejected: out-of-order buffer insertion — it would force the guard to
  partially apply stale datagrams, breaking the "dropped before any field is
  read" invariant proven by `apply_states_drops_a_stale_snapshot_tick`.
  Consequence: per-entity samples always arrive in strictly increasing tick
  order, so buffer insertion is a plain `push_back` (skip if tick ≤ back's).
- **Extrapolation comes from the buffer's own velocity, capped, then holds —
  and recovery is continuous via a synthetic sample.** When the cursor passes
  the newest sample, position continues at the velocity of the last two samples
  for at most `EXTRAP_CAP_TICKS = 15` (250 ms, matching the loss-probe gate),
  then holds. When a fresh sample arrives for an entity whose newest buffered
  tick is already behind the cursor (i.e. it was extrapolating or holding), a
  synthetic sample `(floor(cursor), current Transform.position)` is pushed
  before the real one (skipped if it wouldn't keep ticks strictly increasing),
  so playback resumes by interpolating from where the entity is actually
  displayed — positional continuity without resurrecting today's
  restart-from-displayed lerp, which is precisely the mechanism that converts
  jitter into speed warble (net.rs:654-658). This also smooths the round-robin
  case (distant entities sampled every ~2–5 snapshots).
- **`NetMotion` becomes the derivative of the displayed path.** Today
  `apply_states` derives it from `(snapshot_pos - displayed_pos) * SNAPSHOT_HZ`
  (net.rs:655) — display-relative and warbly. It moves into the playback system:
  the sampled segment's slope while interpolating, the extrapolation velocity
  while extrapolating, zero while holding — so locomotion/facing animate
  exactly what is rendered. The `NetMotion` struct and its consumers
  (locomotion.rs) are untouched.
- **The prediction/reconciliation path is untouched.** A predicted own player
  never carries `NetBuffer` (same rule as `NetLerp` today, net.rs:568-570);
  `reconcile_own`, pending intents, corrections, TRUST/SNAP bands all stay
  as-is. A non-predicting own player (`predict: false`, the comparison mode) is
  buffered like any remote.
- **Genuinely undecidable items: none.** Delay/cap values are engineering
  calls inside the finding's stated band and are recorded above.

## Findings (execution order)

### 1. Tick-indexed sample buffer with fixed-delay playback replaces the one-interval lerp

- **Evidence:** `client/vordar-client/src/net.rs:264-268` — `NetLerp { from,
  to, t }`; `net.rs:1153-1162` — `NetLerpSystem` advances `t` by
  `delta * SNAPSHOT_HZ`, completing in exactly one snapshot interval and then
  holding; `net.rs:646-659` — `apply_states` restarts every lerp from the
  *displayed* position (`lerp.from = transform.position; lerp.t = 0.0`), so a
  late arrival first freezes the entity, then replays the missing distance at
  compressed speed (jitter → speed warble); `net.rs:655` derives `NetMotion`
  from the same display-relative delta. `Snapshot.tick` / `AoiDelta.tick`
  (`game/vordar-protocol/src/lib.rs:89-113`) are used only as a staleness guard
  (`latest_state_tick`, net.rs:240, 610-617), never for timing. Ticks advance at
  60 Hz (`NetServerState.tick` incremented per PostUpdate,
  `server/vordar-server/src/net_plugin.rs:1179`; `POST_HZ = 60.0` at
  net_plugin.rs:72; per-conn snapshots every `STAGGER = 6` ticks,
  net_plugin.rs:76, 1207) but no shared constant says so to the client.
- **Ideal:** Remote entities carry `NetBuffer { samples: VecDeque<(u64, Vec3)> }`
  (cap 16, strictly increasing ticks). `apply_aoi_delta` seeds an entering
  entity's buffer with `(delta_tick, pos)`; `apply_states` (after the unchanged
  tick guard) pushes `(tick, pos)` per addressed entity instead of restarting a
  lerp. A new `NetInterpolateSystem` (registered exactly where `NetLerpSystem`
  was: `Phase::Update, SystemOrder::First`, net.rs:142) advances a playback
  cursor `playback: Option<f64>` on `NetClientState` — `+= delta *
  TICK_HZ as f64` plus a slew toward `latest_state_tick as f64 -
  INTERP_DELAY_TICKS` bounded to ±10 % of the nominal advance, hard-snapping
  when off by more than `RESYNC_TICKS = 30.0` or when `None` — and writes each
  buffered entity's `Transform.position` by interpolating the bracketing
  samples (`cursor` before the first sample → hold at first; past the newest →
  hold at newest, extrapolation is step 2). It also writes `NetMotion` with the
  active segment's velocity (zero while holding). `vordar-protocol` exports
  `pub const TICK_HZ: f32 = 60.0` and net_plugin.rs defines `POST_HZ` from it.
  `INTERP_DELAY_TICKS = 2.0 * (TICK_HZ / SNAPSHOT_HZ) as f64` (= 12 ticks,
  200 ms). `NetLerp` and `NetLerpSystem` are deleted; `teardown_replicated_world`
  (net.rs:374-406) resets `playback` to `None` alongside `latest_state_tick`.
- **Gap:** No buffer exists — each entity remembers exactly two positions; no
  playback clock exists — display timing is arrival timing; a late snapshot
  freezes then warbles by construction. Everything in Ideal is new except the
  tick guard, which is kept byte-for-byte.
- **Suggestion:** One bounded diff in net.rs + one constant in
  vordar-protocol + a one-line `POST_HZ` change in net_plugin.rs. Constructor
  literals that gain the `playback: None` field are all local: three test
  `NetClientState` literals in net.rs's `mod tests`, plus `state_for_bench`
  (net.rs:1191-1213). Update the module header comment (net.rs:3-9), the
  stale bench comments (`benchmarks/benches/client_netcode.rs:6-7, 42-44` —
  they describe the lerp restart this step deletes), and the
  `NEAREST_GUARANTEED` comment's "NetLerp absorbs the lower rate"
  (net_plugin.rs:67 → "playback interpolation absorbs the lower rate").
  Keep `apply_states`'s own-player skip and hp application exactly as they are.
- **Path:**
  1. Write the fail-first test in net.rs `mod tests`:
     `fixed_delay_playback_rides_through_jittered_arrivals`. No network, no
     sleeps — drive the real `apply_aoi_delta` / `apply_states` /
     `NetInterpolateSystem` (today: `NetLerpSystem`) directly, one Update tick
     per loop iteration with `delta = 1/60`. Scenario: one remote entity moving
     +X at 6 u/s; server samples at ticks 6, 12, 18, … with
     `pos.x = tick as f32 / 60.0 * 6.0`; deliver sample k at client tick
     `6k + jitter_k` with a deterministic jitter pattern in {-2..+2} that
     includes at least one late-by-2 arrival. After a 30-tick warmup, record
     the entity's per-tick `Transform.position` step; assert every step length
     is within `[0.5, 1.5] * (6.0 / 60.0)` and total displacement over the
     window is within 5 % of `speed * window`. This FAILS today: the late
     arrival produces zero-steps (freeze) followed by ~2× steps (warble).
  2. Add `TICK_HZ` to vordar-protocol (doc: "rate at which `AoiDelta.tick` /
     `Snapshot.tick` advance"), set `const POST_HZ: f32 =
     vordar_protocol::TICK_HZ;` in net_plugin.rs.
  3. Implement `NetBuffer`, the fill sites, `playback` + slew,
     `NetInterpolateSystem` (including `NetMotion` writing), delete
     `NetLerp`/`NetLerpSystem`, reset `playback` in
     `teardown_replicated_world`.
  4. Rewrite `apply_states_drops_a_stale_snapshot_tick`'s two `NetLerp.to`
     assertions (net.rs:1334, 1358) against `NetBuffer`'s newest sample — the
     guard behavior itself is unchanged and must still pass.
  5. Green gate: `cargo test --workspace` (the e2e tests in net.rs that spawn
     real servers must still pass — they drive `NetReceiveSystem`, not the
     deleted lerp), `cargo bench -p vordar-benches --bench client_netcode`
     still compiles/runs, zero new warnings.

### 2. Capped extrapolation and continuous recovery when the buffer runs dry

- **Evidence:** After step 1, `NetInterpolateSystem` in
  `client/vordar-client/src/net.rs` holds an entity at its newest buffered
  sample when the cursor passes it — a run of 2+ consecutive lost snapshot
  datagrams (BASELINE.md's post-datagram loss probe measured max gaps
  ~300 ms at 5 % loss, i.e. two losses in a row) still freezes the entity, and
  the arrival of the next sample would snap it forward from the held position.
  `NetMotion` (`client/vordar-client/src/locomotion.rs:96-98`) reads zero
  during the hold, so remote characters also pop from idle back to run.
- **Ideal:** When the playback cursor passes an entity's newest sample, its
  position continues at the velocity of the last two buffered samples
  (`(p_n - p_{n-1}) / ((t_n - t_{n-1}) as f32 / TICK_HZ)`; zero if only one
  sample) for at most `EXTRAP_CAP_TICKS: f64 = 15.0` (250 ms) past the newest
  tick, then holds at the capped point. `NetMotion` carries the extrapolation
  velocity while extrapolating and zero while holding. When `apply_states`
  pushes a sample for an entity whose newest buffered tick is `<` the current
  cursor (it was extrapolating/holding), it first pushes a synthetic sample
  `(floor(cursor) as u64, current Transform.position)` — skipped if that tick
  would not be strictly greater than the buffer's back — so playback resumes by
  interpolating from the displayed position toward the real sample: no pop, and
  no return of the restart-from-displayed lerp.
- **Gap:** Step 1 ships hold-when-dry only; extrapolation, the cap, and the
  synthetic-sample recovery do not exist.
- **Suggestion:** Contained in net.rs: the sampling function in
  `NetInterpolateSystem` gains the extrapolate/cap branches; `apply_states`'s
  per-entity push gains the dry-recovery synthetic sample (it needs the
  entity's `Transform` — query it alongside `NetBuffer`, mirroring how the old
  code read `(&mut NetLerp, &Transform)` in one view, net.rs:646).
- **Path:**
  1. Fail-first test `extrapolation_bridges_lost_snapshots_then_caps` in
     net.rs `mod tests`, same deterministic harness as step 1's test: entity
     moving +X at 6 u/s, samples at ticks 6 and 12 delivered on time, ticks 18
     and 24 NOT delivered, tick 30 delivered at its natural client tick.
     Assert (a) after the cursor passes tick 12 the entity keeps advancing —
     per-tick steps stay within `[0.5, 1.5] * 0.1` throughout the dry window
     (fails on step 1's hold: zero-steps), (b) across the arrival of tick 30
     the maximum single-tick step stays `< 2 * 0.1` (no pop — fails without
     the synthetic sample), (c) a second phase that stops delivering entirely
     after tick 30: the entity advances at most
     `EXTRAP_CAP_TICKS / 60 * 6.0 + tolerance` beyond its tick-30 sample
     position and its position is bit-identical across the final 30 ticks
     (capped, then held).
  2. Implement the extrapolate/cap branches and the synthetic-sample recovery.
  3. Assert `NetMotion` in the same test: non-zero during the dry window
     (animation keeps running), zero once capped.
  4. Green gate: `cargo test --workspace`, zero new warnings.

### 3. Rendered-position smoothness probe under WAN impairment, recorded in BASELINE.md

- **Evidence:** The loss probes (`server/vordar-server/tests/loss.rs`) measure
  *arrival* gaps and intent-ack lag only — nothing anywhere measures what a
  player sees: the per-tick rendered motion of a remote entity under loss and
  jitter. BASELINE.md:225-249 records post-rework-3 arrival gaps of up to
  ~300 ms at 5 % loss; under the pre-rework-4 client every such gap was a
  visible freeze plus a catch-up warble, and no test would catch a regression
  that reintroduces one. The client's e2e tests in
  `client/vordar-client/src/net.rs` already prove the pattern this probe
  needs: a real `vordar_server::build_server_app` on a port, a real impaired
  `NetClient`, and the real client systems driven directly
  (`kicked_connection_reconnects_and_relogs_in`,
  `onslaught_dash_replay_never_snaps_at_150ms_rtt`, net.rs:1459-1771).
- **Ideal:** An `#[ignore]`d probe test in net.rs `mod tests` (run like the
  loss probes: `cargo test -p vordar-client --release -- --ignored
  --nocapture`), `remote_render_smoothness_under_loss_probe`: a headless
  server; a "mover" — a second raw `NetClient` (the kicker pattern,
  net.rs:1509) that logs in and streams `ClientMsg::MoveIntents` datagrams
  walking ±X at 6 u/s, reversing every ~2 s to stay in AOI; an "observer"
  built like the onslaught test's world (prefab registry + real
  `NetReceiveSystem` + `NetInterpolateSystem`, `predict: false`) connected via
  `NetClient::connect_impaired` with `Impairment { rtt: 100 ms, jitter: 30 ms,
  downstream_loss: 0.03, .. }`. Over a ≥ 20 s window it records the mover
  entity's `Transform.position` after every Update tick and asserts the
  permanent regression gates: (a) the longest run of consecutive zero-motion
  ticks (step < 1e-4) is ≤ 5 ticks (~83 ms — covers a direction-flip tick,
  nothing else; the pre-rework client freezes 10–18 ticks at every late/lost
  snapshot), and (b) p99 per-tick step ≤ 1.5 × nominal (0.15 u; the
  pre-rework client's catch-up steps run ~2×). It prints the step
  distribution (p50/p99/max, longest zero-run) and BASELINE.md gains a
  "Remote render smoothness" subsection under the loss-probe section
  recording the numbers, citing BASELINE.md:206-249's arrival-gap rows as the
  before-evidence (the freezes those gaps implied are what steps 1–2 removed;
  the fail-first proofs live in steps 1–2's unit tests since this probe lands
  after the fix).
- **Gap:** No such probe exists; smoothness claims are untested end-to-end and
  unrecorded.
- **Suggestion:** Follow loss.rs's structure (window loop, printed
  percentiles, assertions as permanent gates) and the onslaught test's
  observer setup verbatim — no new harness machinery. Use a dedicated port
  (e.g. 25404) per the existing per-test port convention. Impairment loss is
  LCG-deterministic (engine-net `impair.rs`), and the gates have ≥ 2× margin
  against expected post-fix behavior (no dry-out at all until 2+ consecutive
  losses, which interpolation+extrapolation covers to ~450 ms), so the probe
  is stable.
- **Path:**
  1. Write the probe test as above; run it `--release --ignored --nocapture`
     and confirm both gates pass with margin.
  2. Sanity-check the gates are real: temporarily set
     `INTERP_DELAY_TICKS = 0.0` locally (do not commit) and confirm the probe
     FAILS — proof the assertion actually detects the freeze/warble regime —
     then restore.
  3. Record the printed numbers in `docs/benchmarks/BASELINE.md` as a new
     subsection under the packet-loss probe section, stating gates and the run
     command.
  4. Green gate: `cargo test --workspace` (probe excluded by `#[ignore]`),
     plus one documented probe run.

### 4. online-play diagram: the client renders remotes on a delayed playback clock

- **Evidence:** `docs/online-play.mmd:19` — node R2 reads "interpolate remote
  entities<br/>between snapshot positions", describing the deleted
  one-interval lerp; the queue note in
  `docs/reviews/networking/reworks-networking-2026-07-11.md` requires every plan from
  this queue to update `docs/online-play.mmd` + SVG when it changes the
  online-play flow (the diagram was converged by audit finding 19).
  `docs/online-play.svg` is the rendered copy; `scripts/render-mmd.sh` renders
  every `.mmd` via `npx -y @mermaid-js/mermaid-cli`.
- **Ideal:** R2 describes the new mechanism, e.g. "buffer snapshot positions
  by tick;<br/>render remotes at a fixed ~200 ms delay,<br/>brief capped
  extrapolation on loss" (exact wording per the mermaid-diagrams skill's
  technology-agnostic style, matching the diagram's existing voice), and
  `docs/online-play.svg` is regenerated so the two never disagree.
- **Gap:** The diagram still describes the pre-rework client once steps 1–2
  land.
- **Suggestion:** Load the `mermaid-diagrams` skill before editing (repo
  convention, see plan-networking-rework-5 step 5); touch only the R2 node
  text — no structural changes, the receive-flow shape (R1→R2→R3…) is still
  correct.
- **Path:**
  1. Edit `docs/online-play.mmd` node R2 as above.
  2. Regenerate `docs/online-play.svg`
     (`npx -y @mermaid-js/mermaid-cli -i docs/online-play.mmd -o
     docs/online-play.svg -b white`, or `scripts/render-mmd.sh`), verify the
     SVG renders and parses.
  3. The proof for a docs step is the render succeeding and the mmd parsing
     cleanly; no code test applies. Green gate: `cargo test --workspace`
     untouched and still green.
