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

> **finding 1 (user-decides — ask at loop launch) → finding 2 → finding 3 →
> finding 4 → finding 5 → finding 6 → finding 7 → finding 8 → finding 9
> (micro) → finding 10 → rework 1 (user-decides; after finding 5: a queue
> runner must embed the planner-fallback convention finding 5 writes).**
>
> Findings 1–4 lead on the token axis (orchestrator tier, worker read
> channels, silent-breakage prevention, mega-step shape). 5 is the
> attention-axis fix for the measured 45-minute stall. 6–9 remove small
> recurring frictions; 10 is wall-only. Rework 1 goes last because it
> packages conventions the fixes establish.

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
  implement-finding vs a new run-reworks skill), the pause-on-plan question,
  stop conditions, and how loop token reports aggregate.
- **Path:** gate: fixes findings 1 and 5 decided/landed. Then /plan-rework
  with the measured target: the next multi-rework campaign completes with
  ≤1 content-free user prompt per campaign (vs 6 this one) at unchanged
  commit/gate discipline.

## Carried forward from previous report

None — `reworks-devloop-2026-07-15.md`'s single rework (parallel execution)
was declined 2026-07-15 with a standing do-not-refile condition (token spend
outranks wall time), re-verified this audit: the condition still holds. The
superseded report is deleted; this note preserves the decision.

## Resolved since last report

Rework 1 of 2026-07-15 (parallel execution): declined, condition standing —
recorded above and in the new audit file's Resolved section.
