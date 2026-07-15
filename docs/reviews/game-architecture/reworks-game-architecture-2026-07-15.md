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

### 2. PostUpdate-phase key latches (menu navigation, camera cycling) can't adopt the Input-tick edge API without widening its drain lifetime

- **Evidence:** finding 6 (`audit-game-architecture-2026-07-15.md`) added
  `KeyboardState`/`MouseState` `just_pressed`/`just_released` edge sets,
  drained once per fixed Input tick by `InputEdgeFlushSystem`
  (`smirk/engine-app/src/input.rs`, `SystemOrder::Last` in `Phase::Input`) —
  correct for `AbilityCastSystem`, which lives in `Phase::Input`
  (`client/vordar-client/src/net/mod.rs:84`). The Path's "sweep for other
  hand-rolled latches" step found two more `was_*`-style edge trackers that
  hand-roll the exact bug finding 6 fixes for casts: `CycleCameraSystem`
  (`smirk/engine-renderer/src/camera.rs:282`, `Phase::PostUpdate`) and
  `MenuSystem` (`smirk/engine-renderer/src/menu.rs:72-75`,
  `Phase::PostUpdate`, drives Escape/Up/Down/Enter menu navigation). Both run
  after `Phase::Input` has already drained the edge sets for that tick, so
  swapping their `is_pressed` reads for `just_pressed` would read an
  always-cleared set — not a fix.
- **Ideal:** these two consumers get the same never-drop-a-tap guarantee
  cast.rs got, without reintroducing the multi-step-catch-up replay bug the
  Input-phase drain point exists to prevent (widening the edge sets'
  lifetime to survive through `PostUpdate` naively would replay the same
  edge once per fixed step on a multi-step frame — e.g. a menu that jumps 3
  rows on one Down tap during a catch-up frame).
- **Gap:** the drain point finding 6 chose is correct for its one named
  consumer but doesn't generalize; menu navigation and camera-cycling still
  drop input under the same frame-rate-dependent conditions finding 6's
  Evidence describes, and any future PostUpdate/RenderSync input consumer
  will hit the same wall.
- **Suggestion:** design (don't guess) where the edge sets' drain point
  should live so every phase in one tick can observe an edge exactly once —
  candidates worth weighing: move the drain to the end of the whole tick
  (after all phases, not just Input) with the catch-up-replay guard staying
  per-step regardless of which phase reads it; or give consumers outside
  Phase::Input a per-phase "already consumed" marker instead of a single
  shared drain. Either should be validated against the same catch-up-frame
  scenario finding 6's tests cover before converting `CycleCameraSystem` and
  `MenuSystem`.
- **Path:** design pass on the drain-lifetime model → plan document →
  /implement-finding steps converting `CycleCameraSystem` and `MenuSystem` to
  the resulting API, deleting their `was_pressed`/`was_escape`/`was_up`/
  `was_down`/`was_enter` fields.

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run.
