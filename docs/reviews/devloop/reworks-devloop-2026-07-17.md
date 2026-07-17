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
the queue above; it is unblocked and orderable on its own.

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

## Carried forward from previous report

None — `reworks-devloop-2026-07-15.md`'s single rework (parallel execution)
was declined 2026-07-15 with a standing do-not-refile condition (token spend
outranks wall time), re-verified this audit: the condition still holds. The
superseded report is deleted; this note preserves the decision.

## Resolved since last report

Rework 1 of 2026-07-15 (parallel execution): declined, condition standing —
recorded above and in the new audit file's Resolved section.
