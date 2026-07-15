# Game Architecture Audit (Reworks) — 2026-07-15

Rework-scale companion to `audit-game-architecture-2026-07-15.md`: findings
that need a design pass before anyone writes code. Consumed by /plan-rework.

## Ideal end state

Chapter content has one population and progression model that is coherent
whether one player or two hundred stand in the zone: world-resident
populations (camps) as the persistent backbone, wave-style pressure defined
per instance or per player by design rather than by accident of a
`query().next()`, and every progression stat attributed to a player entity —
so chapter 20 is authored against the same model as chapter 1.

## Findings (implementation order)

Cross-type queue (mirrored verbatim from
`audit-game-architecture-2026-07-15.md`):

> **finding 1 → finding 2 → finding 3 → finding 4 (after 3: reuses the shared
> step function) → finding 5 → finding 6 → finding 7 → finding 8 → finding 9
> → finding 10 → finding 11 → finding 12 → finding 13 → rework 1 (after
> finding 12: the XP-attribution fix is the surgical first step of the model
> the rework designs).**

### 1. A multiplayer population & progression model for chapter content

- **Evidence:** `WaveSpawnerSystem` centers spawn rings on
  `world.query::<(&Transform, &Player)>().iter().next()` — an arbitrary
  player (`game/vordar-game/src/world/wave_spawner.rs:64-70`) — and freezes
  timers on a zone-global `max_alive` count (wave_spawner.rs:74-76);
  `ActiveChapter.elapsed` is one clock for the whole zone
  (`world/chapter.rs:74-81`), so "chapter time" has no meaning per player;
  chapter-01's XP lands in a global `PlayerXp` resource
  (`game/chapter-01/src/lib.rs:42`, fixes finding 12 lands the surgical
  attribution piece first). The camp model
  (`world/camp.rs`, deterministic slots, respawn timers) is already
  multiplayer-correct — the world exists whether or not anyone is nearby —
  which sharpens the contrast: half the population model scales, half
  assumes one player.
- **Ideal:** a designed answer to what wave-style chapter content IS in a
  shared persistent zone. Candidate shapes the plan must weigh: (a) waves are
  per-player pressure — each player carries their own wave state and ring,
  budgeted by a zone cap; (b) waves are instanced content — chapters with
  wave phases run in per-party instances and the open zone is camps-only;
  (c) waves become world events — scheduled zone-wide pushes via the existing
  WorldEventsDef machinery, which already solved the "same for everyone"
  problem. Progression (XP, future currencies/quests) is per-player-entity
  state persisted with the character, never a resource.
- **Gap:** the current model silently privileges whichever player iterates
  first, and its state layout (resources, one elapsed clock) cannot express
  any of the candidate designs without restructuring.
- **Suggestion:** /plan-rework this after fixes finding 12 lands (XP
  attribution proves out the per-player-state pattern). The plan should pick
  one candidate shape against DESIGN.md's zone/instance intent, define where
  wave state lives (component vs. instance resource), how `max_alive`
  budgets interact across players, and what persists; then decompose into
  fix-sized steps.
- **Path:** design pass → plan document → /implement-finding steps. Not
  gated on measurements; ordered last in the queue because findings 1–13
  stabilize the scheduler, determinism, and state-modeling ground it builds
  on.

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run.
