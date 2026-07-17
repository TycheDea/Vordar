# Plan: The e2e bots drive wall-clock control loops against a server sim that CPU load can time-starve — 2026-07-17

Source: `docs/reviews/devloop/reworks-devloop-2026-07-17.md` finding 2.

## Ideal end state

An e2e bot's verdict depends on simulated progress, never on the harness
winning a wall-clock race the host's scheduler can revoke. The sim's progress
is already replicated to every bot — `ServerMsg::Snapshot.tick` advances once
per fixed sim step at `TICK_HZ` (60) — so the harness observes sim time for
free, with no protocol change. Bots pace their intent emission to that
observed sim progress (killing the intent-queue backlog that actually kills
the bot under starvation), verdict deadlines are stated in sim seconds, wall
clocks demote to generous hang-guard backstops, and the one assert whose
scenario is *inherently* a wall-clock contract (scheduled_aoe's dodge-before-T
miss) gains an explicit measured precondition. Result: at 3x CPU
oversubscription — where every bot control law measured today fails ~1 in 5 —
the suite is slower but green.

## Design decisions

**1. Sim time = `Snapshot.tick`, not `WorldTime` and not a new channel.**
The finding's Suggestion names "server tick/`WorldTime` progress" as the
candidate clock. These are not equivalent: `WorldTime` is *wall-anchored* —
`NetServerState.world_offset_micros` is fixed at install and `world_micros()`
derives from `server.now_micros()` (an `Instant`), so world time keeps flowing
at wall rate while a starved sim stands still. It measures nothing about
starvation. `NetServerState.tick`, by contrast, increments once per
`SnapshotBroadcastSystem` run, which is a Fixed-phase system — one increment
per 1/60 s fixed step (`vordar-protocol/src/lib.rs:27-31` documents exactly
this contract). Every `Bot` already tracks it as `latest_state_tick`
(`testing/test-support/src/bot.rs:99-103`). Sim seconds = tick delta /
`TICK_HZ`. No protocol change, no server change, no new message.

**2. Why the bot dies — and why pacing intent emission is the fix, not a
wider band or a skip.** The landed fixes-finding-8 change removed
station-keeping; the residual death mechanism is structural: the bot emits
one `MoveIntents` datagram per ~16 ms of *wall* time while the server applies
exactly one intent per *sim* tick (`drain_intents`,
`server/vordar-server/src/net/receive.rs:572-603`) and caps the queue at
`INTENT_QUEUE_CAP = 16`, dropping oldest (receive.rs:680-684). Under a
starved sim (measured ~6x slow at 3x oversubscription), the bot over-sends
several-fold, the queue pins at 16, and every command the bot issues —
including "stop" — applies ~16 sim ticks (~1.6 u at 6 u/s) after the bot
decided it. Add ~0.6 u of snapshot staleness and the bot standing at 3.0
drifts through the 1.0 contact boundary, where edge-triggered contact damage
re-bills 10 HP per separation/re-overlap cycle until 100 HP are gone — the
exact measured failure ("the bot must survive the fight" at 8.4–13.7 s).
The fix: `Bot::send_move` consumes from a token bucket credited by observed
sim-tick advance. Under-sending is *safe by protocol design* (an empty queue
means one tick standing still — receive.rs:569-571); over-sending is the only
hazard, and pacing makes it impossible. A dropped "stop" send is harmless
for the same reason: the queue drains and the player stands. Alternatives
rejected: a separate `send_move_paced` API (forks the harness into a safe
channel and a latently-fragile one — every closed-loop test keeps the bug);
detect-and-skip for the fight test (vacuously green under load, tests
nothing, and the fight *can* state a sim budget, so skipping surrenders a
verdict we can keep).

**3. Verdicts gate on sim budgets; wall clocks demote to an 8x backstop.**
`wait_for`/`walk_until`/`walk_into_portal`/`settle` keep their `Duration`
signatures but interpret them as *sim-time* budgets measured by
`latest_state_tick` advance, with a wall-clock backstop at
`WALL_BACKSTOP_FACTOR = 8` times the budget (measured sim slowdown at 3x
oversubscription is ~6x; 8 adds margin). This changes ~all 30 e2e tests'
guards in one place instead of thirty. The finding's tradeoff — "a sim-time
gate can mask a genuine wall-clock performance regression, so the deadline
assert must survive in some form" — is answered by the backstop: a server
that can't hold 60 Hz *on an idle machine* advances ticks slowly relative to
wall and trips the 8x wall guard, which is precisely the perf-regression
signal. Sim-budget expiry (ticks flowed, condition never came) is a
behavioral failure; backstop expiry (wall passed, ticks didn't) is a
hang/starvation failure — the two panic with distinct messages so triage is
immediate. Anchor rule: a wait's tick anchor is the first nonzero
`latest_state_tick` observed at-or-after the wait starts (a bot's counter is
0 until its first snapshot, and each zone server has its own counter), so a
wait never inherits a foreign epoch.

**4. Wall-contract scenarios get a measured precondition, not a retry and
not a fail.** `scheduled_aoe`'s second cast asserts a *miss* because B
walked out of the blast radius during an 800 ms wall window before the
mechanic's wall-scheduled T (`resolve_at_micros = now + cast_micros`,
receive.rs:298; resolution compares `server.now_micros()` — mechanics.rs:49-56
— while movement integrates per sim tick). Crossing the radius-4 border needs
~670 ms of full-rate sim inside that 800 ms wall window: under a sim running
below ~85% of real time the dodge is *mathematically impossible* — no control
law, budget, or ordering fixes it, because the favor-the-defender contract
itself is wall-vs-sim interplay (real players live in wall time; DESIGN.md
§3). The honest gate is the scenario's own precondition: measure sim rate
across the dodge window (tick delta / (wall elapsed × TICK_HZ)); below 0.9,
print loudly and skip *only the miss assert* — every other assert in the test
still runs. On any idle machine the rate is ~1.0 and the assert always runs;
only a deliberately starved host degrades to green-with-note. This is the
finding's "detects a starved sim and skips" option, scoped to the single
assert whose scenario cannot exist, rather than a blanket test-level skip.

**5. Nothing is evicted from the default suite.** The finding asks whether
tests that cannot state a sim-time budget belong in the default suite. After
this redesign the inventory answers itself: `rend_kills_camped_enemy` states
a sim budget; the connection-lifecycle test (`kicked_connection_reconnects_
and_relogs_in`) and the prediction tests (`onslaught_dash_replay_never_snaps
_at_150ms_rtt`, `predicted_wall_hug_never_snaps_at_150ms_rtt`) have purely
behavioral verdicts (no-snap, fresh-body) whose wall clocks are hang guards,
widened; `scheduled_aoe`'s one wall-contract assert gets the precondition.
No test needs to leave. (Flagged for the plan-approval checkpoint since the
finding raised it as a possible product call — recommendation: keep all.)

**6. No production code changes, and exclusive scheduling stays.** The
server's wall/sim mix (wall-clock cooldowns and mechanic schedules, sim-tick
movement and contact damage) is the shipped design; a starved *production*
server genuinely plays this way, and that is a provisioning concern, not a
harness one. Per the finding's own Path, the mechanism lands behind the
existing exclusive group (`.config/nextest.toml`); relaxing the 6.45 s/run
isolation is a separate, later decision once the mechanism has held over
time.

**7. Proof harness.** A small `scripts/stress-suite.ps1` spawns
`3 × ProcessorCount` busy-loop processes around a nextest invocation — the
repeatable form of the oversubscription measurement finding 8's worker did by
hand. It is a verification tool, never part of the default gate.

## Findings (execution order)

### 1. Bot waits gate on sim-tick budgets with an 8x wall backstop

- **Evidence:** `testing/test-support/src/bot.rs:325-333` — `wait_for` fails
  on `Instant::now() >= deadline` alone; same wall-only pattern in
  `walk_until` (bot.rs:336-348), `walk_into_portal` (bot.rs:10-27), and
  `settle` (bot.rs:382-388). The bot already tracks sim progress:
  `latest_state_tick` (bot.rs:99-103) is updated from every
  `ServerMsg::Snapshot` in `pump` (bot.rs:282-291) and advances once per
  fixed server sim step at `vordar_protocol::TICK_HZ` (= 60.0,
  `game/vordar-protocol/src/lib.rs:27-31`). Under CPU starvation the sim runs
  at ~1/6 wall rate (measured 2026-07-17 at 3x oversubscription), so every
  wall deadline silently shrinks 6x in sim terms.
- **Ideal:** every harness wait interprets its `Duration` as a *sim-time*
  budget (ticks observed via `latest_state_tick`) and fails on wall clock
  only at `WALL_BACKSTOP_FACTOR = 8` times the budget — with distinct panic
  messages for "sim budget exhausted" (behavioral failure) vs "wall backstop
  exceeded" (hang or starved sim). A reusable `SimDeadline` gives manual
  test loops the same semantics.
- **Gap:** no sim-time observation is exposed as a deadline primitive; all
  four wait helpers and every manual test loop race the wall.
- **Suggestion:** in `testing/test-support/src/bot.rs` add:
  - `pub const WALL_BACKSTOP_FACTOR: u32 = 8;` with a constraint comment
    stating the invariant (backstop must exceed the worst sim slowdown the
    suite is expected to survive; ~6x measured at 3x CPU oversubscription).
  - `pub struct SimDeadline { anchor: Option<u64>, budget_ticks: u64, wall_deadline: Instant }`
    with `pub fn new(budget: Duration) -> Self` (`budget_ticks =
    (budget.as_secs_f32() * TICK_HZ) as u64`, `wall_deadline = now +
    budget * WALL_BACKSTOP_FACTOR`) and
    `pub fn check(&mut self, bot: &Bot, what: &str)` which: anchors on the
    first call where `bot.latest_state_tick > 0` (`anchor =
    Some(latest_state_tick)`); panics
    `"sim budget exhausted waiting for {what}"` when
    `latest_state_tick - anchor > budget_ticks`; panics
    `"wall backstop exceeded waiting for {what}"` past `wall_deadline`.
    The anchor-on-first-nonzero rule is load-bearing: a bot's counter is 0
    until its first snapshot, and each zone server mints its own tick epoch,
    so anchoring at construction would either never expire (0 forever) or
    expire instantly (foreign epoch). A deadline that spans a reconnect to a
    different server falls back to the wall backstop (document as a
    constraint comment).
  - Rewrite `wait_for`, `walk_until`, and `walk_into_portal` on top of
    `SimDeadline` (loop body unchanged otherwise). Rewrite `settle(bot, dur)`
    to pump until `dur`-worth of sim ticks elapsed past the same anchor rule,
    returning (not panicking) at the wall backstop.
- **Path:**
  1. Implement the above in `testing/test-support/src/bot.rs`; re-export via
     the existing `pub use bot::*;` in `testing/test-support/src/lib.rs`
     (nothing to add — the module is glob-exported).
  2. New integration test `testing/test-support/tests/harness.rs`
     (test-support already depends on vordar-server, so `spawn_server`
     works from its own tests):
     - `sim_budget_expires_against_a_live_server`: `spawn_server` on
       `127.0.0.1:25501`, connect a `Bot`, `wait_for` welcome + first
       snapshot, then `std::panic::catch_unwind(AssertUnwindSafe(...))` a
       `wait_for("never", Duration::from_millis(500), |_| false)`; assert
       the payload contains `"sim budget exhausted"` and that it fired in
       well under the 4 s backstop (e.g. < 2 s wall) — proving the sim
       clock, not the wall, expired it.
     - `wall_backstop_covers_a_silent_server`: `Bot::connect` to
       `127.0.0.1:25555` where nothing listens (no snapshots ever, ticks
       stay 0); catch_unwind a `wait_for("welcome",
       Duration::from_millis(200), ...)`; assert the payload contains
       `"wall backstop exceeded"` and it fired at ~1.6 s (8 × 200 ms), not
       at 200 ms.
  3. `.config/nextest.toml`: add an override assigning
     `package(test-support) and kind(test)` to the existing `realtime`
     test group (these tests spin real servers; the file's own header says
     the filter list must track such tests).
  4. Gate: `cargo nextest run --workspace` + `cargo test --doc --workspace`
     green, `cargo check --workspace` warning-free. The ~30 existing e2e
     tests exercise the rewritten waits end to end; any that newly fail is a
     real regression in this step — fix before landing.

### 2. Bot::send_move paces intent emission to observed sim progress

- **Evidence:** `Bot::send_move` (`testing/test-support/src/bot.rs:354-366`)
  sends unconditionally on every call; every walk loop calls it each ~16 ms
  of wall time (e.g. `walk_until` bot.rs:336-348,
  `server/vordar-server/tests/e2e_combat.rs:141-163`). The server applies
  exactly one intent per sim tick and caps the queue at 16, dropping oldest
  (`drain_intents` + `queue_move_intents`,
  `server/vordar-server/src/net/receive.rs:572-603,680-684`). Under a ~6x
  starved sim the bot over-sends ~6x, the queue pins full, and every command
  — including stop — applies ~16 sim ticks (~1.6 u at 6 u/s) late; that
  backlog latency is what walked the fight bot through the 1.0 contact
  boundary and killed it (the measured residual flake). An empty queue is
  explicitly safe: the player stands one tick and the deficit stays
  accounted (receive.rs:569-571).
- **Ideal:** the bot can never over-send: `send_move` emission is funded by
  observed sim-tick advance, so the server-side queue depth stays bounded
  (≤ token cap < `INTENT_QUEUE_CAP`) at any CPU load, and control latency
  stays ~one snapshot interval in *sim* terms. Idle behavior is
  indistinguishable from today (credits accrue at exactly 60/s, matching the
  ~62/s call cadence of 16 ms loops; the bucket + server queue pipeline
  absorbs the jitter, so walks hold full speed).
- **Gap:** no pacing exists; emission rate is wall-determined.
- **Suggestion:** token bucket on `Bot`:
  - Field `move_tokens: u32`, initial 0. Const `MOVE_TOKEN_CAP: u32 = 12`
    with a constraint comment: the cap must stay below the server's
    `INTENT_QUEUE_CAP` (16) so a full-bucket burst can never overflow the
    queue and drop intents.
  - In `pump`'s Snapshot arm (bot.rs:282-291), after the tick guard passes:
    if the *previous* `latest_state_tick` was 0 (first snapshot — the
    server's counter is an epoch, not a delta), set
    `move_tokens = MOVE_TOKEN_CAP`; else
    `move_tokens = (move_tokens + (tick - prev)).min(MOVE_TOKEN_CAP)` —
    crediting the tick *delta* keeps funding correct even when an
    intermediate snapshot datagram was lost.
  - `send_move`: return without sending (no seq increment, no ring push)
    when `move_tokens == 0`; otherwise decrement and send exactly as today.
    Constraint comment on the early return: a suppressed send — even a stop
    intent — is safe because the server stands still on an empty queue;
    over-sending is the only hazard.
  - `send_cast` stays unpaced (one-shot, server-side cooldown/range gated).
- **Path:**
  1. Fail-first behavioral test in `testing/test-support/tests/harness.rs`:
     `send_move_never_outruns_the_sim` — `spawn_server` on
     `127.0.0.1:25502`, bot waits for welcome + first snapshot, then for
     2 s of wall time calls `send_move(Vec2::X)` + `pump` every 2 ms
     (~1000 calls); assert `bot.seq <= elapsed_sim_ticks + MOVE_TOKEN_CAP`
     where `elapsed_sim_ticks` is the bot's observed `latest_state_tick`
     delta over the loop (≈ 120 on an idle box), then `send_move(ZERO)` and
     `wait_for("full stream acked", 5 s, |b| b.last_ack == b.seq)` — with
     pacing, no intent is ever dropped by the queue cap, so the ack
     converges to the send counter. Run it before implementing: it must
     fail on the `seq` bound (~1000 ≫ 132). Then implement and see it green.
  2. Full gate: `cargo nextest run --workspace` + doc tests +
     `cargo check --workspace`. Sensitive call sites already verified
     compatible by design-pass reading — re-verify by running them:
     `simulated_latency` (`tests/e2e.rs:84-113`, asserts
     `last_ack == final_seq` and >4.0 u traveled: sends start only after
     the first snapshot, so tokens are seeded; ~94 calls vs ~90+12 credits),
     `scheduled_aoe`'s dodge walk, `aoi_border`'s `walk_until`s, and the
     zones portal walks. The `#[ignore]`d loss/soak probes are outside the
     gate but must stay meaningful: run
     `cargo test -p vordar-server --release --test loss -- --ignored --nocapture`
     once; its lag asserts are upper bounds and pacing only lowers lag —
     if anything trips there, report it rather than tuning the probe.

### 3. Repeatable CPU-oversubscription harness (scripts/stress-suite.ps1)

- **Evidence:** the 2026-07-17 measurement that exposed this rework (every
  control law failing ~1 in 5 at ~3x CPU oversubscription; grunt taking 12 s
  wall to cross ~4 units) was produced with an ad-hoc load rig that was
  never landed; `scripts/` (asset-pipeline, lint-comments.sh, render-mmd.sh)
  has no load tool. The finding's Path demands proof "the suite stays green
  at 3x CPU oversubscription" — unreproducible without a harness.
- **Ideal:** one script pins the load shape so every later step (and every
  future starvation question) runs the same experiment:
  `scripts/stress-suite.ps1 [-Load 3.0] [-Runs 1] [-Filter '<nextest -E expr>']`
  spawns `Load × ProcessorCount` busy-loop processes, runs
  `cargo nextest run --workspace` (with `-E $Filter` when given) `Runs`
  times, reports per-run pass/fail, and always kills the spinners.
- **Gap:** no landed load generator; the proof step has no tool.
- **Suggestion:** Windows PowerShell 5.1 script (the repo's dev platform).
  Spinners: `Start-Process powershell -ArgumentList '-NoProfile','-Command','while($true){}' -WindowStyle Hidden -PassThru`,
  collected into an array; `try/finally` with
  `$spinners | Stop-Process -Force -ErrorAction SilentlyContinue` so a
  Ctrl-C or a failed run never leaks spinners. No PS7-only syntax (no `&&`,
  no ternary). Non-zero exit when any run fails, so it can gate.
- **Path:**
  1. Write `scripts/stress-suite.ps1` as above (~40 lines).
  2. Smoke-verify without burning wall time: `-Load 3.0 -Runs 1 -Filter
     'test(=cooldown_remainders_drops_expired_and_subtracts_correctly)'`
     (a fast unit test) — assert the script exits 0, spinners are gone
     afterward (`Get-Process powershell` count returns to baseline), and a
     deliberately bad filter (`test(=does_not_exist)`) makes nextest report
     zero tests but the script still cleans up.
  3. Gate: full workspace suite + `cargo check --workspace` (untouched by a
     script, but run it — the step ships repo content and the gate is the
     contract).

### 4. rend_kills_camped_enemy verdicts move to sim time

- **Evidence:** `server/vordar-server/tests/e2e_combat.rs:138-142` — the
  fight loop's kill deadline is `Instant::now() + Duration::from_secs(25)`
  asserted as `"grunt survived 25 s of rends"`. At HEAD the bot closes to
  3.0 and stands (the fixes-finding-8 change, commit 844ef14); idle the
  fight takes ~3.3 s. Under a ~6x starved sim the same fight takes 20–30 s
  *wall* — the wall deadline fires on a healthy fight. The other verdicts
  in the test (survival at line 170, hp monotonicity at 173-178) are
  behavioral and stay; the trailing `wait_for`s became sim-aware in step 1.
- **Ideal:** the kill deadline is 25 *sim*-seconds of rends — immune to host
  scheduling, still a real behavioral bound (idle fight ≈ 3.3 sim-s, 7.5x
  headroom), with the step-1 wall backstop (8 × 25 s = 200 s) as the
  hang/perf-regression guard.
- **Gap:** the one remaining wall-clock verdict in the fight.
- **Suggestion:** replace the `deadline` local with
  `let mut deadline = SimDeadline::new(Duration::from_secs(25));` and the
  `assert!(Instant::now() < deadline, ...)` with
  `deadline.check(&bot, "the grunt to die to 25 sim-seconds of rends");`
  as the first statement of the `while` body (after `pump` has run at least
  once the anchor self-seeds; `SimDeadline` anchors on first nonzero tick,
  and the fight starts only after snapshots flow, so the anchor is the
  fight's start tick). `last_cast` stays wall-based deliberately: the
  server's cast cooldown gate is wall-clock (`cooldown_ready`,
  `server/vordar-server/src/net/receive.rs:273-279`), so a 250 ms wall
  cast-attempt cadence matches the thing it is pacing against.
- **Path:**
  1. Make the edit; keep every other assert byte-identical.
  2. Idle verification: `cargo nextest run -p vordar-server -E
     'test(=rend_kills_camped_enemy)'` 5 times — 5/5 green, mean duration
     unchanged (~3.3 s).
  3. Stress verification: `scripts/stress-suite.ps1 -Load 3.0 -Runs 5
     -Filter 'test(=rend_kills_camped_enemy)'`. Outcomes: (a) 5/5 green —
     record durations in the commit message and proceed. (b) a run fails on
     `"wall backstop exceeded"` — the measured slowdown exceeded 8x: raise
     `WALL_BACKSTOP_FACTOR` in `testing/test-support/src/bot.rs` to
     (measured slowdown + 2) and note the measurement in
     `.config/nextest.toml`'s header, rerun, record. (c) a run fails on
     `"the bot must survive the fight"` or the sim budget — the pacing fix
     did not close the death spiral: park the step, report the failing
     seed/timing verbatim, do not tune the control law.
  4. Full gate: workspace suite + doc tests + `cargo check --workspace`.

### 5. scheduled_aoe's wall-contract miss assert gains a sim-health precondition

- **Evidence:** `server/vordar-server/tests/e2e_combat.rs:52-83` — cast 2:
  B walks east from T−800 ms *by its own synced wall clock*
  (`server_now_micros`), crossing the radius-4 border ~T−130 ms, and the
  test asserts the rewound hit test misses B. The mechanic resolves at
  absolute *wall* time (`resolve_at_micros = now + cast_micros`,
  `server/vordar-server/src/net/receive.rs:298`; compared against
  `server.now_micros()` in
  `server/vordar-server/src/net/mechanics.rs:49-56`) while B's walk
  integrates per *sim* tick — the dodge needs ~670 ms of full-rate sim
  inside the 800 ms wall window, so below ~85% sim rate the miss is
  mathematically unreachable by any bot behavior. The test sits in the
  `realtime` group but is not exclusive; at the 3x-oversubscription proof
  run it will fail as a hit.
- **Ideal:** the miss assert runs whenever its scenario existed (any idle or
  moderately loaded machine) and is loudly skipped — assert only, not the
  test — when the measured sim rate through the dodge window proves the
  scenario could not be constructed. Every other assert in the test (cast-1
  hit + caster exclusion, identical-schedule broadcast, backdated-cast
  rejection) runs unconditionally.
- **Gap:** the assert is unconditional; under a starved sim it reports a
  correctness failure for physics that were impossible.
- **Suggestion:** in the test, immediately before the dodge loop
  (e2e_combat.rs:65), capture `let tick0 = b.latest_state_tick;` and
  `let wall0 = Instant::now();` (anchor is valid: snapshots have been
  flowing since login). After `b.send_move(glam::Vec2::ZERO);` (line 77)
  compute
  `let sim_rate = (b.latest_state_tick - tick0) as f32 / (wall0.elapsed().as_secs_f32() * TICK_HZ);`
  (`use vordar_protocol::TICK_HZ;`). Keep the `wait_for("second hit
  result", ...)` unconditional (the result arrives either way; the wait is
  sim-aware since step 1). Then:
  `const DODGE_SIM_RATE_MIN: f32 = 0.9;` — if `sim_rate >= DODGE_SIM_RATE_MIN`
  assert the miss exactly as today; else
  `eprintln!("scheduled_aoe: sim ran at {:.0}% of wall rate through the dodge window — wall-contract miss assert skipped", sim_rate * 100.0);`.
  Constraint comment on the constant: the dodge covers 4 u in an 800 ms
  wall window at 6 u/s (~670 ms of sim required), so below 0.9 the scenario
  cannot exist regardless of bot behavior. The 0.9 also covers the step-2
  pacing warmup (bucket seeds full, so no measurable speed loss idle).
- **Path:**
  1. Make the edit; the backdated-cast segment (lines 85-97) stays after
     and unconditional.
  2. Idle verification: run `scheduled_aoe` 5 times —
     5/5 green AND stderr shows no skip line (proving the assert path ran;
     check with `--no-capture` or nextest's stored output).
  3. Stress verification: `scripts/stress-suite.ps1 -Load 3.0 -Runs 5
     -Filter 'test(=scheduled_aoe)'` — expect green, with the skip line
     appearing in some or all runs. If a stressed run fails the miss assert
     *with* `sim_rate >= 0.9` recorded, the threshold is miscalibrated:
     raise `DODGE_SIM_RATE_MIN` to 0.95, rerun, and record the observed
     rates in the commit message. If it fails any *other* assert, park and
     report.
  4. Full gate: workspace suite + doc tests + `cargo check --workspace`.

### 6. Server-test manual deadline loops adopt SimDeadline or widened wall guards

- **Evidence:** manual `Instant::now() + Duration` verdict loops outside
  the shared helpers: `server/vordar-server/tests/e2e.rs:396-401`
  (respawn_carries_xp probe wait, 10 s), `e2e.rs:441-446` and `458-467`
  (xp_survives_relogin, 5 s each), `e2e_persistence.rs:83` (5 s) and
  `:234` (6 s), `zones.rs:178` (15 s reconnect-across-rebuild poll),
  `zones.rs:274` and `e2e.rs:502` (2 s bandwidth measurement windows),
  `e2e_wireformat.rs:225` (3 s), `e2e_security.rs:50` (5 s),
  `watchdog.rs:89` (10 s). The XP probes wait on *sim-bound* progress
  (GrantXpSystem fires at Update tick 60; a 6x-starved sim needs 6 s wall
  for a "tick 60" event, eating most of a 5 s guard); others wait on
  *wall-bound* progress (DB roundtrips, reconnect dials, the watchdog's own
  wall-clock stall detector, the login rate-limit window).
- **Ideal:** every sim-bound verdict loop uses `SimDeadline` (from step 1);
  every wall-bound guard is wide enough to survive a 6x-starved host
  (multiply by 4 — these events are dominated by wall-clock machinery that
  starves far less than the sim loop); pure measurement windows (bandwidth
  byte counts over 2 s wall) stay untouched — their asserts are upper
  bounds that only loosen under starvation.
- **Gap:** ~8 loops re-implement wall-only deadlines; several are marginal
  or red at 3x oversubscription.
- **Suggestion:** classification per site (worker applies mechanically):
  - `SimDeadline` (sim-bound; the pumping bot supplies the tick source):
    e2e.rs:396-401, e2e.rs:441-446, e2e.rs:458-467, e2e_persistence.rs:83,
    e2e_persistence.rs:234 (each loop already pumps a connected bot —
    replace the `deadline` local + `Instant` check with
    `SimDeadline::new(same Duration)` + `check(&bot, "<same message>")`).
  - Widen ×4, stay wall (wall-bound progress): zones.rs:178 (spans
    connection attempts across a server rebuild — tick epochs change, wall
    is the only continuous clock; 15 s → 60 s), e2e_security.rs:50 (login
    rate-limit window is wall-based server-side), watchdog.rs:89 (the
    watchdog's stall detector is wall-based by design),
    e2e_wireformat.rs:225 (3 s → 12 s).
  - Untouched: zones.rs:274 and e2e.rs:502 bandwidth windows.
  No explanatory comments on widened numbers (comment policy: no
  change-narration); the only new comment allowed is a constraint where a
  site's clock choice is non-obvious (zones.rs:178's "wall is the only
  continuous clock across a rebuild" qualifies).
- **Path:**
  1. Apply the classification above.
  2. Behavioral verification: idle full-file runs of the touched test
     binaries (`cargo nextest run -p vordar-server --test e2e --test
     e2e_persistence --test zones --test e2e_security --test
     e2e_wireformat --test watchdog`) — all green, durations unchanged
     (SimDeadline is idle-equivalent by construction).
  3. Stress spot-check: `scripts/stress-suite.ps1 -Load 3.0 -Runs 2
     -Filter 'test(=respawn_carries_xp) | test(=xp_survives_relogin)'` —
     green. If a converted site fails on "wall backstop", treat per step
     4's outcome (b) rule (measure, raise factor, record); if a widened
     wall site still trips, widen to ×8 and record the measured need in
     the commit message.
  4. Full gate: workspace suite + doc tests + `cargo check --workspace`.

### 7. Client e2e hang guards widened to starvation-proof values

- **Evidence:** `client/vordar-client/src/net/e2e.rs` — hang guards on the
  two exclusive tests and the wall-hug test: `:87` and `:109` (5 s) and
  `:126` (10 s) in `kicked_connection_reconnects_and_relogs_in`; `:230`
  (5 s welcome), `:250` (2 s entity), `:278` (4 s "test loop stalled
  mid-dash" guarding a loop that needs ~84 iterations × ≥16 ms ≈ 1.35 s
  idle) in `onslaught_dash_replay_never_snaps_at_150ms_rtt`; `:394` (5 s),
  `:409` (2 s), `:427` (6 s guarding ~120 iterations ≈ 2 s idle) in
  `predicted_wall_hug_never_snaps_at_150ms_rtt`. Under 3x oversubscription
  the test thread's own iterations stretch (16 ms sleep + starved
  scheduling ≈ 40–80 ms/iteration), so the 4 s and 6 s guards trip on
  healthy runs. The verdicts themselves (`max_recv_jump < SNAP_DISTANCE`,
  fresh-body identity) are behavioral and load-immune by construction:
  prediction reconciliation is seq-aligned, not time-aligned, so a slow
  loop replays correctly.
- **Ideal:** every guard in these three tests survives a 6x-slowed host;
  verdict asserts unchanged. These tests drive a real `NetClientState`
  (not a `Bot`), so `SimDeadline` does not apply — wall widening is the
  correct tool here, and these guards guard hangs, not behavior.
- **Gap:** guards sized for idle wall time; 3–6x too tight for the proof
  run.
- **Suggestion:** multiply each listed guard: 5 s → 20 s, 10 s → 40 s,
  2 s → 8 s, 4 s → 30 s, 6 s → 30 s. No comments added (policy). Nothing
  else in the file changes; `pace_tick`, `drive_mover`, and the ignored
  smoothness probe are untouched.
- **Path:**
  1. Apply the widenings.
  2. Idle verification: `cargo nextest run -p vordar-client` — green,
     durations unchanged (guards only fire on failure).
  3. Stress verification: `scripts/stress-suite.ps1 -Load 3.0 -Runs 5
     -Filter 'test(=kicked_connection_reconnects_and_relogs_in) |
     test(=onslaught_dash_replay_never_snaps_at_150ms_rtt) |
     test(=predicted_wall_hug_never_snaps_at_150ms_rtt)'`. Outcomes:
     (a) 15/15 green — record. (b) a guard still trips — widen that guard
     to ×8 of its original and record the measured iteration stretch in
     the commit message. (c) `max_recv_jump` exceeds `SNAP_DISTANCE` under
     load — the prediction contract itself is load-sensitive: do NOT widen
     `SNAP_DISTANCE`; instead park the step and report the measured jump,
     recommending the step-5 pattern (a measured sim/loop-health
     precondition around the snap assert) as the follow-up design.
  4. Full gate: workspace suite + doc tests + `cargo check --workspace`.

### 8. The 3x-oversubscription proof and the recorded close-out

- **Evidence:** `.config/nextest.toml:20-32` — the header's MEASURED
  (2026-07-17) note ends with "Deep starvation (3x oversubscription) still
  time-starves the server sim itself and fails every bot control law
  equally; see docs/reviews/devloop/reworks-*" — i.e., the recorded state
  of the world is "unfixable test-internally". The finding's Path demands
  the proof: "the suite stays green at 3x CPU oversubscription, where every
  control law measured today fails ~1 in 5", landed behind the existing
  exclusive group (which stays; its 6.45 s/run cost note at
  nextest.toml:16-18 is unchanged).
- **Ideal:** the proof is run, measured, and recorded where the previous
  measurements live: the nextest.toml header states the new failure model
  (verdicts gate on sim ticks; wall backstops at 8x; the dodge-window
  precondition) and the proof result, replacing the "fails every control
  law" dead end. Exclusive scheduling is explicitly retained pending
  longer-term evidence.
- **Gap:** proof not yet run; header describes the pre-rework world.
- **Suggestion & Path:**
  1. Sensitive set ×5 under load: `scripts/stress-suite.ps1 -Load 3.0
     -Runs 5 -Filter 'test(=rend_kills_camped_enemy) |
     test(=scheduled_aoe) | test(=kicked_connection_reconnects_and_relogs_in) |
     test(=onslaught_dash_replay_never_snaps_at_150ms_rtt) |
     test(=predicted_wall_hug_never_snaps_at_150ms_rtt)'` — expect 25/25
     green (steps 4–7 already stress-checked individually; this is the
     combined confirmation).
  2. Full suite ×1 under load: `scripts/stress-suite.ps1 -Load 3.0 -Runs 1`
     — expect green. Any failure: attribute it — "sim budget"/behavioral
     message means a real bug (park, report verbatim); "wall backstop"
     means calibration (apply step 4 outcome (b): raise the factor, record,
     rerun the failing test ×3 under load).
  3. Idle regression check: full workspace suite + doc tests twice — green
     both, total duration within noise of the pre-rework baseline (the
     mechanisms are idle-equivalent by construction; a slowdown is a bug).
  4. Rewrite the second MEASURED paragraph of `.config/nextest.toml`'s
     header (lines 20-32): keep the 2026-07-17 root-cause sentence, replace
     the closing dead-end sentence with the new model — bot verdicts gate
     on replicated sim ticks (`Snapshot.tick`), `send_move` is sim-paced,
     wall clocks are 8x backstops, scheduled_aoe's dodge assert carries a
     0.9 sim-rate precondition — plus the proof result ("suite green at 3x
     CPU oversubscription, N/N runs, YYYY-MM-DD") and that exclusive
     scheduling is retained pending relaxation evidence. This is a comment
     in a config file recording measurement provenance, which is that
     file's established convention.
  5. Full gate: workspace suite + doc tests + `cargo check --workspace`.
