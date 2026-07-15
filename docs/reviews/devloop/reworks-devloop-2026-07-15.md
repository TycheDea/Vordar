# Dev-Loop Audit (Reworks) — 2026-07-15

Rework-scale companion to `audit-devloop-2026-07-15.md`: findings that need a
design pass before anyone writes code. Consumed by /plan-rework.

## Ideal end state

Queue items with no dependency between them execute concurrently, so a loop's
wall time approaches its longest dependency chain instead of the sum of its
items — without corrupting the one-commit-per-finding history or the test
gates that make each step provable.

## Findings (implementation order)

Cross-type queue (mirrored verbatim from `audit-devloop-2026-07-15.md`):

> **~~finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 → finding 8 → finding 9 → finding 10 → finding 11~~ (all landed
> 2026-07-15) → rework 1 — DECLINED by the user 2026-07-15: its gate was met
> (findings 1+4 landed; the game-architecture loop provided the serial
> baseline under the new gates), but parallelism buys wall time at extra
> token cost (worktree overhead, batch-failure re-runs, coordination), and
> the binding constraint is the weekly token budget, not the stopwatch. The
> dev loop optimizes for token spend first; do not re-file this rework while
> that holds.**
>
> Same-day second-pass extension (token axis, mirrored from the audit file's
> addendum): **finding 12 → finding 13 → finding 14 → finding 15 → finding 16
> → finding 17 (user-decides — ask at loop launch).** All fix-scale; no new
> reworks filed by the second pass.

### 1. Parallel execution of independent queue items (declined 2026-07-15)

- **Evidence:** today's hygiene loop ran 20 findings + 8 rework steps
  strictly serially at ~5–20 minutes each (~5 hours of loop wall). The queue
  notes already encode the dependency structure explicitly ("finding 2 before
  finding 10: both edit bot.rs…") — and by that structure, findings 2–5 (four
  disjoint-crate comment purges), 6–9, and several placement findings were
  mutually independent: a 4-wide fan-out on the purges alone would have cut
  ~45 minutes of wall. The Agent tool supports parallel spawns and worktree
  isolation; nothing in the pipeline uses them.
- **Ideal:** loop mode reads the queue's dependency note, batches independent
  items, and runs each batch as parallel workers, preserving one-commit-per-
  finding and a provable gate per item.
- **Gap:** wall time is the sum of items, not the longest chain.
- **Tradeoffs:** *Wins:* 2–4× loop wall reduction on batch-heavy queues; the
  dependency data already exists. *Losses (the design problems):* parallel
  workers in one working tree race on `target/` locks, the three exclusive
  tests, and git state — so either (a) worktree isolation per worker (clean
  merges needed; comment-only diffs merge trivially, placement diffs may
  not), or (b) workers edit-only and a single post-batch gate+commit sequence
  replays them (loses per-item gate provenance); commit messages and queue
  bookkeeping need a batch protocol; a failed item in a batch forces
  re-sequencing; orchestrator complexity rises materially. Parallel suites
  also contend for CPU, so per-worker gates slow each other — batching may
  need the scoped gates of fixes finding 1 to pay off at all.
- **Suggestion:** /plan-rework this only after fixes findings 1 (scoped
  gates) and 4 (worker contract) have landed and a loop has validated them —
  the plan should evaluate worktree-per-worker vs edit-only-plus-batch-gate
  against a real queue's shape, and may conclude only comment-class findings
  parallelize safely (which is still most of the volume in cleanup loops).
- **Path:** parked. Gate: fixes findings 1 and 4 landed + one serial loop's
  telemetry under the new gates as the baseline the plan must beat. Then
  /plan-rework with the measured target: batch-heavy loop wall ≤ 50% of its
  serial projection at unchanged gate guarantees.

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run.
