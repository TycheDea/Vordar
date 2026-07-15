# Game Architecture Audit — 2026-07-15

First run of this audit. Sweep: one full tick traced end-to-end — winit
(`app_loop.rs`) and headless (`app.rs run_headless`) entry, `Scheduler::run_tick`,
every phase from Input to Render, through vordar-game's systems, chapter
plugins, the client's prediction/replay path, and the server's zone App — plus
every file in `smirk/engine-core`, `smirk/engine-physics`, `smirk/engine-app`,
`game/`, and the simulation-relevant parts of `client/` and `server/`.
`docs/architecture.mmd` matches the code at its stated altitude; no divergence
to report.

What already holds up at MMO scale, for the record: the phase/DAG scheduler
with startup cycle detection; SpawnQueue/DespawnQueue discipline; the typed
EventBus; broadphase→narrowphase→started/ended diffing; the two-pass
snapshot-then-apply SeparationSystem (explicitly order-independent); the
engagement-model AI with measured grid/scan break-even constants; the
deterministic damage formula (splitmix64 crit from stable ids, no wall clock,
no RNG); the camp model (deterministic golden-angle slots); the chapter
registry (content-vs-simulation split, dependency-ordered installs); compiled
prefab plans; and the prediction/reconcile bands with leap-aware replay. The
findings below are the gaps between that and the top of the top.

## Ideal end state

One fixed-timestep sim whose phases interleave correctly on every frame shape,
with ordering constraints that are always enforced or loudly rejected; a
single one-tick movement rule shared verbatim by server integration, client
replay, and lag-compensation rewind; simulation results independent of hash
iteration order everywhere the server will ever be authoritative; world shape
and per-player state modeled as data (zone defs, components) rather than
constants and globals; and collision/spatial structures whose per-tick cost is
proportional to what moved, not what exists — so 100× the entities changes
budgets, not architecture.

## Findings (implementation order)

Cross-type queue (mirrored in `reworks-game-architecture-2026-07-15.md`):

> **finding 1 → finding 2 → finding 3 → finding 4 (after 3: reuses the shared
> step function) → finding 5 → finding 6 → finding 7 → finding 8 → finding 9
> → finding 10 → finding 11 → finding 12 → finding 13 → rework 1 (after
> finding 12: the XP-attribution fix is the surgical first step of the model
> the rework designs).**
>
> Findings 1–2 first: they change what the scheduler guarantees, and every
> later finding's tests run on top of those guarantees. 3–5 are the
> determinism/shared-rule cluster. 6–13 are independent, ordered by impact.

### 1. Per-phase accumulators break intra-tick causality on multi-step frames

- **Evidence:** `smirk/engine-app/src/scheduler.rs:243-254` — every Fixed
  phase owns a private accumulator and drains it fully before the next phase
  runs. On a frame carrying 2–8 pending steps (30 FPS dip, lag spike — the cap
  is 8 at `scheduler.rs:246`), the order is Update×N, then SpawnFlush×N, then
  Collision×N…, not (Update→…→Collision)×N. Consequences already visible in
  code: `game/vordar-game/src/world/wave_spawner.rs:3-6` routes spawns around
  the EventBus specifically because "a same-phase event reader would re-read
  step-1 events during fixed-rate catch-up steps and double-spawn";
  `game/vordar-game/src/combat/projectile.rs:76-78` carries a "already queued;
  fixed phases can step twice before a flush" guard. Physically: N movement
  integrations (`MovementSystem`, Update/Last) land before collision tests run
  even once, multiplying effective tunneling velocity by N on exactly the
  frames that are already struggling. `InterpolationAlpha`
  (`scheduler.rs:232-253`) is whichever fixed phase happened to run last.
- **Ideal:** one fixed clock: compute the step count once per frame, then run
  `for step in 0..n { for phase in fixed_phases { … } }`, render phases after,
  alpha from the single accumulator. Every cross-phase invariant (events live
  exactly one tick, spawns visible to the same tick's collision, one
  integration per collision pass) then holds on every frame shape, and the
  two defensive patches above become dead.
- **Gap:** intra-tick causality currently degrades exactly when frame rate
  does; systems have started encoding workarounds, which is how a scheduler
  bug becomes load-bearing.
- **Suggestion:** restructure `run_tick` around a single accumulator. The
  per-phase `TickRate::Fixed(hz)` generality is unused in practice — every
  fixed phase runs 60 Hz; the server's `set_phase_rate(PostUpdate,
  Fixed(POST_HZ))` (`server/vordar-server/src/net/mod.rs:109`) sets it to the
  value it already has, and slower cadences self-gate (STAGGER) instead. Keep
  `Render` vs `Fixed`, make the fixed rate app-wide, delete the redundant
  server call.
- **Path:** (1) rewrite `run_tick` + `PhaseEntry` (shared accumulator, same
  8-step cap); (2) migrate `set_phase_rate` callers (server net/mod.rs, engine
  tests); (3) regression test: a 3-step frame runs phases interleaved
  (extend the existing `LogSystem` tests with two phases and assert the
  sequence); (4) delete the wave-spawner comment workaround rationale and the
  projectile ttl guard only if the new semantics make them provably dead.

### 2. Ordering constraints that reference First/Last or unregistered systems are silently dropped

- **Evidence:** `smirk/engine-app/src/scheduler.rs:136-165` — `build()` pulls
  First/Last systems out of `middle` before building `index_of`, so an
  `After`/`Before` targeting a First/Last system (or a system that was never
  registered) finds no index and the constraint evaporates without a word.
  Live instance: `BroadphaseSystem` is registered
  `After(CellUpdateSystem)` while `CellUpdateSystem` is `First`
  (`smirk/engine-physics/src/lib.rs:40-41`) — the constraint is dropped and
  the ordering holds only because the First group happens to precede the
  middle group. Any future `Before(CellUpdateSystem)` would silently mean
  nothing.
- **Ideal:** a constraint either orders or panics at `build()` — the same
  policy the cycle check already applies (`scheduler.rs:191-193`). After/
  Before naming a First/Last system resolves correctly (First targets order
  against the First block); naming an unregistered system panics with the
  phase and type names.
- **Gap:** the scheduler's one job — making ordering explicit — has a silent
  failure mode, and the engine's own physics plugin is standing on it.
- **Suggestion:** include First/Last systems in the topological sort as nodes
  with implicit edges (First → all middle → Last) instead of separate blocks;
  unresolved TypeIds panic.
- **Path:** (1) unify the sort; (2) panic on unknown target; (3) tests: After
  a First system is honored, Before a First system panics or reorders
  coherently, unknown target panics; (4) re-declare `BroadphaseSystem`'s
  constraint so it means what it says.

### 3. The one-tick movement rule exists in three places, and only one of them clamps

- **Evidence:** `game/vordar-game/src/motion/movement.rs:20-30` integrates
  velocity then clamps XZ to `PLAY_RADIUS` (a `const` = 65.0, movement.rs:15).
  The client's reconciliation replay
  (`client/vordar-client/src/net/prediction.rs:99-108`) re-applies pending
  intents with `pos + velocity * dt` — no clamp; the server's mechanic rewind
  (`server/vordar-server/src/net/mechanics.rs:128-137`) subtracts
  `movement_velocity(dir, speed) * TICK_DT` — no clamp. A player holding W
  against the world edge accumulates replay error every unacked intent (up to
  ~0.6 units at 100 ms of pending intents at speed 6) — inside the Smooth
  band, so the client gets tugged at the boundary for no misprediction.
  Separately, the radius itself is a hardcoded single-zone assumption inside
  the shared sim while zones are already data (`world/zones.rs` ZoneDef).
- **Ideal:** one function — integrate one tick of velocity, apply the world
  bound — used verbatim by MovementSystem, `replay_position`, and
  `rewound_position` (inverse), with the bound coming from a resource the
  zone/sandbox inserts (default 65) so a future zone with a different shape is
  content, not a code change.
- **Gap:** DESIGN.md §6's "both sides must compute the exact same step" is
  already violated at the boundary, and world geometry lives in a constant.
- **Suggestion:** add `motion::step(pos, velocity, dt, bound) -> Vec3` next to
  `movement_velocity`; MovementSystem calls it; prediction replay folds it;
  `PlayRadius(f32)` resource replaces the const (ZoneDef may later carry it).
- **Path:** (1) extract the function + resource; (2) switch the three sites;
  (3) test: replaying N intents into the boundary lands exactly where the
  live system does (error = 0, not Smooth-band).

### 4. Mechanic rewind reconstructs the wrong position for a player who dashed

- **Evidence:** the applied-intent history stores `(client stamp, dir)` only
  (`server/vordar-server/src/net/mod.rs:150-153`); `rewound_position`
  (`mechanics.rs:128-137`) undoes ticks as `movement_velocity(dir, speed)`.
  But during a leap the tick's ACTUAL integration was the LeapImpulse override
  (`combat/leap.rs:44-58`) — velocity up to 30 u/s vs. walk 6 — so rewinding
  through a dash subtracts the wrong vector entirely. Within the 200 ms
  rewind cap that is up to ~6 units of error on the "was E inside at T" test —
  the exact fairness the scheduled-mechanic design exists to provide. The
  client already solved this shape for replay (`PendingIntent.leap`,
  prediction.rs:37-42); the server half never got it.
- **Ideal:** history entries record what was actually integrated that tick —
  the applied per-tick velocity (or displacement), leap included — and rewind
  subtracts exactly that, sharing finding 3's step function.
- **Gap:** favor-the-defender resolves against a fabricated past for the one
  ability (the gap-closer) most likely to interact with a mechanic.
- **Suggestion:** widen the history tuple to carry applied velocity (receive
  side knows about LeapImpulse the same way NetSendInputSystem does
  client-side), and make `rewound_position` its exact inverse.
- **Path:** (1) history records applied velocity; (2) rewind subtracts it;
  (3) test: cast a mechanic at T mid-dash, assert hit/miss matches the
  dash-truth position at T, not the WASD dead-reckoning.

### 5. Collision event emission order is HashSet-nondeterministic; first-contact resolution inherits it

- **Evidence:** `smirk/engine-physics/src/narrowphase.rs:79-91` — `started`
  and `ended` are collected by iterating `overlapping_buf`/`active` HashSets,
  so CollisionStarted events reach the bus in arbitrary, run-varying order.
  `ProjectileHitSystem` commits "only the first valid contact lands"
  (`combat/projectile.rs:108-140`) — when one bolt starts overlapping two
  enemies the same tick, which one takes the damage differs run to run.
  Determinism is otherwise a stated contract (`enemies/behavior.rs:11-12`,
  ties broken by entity id in AI targeting).
- **Ideal:** every event batch the sim emits has a defined order; identical
  world state produces identical outcomes, run to run and (later) replay to
  replay.
- **Gap:** the one nondeterministic ordering source left in the sim path sits
  directly upstream of a gameplay-visible choice.
- **Suggestion:** sort `started`/`ended` by the canonical pair ids before
  emitting (both are small per-tick vecs; the cost is noise).
- **Path:** (1) sort before emit; (2) test: two simultaneous candidate
  victims → the lower-id pair lands the hit on every run (seeded world,
  repeat N times).

### 6. Input has no per-tick edge semantics; every consumer hand-rolls latches and short taps can vanish

- **Evidence:** `smirk/engine-app/src/input.rs` exposes only `is_pressed`
  (level-triggered). `AbilityCastSystem` keeps its own `was_down` array
  (`client/vordar-client/src/cast.rs:24-33`); winit events mutate
  KeyboardState the moment they arrive (`app_loop.rs:84-101`), so a press+
  release that both land inside one frame's event batch (a fast Q tap during
  a 30 FPS dip is < 33 ms) nets to zero before any Input tick observes it —
  the cast is silently dropped.
- **Ideal:** the input resource buffers edges per fixed Input tick:
  `just_pressed`/`just_released` sets populated from events and cleared when
  the Input phase consumes them, so a tap is never lost and no consumer
  re-implements edge tracking.
- **Gap:** correctness of edge-triggered abilities depends on frame rate; the
  latch pattern will be re-rolled by every future edge consumer.
- **Suggestion:** extend KeyboardState/MouseState with event-buffered edge
  sets drained once per Input tick; convert `AbilityCastSystem` (delete
  `was_down`).
- **Path:** (1) engine change + unit tests (press+release within one frame
  still yields one `just_pressed` at the next tick); (2) migrate cast.rs;
  (3) sweep for other hand-rolled latches.

### 7. The "systems never mutate the world mid-frame" contract is stated but not held

- **Evidence:** `smirk/engine-core/src/traits.rs:70-73` declares the queue
  discipline. `CampSystem` calls `spawn_prefab` directly in Update
  (`world/camp.rs:66-71`) because it needs the spawned Entity for its slot
  bookkeeping; `MechanicResolveSystem` calls `world.despawn` directly in
  PostUpdate (`server/vordar-server/src/net/mechanics.rs:119`). Both are
  borrow-safe, but each makes entity visibility depend on registration order
  within the phase — the exact implicitness the queues exist to prevent.
- **Ideal:** either the contract holds everywhere (structural changes go
  through the queues; systems that need identity use components instead of
  captured Entity ids) or the contract says what is actually true. The first
  is strictly better: a `CampMember { camp, slot }` component attached by the
  spawn closure makes CampSystem's bookkeeping queryable (dead-slot detection
  = "no live entity with this camp/slot"), deleting the Entity-id `slots`
  cache entirely; the mechanic entity has no Hitbox/render slot but should
  still ride DespawnQueue for uniformity.
- **Gap:** two systems quietly hold exceptions to the engine's central
  mutation rule.
- **Suggestion:** CampMember component + queued spawns; DespawnQueue for
  resolved mechanics.
- **Path:** (1) CampSystem rework + its existing respawn test still green;
  (2) mechanics despawn via queue; (3) the traits.rs comment then states a
  rule with zero exceptions.

### 8. Sphere-vs-AABB collision and separation treat the sphere as a box

- **Evidence:** `narrowphase.rs:109-114` tests mixed pairs as AABB-vs-AABB on
  the sphere's bounding box (header calls it conservative);
  `motion/separation.rs:111-119` computes mixed-pair MTV the same way. Overlap
  gets corner false-positives ~1.27× the true radius on diagonals; separation
  pushes along box axes, so a character sliding around a round anchored post
  pops along X/Z instead of deflecting radially — feel-critical math, and
  chapter-02's "camp aggro bubbles stay clear of building hitboxes" discipline
  (`game/chapter-02/src/lib.rs:8-11`) inherits the fuzz.
- **Ideal:** exact closest-point-on-AABB sphere test for narrowphase, and a
  radial MTV (closest point → sphere center) for separation — both are
  ten-line functions; parry3d is already in the tree if preferred.
- **Gap:** the two shape kinds the engine supports don't interact accurately.
- **Suggestion:** implement exact tests in `shapes_overlap` and `mtv`.
- **Path:** (1) exact overlap + MTV; (2) tests: diagonal near-miss no longer
  collides; separation from a sphere pushes radially; (3) a quick feel-check
  pass is the user's (walk around a round post).

### 9. The spatial grid is rebuilt from nothing every tick — allocation churn scales with the world, not with change

- **Evidence:** `engine-physics/src/cell_update.rs:34` calls `grid.clear()`
  each Collision tick; `SpatialGrid::clear` is `HashMap::clear`
  (`engine-core/src/spatial.rs:29-31`), which drops every per-cell `Vec` —
  so every tick re-allocates one Vec per occupied cell, forever, and every
  entity re-inserts even when nothing moved. `CellOccupant.cells` already
  stores each entity's previous footprint, i.e. the diff is already computed
  and then thrown away.
- **Ideal:** per-tick grid cost proportional to entities whose footprint
  changed: compare new cells against `CellOccupant.cells`, and only
  remove/insert on difference; cell Vecs live across ticks. Camps, posted
  NPCs, idle enemies — the majority of an MMO zone — become free.
- **Gap:** O(world) work and allocator traffic every 16 ms, growing linearly
  with entity count.
- **Suggestion:** incremental update in CellUpdateSystem (the `remove` API
  already exists); add a `benchmarks/` case first so the win is a number.
- **Path:** (1) bench current rebuild at representative entity counts;
  (2) incremental diff update; (3) bench delta recorded; correctness pinned
  by a test where an entity crosses a cell boundary and one that stands
  still.

### 10. BodyComposeSystem scans every composed entity every frame to find nothing

- **Evidence:** `client/vordar-client/src/body.rs:34-39` — every Update tick
  it queries all `(RaceId, Option<ClassId>)` entities and calls
  `world.get::<&BodyComposed>` per entity to filter the already-done ones.
  Steady state (everyone composed) still pays the full scan + N random
  lookups, per tick, scaling with AOI population.
- **Ideal:** the filter lives in the query — `hecs::Without<BodyComposed, …>`
  — so composed entities are skipped at archetype level and the steady-state
  cost is zero.
- **Gap:** a per-frame cost that exists only to discover there is no work.
- **Suggestion:** `world.query::<hecs::Without<(Entity, &RaceId,
  Option<&ClassId>), &BodyComposed>>()` (adjust to hecs's Without signature).
- **Path:** (1) one query change; (2) existing compose behavior pinned by a
  spawn→compose test if none exists.

### 11. The client's presentation system list is registered twice, verbatim

- **Evidence:** `client/vordar-client/src/lib.rs:227-247` (offline
  ClientPlugin) and `client/vordar-client/src/net/mod.rs:90-110` (online
  install) register the same ~12 presentation systems (ZoneDressing,
  BodyCompose, corpse/hit-react, pose/facing/locomotion, VFX, weapons, UI)
  with the same phases and constraints, differing only in camera-follow and
  input source. Every new presentation system must be added twice or one mode
  silently lacks it.
- **Ideal:** one `PresentationPlugin` owns the shared list; ClientPlugin =
  presentation + local input/cast; the net install = presentation + net
  systems.
- **Gap:** duplication with a divergence failure mode, in the exact shape the
  audit's duplication hunt names.
- **Suggestion:** extract the shared plugin; the two callers shrink to their
  genuine differences.
- **Path:** (1) extract; (2) both binaries compile and the sandbox/net system
  sets diff to exactly {input, cast, camera, net}.

### 12. XP flows into a global resource, not to the player who earned it

- **Evidence:** `game/chapter-01/src/lib.rs:40-64` — `PlayerXp(u32)` is a
  world-global resource and XpGrantSystem sums every XpReward death into it.
  Two players in the zone would share one XP pool; there is no attribution
  even though `DamageDealt.attacker` (events.rs:23-28) already carries it.
- **Ideal:** XP is a component on the player entity, granted to the killer
  (last-hit attribution from DamageDealt/HealthDepleted correlation is enough
  for now); chapter code stays a consumer of shared events.
- **Gap:** the first per-player progression stat in the codebase is modeled
  single-player; every stat that copies this pattern deepens the hole rework
  1 has to dig out of.
- **Suggestion:** `Xp(u32)` component + kill attribution (track last
  DamageDealt attacker per victim this tick, or emit a `Killed { victim,
  killer }` event from DeathSystem); PlayerXp resource deleted.
- **Path:** (1) attribution event; (2) component grant + test (two players,
  one killer — only the killer's Xp moves); (3) feeds rework 1's design as
  its first landed step.

### 13. The frame limiter sleeps on the event-loop thread

- **Evidence:** `engine-app/src/app_loop.rs:152-165` — RedrawRequested
  computes the frame budget and `std::thread::sleep`s the remainder before
  ticking. While asleep, the winit thread pumps nothing — input events wait
  out the sleep, adding up to a full frame budget of latency at the cap, and
  the sleep granularity fights the OS timer (Windows ~1.5 ms).
- **Ideal:** pacing via the event loop itself (`ControlFlow::WaitUntil(next
  frame deadline)`) so events are processed the instant they arrive and the
  wake happens at the deadline without blocking the pump.
- **Gap:** self-inflicted input latency proportional to how well the limiter
  is doing its job.
- **Suggestion:** replace the sleep with WaitUntil-based scheduling (tick on
  deadline reached; request_redraw stays).
- **Path:** (1) restructure the limiter; (2) verify headlessly that tick
  cadence still matches max_fps (DevStats frame-time histogram); input-feel
  confirmation is the user's manual check.

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run.
