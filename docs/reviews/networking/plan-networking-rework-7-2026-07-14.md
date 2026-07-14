# Plan: Collision-aware prediction replay — 2026-07-14

*Stale as of 2026-07-14: hygiene reworks 1 and 2 decomposed client net.rs and server net_plugin.rs into module families — the citations below predate them. Re-run /plan-rework for this finding before executing.*

Source: `docs/reviews/networking/reworks-networking-2026-07-11.md` finding 7.

## Ideal end state

The client's prediction — both the live per-tick optimistic step and the
rewind-and-replay in `reconcile_own` — runs the same movement rule the server
runs, including static-geometry collision response and the `PLAY_RADIUS`
boundary clamp. Pressing into a wall at 150 ms RTT produces zero prediction
error beyond quantization noise: no constant correction tug, no snap. The
shared-rule contract in `docs/online-play.mmd` (the `SHARED` node,
`online-play.mmd:86-88`) is extended to say "movement + static collision" and
stays true by construction: one pure crate-side function is the rule, and the
live server pipeline is equivalence-tested against it.

## Design decisions

**1. The shared rule is a pure function in `vordar-game`, composed from the
exact primitives the live systems already use — not a client-side
re-implementation.** Today the "movement rule" is three places: `movement_velocity`
(`game/vordar-game/src/player/mod.rs:28-31`, already shared), `MovementSystem`'s
integration + `PLAY_RADIUS` clamp (`game/vordar-game/src/motion/movement.rs:19-31`),
and `SeparationSystem`'s damped-MTV response (`game/vordar-game/src/motion/separation.rs:38-80`:
accumulate `mtv * 2.0` per anchored contact, apply the sum `* CORRECTION_PERCENT`).
The plan extracts `motion::movement::integrate` (integration + clamp — the
system calls it) and adds `motion::separation::anchored_push` (gate each static
through `shapes_overlap`, sum full-MTV corrections, damp once by
`CORRECTION_PERCENT` — same module as `mtv`/`SLOP`/`CORRECTION_PERCENT`, so the
constants cannot drift), composed as `motion::predict_step`. `engine-physics`'s
private `shapes_overlap` (`smirk/engine-physics/src/narrowphase.rs:96-116`)
becomes `pub` so the gate is the narrowphase's own test (it includes the y-axis
check the XZ-only `mtv` lacks — omitting it would make `anchored_push` push on
pairs the live `ActivePairs` never contains). Equivalence with the live pipeline
(Movement → CellUpdate → Broadphase → Narrowphase → Separation, the order
`plugin.rs:59-77` + `engine-physics/src/lib.rs:57-63` fix) is proven by a
behavioral test driving the real systems tick-by-tick against the pure fold —
bit-exact for a single contact; a small epsilon for simultaneous multi-contact,
because `SeparationSystem` sums f32 corrections in `HashSet` iteration order.
*Rejected:* refactoring `SeparationSystem` itself to call the pure function —
its accumulation is pair-based across all dynamic entities and doesn't decompose
per-walker without contorting it; the equivalence test is the honest contract.

**2. The predicted set is static geometry only: `Solid + Anchored`.** Dynamic
solids (other players, enemies) are excluded from both replay and local
prediction. Rationale: since rework 4, remotes render from a buffer ~200 ms in
the past — colliding the predicted player against time-delayed positions would
*create* mispredictions, and the server's dynamic separation is half-strength,
mutual, and unpredictable client-side. Server-side dynamic contacts keep
landing in the existing Trust/Smooth bands (`TRUST_DISTANCE = 0.3`,
`client/vordar-client/src/net.rs:44`), exactly as today. Anchored NPCs
(villager/elder prefabs) count as statics — correct, since the server's
`Anchored` side never yields (`separation.rs:56-65`).

**3. Replay context ownership: `reconcile_own` collects statics from the client
world per reconciliation — no new resource, no cache.** The client already holds
every nearby static as a fully-componented entity: AOI enters spawn the complete
prefab via the shared registry (`net.rs:644`, `register_core_components`
registers `Hitbox`/`Solid`/`Anchored`, `smirk/engine-core/src/prefab.rs:264-277`),
and the server replicates everything with a `PrefabId` (`net_plugin.rs:1227`),
buildings included. A `world.query` for `(&Transform, &Hitbox)` filtered on
`Solid + Anchored` (the `hecs::Satisfies` pattern `separation.rs:47-48` already
uses) yields a dozen-odd entries; folding ≤ 240 pending intents × ~dozens of
statics is thousands of cheap float ops. The cost question the finding raises is
answered by measurement: the `client/reconcile` bench gains a with-statics
variant. *Rejected:* a cached `StaticGeometry` resource — invalidation on AOI
enters/leaves buys nothing at this entity count.

**4. Local prediction gains the same rule via a small own-player system, not
`PhysicsPlugin` on the networked client.** Replay alone cannot reach the Ideal:
the live optimistic step (`MovementSystem`, registered in the predict branch,
`net.rs:175-178`) free-flights through walls, so the local position diverges
from a wall-clamped replay at ~6 u/s and snaps every few snapshots. A
`PredictedStaticCollisionSystem` applies `anchored_push` to the own player each
Update tick, after the last position writer. Running the full physics pipeline
client-side instead would fight `NetInterpolateSystem` for remote entities'
transforms and drag dynamic obstacles back into the predicted set (decision 2).

**5. Quantization residue is accepted and absorbed by the trust band.** Client
wall positions and the reconcile rebase point are both `WirePos`-quantized
(1/256 m, `vordar-protocol/src/lib.rs:208-220`, deliberately an order of
magnitude under `TRUST_DISTANCE`), so replayed-vs-server contact points can
differ by ulps-to-millimeters — Trust territory, never a correction.

**6. The dash-correction suppression stays.** `reconcile_own`'s
`still_reconciling_a_dash` early-return (`net.rs:776-796`) has two rationales:
collision non-replay (this rework removes it) and server-side dash timing skew
(the server mirrors the cast one-way-delay later and drains intents one per
tick — still true). Removing the suppression is a separate behavioral change
guarded by `onslaught_dash_replay_never_snaps_at_150ms_rtt`; out of scope here.
The comments citing finding 7 as unimplemented (`net.rs:788-793`, `815-819`) are
updated instead.

No product-level decisions are open; the dynamic-obstacle exclusion is an
engineering call justified by decision 2.

## Findings (execution order)

### 1. Extract the shared movement + static-collision rule as pure functions, equivalence-tested against the live pipeline

- **Evidence:** The live movement rule is spread across three seams that
  prediction cannot currently reuse: `game/vordar-game/src/motion/movement.rs:19-31`
  (`MovementSystem` integrates `Velocity` into `Transform` then clamps XZ to
  `PLAY_RADIUS = 65.0` inline); `game/vordar-game/src/motion/separation.rs:38-80`
  (`SeparationSystem` reads `ActivePairs`, and for a `Solid` walker against a
  `Solid + Anchored` static accumulates `mtv(...) * 2.0` per contact into a
  per-entity sum, then applies `sum * CORRECTION_PERCENT` (0.8); `mtv` at
  `separation.rs:85-121` subtracts `SLOP = 0.01` and is XZ-only for AABBs);
  and `smirk/engine-physics/src/narrowphase.rs:96-116` (`fn shapes_overlap`,
  **private**, the 3-axis test that decides `ActivePairs` membership — the
  phase order Update → Collision → CollisionResolve is fixed by
  `game/vordar-game/src/plugin.rs:59-77` and `smirk/engine-physics/src/lib.rs:57-63`).
  Only `movement_velocity` (`game/vordar-game/src/player/mod.rs:28-31`) is
  already a shared pure function.
- **Ideal:** `vordar_game::motion` exposes the whole per-tick player movement
  rule as pure functions: `movement::integrate(pos, velocity, dt) -> Vec3`
  (integration + `PLAY_RADIUS` clamp; `MovementSystem::run` now calls it),
  `separation::anchored_push(pos, shape, statics: &[(Vec3, CollisionShape)]) -> Vec3`
  (for each static: skip unless `engine_physics::narrowphase::shapes_overlap`
  — made `pub` — says the pair would be in `ActivePairs`, then accumulate
  `mtv(pos, shape, s_pos, s_shape) * 2.0`; return `sum * CORRECTION_PERCENT`),
  and `motion::predict_step(pos, velocity, dt, shape, statics) -> Vec3` =
  `integrate` then `pos + anchored_push`. A behavioral test proves the pure
  step and the live pipeline compute the same positions.
- **Gap:** `integrate` doesn't exist (the clamp is inline in the system),
  `anchored_push`/`predict_step` don't exist, and `shapes_overlap` is private
  to `engine-physics`, so nothing outside the narrowphase can reproduce the
  `ActivePairs` gate.
- **Suggestion:** Pure refactor plus new functions — zero behavior change to
  any registered system. Keep `mtv`, `SLOP`, `CORRECTION_PERCENT` private to
  `separation.rs`; `anchored_push` lives beside them so the constants are
  shared by construction. Document on `predict_step` that it is the prediction
  half of the shared-rule contract (DESIGN.md §6 determinism, same as
  `movement_velocity`'s doc comment).
- **Path:** (1) Make `shapes_overlap` `pub` in
  `smirk/engine-physics/src/narrowphase.rs` (doc: "the `ActivePairs` membership
  test; `anchored_push` in vordar-game must gate through this"). (2) Extract
  `pub fn integrate` in `movement.rs`; `MovementSystem::run` delegates to it
  (existing tests `movement.rs:38-71` keep passing untouched). (3) Add
  `pub fn anchored_push` in `separation.rs` and `pub fn predict_step` in
  `motion/mod.rs`. (4) The proving test, in `game/vordar-game` (e.g. a
  `#[cfg(test)]` mod in `motion/mod.rs`): build a `World` with a walker
  (`Transform` at origin, `Velocity` +X at 6.0, `Hitbox` AABB half-extents 0.5
  — the player prefab's shape, `content/prefabs/player.ron:10` —
  `CellOccupant`, `Solid`) and an anchored wall (`Transform` at (3.0, 0, 0),
  `Hitbox` AABB half-extents (1.6, 0.9, 1.3) — the cottage's,
  `content/chapters/chapter02/prefabs/cottage.ron:11` — `CellOccupant`,
  `Solid`, `Anchored`); insert `SpatialGrid::new(10.0)`, `CandidatePairs`,
  `ActivePairs`, `EventBus`; each of ~60 ticks run the real
  `MovementSystem`, `CellUpdateSystem`, `BroadphaseSystem`,
  `NarrowphaseSystem`, `SeparationSystem` in registration order at dt = 1/60,
  and in parallel fold `predict_step` from the same start; assert the walker's
  `Transform.position` equals the fold's position every tick (exact
  `==` — single contact has one float path). Add a second case with two
  overlapping anchored walls asserting agreement within 1e-5 (multi-contact
  f32 summation order differs). Also assert the walker ended pressed against
  the wall, not through it (the scenario is real, not vacuous). Workspace
  stays green: pure additions + delegating refactor.

### 2. Fold static collision into the reconciliation replay

- **Evidence:** `client/vordar-client/src/net.rs:820-829` — `replay_position`
  folds pending intents as free-flight `pos + velocity * dt` steps (leap
  override at `net.rs:826`), with neither the `PLAY_RADIUS` clamp nor collision;
  `reconcile_own` (`net.rs:768-813`) calls it at `net.rs:786` and classifies the
  error into Trust/Smooth/Snap (`TRUST_DISTANCE = 0.3` at `net.rs:44`,
  `SNAP_DISTANCE = 1.0` at `net.rs:48`). Comments at `net.rs:788-793` and
  `815-819` name this rework as the missing piece. Statics exist in the client
  world as full prefabs (AOI enters spawn via the shared registry,
  `net.rs:644`; `Hitbox`/`Solid`/`Anchored` registered by
  `register_core_components`, `smirk/engine-core/src/prefab.rs:264-277`).
  Existing unit tests calling `replay_position` directly:
  `net.rs:1789`, `1800-1801`, `1822-1829`. Bench:
  `benchmarks/benches/client_netcode.rs:126-152` (`bench_reconcile`, 60/240
  pending, via the `seam` module `net.rs:1354+` whose `reconcile_own` re-export
  keeps its signature).
- **Ideal:** `reconcile_own` collects the static set once per call — a
  `world.query::<(&Transform, &Hitbox, hecs::Satisfies<&Solid>, hecs::Satisfies<&Anchored>)>`
  keeping only `Solid && Anchored` entries as `(Vec3, CollisionShape)` — plus
  the own player's `Hitbox` shape, and `replay_position` folds
  `vordar_game::motion::predict_step` (finding 1) instead of the bare
  integration: each pending intent's velocity (leap override preserved) is
  integrated with the `PLAY_RADIUS` clamp, then pushed out of statics exactly
  as the server's `SeparationSystem` would. Wall-hug replay error vs the
  server's authoritative position is quantization-order — Trust band.
- **Gap:** `replay_position` takes no shape and no statics; the fold is
  `pos + velocity * dt` with no clamp and no push.
- **Suggestion:** Extend `replay_position` to
  `replay_position(server_pos, speed, pending, shape: &CollisionShape, statics: &[(Vec3, CollisionShape)])`;
  if the player has no `Hitbox` (defensive only — the prefab always has one)
  pass an empty statics slice. Update the comments at `net.rs:788-793` and
  `815-819`: collision is now replayed; what remains suppressed during a dash
  is server-side timing skew only. Leave the `still_reconciling_a_dash`
  early-return itself untouched (design decision 6).
- **Path:** (1) Fail-first test in `net.rs`'s test module at the
  `reconcile_own` level (signature unchanged, so it compiles pre-fix): world
  with the own player (`Transform`, `Hitbox` 0.5-half AABB, `Player { speed: 6.0 }`,
  `Solid`) at a wall face, an anchored wall entity (cottage-shaped hitbox,
  `Solid + Anchored`) directly +X of it, and a `NetClientState` (mirror the
  `state_for_bench`-style construction used by neighboring tests) whose
  `pending` holds ~30 intents walking +X; `server_pos` = the live-pipeline
  wall-equilibrium position (compute it by folding `predict_step` — finding 1's
  equivalence test already proved that equals the real pipeline). Pre-fix,
  free-flight replay overshoots ~2 units into the wall → error > `SNAP_DISTANCE`
  → `reconcile_own` snaps `Transform.position` into the wall: assert instead
  that the player's transform moves by less than `TRUST_DISTANCE` across the
  call (Trust classification) — fails today, passes after. (2) Implement the
  fold change + statics collection. (3) Update the two comment blocks and the
  three existing `replay_position` call sites in tests (pass the player shape
  and empty statics — their assertions are about leap-vs-WASD divergence, not
  walls). (4) Extend `bench_reconcile` with a with-statics variant (spawn ~32
  anchored hitbox entities into the bench world; same 60/240 pending cases) so
  the finding's per-reconciliation cost question has a number. (5) Workspace
  green: `cargo test -p vordar-client` plus the touched bench compiling
  (`cargo bench -p benchmarks --bench client_netcode --no-run` or equivalent).

### 3. Local optimistic prediction collides with statics; wall-hug e2e at 150 ms RTT

- **Evidence:** The networked client's predict branch registers
  `PlayerMovementSystem`, `LeapSystem`, `MovementSystem`, `NetCorrectionSystem`
  (`client/vordar-client/src/net.rs:175-178`) and no collision system of any
  kind — only the sandbox (`client/vordar-client/src/bin/sandbox.rs:41`) runs
  `PhysicsPlugin`. So even with finding 2's replay fix, the locally displayed
  player free-flights through a wall at 6 u/s while the replayed position stays
  wall-clamped: error grows ~6 u/s, crosses `SNAP_DISTANCE = 1.0` within
  ~0.17 s, and `reconcile_own` snaps every few snapshots — the finding's
  "constant correction tug", now expressed as local-vs-replay divergence. The
  existing 150 ms harness this step's test extends is
  `onslaught_dash_replay_never_snaps_at_150ms_rtt`
  (`net.rs:2008-2187`): boots `vordar_server::build_server_app(addr, ":memory:")`
  in a thread, drives the exact predict-branch systems directly, and watches
  every `NetReceiveSystem::run` for a > `SNAP_DISTANCE` position jump. Server
  facts the test needs: players spawn on a 3-unit ring (`spawn_position`,
  `server/vordar-server/src/net_plugin.rs:316-319`); the zone prefab table is
  built lazily from the fully-loaded `PrefabLibrary` at first login
  (`net_plugin.rs:760-786`), so a test may `app.add_prefab_dir(...)` after
  `build_server_app` and before `run_headless`; everything with a `PrefabId`
  replicates (`net_plugin.rs:1227`).
- **Ideal:** A `PredictedStaticCollisionSystem` in `net.rs` applies
  `vordar_game::motion::separation::anchored_push` to the own player's
  `Transform` each Update tick (querying statics the same way finding 2's
  `reconcile_own` does), registered only in the predict branch after the last
  position writer (`NetCorrectionSystem`), so the displayed position obeys
  walls tick-by-tick exactly like the server. Walking into a wall for seconds
  at 150 ms RTT produces zero Snap-class corrections.
- **Gap:** No such system exists; local prediction and (post-finding-2) replay
  disagree at every wall.
- **Suggestion:** Keep the system minimal: resolve the own entity from
  `NetClientState`, read its `Hitbox`, collect `Solid + Anchored` statics,
  `transform.position += anchored_push(...)`. Register with
  `Phase::Update, SystemOrder::after::<NetCorrectionSystem>()` in the
  `net.rs:175-178` block (if the scheduler's `after` interacts badly with two
  `SystemOrder::Last` peers, register it as the final `Last` in registration
  order — behavior, not mechanism, is the contract and the e2e proves it).
- **Path:** (1) Implement the system + registration. (2) The wall-hug e2e in
  `net.rs`'s test module, patterned on `onslaught_dash_replay_never_snaps_at_150ms_rtt`:
  build the server app, `.add_prefab_dir("content/chapters/chapter02/prefabs")`,
  and `.add_system(...)` a test-local spawn-once system that waits for the
  first `Player` entity and spawns the `"cottage"` prefab (Anchored,
  1.6×0.9×1.3-half hitbox) 6 units +X of it via `spawn_prefab`; then
  `run_headless(60.0, ...)`. Client side: same registry/prefab setup as the
  onslaught test **plus** `prefabs.load_dir("content/chapters/chapter02/prefabs")`,
  connect with `connect_with_latency(addr, PROTOCOL_VERSION, 150 ms)`, wait for
  Welcome + own entity, then hold a +X `MoveIntent` (the normal
  `PlayerInputSystem`-equivalent path the harness already drives through
  `NetSendInputSystem`) for ~2 s of ticks, running the predict-branch systems
  plus the new `PredictedStaticCollisionSystem` each tick, tracking
  `max_recv_jump` across every `NetReceiveSystem::run` exactly as the
  onslaught test does. Assert `max_recv_jump < SNAP_DISTANCE` (the finding's
  Path wording), and additionally that the displayed position's X never
  exceeds the wall face by more than ~0.15 (equilibrium penetration
  `SLOP + v*dt/CORRECTION_PERCENT` ≈ 0.135 — proves the hug is real). Verify
  fail-first by running the test with the new system's registration commented
  out locally before finalizing (with finding 2 landed but no local collision,
  the divergence still snaps). (3) Green: full `cargo test -p vordar-client`.

### 4. Documentation: shared-rule contract in the online-play diagram, baseline number, queue strike

- **Evidence:** `docs/online-play.mmd:86-88` — the `SHARED` node reads
  "shared movement rule<br/>same math on client and server" and is linked from
  `CP` (client predict) and `SM` (server sim); node `R3` (`online-play.mmd:20`)
  describes replay as "replay not-yet-processed intents". `docs/online-play.svg`
  is the rendered copy. The reworks queue note
  (`docs/reviews/networking/reworks-networking-2026-07-11.md:10-41`) requires every plan
  touching the online-play flow to update the diagram, and rework 7 is still
  unstruck in the queue line. `docs/benchmarks/BASELINE.md` carries the client
  netcode numbers (WEAKPOINTS gap B) that `benchmarks/benches/client_netcode.rs`
  produces; finding 2 added a with-statics `client/reconcile` variant whose
  number is not yet recorded.
- **Ideal:** The diagram's shared-rule contract names collision: `SHARED`
  says the rule is movement **and static-geometry collision**, `R3` says
  "replay not-yet-processed intents<br/>(full rule: movement + static collision)";
  SVG regenerated and in sync. BASELINE.md records the measured
  with-statics reconcile cost next to the existing 60/240-pending numbers.
  The reworks file's queue note strikes 7 with a pointer to this plan.
- **Gap:** Diagram still describes the pre-rework contract; no baseline entry
  for the new bench variant; queue note unstruck.
- **Suggestion:** Load the `mermaid-diagrams` skill for the `.mmd` edit + SVG
  regeneration; keep the wording changes to the two nodes (no structural
  changes). Run the `client_netcode` bench once to get the with-statics
  number for BASELINE.md.
- **Path:** (1) Edit `docs/online-play.mmd` nodes `SHARED` and `R3`; regenerate
  `docs/online-play.svg` (mermaid-diagrams skill). (2) Run
  `cargo bench -p benchmarks --bench client_netcode` and add the
  `client/reconcile` with-statics row to `docs/benchmarks/BASELINE.md`'s client
  netcode section with a one-line note (statics count, per-reconcile cost).
  (3) Strike rework 7 in `docs/reviews/networking/reworks-networking-2026-07-11.md`'s
  queue note ("7 done 2026-MM-DD (plan-networking-rework-7-2026-07-14.md,
  4 steps; ...)" following the existing pattern). (4) Verification: the SVG
  renders (skill's own check), `git diff` shows only the named docs, and the
  workspace test suite still passes untouched.
