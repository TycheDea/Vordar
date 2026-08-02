# Game Architecture Audit — 2026-07-28

Extraction pass, not a fresh sweep: source material is two external expert
reviews (grok, 2026-07-27 — simulation/combat and engine/ECS, formerly
`docs/reviews/grok/02,04-*.md`, deleted after this extraction; see git
history). Every finding was re-verified against the current tree by an
independent read of the cited code, cross-checked against the 2026-07-15
audit's landed queue (all 13 findings + reworks 1–2), and filtered through
project stage: zones deliberately run `chapter: None` during the model-
iteration loop, and pre-content deferral covers enemy/boss/death-design work.

What the reviews confirmed holds up, verified: the ownership-shaped module
map in `vordar-game/src/lib.rs` is accurate; intent-only input; the pure
shared movement math (`movement_velocity`/`step`/`leap_velocity`/
`predict_step`) with dash-truth rewind tests on both sides; the scheduled-
snapshot spine; deterministic seeded damage; the chapter registry's
content-vs-sim install split with the `requires` graph; world events by
clock math; camps as resident populations; the crate layering (engine-net
free of ECS, `vordar-game` free of winit/wgpu outside the lint dev-dep);
deferred spawn/despawn queues and the interleaved fixed-step scheduler with
its tests; and workspace hygiene (lints, deny.toml, trimmed debug info).
Two of the reviews' complaints were refuted outright and are recorded as
strengths instead: `SeparationSystem` is snapshot-then-apply and explicitly
order-independent (residual f32-sum noise is ULP-scale, documented and
tolerance-tested), with projectile collision events canonically sorted since
the 07-15 queue; and the engine already owns `PreviousTransform` snapshotting
(`SaveTransformSystem`, `smirk/engine-renderer/src/instance_sync.rs:15-26`,
registered Update/First).

## Ideal end state

Every damage path encodes faction intent the way projectiles already do;
every content reference resolves at boot or fails loudly; API surfaces and
doc comments state exactly what the single-clock scheduler and engine
actually do; misconfiguration (duplicate systems, missing resources,
unloadable prefab dirs) panics at build/boot with names, not mid-tick; and
one headless pipeline test pins the cross-system order that today only
full-QUIC e2e exercises.

## Findings (implementation order)

Cross-type queue:

> **QUEUE CLEARED — ~~finding 1~~ → ~~finding 2~~ → ~~finding 3~~ →
> ~~finding 4~~ → ~~finding 5~~ → ~~finding 6~~ → ~~finding 7~~ →
> ~~finding 8~~ → ~~finding 9~~.**
>
> All entries done 2026-08-02: finding 1 `fd5a82e` (side rule + two
> pass-through tests), finding 2 `7f9ca61`, finding 3 `48bdbd1` + `.claude`
> `96b9d98` (`TickRate` and `set_phase_rate` deleted outright for one
> `set_fixed_hz`), finding 4 `e42e73f`, finding 6 `54a28c4`, finding 7
> `0a44d52`, finding 8 `e0b699b`, finding 9 `00f5321`. Loop-final gate:
> clippy clean, `cargo nextest run --workspace` 440 passed / 5 skipped.
>
> finding 2 landed as steps (1)(3)(4) only: `check_world_events` panics at
> zone boot on any `spawns`/`waves` prefab name absent from that zone's
> `PrefabLibrary`, with three boot tests. Step (2) and the park-vs-panic
> question were pre-empted by `32c4394`, which removed the blood_moon event
> outright — `events.ron` ships with no spawn references at all, so the
> `load_chapter` fail-loud policy applies with no chapterless-config
> conflict left to arbitrate.
>
> finding 5 done 2026-08-02 (`06a3b77`) in the direction OPPOSITE to its own
> Ideal. Bounding `Resources::insert` to `Send + Sync` does not compile:
> `SpawnQueue`/`DespawnQueue` hold `Box<dyn FnOnce(&mut SpawnContext) + Send>`,
> which is not `Sync`. The user ruled the doors unify downward instead — the
> App is thread-affine and its systems are `!Send`, so a `Sync` bound buys a
> guarantee the architecture does not use. `App::insert_resource` lost its
> bound; the dialect sweep landed as specified.
>
> finding 9 measured a defect its own Suggestion assumed away: mechanic-caused
> kills never grant XP, because `DeathSystem` (CollisionResolve) reads
> `DamageDealt` from the current tick while `MechanicResolveSystem` emits it in
> PostUpdate. Filed as reworks finding 3; the pipeline test pins today's
> behavior and will flip when that rework lands.
>
> 1–2 are live defects (one silent, one actively logging errors). 3–7 are
> the truth-and-fences cluster: 3 makes the docs/API match the landed
> scheduler design, 4–7 are small correctness fences, independent. 8–9 add
> the missing lint and pipeline test on top of settled behavior.

### 1. Contact damage has no side rule — enemies packing onto a target mutually damage each other

- **Evidence:** `game/vordar-game/src/combat/contact_damage.rs:33-46` —
  every `CollisionStarted` pair expands both directions and the only gate is
  "attacker bears `ContactDamage`"; `apply()` (`:49-72`) damages any
  `Health`. Contrast `combat/projectile.rs:119-123`: `hits_players` selects
  Player-vs-Enemy with wrong-side pass-through pinned by test
  (`wrong_side_contact_passes_through`, `projectile.rs:242-255`). Enemy
  prefabs carry `Solid` + `ContactDamage` (e.g.
  `content/chapters/chapter01/prefabs/grunt.ron:16-20`), so two grunts
  overlapping while chasing emit `CollisionStarted` and damage each other —
  and `SeparationSystem` re-separates, so re-overlap re-fires `Started`.
  Reachable today in the e2e/chapter and blood-moon paths. Players carry no
  `ContactDamage` (grep of `content/prefabs`: zero), so the player side is
  currently safe by accident.
- **Ideal:** contact damage lands only across factions, same as projectiles
  — the rule lives in the system, not in content authors remembering not to
  let hitboxes overlap.
- **Gap:** structural friendly-fire hole; fires silently in any enemy pack
  today and corrupts every future collision archetype (pets, thorns,
  knockback bodies). Note: `Solid` is irrelevant to the bug — any `Hitbox`
  overlap emits `CollisionStarted`; the narrowphase has no layer filter.
- **Suggestion:** mirror the projectile side rule directly in
  `ContactDamageSystem`: a contact lands only when attacker and target are
  on opposite sides (`Enemy` → `Player` and vice versa), using the same
  component checks projectile.rs already uses. No `Team` component until a
  third faction exists.
- **Path:** (1) filter in the collect step; (2) tests: enemy-enemy and
  player-player pairs pass through (mirror the projectile test); (3) fix the
  `combat/buff.rs:203-226` rage-test fixture, whose ClassId-only attacker
  must satisfy the new side rule; (4) game + e2e suites green.

### 2. `events.ron` spawns a prefab no chapterless zone owns — the blood moon error-spams forever

- **Evidence:** `server/vordar-server/src/main.rs:103` installs
  `content/zones/events.ron` unconditionally into both zones; the def spawns
  `"grunt"` (`events.ron:16,29`), which only chapter-01's prefab dir
  provides — and both zones run `chapter: None`. `WorldTime` publishes every
  Input tick, so each 120 s world day attempts 6 one-shot + 3×4 wave spawns
  per zone, each failing with `log::error!("wave spawn '{prefab}' failed…")`
  (`game/vordar-game/src/world/mod.rs:242`; one-shot path
  `smirk/engine-core/src/prefab.rs:254`). The wave cap never engages
  (failures leave `alive` at 0), so all pulses always fire — ~18 errors per
  day-cycle per zone, indefinitely, in the shipped default config.
- **Ideal:** every prefab string a zone's events/chapters reference resolves
  against that zone's `PrefabLibrary` at install time, so a dangling
  reference is a boot-time diagnostic, never a runtime error loop.
- **Gap:** the population half of the reviews' complaint is the deliberate
  chapterless stage (not extracted); the dangling reference is a live defect
  in today's config regardless.
- **Suggestion:** validate at event install: resolve every `spawns`/`waves`
  prefab name when the events def enters a zone App. For the current stage,
  the consistent handling is to park the blood moon's spawn/wave lists in
  `events.ron` (tint-only event) until zones carry chapters again — a
  boot-time panic (the `load_chapter` policy) would refuse to boot the
  deliberately-chapterless config. **Flag the park-vs-panic choice to the
  user at implementation.**
- **Path:** (1) install-time resolution check; (2) events.ron adjustment per
  the user's call; (3) test: an events def naming an unknown prefab fails
  install; (4) server boots clean with zero spawn errors across a world day
  (existing world-event tests green).

### 3. The scheduler's API and docs still describe the pre-rework per-phase world

- **Evidence:** behavior is the landed single-clock design — one accumulator,
  interleaved fixed phases (`smirk/engine-app/src/scheduler.rs:263-269`) —
  but the surfaces still sell per-phase rates: `set_phase_rate` per-phase
  (`scheduler.rs:146-148`) while `build()` collapses every
  `TickRate::Fixed(hz)` into the one `fixed_dt`, last phase in ord order
  winning (`scheduler.rs:248-257`, comment "Fixed cadence is app-wide");
  zero callers exist outside scheduler tests, so any future
  `set_phase_rate(Update, Fixed(30))` is silently overwritten by later
  phases' default 60. Lying surfaces: `scheduler.rs:31-32,48-49`,
  `tick_rate.rs:6-8`, the `app.rs:7` usage example, and DESIGN.md §5
  ("per-phase rates → movement at 60 Hz, snapshot phase at 5–10 Hz" —
  snapshot cadence actually self-gates via `STAGGER`). Same species:
  `events.rs:2-3` and `:73-74` say events live "one frame" while the actual
  (designed) lifetime is one fixed step (`ClearEventsSystem` at Input/First
  runs per step; `scheduler.rs:268` already states the truth). And the
  engine-app crate root nowhere states the App-is-thread-affine /
  systems-are-`!Send` invariant the server relies on
  (`server/vordar-server/src/lib.rs:61-62`), while DESIGN.md §6's "all
  cross-module communication via EventBus" over-claims — physics pair data
  is consumed directly via `engine_physics::narrowphase` (a `pub` API, by
  design; `game/vordar-game/src/motion/separation.rs:21`).
- **Ideal:** one API and one set of docs that state the single-clock design:
  phases are render-rate or fixed-rate by membership; the fixed rate is
  app-wide and set once; events live one fixed step; the thread-affinity and
  EventBus-scope contracts are written where authors look.
- **Gap:** this is the un-executed half of the 07-15 rework's own suggestion
  ("make the fixed rate app-wide") — the redundant server call was deleted,
  the API illusion stayed. The 07-27 review rated it Critical; with zero
  live callers the damage is confined to docs/API truth, but DESIGN.md's §5
  claim will mislead the next rate-tuning attempt.
- **Suggestion:** delete the per-phase `TickRate` surface: phase membership
  becomes `is_render()`, one `set_fixed_hz(f32)` on App/Scheduler (default
  60). Rewrite `events.rs` lifetime docs to "one fixed step". Add the
  thread-affinity line to the engine-app crate root. Fix DESIGN.md §5's rate
  sentence and narrow §6's EventBus claim to gameplay intents. Reject
  true multi-rate phases — it contradicts the interleaving invariants the
  07-15 rework established.
- **Path:** (1) scheduler/tick_rate/app API change + test migration;
  (2) events.rs doc lines; (3) engine-app crate-root line; (4) DESIGN.md §5
  and §6 sentences; (5) workspace compiles, scheduler tests green
  (`multi_step_frame_interleaves_phases` untouched).

### 4. Duplicate system `TypeId` silently corrupts ordering constraints

- **Evidence:** `smirk/engine-app/src/scheduler.rs:163-169` —
  `index_of.insert(type_id, i)` overwrites on duplicate registration while
  both boxed instances stay in the systems vec; an `After`/`Before` naming
  the duplicated type resolves only against the last instance. (The
  neighboring silent-drop failure — unresolved constraint targets — was
  fixed by 07-15 finding 2 and now panics with names,
  `scheduler.rs:200-218`.)
- **Ideal:** registering the same system type twice in one phase panics at
  `build()` with phase and type names — same policy as cycles and unknown
  targets.
- **Gap:** low probability today (all systems are distinct types), silent
  and confusing when it happens via type-erased wrappers or copy-paste.
- **Suggestion:** panic when the insert returns `Some`.
- **Path:** (1) the check; (2) `should_panic` test; done.

### 5. `Resources` insertion bounds and panic dialects are inconsistent

- **Evidence:** `smirk/engine-core/src/traits.rs:28` — `Resources::insert<T:
  Any>` (no `Send + Sync`) vs `App::insert_resource<T: Any + Send + Sync>`
  (`smirk/engine-app/src/app.rs:176`); a system holding `&mut Resources` can
  insert non-thread-safe values through the hole. Failure dialects split:
  `.get_mut::<SpatialGrid>().expect("SpatialGrid not in resources")`
  (`smirk/engine-physics/src/cell_update.rs:43-45`) vs the provided
  `Resources::expect/expect_mut` (`traits.rs:42-53`) used elsewhere.
- **Ideal:** one insertion bound (`Send + Sync + 'static`) on both doors and
  one failure dialect (`resources.expect`) with its standard message.
- **Gap:** the EventBus queues and spawn hooks already carry the bounds — the
  type-map is the only hole; the dialect split is a sweep, not new API.
- **Suggestion:** bound `Resources::insert`; sweep the remaining
  `Option::expect`-on-get sites to `resources.expect`. If any existing
  resource fails the new bound, that resource is itself a finding — report
  it rather than weakening the bound. Skip plugin-declared `requires::<T>()`
  machinery — speculative at the current plugin count.
- **Path:** (1) bound + compile check; (2) dialect sweep; (3) workspace
  green.

### 6. Prefab-dir load failures are soft — a zone can boot "healthy" with holes in its library

- **Evidence:** `smirk/engine-core/src/prefab.rs:153-178` — `load_dir` logs
  and skips bad files; an unreadable dir is one `log::error!` and return.
  Chapter RON failures rightly panic (`game/vordar-game/src/world/chapter.rs:
  47-56`), unknown chapter names panic at install (`main.rs:98-102`) — but
  content resolution is cwd-relative (`main.rs:14` documents "run from the
  workspace root"), so a wrong-cwd boot yields an empty library, error logs,
  and a zone that comes up serving nothing.
- **Ideal:** the server refuses to mark a zone healthy when any prefab dir
  failed to load or the library is empty after plugins ran; tools keep the
  soft path.
- **Gap:** the two content error surfaces (soft prefab skip, hard chapter
  panic) are individually documented intent, but together the fail-soft path
  covers exactly the failure operators hit (wrong cwd, corrupt file). This
  also removes the practical sting of cwd-relative paths — a `ContentRoot`
  abstraction stays deferred until packaging (see Not extracted).
- **Suggestion:** lean version — `load_dir` accumulates an error count on
  `PrefabLibrary`; server zone boot panics if any dir failed or the library
  is empty. No report type, no strict-mode flag; `load_chapter` keeps its
  panic.
- **Path:** (1) error count; (2) boot check in `build_zone_app`/main;
  (3) test: a corrupt prefab file fails server boot but not `load_dir`
  itself; suites green.

### 7. `query_cells_overlapping` returns a cell superset, and the spatial header describes a deleted pattern

- **Evidence:** `smirk/engine-core/src/spatial.rs:60-79` —
  `query_cells_overlapping`/`query_cells_overlapping_into` return every entity in overlapped
  cells, no Euclidean filter; all current callers compensate (enemy AI
  distance-filters, broadphase wants candidates). The module header
  (`spatial.rs:7-12`) still shows the per-frame `clear()` + reinsert usage
  that the incremental `CellUpdateSystem` (07-15 finding 9) replaced — a
  stale-claim comment-policy violation.
- **Ideal:** the name says what the function returns; the header shows the
  live usage pattern.
- **Gap:** a future caller trusting the name (interest management, gameplay
  radius checks) over-includes silently. Cell size stays hard-coded until a
  second consumer needs a different one — a config knob for one caller is
  speculative.
- **Suggestion:** rename to `query_cells_overlapping` (or equivalent) with a
  "superset of the radius — callers distance-filter" doc line; rewrite the
  header to the incremental-diff reality.
- **Path:** (1) rename + call-site sweep (mechanical); (2) header;
  (3) workspace green.

### 8. Nothing pins cast time against the 10 Hz resolve slice

- **Evidence:** `MechanicResolveSystem` self-gates to every 6th PostUpdate
  tick (`server/vordar-server/src/net/mechanics.rs:50`; `STAGGER = 60/10`),
  so damage lands on the first 100 ms slice past T — player fairness closed
  by rewind to `t_eff`, but the effective resolve time of short casts
  quantizes: Rend at `cast_micros: 300000` (`content/classes/ravager.ron:23`)
  is 3 slices, +0–33 % relative error.
- **Ideal:** content lint enforces `cast_micros ≥ 2 × resolve slice` for
  every Scheduled/Leap ability, with the slice derived from
  `TICK_HZ`/`SNAPSHOT_HZ`, so the quantization bound is an invariant, not a
  convention.
- **Gap:** current abilities all pass (300 ms ≥ 200 ms); the lint prevents a
  future 150 ms cast from shipping with ±66 % resolve error. Per-PostUpdate
  resolve is not worth its cost — cast times dwarf the slice.
- **Suggestion:** add the assertion to
  `game/vordar-game/tests/content_lint.rs`, deriving the slice from the
  protocol constants.
- **Path:** (1) lint; (2) falsify it once against a fake 100 ms ability;
  done.

### 9. No headless full-pipeline sim test — cross-system order is pinned only at e2e latency

- **Evidence:** `vordar-game`'s only integration test is `content_lint.rs`;
  in-crate tests are single-system/pure; the mechanic resolve tests cover
  despawn-queue discipline and dash rewind, not the damage → death → XP
  matrix. Cross-system order (rage before mechanics, death after contact)
  is exercised only by server e2e over real QUIC (`tests/e2e_combat.rs`).
  The harness shape already exists: `benchmarks/benches/full_tick.rs` builds
  `CoreGamePlugin` headless.
- **Ideal:** one clock-injected headless test drives Input → Update →
  CollisionResolve → PostUpdate through a due mechanic and asserts the
  observable chain, so a system-order regression fails in seconds, not at
  e2e latency.
- **Gap:** pure tests are excellent anchors; the pipeline belt between them
  and e2e is missing.
- **Suggestion:** place it server-side (`MechanicResolveSystem` is
  server-only — keeps the crate boundary): build the app, spawn player +
  enemy, insert a `Mechanic` due now, tick through PostUpdate, assert health
  delta, `Killed`/XP grant, and mechanic despawn. No sleeps; inject the
  clock.
- **Path:** (1) test in server lib/tests reusing the bench harness shape;
  (2) prove it fails when a system registration is reordered; (3) suite
  green.

## Not extracted

- 02-F1 (zones ship `chapter: None`), 02-F14 (density stress content),
  04-F6 (sandbox installs no chapter) — deliberate model-iteration-loop
  stage (`zones.ron` documents the deferral in-file); the sandbox path is
  not otherwise broken. Revisit all three when chapters return; the
  `--chapter` sandbox flag idea is sound then.
- 02-F2 (mechanic shapes + encounter timelines) — the boss-director half is
  content-stage; the shape enum is speculative until any content needs a
  non-circle. Re-file trigger: first cone/rect mechanic designed.
- 02-F3 (sandbox can't resolve Scheduled/Leap) — the dev single-player pack
  IS the local combat lab the review asks for; in-process resolve in the
  sandbox would erode the client/server boundary its own F12 defends.
- 02-F5 (NPCs not rewound at resolve) — asymmetry is real, attacker-
  favorable, and already documented as intentional in the mechanics.rs
  header; the actionable half (freeze casters / NPC history) is enemy-
  telegraph work that doesn't exist yet.
- 02-F8 (`PLAYER_PREFAB = "ravager"` + hardcoded passives) — deliberate
  placeholder until character creation enters scope
  (`receive.rs:43-45` comment); passives stay code-backed until a third
  class, per the review's own recommendation.
- 02-F9 (leash/navmesh), 02-F11 (death state machine) — pre-content
  deferral; behavior seam (`EnemyBehavior` registry) verified present and
  tested for when this work starts.
- 02-F7 (separation/projectile order-dependence) — **refuted**; recorded as
  a strength in the intro. Residual: bit-exact multi-contact MTV summation
  would need sorted accumulation on both sides — only relevant if
  replay/lockstep ever becomes a goal.
- 04-F13 (PreviousTransform not engine-enforced) — **refuted**;
  `SaveTransformSystem` is the engine-side writer.
- 04-F9 (cwd-relative content paths) — deferred until packaging/content
  distribution; finding 6 removes the silent-failure sting.
- 04-F11 (empty engine-audio stub) — deferred until audio is scheduled;
  churn for zero behavior change now.
- 04-F12 (render components in engine-core) — deliberate for the shared
  data-driven prefab path (headless server must deserialize them); revisit
  only if server memory/cold-start ever measures as a problem.
- 04-F14 (panic-as-control-flow risk in gameplay paths) — prospective, no
  live violation found in the hostile-input path (receive-path
  unwraps are guarded or startup-invariant); fold a scoped
  `clippy::unwrap_used` deny into a future hardening pass.
- 04-F3 (thread-affinity documentation) — absorbed into finding 3's doc
  batch.
