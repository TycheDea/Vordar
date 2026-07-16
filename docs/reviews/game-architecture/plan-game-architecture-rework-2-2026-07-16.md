# Plan: PostUpdate-phase key latches (menu navigation, camera cycling) can't adopt the Input-tick edge API without widening its drain lifetime — 2026-07-16

Source: `docs/reviews/game-architecture/reworks-game-architecture-2026-07-15.md` finding 2.

## Ideal end state

The `just_pressed`/`just_released` edge sets are visible to **every fixed phase
of a step** and drained exactly once per step, so any fixed-phase system —
`AbilityCastSystem` in `Phase::Input` and the `Phase::PostUpdate` consumers
alike — observes each edge exactly once, and a multi-step catch-up frame never
replays an edge into later steps. `MenuSystem`, `CycleCameraSystem`, and
`DevOverlaySystem` (a third hand-rolled latch the audit's sweep missed, same
phase, same bug) consume that API; their `was_*` latch fields are deleted, and
a fast tap that lands press+release inside one frame's event batch — the exact
drop finding 6 fixed for casts — now opens the menu, cycles the camera, and
toggles the overlay instead of vanishing.

## Design decisions

**Drain point: end of the fixed step, i.e. `Phase::PostUpdate` /
`SystemOrder::Last` — not end of frame, not per-phase consumed markers.**
In this scheduler a "tick" is one fixed step: `Scheduler::run_tick`
(`smirk/engine-app/src/scheduler.rs:264-287`) runs every Fixed phase once per
step inside the accumulator loop, and `RenderSync`/`Render` are frame-cadence
(`Phase::default_tick_rate`, scheduler.rs:50-55) — they run once per frame
*after all steps*. So "drain at the end of the whole tick" from the finding's
candidate list maps precisely to the last fixed phase, `PostUpdate`. Moving
`InputEdgeFlushSystem` from `Phase::Input`/`Last` (`app.rs:87`) to
`Phase::PostUpdate`/`Last` keeps the catch-up-replay guard per-step by
construction (the drain still runs inside the step loop) while widening the
observation window from one phase to all eight fixed phases. Rejected
alternatives: (a) end-of-frame drain — steps 2..N of a catch-up frame would
replay the edge, the exact bug the current drain point exists to prevent;
(b) per-phase "already consumed" markers — adds per-consumer state and a wider
API to solve a problem the per-step drain solves with a one-line registration
change; (c) draining inside `Scheduler::run_tick` itself — couples the generic
scheduler to the input module (or adds a one-user hook mechanism), machinery
the System model already covers.

**Contract: the edge API is fixed-step-scoped.** Render-cadence systems
(`RenderSync`/`Render`) run once per frame after all steps, when edges have
already been drained by whichever steps ran — they must read level state
(`is_pressed`), never `just_*`. No such consumer exists today (grep confirms
`just_pressed` is consumed only by `cast.rs:87-91` and, after this plan, the
three converted systems). This contract goes into the `input.rs` module
header, together with: edge consumers register at `First`/`Default` in any
fixed phase — never `PostUpdate`/`Last`, where ordering against the flush
among `Last` peers is a registration-order tie the scheduler does not
guarantee (`PhysicsStatsSystem`, `smirk/engine-physics/src/lib.rs:46`, is the
existing `PostUpdate`/`Last` peer; it reads no input, so its order relative to
the flush is irrelevant).

**Repeat guard: `press()` records an edge only on a genuine up→down
transition.** The winit handler (`app_loop.rs:84-93`) forwards *every*
`Pressed` event — including OS key repeats — into `KeyboardState::press`,
which unconditionally inserts into `just_pressed`. That already contradicts
the documented contract ("True if `key` transitioned up→down", input.rs:33)
and would make the menu conversion a regression: a held Escape would toggle
the menu at OS repeat rate where the old level-latch toggled once. Fix in the
type itself: `if self.pressed.insert(key) { self.just_pressed.insert(key); }`
(a `Pressed` event for an already-down key is by definition a repeat). Rejected:
filtering on winit's `event.repeat` flag in `app_loop` — equivalent for real
events, but leaves the `KeyboardState` contract enforceable only by the
caller, and drops the press that re-arrives after a `Focused(false)` clear
while the key is still physically held; the insert-guard handles both.
Accepted consequence: holding an ability slot key no longer re-triggers
`AbilityCastSystem` at OS repeat rate when its cooldown expires (that cadence
was an accident of OS repeat settings, not a design; if hold-to-recast is ever
wanted, it should be level-based `is_pressed` + cooldown, not repeat-based).
This is an engineering call consistent with the documented contract, not a
product fork — flagging it here for visibility.

**`InputEdgeFlushSystem` goes `pub`.** The behavioral tests for the converted
engine-renderer systems must drive the real drain through a real `Scheduler`,
and the flush is currently `pub(crate)` (input.rs:143). Precedent:
`KeyboardState::press` went `pub` for exactly this cross-crate headless input
path (input.rs:43-46).

**`DevOverlaySystem` is swept along.** Its `was_f3` latch
(`smirk/engine-renderer/src/dev_overlay.rs:15-33`) is the identical bug in the
identical phase; the finding's Gap explicitly generalizes to "any future
PostUpdate/RenderSync input consumer", and finding 6's sweep evidently missed
this one. Converting it also supplies the headless-testable exemplar for the
conversion pattern, which `CycleCameraSystem` cannot provide
(`RendererState` requires a live wgpu device, so its trigger path cannot run
in a headless test; its conversion is line-identical to DevOverlay's).

## Findings (execution order)

### 1. Edge sets record only genuine up→down transitions (OS key repeats no longer re-fire `just_pressed`)

- **Evidence:** `smirk/engine-app/src/input.rs:46-49` —
  `KeyboardState::press` unconditionally does `self.pressed.insert(key);
  self.just_pressed.insert(key);`; `MouseState::press` (input.rs:113-116) is
  identical. The winit handler (`smirk/engine-app/src/app_loop.rs:84-93`)
  forwards every `ElementState::Pressed` event without checking
  `event.repeat`, so a held key re-inserts into `just_pressed` at OS repeat
  rate — contradicting the documented contract at input.rs:33 ("True if `key`
  transitioned up→down since the last Input-tick drain").
- **Ideal:** `just_pressed` records an edge only when the key/button was
  actually up: a `press` while already held (an OS repeat) leaves the edge
  sets untouched; a press after a release re-fires. The type enforces this
  itself, so every caller (winit handler, headless-test `press` seam) gets the
  documented semantics.
- **Gap:** repeats currently re-fire edges; converting `MenuSystem` to
  `just_pressed` without this guard would make a held Escape toggle the menu
  repeatedly at OS repeat rate, where the existing level-latch toggles once.
- **Suggestion:** in `KeyboardState::press` use the `HashSet::insert` return
  value: `if self.pressed.insert(key) { self.just_pressed.insert(key); }`.
  Same pattern in `MouseState::press` for `button`. Update the `press` doc
  comment (input.rs:43-45) to state the guarantee: a press while the key is
  already down (an OS key repeat) records no edge — only genuine up→down
  transitions do. No change to `release`, `clear`, or `drain_edges`. No change
  to `app_loop.rs`.
- **Path:**
  1. Add failing tests to the existing `#[cfg(test)] mod tests` in
     `smirk/engine-app/src/input.rs`:
     - `repeat_press_while_held_does_not_refire_edge`: `kb.press(KeyCode::KeyQ)`;
       `kb.drain_edges()`; `kb.press(KeyCode::KeyQ)` again (simulating an OS
       repeat — key never released); assert `!kb.just_pressed(KeyCode::KeyQ)`.
       Then `kb.release(KeyCode::KeyQ)`; `kb.drain_edges()`;
       `kb.press(KeyCode::KeyQ)`; assert `kb.just_pressed(KeyCode::KeyQ)`
       (a genuine re-press after release must still fire).
     - `mouse_repeat_press_while_held_does_not_refire_edge`: same shape on
       `MouseState` with `MouseButton::Left`.
     Run `cargo test -p engine-app` — the first assertion of each new test
     must fail against the current code (repeat re-fires). If it unexpectedly
     passes, stop and report: the evidence above is then stale.
  2. Apply the insert-guard to both `press` methods; update the `press` doc
     comment.
  3. `cargo test -p engine-app` green, then the full workspace test suite
     green with zero new warnings (the only production consumer of
     `just_pressed` today is `client/vordar-client/src/cast.rs:87-91`, and no
     headless test generates repeat presses — expect no fallout; if any
     workspace test fails, it is asserting repeat-refire behavior that
     contradicts the documented contract — stop and report it rather than
     adapting the test).

### 2. Edge drain moves to the end of the fixed step so every fixed phase observes an edge exactly once

- **Evidence:** `smirk/engine-app/src/app.rs:87` registers
  `crate::input::InputEdgeFlushSystem` at `Phase::Input`,
  `SystemOrder::Last` (behind `#[cfg(feature = "winit")]`). `Phase` runs
  Input → PreUpdate → Update → SpawnFlush → Collision → CollisionResolve →
  DespawnFlush → PostUpdate per fixed step, then RenderSync/Render once per
  frame (`smirk/engine-app/src/scheduler.rs:34-45`, `:50-55`, `:264-287`).
  So any consumer after `Phase::Input` — e.g. the three `Phase::PostUpdate`
  latch systems registered at `smirk/engine-renderer/src/lib.rs:58-62` —
  reads an always-already-drained set. `InputEdgeFlushSystem` is
  `pub(crate)` (`smirk/engine-app/src/input.rs:143`). The module header
  (input.rs:10-12) and the flush's doc comment (input.rs:139-142) both state
  the old drain point, as does a test comment at input.rs:182 ("end of step
  1's Input phase").
- **Ideal:** the flush is registered at `Phase::PostUpdate`,
  `SystemOrder::Last` — the last fixed phase of the step — so all eight fixed
  phases of a step observe an edge exactly once, and the drain still runs
  once per step, preserving the no-replay guarantee on multi-step catch-up
  frames. `InputEdgeFlushSystem` is `pub` so cross-crate tests can reproduce
  the app wiring with a real `Scheduler`. The documented contract states:
  edges live for exactly one fixed step; edge consumers register at
  `First`/`Default` of any fixed phase, never `PostUpdate`/`Last`;
  render-cadence phases (RenderSync/Render, once per frame after all steps)
  must use level state (`is_pressed`), never `just_*`.
- **Gap:** the drain fires seven phases too early; the flush type is not
  reachable from other crates' tests; three comments state the old lifetime.
- **Suggestion:** one registration change plus visibility and comment
  updates. Do **not** add `After`/`Before` edges against `PhysicsStatsSystem`
  (the existing `PostUpdate`/`Last` peer, `smirk/engine-physics/src/lib.rs:46`):
  ordering among `Last` peers is an unguaranteed tie, and it does not matter
  here — `PhysicsStatsSystem` reads no input. Blanket `SystemOrder::Last` is
  correct.
- **Path:**
  1. Add a failing behavioral test in the `#[cfg(test)] mod tests` of
     `smirk/engine-app/src/app.rs`, gated `#[cfg(feature = "winit")]` (the
     `input` module is winit-gated, `lib.rs:25-26`; the feature is default-on).
     Test `edge_visible_to_every_fixed_phase_exactly_once_per_press`:
     - Define a local counting system:
       `struct EdgeCounter { count: Arc<AtomicU32> }` whose `run` increments
       `count` when `resources.get::<KeyboardState>()` reports
       `just_pressed(KeyCode::KeyQ)`.
     - `let mut app = App::new();` register one `EdgeCounter` at
       `Phase::Input`, `SystemOrder::Default` and a second (own `Arc`) at
       `Phase::PostUpdate`, `SystemOrder::First`.
     - Press the key through the real resource (same-crate access to
       `app.resources` is allowed — the field is `pub(crate)`):
       `app.resources.get_mut::<KeyboardState>().unwrap().press(KeyCode::KeyQ);`
     - `app.run_ticks(3.5 / 60.0, 1);` — one frame, exactly 3 fixed steps
       (this both builds the scheduler and runs; the fp-safe 3.5 multiplier
       matches existing scheduler tests).
     - Assert both counters == 1: the Input-phase consumer keeps its
       guarantee, and the PostUpdate consumer sees the edge exactly once —
       not 0 (dropped) and not 3 (replayed per catch-up step).
     - `app.run_ticks(3.5 / 60.0, 1);` again with no new input (repeat
       `run_ticks` is safe: `Scheduler::build` on an empty pending map is a
       no-op); assert both counters are still 1.
     Run it: the PostUpdate counter must be 0 under the current Input-phase
     drain — fail-first confirmed. (If it reads 1 already, stop and report;
     the registration evidence is then stale.)
  2. Change `smirk/engine-app/src/app.rs:87` to
     `scheduler.add(crate::input::InputEdgeFlushSystem, Phase::PostUpdate, SystemOrder::Last);`
     keeping the `#[cfg(feature = "winit")]` attribute.
  3. In `smirk/engine-app/src/input.rs`: make the flush
     `pub struct InputEdgeFlushSystem;` with a doc comment stating: drains
     the edge sets once per fixed step; registered by `App::new()` at
     `Phase::PostUpdate`, `SystemOrder::Last` (the last fixed phase) so every
     fixed phase in a step observes an edge exactly once and multi-step
     catch-up frames don't replay it; `pub` so cross-crate headless tests can
     reproduce this wiring on a real `Scheduler`. Update the module header
     (lines 7-12): edges are drained once per fixed step at the end of
     `Phase::PostUpdate`; edge consumers register at `First`/`Default` of any
     fixed phase, never `PostUpdate`/`Last`; render-cadence systems
     (RenderSync/Render) run once per frame after all steps and must read
     level state, never `just_*`. Fix the stale test comment at input.rs:182
     ("end of step 1's Input phase" → end of step 1).
  4. `cargo test -p engine-app` green (new test passes, all prior tests
     unaffected), then full workspace suite green with zero new warnings
     (the client e2e tests drive systems manually without the scheduler, so
     they never run the flush; `AbilityCastSystem` still sees edges in
     `Phase::Input` of the first step exactly as before).

### 3. CycleCameraSystem and DevOverlaySystem consume the edge API; `was_pressed`/`was_f3` deleted

- **Evidence:** `smirk/engine-renderer/src/camera.rs:281-306` —
  `CycleCameraSystem { was_pressed: bool }` reads
  `kb.is_pressed(KeyCode::KeyC)` and hand-latches (`pressed &&
  !self.was_pressed`). `smirk/engine-renderer/src/dev_overlay.rs:15-33` —
  `DevOverlaySystem { was_f3: bool }` does the same for `KeyCode::F3`
  toggling `DevStats::open`. Both are registered at `Phase::PostUpdate`,
  `SystemOrder::First` via `::new()` (`smirk/engine-renderer/src/lib.rs:60`,
  `:62`). Both drop a tap whose press+release land inside one frame's event
  batch (level reads false at the step). After step 2, edges survive through
  `PostUpdate`, drained by `InputEdgeFlushSystem` (`Phase::PostUpdate`,
  `SystemOrder::Last` — now `pub` in `engine_app::input`).
- **Ideal:** both systems are fieldless unit structs whose `run` gates on
  `kb.just_pressed(...)`; a fast tap fires exactly once, a multi-step
  catch-up frame fires exactly once, a held key fires exactly once (repeat
  guard from step 1).
- **Gap:** both still hand-roll the level-latch and still drop
  frame-rate-dependent taps; `::new()` constructors exist only to zero the
  latch fields.
- **Suggestion:** in each `run`, replace the `is_pressed` read + latch
  compare with a single `just_pressed` check guarding the existing body;
  delete `was_pressed`/`was_f3`, make both `pub struct X;`, delete both
  `new()` constructors (their only callers are the two registration lines),
  and update `smirk/engine-renderer/src/lib.rs:60` and `:62` to register
  `CycleCameraSystem` and `DevOverlaySystem`. Everything else in each body
  (RendererState write-through, DevStats counter publishing including the
  unconditional `stats.set` calls when open) stays byte-identical.
- **Path:**
  1. Add a failing behavioral test in a new `#[cfg(test)] mod tests` in
     `smirk/engine-renderer/src/dev_overlay.rs` driving the real pipeline:
     `fast_f3_tap_toggles_overlay_once_across_catch_up_steps`:
     - `use engine_app::scheduler::{Phase, Scheduler, SystemOrder};`
       `use engine_app::input::{InputEdgeFlushSystem, KeyboardState};`
       `use engine_app::dev_stats::DevStats;`
     - Build `World::new()`, `Resources::new()`; insert `KeyboardState::new()`
       and `DevStats::new()` (no `InstancePool` needed — the system reads it
       with `unwrap_or(0)`).
     - `Scheduler::new()`; add `DevOverlaySystem` at `Phase::PostUpdate`,
       `SystemOrder::First` and `InputEdgeFlushSystem` at `Phase::PostUpdate`,
       `SystemOrder::Last` (mirrors `App::new`'s wiring); `build()`.
     - Fast tap: `kb.press(KeyCode::F3); kb.release(KeyCode::F3);` (both land
       before any step — the drop scenario).
     - `sched.run_tick(&mut world, &mut resources, 3.5 / 60.0)` (3 fixed
       steps). Assert `resources.get::<DevStats>().unwrap().open == true` —
       toggled exactly once (a per-step replay would toggle 3 times and land
       back on `true` only by parity, so also run a second
       `run_tick(…, 1.5 / 60.0)` with no new input and assert `open` is
       *still* `true`, proving no residual edge).
     Against the current latch code the first assertion fails (`is_pressed`
     reads false at the step, `open` stays `false`) — fail-first confirmed.
  2. Convert `DevOverlaySystem`: fieldless struct, `let f3 =
     resources.get::<KeyboardState>().map(|kb|
     kb.just_pressed(KeyCode::F3)).unwrap_or(false);` and `if f3 {
     stats.open = !stats.open; }`; delete `new()`; test passes.
  3. Convert `CycleCameraSystem` identically (`just_pressed(KeyCode::KeyC)`
     guarding the existing cycle+uniform-upload body); fieldless struct,
     delete `new()`. No headless test is possible for its trigger path — the
     body `expect`s `RendererState`, which requires a live wgpu device; the
     edge mechanism is proven by step 2's engine-app test and this step's
     DevOverlay test, and the conversion is line-identical. Do not weaken the
     `expect`.
  4. Update both registrations in `smirk/engine-renderer/src/lib.rs`
     (lines 60, 62): drop the `::new()` calls.
  5. `cargo test -p engine-renderer` green, full workspace suite green, zero
     new warnings.

### 4. MenuSystem consumes the edge API; MenuState's four latch fields deleted

- **Evidence:** `smirk/engine-renderer/src/menu.rs:68-96` — `MenuState`
  carries `was_escape`/`was_up`/`was_down`/`was_enter`;
  `MenuSystem::run` (menu.rs:283-351) reads levels
  (`kb.is_pressed(Escape/KeyW/ArrowUp/KeyS/ArrowDown/Enter/NumpadEnter)`) and
  hand-latches each (`escape && !menu.was_escape`, etc.), so a tap whose
  press+release land inside one frame's event batch is dropped — the menu
  never opens. Registered at `Phase::PostUpdate`, `SystemOrder::First`
  (`smirk/engine-renderer/src/lib.rs:58`). `frame.rs:83` clones `MenuState`
  each frame (field removal is safe — derive(Clone) adjusts itself). After
  steps 1-2, `just_pressed` survives to `PostUpdate`, fires once per genuine
  transition, and is drained once per fixed step by the now-`pub`
  `InputEdgeFlushSystem` (`Phase::PostUpdate`, `SystemOrder::Last`).
- **Ideal:** `MenuSystem::run` computes
  `escape = kb.just_pressed(Escape)`,
  `up = kb.just_pressed(KeyW) || kb.just_pressed(ArrowUp)`,
  `down = kb.just_pressed(KeyS) || kb.just_pressed(ArrowDown)`,
  `enter = kb.just_pressed(Enter) || kb.just_pressed(NumpadEnter)`,
  and uses them directly (`if escape { … }`); the four `was_*` fields, their
  initializers in `MenuState::new` (menu.rs:88-91), and the four latch
  assignments (menu.rs:315, 330-331, 350) are gone. A fast Escape tap opens
  the menu; a 3-step catch-up frame moves the selection one row per tap, not
  three; a held key navigates once (repeat guard — OS repeats never reach the
  edge set).
- **Gap:** menu navigation still hand-rolls the latch and still drops
  frame-rate-dependent taps.
- **Suggestion:** pure substitution inside `run` — the state machine
  (Escape toggle/back, selection clamp, Enter dispatch, `quit_requested`)
  stays byte-identical; only the four booleans' derivation and the latch
  bookkeeping change. Keep the early-`return` when `!menu.open` — with the
  shared per-step drain, unread edges are cleared centrally, so no
  consumer-side cleanup is needed.
- **Path:**
  1. Add failing behavioral tests in a new `#[cfg(test)] mod tests` in
     `smirk/engine-renderer/src/menu.rs`, using the same real-Scheduler
     harness as step 3 (`Scheduler` + `MenuSystem` at `Phase::PostUpdate`/
     `First` + `engine_app::input::InputEdgeFlushSystem` at
     `Phase::PostUpdate`/`Last`; `Resources` with `KeyboardState::new()` and
     `MenuState::default()`):
     - `fast_escape_tap_opens_menu`: `kb.press(KeyCode::Escape);
       kb.release(KeyCode::Escape);` then
       `run_tick(&mut world, &mut resources, 1.5 / 60.0)` (one step); assert
       `resources.get::<MenuState>().unwrap().open`. Fails against the
       current latch code (`is_pressed` reads false at the step) —
       fail-first.
     - `held_down_key_moves_selection_once_across_catch_up_steps`: set
       `menu.open = true` directly (screen stays `Main`, `selected` 0);
       `kb.press(KeyCode::ArrowDown);` (held — no release);
       `run_tick(…, 3.5 / 60.0)` (3 steps); assert `selected == 1` — the
       finding's own regression scenario: a menu that jumps 3 rows on one
       Down tap during a catch-up frame means the drain-lifetime widening
       broke the per-step guard. Then `run_tick(…, 1.5 / 60.0)` with the key
       still held and no new events; assert `selected` is still 1.
  2. Convert `MenuSystem::run` to the four `just_pressed` derivations; delete
     the latch compares (`&& !menu.was_*`) and assignments; delete the four
     fields from `MenuState` and their initializers in `MenuState::new`.
  3. Both tests pass; `cargo test -p engine-renderer` green; full workspace
     suite green with zero new warnings. (Egui interplay is unchanged: the
     `ui_consumed` gate in `app_loop.rs:59-69` filters events *before* they
     reach `KeyboardState`, and `press()` updates level and edge in the same
     call — whatever reached the latch before reaches `just_pressed` now.)
