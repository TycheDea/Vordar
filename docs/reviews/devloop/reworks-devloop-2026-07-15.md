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

> **finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 → finding 8 → finding 9 → finding 10 → finding 11 → rework 1
> (parked: gated on findings 1+4 landing first — parallelism multiplies
> whatever per-worker cost policy exists, so fix the policy before
> multiplying it).**

### 1. Parallel execution of independent queue items (parked)

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
