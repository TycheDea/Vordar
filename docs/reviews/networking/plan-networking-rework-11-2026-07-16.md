# Plan: Playback-cursor RESYNC threshold leaves almost no margin against the extrapolation cap — a sustained stall pops backward every ~35 ticks — 2026-07-16

Source: docs/reviews/networking/reworks-networking-2026-07-11.md finding 11.

## Ideal end state

The shared playback cursor is monotone non-decreasing for the life of a
session: once a remote entity is rendered at a position, no cursor mechanism
can ever re-render it at an earlier one. Under a sustained stall the cursor
advances only up to the extrapolation horizon (`latest_state_tick +
EXTRAP_CAP_TICKS`) and holds there — the capped-and-held position is a genuine
terminal state until real data arrives. Recovery is graded and always forward:
a sub-second stall resumes by the existing slew plus `apply_states`' dry-
recovery synthetic sample (no pop at all); a reconnect-scale stall snaps the
cursor forward to the fresh data (a forward pop — correct, the world moved
on). The periodic ~2.7-unit backward pop every ~35 ticks is impossible by
construction, and a regression test running well past the old RESYNC boundary
asserts no backward position step ever occurs while stalled.

## Design decisions

- **Root cause is cursor over-advance, not the resync target.** During a stall
  `latest_state_tick` freezes but `advance_playback`
  (`client/vordar-client/src/net/interpolate.rs:124-134`) keeps advancing the
  cursor at 0.9× nominal (the slew clamp only dampens, never stops). Every tick
  of that advance past `latest_state_tick + EXTRAP_CAP_TICKS` changes **no
  rendered position** — every entity's `sample_buffer` is already capped-and-
  held there — so it is pure divergence debt, which RESYNC eventually pays back
  as a backward snap. Fix the debt, not the collector.
- **Horizon clamp: the cursor never advances past `latest_state_tick as f64 +
  EXTRAP_CAP_TICKS`.** Beyond that point advancing is rendering-invisible
  (the entity with the newest possible sample — tick `latest_state_tick` — is
  exactly at its cap there; entities with older newest samples capped earlier),
  so the clamp costs nothing and bounds cursor-vs-target divergence at
  `-(INTERP_DELAY_TICKS + EXTRAP_CAP_TICKS)` = −27 ticks, strictly inside
  RESYNC's 30. The capped hold becomes terminal. The clamp is against the
  global `latest_state_tick`, not per-entity newest ticks, because the cursor
  is shared and per-entity cadence legitimately differs (server round-robin) —
  the shared horizon only needs to bound the point where *no* entity can
  change. Bonus: at the clamp the cursor sits exactly on the tick where
  `apply_states`' dry-recovery synthetic sample gets spliced
  (`floor(cursor) == cursor` once clamped, both integers), so resume
  interpolates from *exactly* the displayed position.
- **RESYNC becomes forward-only (`error > RESYNC_TICKS`, not
  `error.abs()`).** A backward cursor snap has no legitimate client: within a
  session it re-renders data the player already saw further along, and the one
  case where restarting genuinely makes sense — reconnect / zone redirect —
  already resets `playback = None` in `teardown_replicated_world`
  (`client/vordar-client/src/net/lifecycle.rs:176-179`). With the horizon
  clamp the negative branch is unreachable at current constants anyway
  (−27 > −30); making it one-sided removes the silent trap where a future
  `EXTRAP_CAP_TICKS` bump (e.g. 15 → 20 gives −32) would quietly resurrect the
  pop, and states the real invariant: the cursor is monotone non-decreasing
  within a session.
- **Graded recovery falls out of existing constants — no new tunables.** After
  a stall of S ticks, resume error = `S − 27`. S ≤ 57 (~950 ms): slew catches
  up at ≤1.1× playback through the spliced synthetic sample — zero pop
  (simulated: steps exactly 1.1× nominal). S > 57: forward snap to
  `latest_state_tick − INTERP_DELAY_TICKS`, landing mid-segment between the
  spliced hold-point sample and the fresh one — a single forward pop, then
  in-band tracking (simulated: +4.3-unit forward step, then 0.9–1.1× nominal).
  This matches the finding's Ideal: snapping IS eventually correct, but only
  forward.
- **Rejected: resync toward the capped point's projection** (finding's first
  option) — there is no single "capped point" for the shared cursor to project
  from: entities cap at different ticks (round-robin sampling), so this
  requires per-entity state feeding the shared cursor decision. Complexity
  that the horizon clamp makes unnecessary.
- **Rejected: per-entity "currently capped" flag feeding the resync decision**
  — same coupling, and it treats the symptom (the snap) instead of the cause
  (the over-advance).
- **Rejected: widening the margin (e.g. RESYNC 30 → 60)** — a constant tweak
  the finding explicitly rules out; the pop still occurs under a long enough
  stall, just later and bigger.
- **Rejected: splicing a synthetic sample inside `advance_playback`'s snap
  branch** — machinery added to a branch that should not fire; the splice
  already exists at the right seam (`apply_states`,
  `client/vordar-client/src/net/apply.rs:182-185`) and the clamp makes it
  exact.

Verified by simulation of the exact production arithmetic: current code pops
backward at tick ~66 (4.5 → 1.8); with clamp + forward-only resync the same
scenario holds bit-stable at 4.5 from tick ~62 forever, and both resume
scenarios (short stall, reconnect-scale) produce zero backward steps.

## Findings (execution order)

### 1. Clamp the playback cursor at the extrapolation horizon and make RESYNC forward-only

- **Evidence:** `client/vordar-client/src/net/interpolate.rs:124-134` —
  `advance_playback` slews toward `target = latest_state_tick as f64 -
  INTERP_DELAY_TICKS` and hard-snaps whenever `error.abs() > RESYNC_TICKS`
  (30.0). During a stall (`latest_state_tick` frozen) the cursor keeps
  advancing at 0.9× nominal with nothing to stop it; meanwhile
  `sample_buffer` (same file, lines 144-177) caps extrapolation at
  `EXTRAP_CAP_TICKS` (15.0) past the newest sample and holds. Cap engages at
  divergence −27; RESYNC fires at −30; result: under any sustained stall the
  rendered position pops backward ~2.7 units every ~35 ticks. The test
  `extrapolation_bridges_lost_snapshots_then_caps` (same file, lines 287-375)
  deliberately stops at `TOTAL_TICKS = 65`, one tick short of the pop, and its
  doc comments (lines 281-286, 293-296) describe the pop as out-of-scope.
- **Ideal:** The cursor never advances past `latest_state_tick as f64 +
  EXTRAP_CAP_TICKS` (advancing further changes no rendered position — every
  entity is capped-and-held there), and RESYNC snaps only forward. The cursor
  is monotone non-decreasing for the life of a session; a sustained stall is a
  terminal capped hold. The regression test runs to tick 120 and asserts no
  backward step ever occurs.
- **Gap:** No clamp exists; the RESYNC comparison is two-sided
  (`error.abs()`); the test stops short of the defect and documents it as a
  known hole.
- **Suggestion:** In `advance_playback`, change the resync condition from
  `error.abs() > RESYNC_TICKS` to `error > RESYNC_TICKS`, and clamp the slewed
  result: the function body becomes

  ```rust
  fn advance_playback(playback: Option<f64>, latest_state_tick: u64, delta: f32) -> f64 {
      let target = latest_state_tick as f64 - INTERP_DELAY_TICKS;
      let Some(prev) = playback else { return target };
      let error = target - prev;
      if error > RESYNC_TICKS {
          return target;
      }
      let nominal = delta as f64 * TICK_HZ as f64;
      let max_correction = nominal * MAX_SLEW_FRACTION;
      (prev + nominal + error.clamp(-max_correction, max_correction))
          .min(latest_state_tick as f64 + EXTRAP_CAP_TICKS)
  }
  ```

  Monotonicity holds: the slewed advance is ≥ 0.9 × nominal > 0,
  `latest_state_tick` is monotone within a session (the tick guard in
  `apply_states` drops stale snapshots), so the min-bound never moves below a
  previously returned value; the forward snap only fires when `target > prev +
  RESYNC_TICKS`. No signature change; `advance_playback` is private to
  interpolate.rs and its only caller is `NetInterpolateSystem::run`. The other
  two consumers of the `playback` field — the dry-recovery splice in
  `apply.rs:182-185` and the `playback = None` reset in `lifecycle.rs:179` —
  need no change.

  Comment updates required by the change (constraint comments, not narration):
  - `RESYNC_TICKS` doc (interpolate.rs:26-29): now describes **forward**
    divergence only; state the invariant that the cursor never moves backward
    within a session — backward divergence is bounded by the horizon clamp at
    `INTERP_DELAY_TICKS + EXTRAP_CAP_TICKS` ticks, and a genuine reconnect
    resets `playback` to `None` instead.
  - `advance_playback` doc (interpolate.rs:118-123): mention the horizon clamp
    (advance past `latest_state_tick + EXTRAP_CAP_TICKS` changes no rendered
    position, so the cursor holds there through a stall) and that the snap is
    forward-only.
  - Module header (interpolate.rs:1-5): "capped extrapolation bridging short
    gaps in arrivals" gains "…and holding terminally through a sustained
    stall".
  - The test doc paragraphs describing the pop as out-of-scope
    (interpolate.rs:281-286 "The held window asserted below…", 293-296 "Stops
    strictly before the measured RESYNC pop…", and the comment on the held-
    window assertion at 365-367) are now false — rewrite them to describe the
    terminal hold (stale claims are forbidden by the comment policy).
- **Path:**
  1. **Fail-first.** In
     `client/vordar-client/src/net/interpolate.rs`, test
     `extrapolation_bridges_lost_snapshots_then_caps`: change `TOTAL_TICKS`
     from 65 to 120 (leave `DELIVERIES: [6, 12, 30]` alone) and add assertion
     (d) after the existing (c): for every consecutive pair over the whole
     run, the position never steps backward —

     ```rust
     // (d) The capped hold is terminal: the playback cursor never moves
     // backward, so no tick may render an earlier position than the last.
     for t in 1..TOTAL_TICKS {
         let step = (positions[t] - positions[t - 1]).x;
         assert!(step >= -1e-6, "tick {t}: position stepped backward by {step:.4} during the stall");
     }
     ```

     Run `cargo test -p vordar-client extrapolation_bridges` — expect failure
     at tick ≈ 66-67 with a step of ≈ −2.7 (simulated: pop from 4.5 to 1.8).
     A failing tick a few ticks off is the same defect — proceed. If it
     *passes*, stop and report: the defect model is wrong.
  2. Apply the `advance_playback` change exactly as in the Suggestion, plus
     the four comment updates listed there.
  3. Rerun the test — it must now pass: position holds bit-identical at 4.5
     from ≈ tick 62 through 119 (the existing last-3 held-window and
     zero-`NetMotion` assertions keep working unchanged; the cap-bound
     assertion (c) still bounds max position at 4.5 + 0.01).
  4. Green gate: `cargo test -p vordar-client`, then the full workspace test
     suite. The e2e probes (`net/e2e.rs`) never stall longer than the
     extrapolation cap, so none should change behavior; if one fails,
     stop and report rather than adjusting it.

### 2. Behavioral tests for both stall-recovery paths: slew resume after a sub-second stall, forward snap after a reconnect-scale stall

- **Evidence:** After finding 1, `advance_playback`
  (`client/vordar-client/src/net/interpolate.rs`) clamps the cursor at
  `latest_state_tick as f64 + EXTRAP_CAP_TICKS` and resyncs forward-only, and
  `apply_states` (`client/vordar-client/src/net/apply.rs:182-187`) splices a
  dry-recovery synthetic sample at `floor(cursor)` with the currently
  displayed position before pushing a real sample whose buffer had fallen
  behind the cursor. The existing tests cover jittered arrivals, the dry
  window, and the terminal hold — but nothing exercises what happens when
  data *resumes* after a hold: the slew path (stall short enough that
  `target − cursor ≤ RESYNC_TICKS`) and the forward-snap path (longer).
- **Ideal:** Two tests in interpolate.rs's `tests` mod, same deterministic
  harness as `extrapolation_bridges_lost_snapshots_then_caps` (real
  `apply_states` + real `NetInterpolateSystem`, one Update tick `delta =
  1/60` per iteration, no network): both prove no backward step ever occurs,
  the short stall resumes with zero pop, and the long stall snaps forward
  once then tracks in-band.
- **Gap:** Neither recovery path has a test; a future regression in the
  clamp/splice interaction (e.g. a synthetic-sample tick mismatch) would go
  unseen.
- **Suggestion:** Both tests copy the harness of
  `extrapolation_bridges_lost_snapshots_then_caps` verbatim: a `World` with
  one entity `(Transform::new(Vec3::ZERO), NetBuffer::seeded(0, Vec3::ZERO))`
  mapped as id 1 in `NetClientState::new(None, "127.0.0.1:9".parse().unwrap(),
  "unit-test".into(), [0u8; 32], false, Duration::ZERO)`, `SPEED = 6.0`,
  `pos_at = |tick| Vec3::new(tick as f32 / 60.0 * SPEED, 0.0, 0.0)`,
  `nominal_step = SPEED * DT = 0.1`, `TOTAL_TICKS = 120`, delivering
  `apply_states(&mut world, &mut resources, tick, 0, vec![EntityPos { id: 1,
  pos: WirePos(pos_at(tick)), hp: None }])` at each delivery tick, then
  `render_sys.run(...)` and recording `Transform.position` every tick.

  Both stalls start identically: deliveries at server ticks 6, 12, 30, then
  silence. The cursor reaches the 45-tick horizon (`latest 30 +
  EXTRAP_CAP_TICKS 15`) by ≈ tick 63 and the position holds bit-identical at
  4.5 (`pos_at(30).x = 3.0` plus 15 ticks of 6 u/s extrapolation).

  **Test A — `capped_hold_resumes_by_slew_after_short_stall`:** deliveries
  `[6, 12, 30, 80, 86, 92, 98, 104, 110, 116]`. At the tick-80 resume, target
  = 68, cursor = 45.0 exactly, error = 23 ≤ RESYNC_TICKS → slew. Assert:
  - no backward step anywhere: for t in 1..120, `(positions[t] −
    positions[t−1]).x >= -1e-6`;
  - the hold was reached and is bit-identical just before resume:
    `positions[77] == positions[78] && positions[78] == positions[79]`, and
    `(positions[79].x − 4.5).abs() < 1e-3`;
  - the resume is smooth, never a pop: for t in 81..=119, step.x within
    `[0.5 * nominal_step, 1.5 * nominal_step]` (simulated value: exactly
    0.11 = 1.1× nominal, the slew ceiling, for the whole window).

  **Test B — `reconnect_scale_stall_snaps_forward_never_backward`:**
  deliveries `[6, 12, 30, 100, 106, 112, 118]`. At the tick-100 resume,
  target = 88, cursor = 45.0, error = 43 > RESYNC_TICKS → forward snap; the
  dry-recovery splice put a synthetic sample at tick 45 (position 4.5) before
  the real tick-100 sample (10.0), so the snapped cursor renders mid-segment
  at 8.8. Assert:
  - no backward step anywhere: for t in 1..120, step.x `>= -1e-6`;
  - held bit-identical before resume: `positions[97] == positions[98] &&
    positions[98] == positions[99]`, and `(positions[99].x − 4.5).abs() <
    1e-3`;
  - the snap is a single **forward** jump: `(positions[100] −
    positions[99]).x > 2.0 * nominal_step` (simulated: +4.3);
  - in-band tracking after the snap: for t in 102..=119, step.x within
    `[0.5 * nominal_step, 1.5 * nominal_step]` (simulated: 0.09–0.11).

  Give each test a doc comment stating the recovery contract it locks in
  (short stall → slew through the spliced sample, zero pop; reconnect-scale
  stall → one forward snap, never backward), matching the style of the
  neighboring tests.
- **Path:**
  1. Add both tests to the `tests` mod of
     `client/vordar-client/src/net/interpolate.rs` as specified above.
  2. Run `cargo test -p vordar-client capped_hold_resumes
     reconnect_scale_stall` (or the whole crate). Both must pass with
     finding 1's production code **unchanged** — the numbers above were
     simulated against the exact production arithmetic. If an in-band
     assertion fails only at the window's first tick (cadence-phase
     off-by-one), narrow that window's start by one tick and note the
     measured value in the test's doc comment; any other failure (a backward
     step, a missing hold, a slew where the snap was expected or vice versa)
     is a real defect in finding 1's change — stop and report it, do not
     weaken the assertion.
  3. Green gate: `cargo test -p vordar-client`, then the full workspace test
     suite.

### 3. Docs close-out: online-play diagram wording, SVG regen, reworks queue note (docs-only)

- **Evidence:** `docs/online-play.mmd:19` — node R2 reads "buffer snapshot
  positions by tick;<br/>render remotes at a fixed ~200 ms delay,<br/>brief
  capped extrapolation on loss". After findings 1-2 the rule has a third
  clause: a sustained stall holds at the cap terminally and playback never
  rewinds. `docs/online-play.svg` is the rendered copy.
  `docs/reviews/networking/reworks-networking-2026-07-11.md:9-45` — the
  cross-type queue note records each completed rework as a "N done DATE
  (plan…, K steps; summary)" line (see the "7 done 2026-07-16 (…)" line);
  finding 11 has no such line yet (it is not in the ordered strike list — it
  was discovered after the queue was laid — so a done line is the whole
  update).
- **Ideal:** The diagram's R2 node names the stall behavior so the diagram
  stays true to the shipped rule; the SVG matches the .mmd; the reworks file
  records 11 as done.
- **Gap:** Diagram says only "brief capped extrapolation on loss" — silent on
  what a sustained stall does; no done line for 11.
- **Suggestion:** Load the `mermaid-diagrams` skill before touching the .mmd.
  Change line 19 of `docs/online-play.mmd` to:

  ```
      R2["buffer snapshot positions by tick;<br/>render remotes at a fixed ~200 ms delay,<br/>capped extrapolation on loss — a sustained<br/>stall holds at the cap, playback never rewinds"]
  ```

  Regenerate only this diagram's SVG (mirrors `scripts/render-mmd.sh:29`):
  `npx -y @mermaid-js/mermaid-cli -i docs/online-play.mmd -o
  docs/online-play.svg -b white`. Do not commit changes to any other SVG.

  In `docs/reviews/networking/reworks-networking-2026-07-11.md`, insert after
  the "7 done 2026-07-16 (…)" lines in the queue note:
  "11 done <date the implementation lands>
  (plan-networking-rework-11-2026-07-16.md, 3 steps; the playback cursor now
  clamps at `latest_state_tick + EXTRAP_CAP_TICKS` and resyncs forward-only —
  a sustained stall is a terminal capped hold with graded forward-only
  recovery, no periodic backward pop)."
- **Path:**
  1. Invoke the `mermaid-diagrams` skill, then apply the R2 wording change to
     `docs/online-play.mmd` and regenerate `docs/online-play.svg` with the
     command above; confirm the .mmd still parses (the skill's workflow
     covers this).
  2. Add the "11 done" line to the reworks file's queue note.
  3. No source code, no tests — workspace stays green by construction.
