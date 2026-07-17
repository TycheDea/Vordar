# Plan: Two client-prediction e2e tests fail their SNAP_DISTANCE guarantee under real 3x CPU oversubscription — 2026-07-17

Source: docs/reviews/devloop/reworks-devloop-2026-07-17.md finding 3.

## Ideal end state

`onslaught_dash_replay_never_snaps_at_150ms_rtt` and
`predicted_wall_hug_never_snaps_at_150ms_rtt` are green at
`stress-suite.ps1 -Load 3.0` run combined with the sensitive set and the full
suite, and any future failure of either carries a self-attributing dump that
says which layer broke: the production reconciliation path, or the wire the
test actually experienced. One genuine production hole found during design —
the dash-suppression predicate in `reconcile_own` ignores a still-running
local `LeapImpulse` — is closed with its own deterministic fail-first test.
`SNAP_DISTANCE` is untouched. The proof runs produce the evidence on which
rework 2 closes or stays open.

## Design decisions

**The central attribution question — test loops vs production code — has a
code-grounded answer that is "neither, exactly," and the plan pre-commits
fixes for all three mechanisms the code admits.** Reading the full seam
(`client/vordar-client/src/net/{e2e,prediction,apply,lifecycle}.rs`,
`server/vordar-server/src/net/receive.rs`, `smirk/engine-app/src/scheduler.rs`,
`game/vordar-game/src/combat/leap.rs`) yields exactly three candidate
mechanisms consistent with the finding's evidence:

- **Mechanism A — cast refused under a degraded wire (best fit for the dash
  test's "always exactly 6.00, zero incremental correction").** The server
  rejects any intent arriving more than `max(rtt, MAX_REWIND_MICROS=200ms) +
  ARRIVAL_MARGIN=100ms` ≈ 300 ms after its stamp
  (`receive.rs:validate_intent`, `dispatch_cast`). Under combined load the
  starved delivery path can exceed that: the `CastIntent` is rejected, the
  server never mirrors the dash, the client's predicted dash completes the
  full 6.00 units locally, and once its (zero-dir) move intents are acked the
  leap-tagged pending queue empties, suppression lifts, and reconciliation
  snaps the player back exactly the full leap distance. Production working as
  designed (optimistic cast, no cast-ack protocol; snap is the documented
  recovery); the test's premise ("a clean 150 ms wire") was broken by the
  environment, not by the reconciliation code.
- **Mechanism B — the dash-suppression predicate hole (deterministically
  provable today, production-reachable).** `reconcile_own`
  (`prediction.rs:65-85`) suppresses reconciliation only while a *pending*
  intent carries `leap: Some(..)`. Its comment assumes the server's dash
  mirror always finishes later than the local one — true for network delay,
  inverted by a client-side stall: a client that stops iterating for ≥ ~RTT
  sends no new intents, its outstanding dash intents all get acked, the
  predicate reads false while the local `LeapImpulse` is still active, and
  the next snapshot reconciles mid-dash. A plain unit test (active
  `LeapImpulse`, empty pending, server ahead) proves the snap without any
  load. This is a genuine prediction-path correctness gap (a ≥ RTT frame
  hitch mid-dash on a real client snaps on resume), and it is fixed here — in
  production code, with a fail-first test — not papered over in the test.
- **Mechanism C — late-intent burst-drop plus ack-prune (fits the wall-hug's
  1.44-1.94 u snaps ≈ 14-19 intents ≈ 240-320 ms of movement).** Intents
  arriving past the 300 ms deadline are rejected and never applied, but a
  later on-time intent advances `applied_seq`, and the client prunes pending
  by `seq > last_processed_seq` — the replay silently loses the dropped
  intents' movement and the accumulated error snaps back. Also production
  working as designed (`INTENT_QUEUE_CAP` comment: "the client re-converges
  via reconciliation"); again the test's wire premise broke.

**Divergence from the finding's Suggestion, stated per the wall-is-information
rule:** the fixed-DT hand-rolled loops are *not* the causal fault under any of
these mechanisms, so this plan does **not** rewrite them to elapsed-time /
accumulator stepping. The production scheduler
(`scheduler.rs:run_tick`) does catch up (8-step cap) where the test loop does
not, but none of the three snap mechanisms runs through that difference —
A and C are wire-degradation effects, and B fires on the first post-stall
receive *before* any catch-up movement could run, in production and test
alike. A pacing rewrite would be harness churn that fixes nothing. Step 1's
instrumentation still runs first (the finding mandates attribution before
fixes) and its decision rules include a park-and-report branch if the captured
traces match none of A/B/C.

**Why the production fix (B) lands in this rework instead of a networking
filing:** it is two lines plus a unit test, local to the prediction path this
finding already owns, and deterministically provable — filing it elsewhere and
waiting would leave this rework's Ideal unreachable if B is what fires under
the harness. The finding's "belongs in a networking finding, not a test fix"
is honored in substance: the fix is a production-correctness step proven by a
production-path test, not a test-side disguise. Conversely, A and C are
*designed* recovery behavior, so for them the tests' stated premise is made
explicit and measured instead of touching production thresholds —
`SNAP_DISTANCE` is not widened, per the finding's explicit bar.

**Premise guards measure the thing itself, never a proxy** — the lesson from
`scheduled_aoe`'s precondition failing sideways (it measured the server's sim
rate while the bot's own thread starved). The dash test's guard is the
server's actual acceptance signal (the `MechanicScheduled`-spawned
`TelegraphVisual` entity — reliable stream, exists iff the server accepted
the cast and mirrored the dash). The wire-health guard is built from three
directly observed gaps (ack-advance, snapshot-arrival, own-iteration stall),
each of which is a direct precondition of mechanisms A/C, with thresholds
derived from the server's own 300 ms arrival deadline.

**`scheduled_aoe` is out of this rework's fix scope.** Its residual failures
(dodge assert firing despite a healthy sim-rate precondition; a sim-budget
exhaustion) live in the `test-support` server-bot harness family that rework 2
owns — the starving party is the bot's own thread, which rework 2's
server-tick-anchored `SimDeadline` cannot see. The proof step records its
outcomes as rework-2 ledger evidence; this rework's pass bar is the two
prediction tests. Rework 2 closes only if the proof evidence supports its
Ideal end to end; otherwise it stays open with the new evidence appended.

**Other choices:** the failure-dump instrumentation is permanent, not
temporary — the Ideal requires failures that self-attribute. Nextest
scheduling (exclusive groups, realtime caps) is deliberately untouched:
exclusivity cannot help against external spinner load, and the idle suite is
already 403/403. No product questions are open; everything here is
engineering.

## Findings (execution order)

### 1. The two prediction e2e tests get a self-attributing trace, and the failure is captured and attributed under combined load

- **Evidence:** `client/vordar-client/src/net/e2e.rs:203-219` and `:370-384`
  — both tests fold every `NetReceiveSystem::run` into a single
  `max_recv_jump` float; when the assert at `:288`/`:437` fires, the message
  carries one number and nothing else. The finding's evidence (snap always
  exactly 6.00 mid-dash; 1.44-1.94 into the wall) was reconstructed from
  ad-hoc reruns, and the finding explicitly records that no root-cause
  tracing was done.
- **Ideal:** each test keeps a bounded ring of per-iteration diagnostics and,
  on any snap-sized jump, records a snap event with its full context; a
  failing assert dumps both, so a single failing run under load attributes
  itself against the three candidate mechanisms (A: cast refused; B:
  suppression hole; C: intent burst-drop) without a rerun.
- **Gap:** no per-iteration context exists; the observed failures cannot be
  attributed without new instrumentation, and the finding mandates
  attribution before any fix.
- **Suggestion:** in `e2e.rs`, give both tests a test-local `TraceRing`
  (VecDeque capped at ~600 entries, one entry per loop iteration) recording:
  wall millis since test start, sim `elapsed` (dash/walk loops only), own
  position, this-recv position jump as a *signed* component along +X plus
  magnitude, `NetClientState.pending.len()`, count of pending entries with
  `leap.is_some()`, whether the own entity currently has a
  `vordar_game::combat::LeapImpulse` (dash test), `latest_state_tick`, the
  last acked seq implied by pending pruning (track the last
  `last_processed_seq` seen — expose it by recording `state.pending.front()`
  seq or by keeping the previous `pending.len()`; simplest: record
  `state.seq` and `pending.len()` and derive acked = seq − pending), and (dash
  test) `world.query::<&crate::telegraph::TelegraphVisual>().iter().count()`.
  Additionally keep a `Vec` of snap events: any single-recv jump with
  magnitude > `SNAP_DISTANCE` stores (wall ms, signed jump, position before,
  the trailing 1.5 s of ring entries cloned). On assert failure, `eprintln!`
  the snap events and the last ~200 ring entries before panicking (nextest
  prints captured output for failed tests). The existing asserts and
  `max_recv_jump` stay exactly as they are in this step — behavior unchanged,
  only reporting added.
- **Path:**
  1. Implement the ring + snap-event capture in both tests
     (`onslaught_dash_replay_never_snaps_at_150ms_rtt`,
     `predicted_wall_hug_never_snaps_at_150ms_rtt`) inside their existing
     `run_input` closures; no production code changes.
  2. Gate green idle: `cargo check --workspace` (no new warnings), then
     `cargo nextest run --workspace` (403/403) and
     `cargo test --doc --workspace`.
  3. Capture: `powershell scripts/stress-suite.ps1 -Load 3.0 -Runs 5 -Filter
     'test(=onslaught_dash_replay_never_snaps_at_150ms_rtt) | test(=predicted_wall_hug_never_snaps_at_150ms_rtt) | test(=scheduled_aoe) | test(=rend_kills_camped_enemy) | test(=kicked_connection_reconnects_and_relogs_in)'`
     — repeat batches (up to 4, i.e. 20 runs total) until at least one
     failure of each prediction test is captured, or the batches are
     exhausted. Observed historical rate: ~1 in 4-5 attempts per test.
  4. Attribute each captured failure by these decision rules:
     - Dash test, jump *negative* along +X with magnitude ≈ the full 6.00 and
       telegraph count 0 → **mechanism A** (cast refused; server never
       dashed).
     - Dash test, jump *positive* along +X with `LeapImpulse` present and
       leap-tagged pending count 0 → **mechanism B** (suppression hole).
     - Wall-hug, jump negative along +X (pull-back) preceded within ~1 s by a
       stall/ack gap ≥ ~300 ms visible in the ring → **mechanism C**
       (burst-drop + ack-prune).
     - Anything else → **park**: append the dump verbatim to the finding-3
       addendum (next bullet) and stop the plan here, reporting that the
       enumerated mechanisms do not explain the failure.
  5. Record the verdict as a dated addendum paragraph at the end of finding
     3's section in `docs/reviews/devloop/reworks-devloop-2026-07-17.md`
     ("ATTRIBUTED 2026-MM-DD: …" naming which mechanisms were observed, with
     the key trace numbers). If 20 combined runs reproduce nothing, record
     exactly that — steps 2-4 remain justified (B is deterministic; A/C
     guards make stated premises explicit) and the plan proceeds.

### 2. `reconcile_own` suppresses reconciliation while the local dash is still running (production fix, fail-first)

- **Evidence:** `client/vordar-client/src/net/prediction.rs:65-85` —
  `still_reconciling_a_dash` is true only while some *pending* intent carries
  `leap: Some(..)`; the comment (`:68-74`) reasons from "the server's copy of
  the dash finishes strictly later than the local one", which network delay
  guarantees but a client-side stall inverts: a client that stops iterating
  for ≥ ~RTT sends nothing new, its outstanding dash-tagged intents all get
  acked, pending empties, and the next snapshot reconciles — and snaps —
  while the entity's `LeapImpulse`
  (`game/vordar-game/src/combat/leap.rs:20-24`) is still active. The
  suppression mechanism's stated purpose (never snap mid-dash) has a hole on
  exactly the client-stall side.
- **Ideal:** reconciliation is suppressed while *either* side of the dash is
  unfinished: any unacked leap-tagged pending intent (server not caught up)
  **or** an active local `LeapImpulse` (local dash not finished). A stalled
  client resumes its dash locally, and corrections wait until both sides are
  done — no mid-dash snap regardless of scheduling.
- **Gap:** the predicate checks only the pending side; the deterministic unit
  scenario (active `LeapImpulse`, empty pending, server ahead by more than
  `SNAP_DISTANCE`) snaps today.
- **Suggestion:** in `reconcile_own`, alongside the existing early reads of
  `Player`/`Hitbox` (before the `NetClientState` borrow), read
  `let local_dash_active = world.get::<&vordar_game::combat::LeapImpulse>(entity).is_ok();`
  and change the suppression return to
  `if still_reconciling_a_dash || local_dash_active { return; }`. Update the
  comment block to state the two-sided constraint (pending side covers the
  server finishing later; the local-impulse side covers a stalled client
  whose intents were all acked while its own dash still runs). `LeapImpulse`
  is already imported into the module's scope via `super::*`/existing usage —
  match whatever import shape `prediction.rs` already resolves
  (`vordar_game::combat::LeapImpulse` is used by `start_predicted_leap` at
  `:211`).
- **Path:**
  1. Write the failing test first, in `prediction.rs`'s `tests` module, named
     `reconcile_never_snaps_while_the_local_dash_is_still_running`: world
     with an entity carrying `Transform` at origin, `Player { speed: 6.0 }`,
     `Hitbox` (aabb half-extents 0.5, the existing `walker_shape()`), and
     `LeapImpulse { velocity: Vec3::new(15.0, 0.0, 0.0), remaining: 0.2 }`;
     resources with `PlayRadius` default and a `NetClientState` built like
     `reconcile_against_a_wall_stays_in_the_trust_band` does (`:406-411`) but
     with `pending` left **empty** (every intent acked). Call
     `reconcile_own(&mut world, &mut resources, entity, Vec3::new(6.0, 0.0, 0.0), 10)`
     and assert the entity's `Transform.position` is unchanged (still origin)
     and `state.correction == Vec3::ZERO`. Run it and confirm it FAILS
     against current code (position snapped to x=6.0). If it unexpectedly
     passes, stop: park this step and report — the hole analysis was wrong
     and the fix must not land.
  2. Apply the two-line fix + comment update; the test goes green.
  3. Check `client/vordar-client/src/net/bench.rs` (`reconcile_own` wrapper,
     `:56-63`): if its benchmark scenario inserts a `LeapImpulse` on the
     reconciled entity, the fix would turn that bench into an early-return
     no-op — in that case report it in the step summary (do not silently
     change the bench); if it doesn't (expected), nothing to do.
  4. Full gate: `cargo check --workspace` clean, `cargo nextest run
     --workspace` green (403 + 1 new), `cargo test --doc --workspace` green —
     in particular every existing prediction/e2e test must hold, since the
     fix only ever *adds* suppression while a local dash is active.

### 3. The dash test's strict assert is gated on the server actually accepting the cast

- **Evidence:** `client/vordar-client/src/net/e2e.rs:271-273` — the test
  sends the `CastIntent` and inserts the predicted leap, then asserts
  never-snap unconditionally at `:288`. Server-side,
  `server/vordar-server/src/net/receive.rs:618-645` (`validate_intent`)
  rejects any intent arriving later than `max(rtt, 200ms) + 100ms ≈ 300 ms`
  after its stamp, and `dispatch_cast` (`:244-262`) silently drops such a
  cast — the server then never mirrors the dash, making the client's
  completed 6.00-unit predicted dash a genuine full misprediction whose
  designed recovery IS the snap. On acceptance the server broadcasts
  `MechanicScheduled` (reliable stream), which the client turns into a
  `TelegraphVisual` entity
  (`client/vordar-client/src/net/lifecycle.rs:102-106`,
  `client/vordar-client/src/telegraph.rs:28-46`; the `telegraph` prefab
  exists in `content/prefabs/telegraph.ron`, which the test already loads via
  `insert_game_prefabs` + `load_dir("content/prefabs")`).
- **Ideal:** the never-snap contract is asserted only on runs where the
  server accepted the cast — measured by the acceptance signal itself
  (telegraph entity present), not a proxy. A refused cast (possible only on a
  degraded wire; never observed idle) prints a loud, unambiguous environment
  message and passes vacuously instead of reporting a reconciliation bug that
  isn't one.
- **Gap:** today a refused cast fails the test with "reconciliation snapped
  6.00 units mid-dash", indistinguishable from a genuine production bug.
- **Suggestion:** after the existing dash-and-settle loop (`:278-286`)
  finishes, count telegraphs:
  `world.query::<&crate::telegraph::TelegraphVisual>().iter().count()`. If
  ≥ 1 → run the strict assert exactly as today. If 0 → pump
  (`run_input`/`run_update`, 16 ms sleeps) for up to 5 more wall seconds for
  a late-delivered accept (reliable stream: delayed, never dropped); if one
  arrives, run the strict assert; if none arrives, `eprintln!` a clearly
  worded environment verdict ("onslaught cast was never accepted by the
  server — the wire degraded past the 300 ms intent arrival deadline; the
  never-snap contract is not evaluable this run") and `return` (vacuous
  pass). No retry-cast logic: a re-cast from the dashed-back position would
  need cooldown waits and target recomputation for near-zero added teeth.
  `TelegraphVisual` is `pub(crate)`; `net::e2e` is a child module of the
  crate, so the query compiles as-is. Do not touch
  `predicted_wall_hug_never_snaps_at_150ms_rtt` in this step.
- **Path:**
  1. Implement the acceptance gate as above; keep the step-1 trace/dump
     untouched (the telegraph count is already in the ring from step 1).
  2. Behavioral check that the gate cannot mask a real bug: temporarily (in
     the working tree only, reverted before commit) skip the
     `send_cast_intent` call while still calling `start_predicted_leap` — the
     test must now take the vacuous-pass branch and print the environment
     verdict, proving the accept signal really keys off the server. Restore
     the line.
  3. Run the dash test 3x idle (`cargo nextest run -p vordar-client -E
     'test(=onslaught_dash_replay_never_snaps_at_150ms_rtt)'`) — all green
     through the strict-assert branch (telegraph present idle; the vacuous
     branch must NOT be taken — add a `bool` and assert-print which branch
     ran, via the eprintln only).
  4. Full gate: `cargo check --workspace` clean, `cargo nextest run
     --workspace`, `cargo test --doc --workspace` green.

### 4. Snap events are classified against measured wire health in both tests; the strict assert covers healthy-context events only

- **Evidence:** `client/vordar-client/src/net/e2e.rs:288-292` and `:437-441`
  — one whole-run `max_recv_jump` gate. Mechanism C (intent burst-drop:
  `receive.rs:665-668` reject on `validate_intent`'s ≈ 300 ms arrival
  deadline, then ack-prune at `prediction.rs:67` losing the dropped movement
  from replay) and mechanism A both *require* a starvation episode ≥ ~225 ms
  beyond the injected 75 ms one-way latency; both are designed recovery
  behavior, not reconciliation bugs. Idle, snapshots arrive on the 100 ms
  `SNAPSHOT_HZ = 10` cadence (`game/vordar-protocol/src/lib.rs:25`), acks
  advance with every snapshot, and loop iterations run at ~16 ms — none of
  the three gaps ever approaches 300 ms.
- **Ideal:** each snap-sized jump is recorded as an event tagged
  healthy/degraded by the wire the test measured around it; the assert fails
  on any *healthy-context* snap (a genuine bug, even in a loaded run's calm
  stretches) and reports degraded-context snaps without failing. Idle runs
  keep exactly today's full strength.
- **Gap:** no wire measurement exists; every snap fails the run identically.
- **Suggestion:** add a test-local `WireHealth` struct in `e2e.rs`, shared by
  both tests, updated once per loop iteration with `Instant::now()`:
  - tracks last iteration end (own-thread stall gap), last snapshot arrival
    (`NetClientState.latest_state_tick` advanced this recv), and last ack
    advance (derive acked = `state.seq − state.pending.len()`; it advanced
    this recv) — three `Instant`s;
  - whenever a gap in ANY of the three exceeds `DEGRADED_GAP = 300 ms`
    (constant with a comment deriving it from the server's arrival deadline:
    `max(rtt, MAX_REWIND_MICROS = 200 ms) + ARRIVAL_MARGIN_MICROS = 100 ms`,
    `server/vordar-server/src/net/receive.rs`), push a timestamped
    degradation mark onto a small deque;
  - `degraded(now)` = any mark within `LOOKBACK = 1.0 s`.
  In both tests' `run_input` closures, replace the `max_recv_jump` fold with
  snap-event recording (jump magnitude > `SNAP_DISTANCE` ⇒ event with signed
  jump, wall ms, and the `degraded` flag *evaluated at event time*). Final
  asserts become: fail — with the step-1 dump — if any event has
  `degraded == false`, message stating this is a genuine reconciliation
  violation under a measured-healthy wire; `eprintln!` a per-event summary
  for degraded events and pass. The dash test keeps its step-3 acceptance
  gate in front of this. Both tests' wall backstops (`:278`, `:427`) stay.
- **Path:**
  1. Implement `WireHealth` + event classification in both tests; the
     whole-run `max_recv_jump` variable disappears (its assert is subsumed).
  2. Behavioral check idle (healthy path must retain teeth): in the working
     tree only, temporarily lower `SNAP_DISTANCE`'s use in the event
     predicate to `0.01` in one test — an ordinary idle run must now FAIL
     with the healthy-context message (idle wire is healthy, ordinary jumps
     exceed 0.01), proving the assert fires through the classifier. Revert.
  3. Run both tests 3x idle — green; confirm zero degradation marks were
     recorded (print the mark count in the run's eprintln summary).
  4. Full gate: `cargo check --workspace` clean, `cargo nextest run
     --workspace`, `cargo test --doc --workspace` green.
  5. Calibration contingency (for step 5's proof to consume): if a proof run
     later fails with a healthy-context snap whose dump shows an ack or
     snapshot gap in the 200-300 ms band just under threshold, lower
     `DEGRADED_GAP` to 250 ms once, record the change and its trace evidence
     in the nextest.toml header note, and rerun; if a healthy-context snap
     shows NO gap anywhere in its lookback window, do not touch thresholds —
     that is a genuine bug: park and report with the dump.

### 5. Proof at 3x oversubscription, and the honest-state bookkeeping across both reworks

- **Evidence:** finding 3's Path (4): "sensitive-set x10 and one full-suite
  run green at `-Load 3.0`". `.config/nextest.toml:32-55` currently records
  "The suite is NOT proven green at 3x CPU oversubscription; see rework 3".
  `docs/reviews/devloop/reworks-devloop-2026-07-17.md:35-45` records that
  rework 2 "reopens or closes on rework 3's evidence". Historical baseline:
  9/20 combined-set runs green, full suite 356/358, the two prediction tests
  the reproducible failures, `scheduled_aoe` the residual separate signal.
- **Ideal:** the two prediction tests are green in every proof run; the
  measured state note in nextest.toml, finding 3's addendum, and both
  reworks' queue notes say exactly what was measured; rework 3 is struck;
  rework 2 is struck only if the evidence meets its own Ideal ("a loaded
  machine makes tests slower, never red"), otherwise its note records
  precisely which test keeps it open.
- **Gap:** no post-fix load measurements exist; three documents carry the
  stale "not proven" state.
- **Suggestion:** run the proof, then update all three records in one
  bounded diff. Pass bar for THIS rework: the two prediction tests green in
  all runs (strict or explicitly-printed vacuous/degraded branches both
  count as green — the Ideal is "never red under load, full teeth idle").
  Failures of other sensitive-set tests do not fail this bar but must be
  reported and recorded as rework-2 ledger evidence.
- **Path:**
  1. Sensitive set, 10 runs:
     `powershell scripts/stress-suite.ps1 -Load 3.0 -Runs 10 -Filter
     'test(=onslaught_dash_replay_never_snaps_at_150ms_rtt) | test(=predicted_wall_hug_never_snaps_at_150ms_rtt) | test(=scheduled_aoe) | test(=rend_kills_camped_enemy) | test(=kicked_connection_reconnects_and_relogs_in)'`.
  2. Full suite, 1 run: `powershell scripts/stress-suite.ps1 -Load 3.0`.
  3. One idle full gate afterward: `cargo nextest run --workspace` +
     `cargo test --doc --workspace` — must be 403/403 within noise.
  4. If either prediction test reds any run: apply step 4's calibration
     contingency exactly as written (one recorded 250 ms recalibration at
     most, or park with the dump appended to finding 3's addendum). Never
     widen `SNAP_DISTANCE`, never add sleeps, never weaken an assert beyond
     the classifier already designed.
  5. Bookkeeping diff: (a) replace nextest.toml's 2026-07-17 "NOT proven"
     paragraph with the new measured results (runs, green counts, which
     branches fired, any recalibration); (b) append the proof numbers to
     finding 3's addendum in `reworks-devloop-2026-07-17.md` and strike
     rework 3 with a done-note naming this plan file; (c) in the same file's
     rework-2 paragraph (`:35-45`), either record closure (if every
     sensitive-set test held across all runs, i.e. rework 2's proof bar is
     now met) or record exactly which failures keep it open (expected
     candidate: `scheduled_aoe`), citing run counts. Report per-test
     `scheduled_aoe` outcomes verbatim in the loop summary either way.
  6. Gate: full idle suite green, `cargo check --workspace` clean, no new
     warnings.
