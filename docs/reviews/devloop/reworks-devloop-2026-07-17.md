# Dev-Loop Audit (Reworks) — 2026-07-17

Rework-scale companion to `audit-devloop-2026-07-17.md`: findings that need a
design pass before anyone writes code. Consumed by /plan-rework.

## Ideal end state

The user's attention during a multi-rework campaign is spent exactly twice
per domain: approving the queue at audit time, and feel-checking the result —
everything between (plan, loop, commit, strike, token report) runs off the
queue file as the contract, with the user free to interrupt at any commit
boundary.

## Findings (implementation order)

Cross-type queue (mirrored verbatim from `audit-devloop-2026-07-17.md`):

> **~~finding 1 (user-decides — ask at loop launch) → finding 2 → finding 3 →
> finding 4 → finding 5 → finding 6 → finding 7 → finding 8 → finding 9
> (micro) → finding 10~~ → ~~rework 1 (user-decides; after finding 5: a queue
> runner must embed the planner-fallback convention finding 5 writes)~~.**
> Findings 1–10 done 2026-07-17 (1–5, 7: pipeline rules, pushed to
> ClaudeConfig; 6: allowlist content-keying; 8: measurement contradicted the
> stated failure mode — fixed the real cause, filed rework 2; 9: micro, applied
> inline; 10: dedicated sessions declined — plugin disabled, measurement folded
> into the next loop).
> Rework 1 done 2026-07-17 (plan-devloop-rework-1-2026-07-17.md, 3 steps; pause-on-plan queue runner: a run-queue skill chains plan-rework + implement-finding by invocation, rework-planner pinned to fable so an opus queue session cannot downgrade plans).
>
> Findings 1–4 lead on the token axis (orchestrator tier, worker read
> channels, silent-breakage prevention, mega-step shape). 5 is the
> attention-axis fix for the measured 45-minute stall. 6–9 remove small
> recurring frictions; 10 is wall-only. Rework 1 goes last because it
> packages conventions the fixes establish.

Rework 2 was filed 2026-07-17 during finding 8's implementation and is not in
the queue above. Its plan ran to completion 2026-07-17
(`plan-devloop-rework-2-2026-07-17.md`, 8 steps); its Ideal is now reached and
it is STRUCK 2026-07-17, closed by rework 5
(`plan-devloop-rework-5-2026-07-17.md`, finding 4): `scheduled_aoe`'s dodge
assert gained a bot-cadence precondition
(`server/vordar-server/tests/e2e_combat.rs:104`) alongside `sim_rate`, and
sensitive-set x10 at -Load 3.0 held it green in all 10 runs — see the
close-out note on finding 5 below for the full record. Rework 3 (filed
2026-07-17 by the finding-8 proof step, carrying the two client-prediction
SNAP_DISTANCE failures) is STRUCK 2026-07-17: neither
`onslaught_dash_replay_never_snaps_at_150ms_rtt` nor
`predicted_wall_hug_never_snaps_at_150ms_rtt` reproduced a snap across
rework 3's own 20-run attribution pass or finding 5's 10-run sensitive-set
proof plus a clean 404/404 full-suite run at -Load 3.0 — see rework 3's
section below for the full record.

### 1. (user-decides) A queue-runner convention: one launch decision per report instead of one prompt per rework

- **Evidence:** the rendering campaign consumed six near-identical user
  prompts ("yes, plan rework N and loop it" — 2026-07-16, six times), each
  triggering the same orchestrator sequence: /plan-rework N → show plan →
  loop /implement-finding over its steps → commit each → strike the queue
  note in both report files → report loop tokens. The sequence's steps are
  all already individually codified (plan-rework/SKILL.md,
  implement-finding/SKILL.md's loop behavior, the reworks-queue-mark-done
  convention); only the chaining is manual. The user's one real mid-campaign
  intervention ("stop after finishing step 6 and commiting") was a stop
  order — which a runner honors at any commit boundary anyway.
- **Ideal:** "run the reworks queue" is a single instruction: the
  orchestrator walks the report's cross-type queue in order — plan each
  rework, batch any (user-decides) questions at launch per the existing
  convention, loop the steps, commit, strike, report tokens per loop — and
  stops at any blocker, plan that surfaces a product question, or user
  interrupt.
- **Gap:** ~6 attention round-trips per campaign that carry no decision
  content (every one was "yes"), plus the latency of the user noticing each
  loop finished.
- **Tradeoffs:** *Wins:* user attention drops to the launch decision plus
  genuine blockers; between-rework idle gaps disappear from campaign wall;
  the conventions this campaign applied ad hoc become one written sequence
  instead of orchestrator memory. *Losses:* the per-rework checkpoint dies —
  the user currently reads each plan before its loop starts, and a runner
  would start looping unread plans (mitigation the design pass must weigh:
  pause-on-plan as a mode, or trust + interrupt); a runaway failure mid-queue
  compounds across reworks before the user looks; longer unattended runs
  raise compaction pressure in the orchestrator session (this campaign
  compacted twice attended). This changes who holds the campaign's reins,
  hence user-decides at plan time, not just at implementation.
- **Suggestion:** /plan-rework this only after fixes finding 5 lands (the
  runner must embed the planner-fallback playbook) and finding 1's
  orchestrator-tier decision is made (a runner amplifies whatever tier the
  orchestrator runs on). The plan should decide: skill shape (extend
  implement-finding vs a new run-reworks skill), stop conditions, and how
  loop token reports aggregate.
- **DECIDED 2026-07-17: adopted, pause-on-plan mode.** The runner plans a
  rework, shows the plan, and waits for one go/stop before looping it — the
  per-plan checkpoint survives; everything after approval (loop, commit,
  strike, token report, advance to next rework) is automatic. The
  fully-autonomous mode was not chosen; the design pass plans pause-on-plan
  as THE behavior, not a mode flag. Finding 1's tier decision also landed
  (opus for loops), so only fixes finding 5 remains as gate.
- **Path:** gate: fixes finding 5 landed (finding 1 decided 2026-07-17:
  opus). Then /plan-rework with the measured target: the next multi-rework
  campaign completes with one plan-approval word per rework and zero other
  content-free prompts (vs 6 full prompts this campaign) at unchanged
  commit/gate discipline.

### 2. The e2e bots drive wall-clock control loops against a server sim that CPU load can time-starve

Filed 2026-07-17 by the finding-worker implementing fixes finding 8, whose
stated failure mode the measurement contradicted. Finding 8's surgical part
landed (see below); this is the rework-scale remainder.

- **Evidence:** fixes finding 8 recorded `rend_kills_camped_enemy`'s flake as
  "its closed-loop bot missing a wall-clock kill deadline under CPU load", and
  prescribed widening that deadline from a measured worst case. Reproduced
  under CPU oversubscription, the test never missed the deadline: every
  failure fired `"the bot must survive the fight"`
  (`server/vordar-server/tests/e2e_combat.rs:169`) at 8.4-13.7s wall against a
  25s deadline. The bot *died*; the deadline had >2x headroom in every failing
  run, so the prescribed fix was inert. Root cause measured by tracing the
  bot's observed distance: snapshots arrive ~100ms apart under load, and
  `movement_velocity` (`game/vordar-game/src/player/mod.rs:29-32`) normalizes
  intents, so the bot only ever moves at a fixed 6.0 u/s — ~0.6u of position
  uncertainty per update, wider than any hold-station band that fits between
  the 1.0 contact boundary and rend's 2.5 `max_range` (a 1.5u window). The bot
  oscillated across contact, re-billing 10 damage per new overlap
  (`CollisionStarted` is edge-triggered, re-armed each cycle by
  `SeparationSystem`) until its 100 HP ran out.
- **Ideal:** an e2e bot's verdict depends on simulated progress, not on the
  harness winning a wall-clock race the host's scheduler can revoke — so a
  loaded machine makes tests slower, never red.
- **Gap:** at ~3x CPU oversubscription the *server sim itself* time-starves:
  a grunt needs 12s of wall clock to cross ~4 units, and every bot control law
  measured (original kite band 1.6-2.2; a widened 1.9-2.4 band; stand-at-2.0;
  stand-at-3.0/6.0) fails indistinguishably, ~1 in 5. The landed fix removes
  the *self-inflicted* oscillation and is clean 5/5 idle and 5/5 at 20-way
  oversubscription, but nothing test-internal survives a sim starved that
  deep. The 2026-07-15 decision "future flakes get fixed via test-internal
  budget tuning, not scheduling" was recorded against the wrong failure mode
  and cannot generalize: budget tuning has no lever once the sim is the thing
  running slow.
- **Tradeoffs:** *Wins:* removes a whole class of load-induced red from the
  e2e suite; makes exclusive scheduling (6.45s/run) justifiable on its own
  terms rather than as flake insurance; the same fragility latently affects
  every closed-loop bot test, not just this one. *Losses:* touching how
  `test-support`'s bots observe time is a harness-wide change with real
  regression surface across ~30 e2e tests; a sim-time gate can mask a genuine
  wall-clock performance regression, so the deadline assert must survive in
  some form; the payoff is bounded — the residual flake is rare under the
  isolation already in place.
- **Suggestion:** /plan-rework. The design pass should decide whether the bots
  gate on server tick/`WorldTime` progress instead of `Instant::now()`
  (deadlines become "N sim-seconds of rends", immune to host scheduling), or
  whether the harness detects a starved sim and skips rather than fails. It
  should also settle whether tests that cannot state a sim-time budget belong
  in the default suite at all.
- **Path:** (1) inventory the closed-loop bot tests and which of them race a
  wall clock; (2) decide the sim-time-vs-skip mechanism above; (3) land it
  behind the existing exclusive group so scheduling can be relaxed only if the
  mechanism proves out; proof: the suite stays green at 3x CPU
  oversubscription, where every control law measured today fails ~1 in 5.

### 3. Two client-prediction e2e tests fail their SNAP_DISTANCE guarantee under real 3x CPU oversubscription

Filed 2026-07-17 by the finding-worker landing fixes finding 8 (the
3x-oversubscription proof). The Path expected "25/25 green" from the
sensitive-set combined run; measurement instead found a reproducible real
failure, distinct from every failure mode finding 8's mechanisms (sim-tick
budgets, 8x wall backstop, scheduled_aoe's 0.9 sim-rate precondition) were
built to tolerate.

- **Evidence:** `powershell scripts/stress-suite.ps1 -Load 3.0` (60 spinners
  on this 20-logical-core machine) run against the sensitive-set filter 4x5
  and against the full workspace suite 1x403 reproducibly fails
  `net::e2e::onslaught_dash_replay_never_snaps_at_150ms_rtt` and
  `net::e2e::predicted_wall_hug_never_snaps_at_150ms_rtt`
  (`client/vordar-client/src/net/e2e.rs:288` and `:437`) with their own stated
  assertion, not a budget or backstop message: "reconciliation snapped 6.00
  units mid-dash" (observed 3x, always exactly the full leap distance — i.e.
  zero incremental correction landed before the snap) and "reconciliation
  snapped 1.44-1.94 units walking into the wall" (observed 3x). Combined-set
  batches: 4/5 green, 5/5 green, 0/5 green, 0/5 green (20 runs, 9 green);
  full-suite run: 356/358 attempted passed, these 2 failed. `scheduled_aoe`
  also failed twice more in this same evidence gathering, once as "sim budget
  exhausted waiting for A gets MechanicScheduled" (`testing/test-support/src/bot.rs:65`)
  and twice as its dodge-window miss assert firing *despite* the 0.9 sim-rate
  precondition reading pass (`server/vordar-server/tests/e2e_combat.rs:91`) —
  a second, separate signal that the precondition's own sim-rate measurement
  can read healthy while the underlying race still loses. Both failing
  prediction tests drive their client-side systems in a hand-rolled loop
  (`client/vordar-client/src/net/e2e.rs:280-286`, `:427-434`) that advances a
  local `elapsed` counter by a fixed `DT = 1/60.0` per iteration and bounds
  the loop only with a wall-clock deadline (`Instant::now() < dash_deadline`)
  — under real CPU contention each `sleep(16ms)` can take far longer than
  16ms in wall time while `elapsed` still advances by the fixed step, so the
  loop's real duration balloons past what the local prediction assumes while
  the real server (and the injected 150ms latency) keep moving on actual wall
  time; this is a plausible mechanism for a snap this large but is not
  confirmed — no root-cause tracing was done, per the execution-tier scope
  that filed this finding instead of chasing it.
- **Ideal:** every test in the sensitive/full-suite gate is green at 3x CPU
  oversubscription, or it fails with a message that unambiguously means "this
  is a genuine game-logic bug" — never leaves an open question of whether the
  test's own driving loop (rather than the production reconciliation code) is
  what falls over under load.
- **Gap:** unknown whether `PredictedStaticCollisionSystem` / leap-aware
  replay genuinely violates SNAP_DISTANCE under real client-side stalls, or
  whether these two tests' fixed-DT hand-rolled loops (a different, older
  pattern than the `Bot`/`SimDeadline` mechanism finding 8 built) are
  themselves the thing that falls over under load — same root shape as rework
  2 above (a wall-clock-paced driver racing a sim that CPU load can starve),
  but in test-local ECS-loop code, not `test-support`.
- **Suggestion:** /plan-rework. Root-cause first: reproduce
  `onslaught_dash_replay_never_snaps_at_150ms_rtt` under load with
  instrumentation on `max_recv_jump`'s growth curve (does it climb gradually
  then jump, or arrive as one shot) to tell "test loop lost track of real
  time" from "production code doesn't reconcile incrementally under load".
  If the loop is at fault, replace the fixed-DT/wall-deadline pattern with
  real elapsed-time-driven stepping (or borrow `SimDeadline`'s sim-vs-wall
  split) the same way rework 2 fixed the server-bot side. If production code
  is at fault, that is a real correctness bug in the prediction/reconciliation
  path and belongs in a networking or rendering finding, not a test fix.
- **Path:** (1) reproduce with `RUST_LOG` and a temporary jump-curve trace
  under `stress-suite.ps1 -Load 3.0`, run until captured (observed rate here:
  roughly 1 in 4-5 of the two tests' individual attempts); (2) attribute per
  the Suggestion; (3) fix at the attributed layer; (4) proof: sensitive-set
  x10 and one full-suite run green at `-Load 3.0` on this machine.

ATTRIBUTED 2026-07-17: `onslaught_dash_replay_never_snaps_at_150ms_rtt` and
`predicted_wall_hug_never_snaps_at_150ms_rtt` gained a test-local `TraceRing`
(`client/vordar-client/src/net/e2e.rs`) that records per-iteration context
(wall time, sim `elapsed`, position, signed/magnitude recv jump, pending
queue depth and leap-tagged count, `latest_state_tick`, seq/acked, telegraph
count) and dumps it on any snap-sized jump. 4 batches x 5 runs (20 total) of
`stress-suite.ps1 -Load 3.0` against both prediction tests plus
`scheduled_aoe`, `rend_kills_camped_enemy`, and
`kicked_connection_reconnects_and_relogs_in` reproduced **no snap** in
either prediction test — `max_recv_jump` never reached `SNAP_DISTANCE`, so
the ring's dump path never fired and none of the three candidate mechanisms
(A: cast refused, B: suppression hole, C: burst-drop) has any trace data to
attribute against this run. One unrelated failure was captured (batch 1, run
4/5): `scheduled_aoe` panicked at `server/vordar-server/tests/e2e_combat.rs:91`
("B stepped out before T — the rewound test must miss it") — the same
wall-contract miss already tracked elsewhere in this report, not a
prediction-test snap. Per the Path's own fallback for a 20-run miss, steps
2-4 remain justified as written (mechanism B's suppression-hole guard is
deterministic regardless of reproduction; the A/C guards make this report's
stated premises explicit) and the plan proceeds without a further fix from
this step.

STRUCK 2026-07-17 (`plan-devloop-rework-3-2026-07-17.md`, finding 5): the
proof bar from this finding's own Path step 4 — "sensitive-set x10 and one
full-suite run green at -Load 3.0" — is met. Sensitive-set x10 at -Load 3.0
(60 spinners, 20-logical-core machine) held
`onslaught_dash_replay_never_snaps_at_150ms_rtt` and
`predicted_wall_hug_never_snaps_at_150ms_rtt` green in all 10 runs (9/10
runs fully green across all 5 tests; run 9's lone failure was
`scheduled_aoe`'s dodge assert, unrelated to this rework). The full 404-test
suite ran once at -Load 3.0 clean: 404 passed, 0 failed. An idle full gate
afterward (`cargo nextest run --workspace` + `cargo test --doc --workspace`)
held 404/404. No calibration contingency was needed — neither prediction
test reddened in any run. This proof pass also found and fixed a
pre-existing bug in `.config/nextest.toml`'s exclusive-override filter: both
`kicked_connection_reconnects_and_relogs_in` and
`onslaught_dash_replay_never_snaps_at_150ms_rtt` only exist as
`net::e2e::<name>`, and nextest's `test(=NAME)` exact-match requires the
fully-qualified name — the bare names in the filter matched zero tests
(confirmed via `cargo nextest list`), so both tests silently ran in the
capped 'realtime' group instead of exclusively. Fixed by qualifying both
names.

### 4. Finding 4's own step-2 calibration mechanism (predicate/const threshold tweak) can't produce a healthy-context snap in either prediction e2e test

- **Evidence:** `client/vordar-client/src/net/prediction.rs:19` (`SNAP_DISTANCE`)
  and `:91-102` (`classify_error`'s call site in `reconcile_own`):
  `Correction::Trust` is checked before `Correction::Snap`
  (`classify_error`, `:148-156`), so any reconciliation error under
  `TRUST_DISTANCE` (0.3, `:15`) never reaches the Snap branch regardless of
  `SNAP_DISTANCE`'s value; `reconcile_own` only writes `transform.position`
  on that Snap branch — `Trust`/`Smooth` never touch it within a single
  `NetReceiveSystem::run` call. Measured directly: lowering the
  `TraceRing::record` predicate's threshold to `0.01` (reverted) produced 0
  recorded snap events across a full `onslaught_dash_replay_never_snaps_at_150ms_rtt`
  run; lowering the actual `SNAP_DISTANCE` const to `0.01` (reverted) also
  produced 0 events — in both cases because this test's deterministic
  fixed-latency conditions (no jitter, no loss) keep the real reconciliation
  error under `TRUST_DISTANCE` for the whole run.
- **Ideal:** finding 4's step-2 calibration proves "the healthy path retains
  teeth" — that a genuine healthy-context snap-sized jump fails the
  never-snap assert — using a mechanism that actually produces such a jump
  against this codebase's real reconciliation mechanics.
- **Gap:** the literal instruction ("lower SNAP_DISTANCE's use in the event
  predicate to 0.01") assumes ordinary sub-SNAP_DISTANCE jumps occur during
  idle play; they don't — `transform.position` is binary (untouched, or
  snapped) within `recv.run()`, so no intermediate signal exists to
  threshold against with either lever.
- **Suggestion:** use a direct, local-only position perturbation instead:
  offset the predicted entity's `Transform.position` once (outside
  `recv.run()`), then let the next real snapshot's `reconcile_own` see the
  genuine, un-self-correcting divergence from the server's untouched
  authoritative position. Do this in `predicted_wall_hug_never_snaps_at_150ms_rtt`,
  not the dash test — the dash's `LeapImpulse` velocity is solved to reach
  an absolute world-space target regardless of start position
  (`leap_velocity(origin, target, cast_secs)`), so a start-position
  perturbation there converges back to the same landing point and never
  manifests as a reconciliation error.
- **Path:** already executed as part of finding 4's implementation
  (2026-07-17): perturbed `predicted_wall_hug_never_snaps_at_150ms_rtt`'s
  entity by `Vec3::new(5.0, 0.0, 0.0)` right after entity-spawn, confirmed
  the test FAILED with the healthy-context message and a dumped
  `SnapEvent { degraded: false, jump_mag: 5.0, ... }`, then reverted the
  perturbation before landing the real fix. No further action needed — this
  is a record of the calibration mechanism actually used, for any future
  rework touching these tests' teeth-check.

### 5. `scheduled_aoe`'s dodge-window sim-rate precondition measures the wrong party — the bot's own send loop can starve while the precondition reads healthy

- **Evidence:** `server/vordar-server/tests/e2e_combat.rs:62-100` — the dodge
  scenario (cast 2) walks bot B east through a wall-clock-paced loop
  (`while ... { ... std::thread::sleep(Duration::from_millis(16));
  a.pump(); b.pump(); }`, `:67-78`) bounded only by
  `b.client.server_now_micros()` reads against the mechanic's `resolve_at`.
  The `DODGE_SIM_RATE_MIN = 0.9` precondition (`:86-90`, landed 2026-07-17 by
  rework 2 step 5) computes `sim_rate` from `(b.latest_state_tick - tick0) /
  (wall0.elapsed() * TICK_HZ)` — purely the SERVER's tick advancement over
  wall time. It gates the miss assert on the SERVER's sim health, not on
  whether bot B's own `send_move` calls inside this loop kept pace with
  `resolve_at`'s wall-clock deadline. Rework 3's finding 3 evidence-gathering
  (2026-07-17) observed the dodge assert fire twice with this precondition
  reading pass, and finding 5's proof pass
  (`plan-devloop-rework-3-2026-07-17.md`) reproduced it a third time (1/10
  sensitive-set runs at `-Load 3.0`) — "B stepped out before T — the rewound
  test must miss it" (`:93`) firing despite `sim_rate >= 0.9`. Rework 3's own
  plan named the suspected mechanism without confirming it: "the starving
  party is the bot's own thread, which rework 2's server-tick-anchored
  SimDeadline cannot see" — the `Bot`/`SimDeadline` mechanism rework 2
  landed measures server-tick pacing for the bot's WAIT conditions, not this
  loop's own iteration cadence.
- **Ideal:** the dodge assert's precondition measures whatever party can
  actually cause a false miss-assert — if that's bot B's own send-loop
  cadence under CPU load, the precondition (or an equivalent skip/park
  mechanism) detects that directly, the same way `WireHealth` in the
  prediction tests measures the wire's own gaps instead of a proxy.
- **Gap:** `sim_rate` is a proxy for server health; it says nothing about
  whether B's 16ms-sleep loop actually sent enough `send_move` calls before
  `resolve_at` under CPU contention. Root cause is not confirmed — only
  suspected — per rework 3's execution-tier scope, which explicitly declined
  to chase it.
- **Suggestion:** /plan-rework. Root-cause first: reproduce under
  `stress-suite.ps1 -Load 3.0` with instrumentation on the dodge loop's own
  iteration timing (wall gap between successive `send_move` calls, and how
  many actually landed before `resolve_at`) to confirm or rule out the
  bot-thread-starvation mechanism against the recorded failures. If
  confirmed, extend the precondition (or the loop itself) to measure the
  bot's own cadence directly, mirroring the `WireHealth`-style "measure the
  thing itself" pattern rework 3 used for the prediction tests, rather than
  widening `DODGE_SIM_RATE_MIN` or adding sleep-based padding. If the trace
  shows the bot's loop kept pace and the server still resolved the miss
  wrong, that is a genuine scheduled-cast rewind bug, not a harness fix.
- **Path:** (1) instrument the dodge loop's per-iteration timing and
  reproduce under `-Load 3.0` (historical rate: ~1/10 to ~1/20 runs) until
  at least one failure with the precondition reading healthy is captured;
  (2) attribute per the Suggestion; (3) fix at the attributed layer;
  (4) proof: this rework's own bar is rework 2's outstanding gap —
  sensitive-set x10 at `-Load 3.0` green on `scheduled_aoe`, closing rework 2
  on the recorded evidence in this file's intro paragraph.

**ATTRIBUTED 2026-07-17 (Case A):** Stress suite at `-Load 3.0` ran 30 total runs
(6 batches of 5) with the established 5-test sensitive set. 3 `scheduled_aoe`
dodge-assert failures were captured (batches 2/4, 3/5, 6/3):
- Batch 2, Run 4: `sends_pre_t=15`, no sends for first 1018ms into 2200ms trace window
- Batch 3, Run 5: `sends_pre_t=20`, no sends for first 1048ms into ~1900ms window  
- Batch 6, Run 3: `sends_pre_t=26`, iteration wall gaps up to 159ms at trace start

All three fired despite `sim_rate >= 0.9` (readings: 0.93, 0.99, 0.92 respectively),
with `sends_pre_t` far below the physics bound of 41. **Bot-thread starvation
CONFIRMED**: the bot's 16ms-sleep dodge loop cannot maintain cadence under 3x CPU
oversubscription, collapsing the send queue. Per finding 2's Path case A routing,
proceeding to step 3: implement the pre-T send counter as a second precondition
alongside `sim_rate` to gate the miss assert and prevent false red.

CLOSED 2026-07-17 (`plan-devloop-rework-5-2026-07-17.md`, finding 4): the
proof bar from this finding's own Path step 4 — "sensitive-set x10 at
-Load 3.0 green on `scheduled_aoe`" — is met, closing rework 2 (see the intro
paragraph above). Sensitive-set x10 at -Load 3.0 (60 spinners,
20-logical-core machine, `--no-fail-fast` so each run actually reached
`scheduled_aoe` regardless of an earlier failure in the same invocation —
nextest's default fail-fast otherwise cancelled 4/10 runs before
`scheduled_aoe` got to execute) held `scheduled_aoe` green in all 10 runs. A
focused x10 rerun of `scheduled_aoe` alone with `--success-output final`
recorded the branch split: 0/10 took the assert branch, 10/10 took the skip
branch (`sends_pre_t` 19-24 against the 45 minimum, `sim_rate` 0.95-1.01 —
both preconditions read healthy every run). Skips dominate completely at
this load; recorded as evidence for the catch-up follow-up named in the
plan's Design decisions, not implemented here. `rend_kills_camped_enemy`
failed 3/10 on its own pre-existing "the bot must survive the fight" assert
(`server/vordar-server/tests/e2e_combat.rs:216`) — unrelated to this rework
and outside its fix, recorded per the Path, not chased. An idle full gate
afterward (`cargo nextest run --workspace` + `cargo test --doc --workspace`)
held 404/404 with doc-tests clean.

## Tracked observations (not yet plannable)

- **`rend_kills_camped_enemy` failed 3/10 in rework 5 finding 4's proof
  (2026-07-17)**, on its own pre-existing "the bot must survive the fight"
  assert (`server/vordar-server/tests/e2e_combat.rs:216`) — unrelated to that
  rework and not chased there. This is the first red this test has shown at
  3x load all session: it held 5/5 idle and 5/5 at 20-way oversubscription
  when rework 2 landed its fix, and "in every run it reached" during rework
  3 finding 5's proof. 3/10 is too thin a sample to tell a real regression
  from noise, and no attribution pass has been run (no trace on what the bot
  observed before it died, unlike the trace-first treatment `scheduled_aoe`
  and the prediction tests got). Not filed as a plannable finding yet —
  watch for a repeat in a future proof run before spending a design pass on
  it; if it recurs, root-cause first the same way finding 5 did for
  `scheduled_aoe`.

## Carried forward from previous report

None — `reworks-devloop-2026-07-15.md`'s single rework (parallel execution)
was declined 2026-07-15 with a standing do-not-refile condition (token spend
outranks wall time), re-verified this audit: the condition still holds. The
superseded report is deleted; this note preserves the decision.

## Resolved since last report

Rework 1 of 2026-07-15 (parallel execution): declined, condition standing —
recorded above and in the new audit file's Resolved section.
