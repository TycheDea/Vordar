# Plan: `scheduled_aoe`'s dodge-window sim-rate precondition measures the wrong party — the bot's own send loop can starve while the precondition reads healthy — 2026-07-17

Source: docs/reviews/devloop/reworks-devloop-2026-07-17.md finding 5.

## Ideal end state

The dodge miss-assert in `scheduled_aoe` (`server/vordar-server/tests/e2e_combat.rs:90-100`)
is gated on every party whose starvation can make the miss mathematically
unreachable: the server's sim pacing (the existing `DODGE_SIM_RATE_MIN`) AND
bot B's own send cadence through the dodge window (new — the party the
recorded failures actually starved). A failure that still fires with both
preconditions healthy carries a per-iteration dump that attributes it, and is
then by construction a genuine scheduled-cast rewind bug, not harness noise.
Proof closes rework 2's outstanding gap: sensitive-set x10 at `-Load 3.0`
green on `scheduled_aoe`.

## Design decisions

**The physics that shapes everything.** Cast 2 targets B's own position
(`e2e_combat.rs:56-57`), so B starts at the blast center; cleave's radius is
4.0 (`content/classes/ravager.ron:37`). The server applies exactly ONE queued
move intent per connection per tick (`drain_intents`,
`server/vordar-server/src/net/receive.rs:572-603`), each integrating
`movement_velocity(dir, 6.0) * TICK_DT` = 0.1 u (`TICK_DT = 1/60`,
`server/vordar-server/src/net/mechanics.rs:23`; speed 6.0,
`content/prefabs/ravager.ron:13`). The resolve-time rewind
(`rewound_position`, `mechanics.rs:128-137`) undoes every applied tick
STAMPED after T — so B's distance from center at T is exactly
`0.1 u x (applied intents stamped <= T)`. A miss (`distance > 4.0`) therefore
requires **>= 41 pre-T-stamped applied intents**. The bot's dodge loop
(`e2e_combat.rs:67-78`) creates at most ONE new intent per wall iteration
(`Bot::send_move` advances `seq` once; the last-3 ring only recovers losses,
`testing/test-support/src/bot.rs:435-455`), and the send window is 800 ms —
so the loop's mean iteration period must stay under ~19.5 ms or the miss
becomes unreachable no matter what the server does. That razor-thin margin is
the suspected mechanism: under `-Load 3.0` a 16 ms sleep balloons, sends drop
below 41, B is rewound to inside the radius, and the "must miss" assert fires
— all while `sim_rate` (server ticks over wall, `e2e_combat.rs:87`) reads
healthy, because the server WAS healthy. The bot is the starving party.

**Measure the thing itself: a pre-T send counter as a second precondition.**
The fix counts the sends that actually left (detected by `b.seq` advancing —
this also correctly excludes sends suppressed by `move_tokens` exhaustion,
`bot.rs:440-442`) whose stamp is <= `resolve_at`, and gates the miss assert
on `sends_pre_t >= DODGE_PRE_T_SENDS_MIN` alongside the existing
`sim_rate >= DODGE_SIM_RATE_MIN`. This is the `WireHealth` pattern
(`client/vordar-client/src/net/e2e.rs:187-226`): measure the party that can
cause the false verdict, not a proxy. Skip-with-diagnostic on a tripped
precondition, exactly like the existing sim-rate skip — a loaded machine
makes the assert skip, never red.

**Threshold 45, floor 41, idle-calibrated.** 41 is the hard physics bound.
+4 margin covers the arrival-race tail: B rides 150 ms RTT, so a send stamped
in the last ~75 ms before T can arrive after the resolve tick (first 10 Hz
gate past T, `mechanics.rs:44-57`) and never apply. Idle capacity is ~46-50
sends (800 ms at 16 ms + pump overhead), so 45 leaves thin but real idle
slack; step 3 carries a measured calibration recipe with explicit outcomes,
including a park branch if idle cannot clear the bound with margin (that
would mean the assert has no honest teeth even idle — a design fact to
report, not to tune away).

**Keep `DODGE_SIM_RATE_MIN`; do not merge or delete it.** The send counter
NEARLY subsumes it (`move_tokens` are funded by observed server ticks,
`bot.rs:361-367`, so a starved sim also throttles sends), but the subsumption
argument leans on the 12-token cushion and queue-cap arithmetic
(`INTENT_QUEUE_CAP = 16`, `receive.rs:40`) — fragile to prove, cheap to not
need. Two preconditions with distinct skip messages also name which party
starved, which is the attribution the ledger wants.

**Root-cause first, permanently landed.** The instrumentation (step 1) is a
test-local per-iteration trace that dumps only when the miss assert is about
to fire — same shape as the `TraceRing` precedent rework 3 landed in
`client/vordar-client/src/net/e2e.rs`. It lands for good (dump-on-failure is
free when green) and the attribution step (2) has an explicit park branch:
if a captured failure shows the bot KEPT pace and the server still resolved
a hit, that is a genuine rewind bug — the plan stops and reports rather than
landing a precondition that would mask it.

**Rejected alternatives.**
- *Widening the 800 ms dodge window*: the window is load-bearing — B must
  cross the border only ~130 ms before T so its last pre-T intents arrive
  late and exercise the favor-the-defender rewind (`e2e_combat.rs:62-64`).
  Starting earlier deletes what the test proves.
- *Widening `DODGE_SIM_RATE_MIN` or sleep padding*: measures/patches the
  wrong party; explicitly barred by the finding.
- *Token-funded catch-up bursts in the loop* (send once per observed sim
  tick, mirroring the real client's fixed-timestep catch-up after a stall):
  higher fidelity and would keep the assert RUNNING under load instead of
  skipping, but it is a bot-behavior change beyond the finding's Ideal with
  its own race surface. Noted as the follow-up if proof runs show the skip
  branch dominating under load; not planned here.
- *Adding `scheduled_aoe` to the nextest exclusive list*
  (`.config/nextest.toml:67-70`): every recorded failure happened under
  EXTERNAL spinners (`stress-suite.ps1`), where exclusivity is inert; it
  would only add ~1-3 s per full run against suite-internal contention the
  skip already covers.

## Findings (execution order)

### 1. The dodge loop gains a permanent per-iteration trace and a pre-T send counter, dumped only when the miss assert is about to fire

- **Evidence:** `server/vordar-server/tests/e2e_combat.rs:67-78` — the dodge
  loop reads `now = b.client.server_now_micros()`, sends
  `b.send_move(Vec2::new(1.0, 0.0))` when `now + 800_000 >= resolve_at`,
  pumps both bots, sleeps 16 ms. Nothing records iteration cadence or how
  many sends actually left (a send can silently no-op on `move_tokens == 0`,
  `testing/test-support/src/bot.rs:440-442`; a real send advances `b.seq`,
  `bot.rs:446`). The assert block at `:86-100` prints only `sim_rate` on
  skip and nothing on failure. `wall0`/`tick0` anchors exist at `:65-66`.
- **Ideal:** every dodge-assert failure carries the data to attribute it:
  per-iteration (wall offset, clock position relative to T, whether a send
  left, token balance), plus a `sends_pre_t` total — printed to stderr only
  when the assert is about to fire, and a one-line summary
  (`sends_pre_t`, `sim_rate`) printed on every run for calibration and
  ledger evidence.
- **Gap:** failures reproduce at ~1/10-1/20 under `-Load 3.0` with zero
  attribution data; rework 3 declined to chase it for exactly this reason.
- **Suggestion:** test-local instrumentation inside `scheduled_aoe` only —
  no `test-support` changes, no production changes. Before the loop declare
  `let mut trace: Vec<(u64, i64, bool, u32)> = Vec::new();` and
  `let mut sends_pre_t: u32 = 0;`. Inside the loop, after the existing
  `now` read: capture `let seq_before = b.seq;`, run the existing
  conditional send, then
  `let sent = b.seq != seq_before; if sent && now <= resolve_at { sends_pre_t += 1; }`
  and push `(wall0.elapsed().as_millis() as u64, (now as i64 - resolve_at as i64) / 1000, sent, b.move_tokens)`.
  Restructure the assert block: compute
  `let hit = a.hit_results[&mech2].contains(&b_id);` first; always
  `eprintln!("scheduled_aoe dodge: sends_pre_t={sends_pre_t} sim_rate={sim_rate:.2}");`
  then inside the healthy-`sim_rate` branch, if `hit`, dump every trace row
  (`wall_ms`, `ms_to_T`, `sent`, `tokens`) to stderr before the existing
  `assert!(!hit, "B stepped out before T — the rewound test must miss it")`.
  Keep the existing `else` skip-print unchanged. One short comment on the
  trace, comment-policy compliant (why: attributes a dodge failure between
  bot-cadence starvation and server rewind — a fact the code cannot show).
- **Path:** (1) implement exactly the shape above in
  `server/vordar-server/tests/e2e_combat.rs`; (2) test:
  `cargo nextest run -p vordar-server -E 'test(=scheduled_aoe)'` idle —
  green, and the summary line appears in captured output (nextest shows
  output for failed tests only by default; verify the line via
  `cargo nextest run -p vordar-server -E 'test(=scheduled_aoe)' --no-capture`
  once); expect `sends_pre_t` in the ~44-50 range idle — record the value in
  the step summary; if it reads below 41 even idle, something in this plan's
  arithmetic is wrong at the seam — park and report the measured value, do
  not proceed; (3) full gate: `cargo check --workspace` (zero new warnings),
  `cargo nextest run --workspace` (404/404), `cargo test --doc --workspace`.

### 2. Attribution under real load: capture a failing run's trace and route on it (docs-only)

- **Evidence:** recorded failures: rework 3's evidence-gathering saw the
  dodge assert fire twice with `sim_rate` reading pass, and rework 3's
  finding-5 proof reproduced it once more (1/10 sensitive-set runs at
  `-Load 3.0`) — "B stepped out before T — the rewound test must miss it"
  (`server/vordar-server/tests/e2e_combat.rs:93`). Historical rate ~1/10 to
  ~1/20. Step 1's trace is now landed, so the next captured failure
  self-attributes.
- **Ideal:** at least one captured failure with `sim_rate >= 0.9` whose dump
  answers: did the bot's loop starve (few/late sends), or did it keep pace
  while the server still resolved a hit?
- **Gap:** root cause is suspected (bot-thread starvation), never confirmed.
- **Suggestion:** run the stress harness against the established 5-test
  sensitive set and read the dump of any `scheduled_aoe` dodge failure.
  Attribution is mechanical against the physics bound (>= 41 pre-T sends
  required for a miss; see this plan's Design decisions).
- **Path:** (1) run
  `powershell scripts/stress-suite.ps1 -Load 3.0 -Runs 5 -Filter 'test(=onslaught_dash_replay_never_snaps_at_150ms_rtt) | test(=predicted_wall_hug_never_snaps_at_150ms_rtt) | test(=scheduled_aoe) | test(=rend_kills_camped_enemy) | test(=kicked_connection_reconnects_and_relogs_in)'`
  in batches of 5, up to 6 batches (30 runs), stopping early once a
  `scheduled_aoe` dodge-assert failure with `sim_rate >= 0.9` is captured;
  (2) route on the dump — **Case A** (`sends_pre_t < 41`, or trace rows show
  iteration gaps >= ~100 ms inside the send window): bot-cadence starvation
  CONFIRMED — append a short `ATTRIBUTED 2026-MM-DD` note to finding 5's
  section in `docs/reviews/devloop/reworks-devloop-2026-07-17.md` quoting
  `sends_pre_t` and the worst gaps, then proceed to step 3; **Case B**
  (`sends_pre_t >= 45` with no gap anomaly, yet B was hit): genuine
  scheduled-cast rewind bug — append the note with the full dump summary,
  STOP this plan (steps 3-4 do not run), and report: the fix belongs in a
  networking-domain finding against
  `server/vordar-server/src/net/mechanics.rs`, not the harness; **Case C**
  (no dodge failure in 30 runs): append a note recording 0/30 and proceed to
  step 3 — the precondition remains justified by the physics bound alone,
  the same footing `DODGE_SIM_RATE_MIN` stands on; (3) gate: no source
  changes in this step; other tests' failures during these runs are recorded
  in the note, never chased.

### 3. The miss assert gains the bot-cadence precondition `DODGE_PRE_T_SENDS_MIN`, calibrated idle and teeth-checked under injected starvation

- **Evidence:** `server/vordar-server/tests/e2e_combat.rs:86-100` — the miss
  assert is gated only on `sim_rate >= DODGE_SIM_RATE_MIN` (0.9), a
  server-side proxy blind to the bot's own cadence. Step 1 landed
  `sends_pre_t` (count of sends that actually left with stamp
  `<= resolve_at`) and an always-printed summary line. Physics bound: a miss
  requires >= 41 pre-T-stamped applied intents (0.1 u each vs radius 4.0 —
  derivation in this plan's Design decisions); sends stamped in the last
  ~75 ms before T (B's one-way latency at 150 ms RTT) may arrive after the
  10 Hz resolve tick and never apply, hence +4 margin.
- **Ideal:** the assert runs only when the bot demonstrably sent enough
  pre-T intents for the miss to be mathematically reachable; otherwise it
  skips with a message naming the bot's cadence as the starved party and
  both readings — mirror of the existing sim-rate skip.
- **Gap:** a starved bot loop currently produces a red "must miss" assert
  with the precondition reading healthy — the recorded rework-2 residual.
- **Suggestion:** in `e2e_combat.rs`, next to `DODGE_SIM_RATE_MIN`:

  ```rust
  /// The dodge needs > 4.0 u of pre-T movement and each applied intent
  /// stamped <= T integrates exactly 0.1 u (6.0 u/s x 1/60 s tick), so a
  /// miss is mathematically unreachable below 41 pre-T sends; 45 adds
  /// margin for pre-T sends that arrive (75 ms one-way) after an early
  /// resolve tick. Idle measurement 2026-MM-DD: <min/max of 5 runs>.
  const DODGE_PRE_T_SENDS_MIN: u32 = 45;
  ```

  Gate: `if sim_rate >= DODGE_SIM_RATE_MIN && sends_pre_t >= DODGE_PRE_T_SENDS_MIN { ...dump-if-hit + assert... }`
  with an `else` that eprintlns which precondition(s) tripped and both
  readings (keep the existing sim-rate wording for that arm; add e.g.
  "scheduled_aoe: bot sent only {sends_pre_t} pre-T intents (min
  {DODGE_PRE_T_SENDS_MIN}) — wall-contract miss assert skipped").
- **Path:** (1) implement the gate; (2) idle calibration — run
  `cargo nextest run -p vordar-server -E 'test(=scheduled_aoe)' --no-capture`
  5x, reading `sends_pre_t` from the summary line: if min >= 48 keep 45; if
  min in 43..=47 set the const to `min - 2` (never below 41) and record all
  5 readings in the const's comment; if min < 43, PARK and report the
  readings — the loop cannot guarantee the physics bound with margin even
  idle, and landing a threshold that skips idle runs would silently delete
  the assert's teeth (a design fact for the user, not a tuning knob); (3)
  teeth check (transient, then reverted — the finding-4 calibration-record
  precedent): insert `std::thread::sleep(Duration::from_millis(120));` at
  the top of the dodge loop body, run `scheduled_aoe` idle once — the test
  must PASS via the new skip arm with `sends_pre_t` ~5-8 while `sim_rate`
  reads healthy (exactly the recorded failure signature, now skipped);
  revert the sleep and record both observations in the step summary; (4)
  behavioral test named for this step: `scheduled_aoe` itself, idle, through
  the assert branch (summary line shows `sends_pre_t >=` threshold and the
  run is green); (5) full gate: `cargo check --workspace` (zero new
  warnings), `cargo nextest run --workspace` (404/404),
  `cargo test --doc --workspace`.

### 4. Proof at 3x oversubscription and the rework-2 close-out bookkeeping (docs-only)

- **Evidence:** rework 2's outstanding gap
  (`docs/reviews/devloop/reworks-devloop-2026-07-17.md:35-51`): "the proof
  bar ('the suite stays green at 3x CPU oversubscription') is still not met:
  `scheduled_aoe` failed 1/10 sensitive-set runs at -Load 3.0 ... Rework 2
  stays open on `scheduled_aoe` alone." `.config/nextest.toml:50-60` records
  the same open state ("scheduled_aoe's residual flake ... rework 2 ...
  stays open on that evidence"). Finding 5's own Path (4) names this exact
  bar as this rework's proof.
- **Ideal:** sensitive-set x10 at `-Load 3.0` green on `scheduled_aoe`
  (assert-branch and skip-branch passes both count — the Ideal is "never
  red under load, full teeth idle"), one idle full gate green, and every
  ledger that says rework 2 is open updated to closed — no stale claims
  left (comment policy forbids them).
- **Gap:** proof not yet run against the landed precondition; two documents
  record rework 2 as open.
- **Suggestion:** run the proof, then close the books in both files; report
  honestly if the bar is missed — never tune a threshold mid-proof to make
  it pass.
- **Path:** (1) run
  `powershell scripts/stress-suite.ps1 -Load 3.0 -Runs 10 -Filter 'test(=onslaught_dash_replay_never_snaps_at_150ms_rtt) | test(=predicted_wall_hug_never_snaps_at_150ms_rtt) | test(=scheduled_aoe) | test(=rend_kills_camped_enemy) | test(=kicked_connection_reconnects_and_relogs_in)'`
  — bar: `scheduled_aoe` green in all 10; from the summary lines record how
  many runs took the assert branch vs a skip branch (if skips dominate,
  record that as evidence for the catch-up follow-up named in Design
  decisions — do not implement it); failures of the OTHER four tests do not
  fail this bar but are recorded; (2) if `scheduled_aoe` reds: with the
  dodge assert despite both preconditions healthy → genuine rewind bug —
  record the dump in the reworks file, report, and leave rework 2 open (do
  NOT strike); with any other assert (e.g. "sim budget exhausted waiting
  for A gets MechanicScheduled", `testing/test-support/src/bot.rs:65`) →
  record the failure text as new rework-2 ledger evidence and report — that
  mode is outside this rework's fix and must not be improvised against; (3)
  idle full gate: `cargo nextest run --workspace` (404/404) +
  `cargo test --doc --workspace`; (4) on a met bar, bookkeeping — in
  `docs/reviews/devloop/reworks-devloop-2026-07-17.md`: rewrite the intro
  paragraph's rework-2 status (lines 35-51 region) to record closure on this
  proof (date, 10/10 result, branch counts), and append a close-out note to
  finding 5's section naming this plan file; in `.config/nextest.toml`:
  update only the stale sentences in the 2026-07-17 MEASURED block
  (`:50-60`) that say the flake is tracked and rework 2 stays open —
  replace with one sentence recording the cadence precondition and the
  10/10 proof (comment-only edit; the full gate in (3) already proves the
  file still parses); (5) gate: content of this step is docs/comments only,
  but the full gate from (3) stands as the step's green proof.
