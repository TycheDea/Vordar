# Expert Review: Engine Architecture & ECS
**Reviewer persona:** Principal Rust Game-Engine Architect
**Date:** 2026-07-27
**Scope:** smirk/* engine, app composition, workspace layering

## Executive summary

Smirk is a deliberately thin, Bevy-shaped-but-not-Bevy plugin ECS: `hecs::World` + a typed resource map + a phase scheduler + a single-frame `EventBus`, composed by `Plugin`s. The crate graph is clean for an MMO-shaped dual binary (shared `vordar-game`, headless server without winit/wgpu, client with renderer). Zone-per-thread is real and operational today — one `App` per zone thread, shared DB worker, shared world-time origin, panic supervision with restart budget.

The architecture’s strongest moves are the **deferred spawn/despawn queues**, **fixed-step phase interleaving** (same-step spawn → collision visibility), **prefab string-addressable spawn seam**, and the **content vs simulation plugin split** (chapters install full sim on the server, content-only on the networked client). Those are production patterns worth protecting.

The single most important defect is that **`set_phase_rate` is a lie at the API surface**: docs and DESIGN.md advertise independent per-phase fixed rates, but the scheduler collapses every fixed phase onto one app-wide `fixed_dt`, and later phases in `Phase` enum order overwrite earlier overrides during `build()`. A call like `.set_phase_rate(Phase::Update, TickRate::Fixed(30.0))` is effectively a no-op while any later fixed phase still defaults to 60 Hz. That blocks the DESIGN.md combat model’s “movement 60 / snapshot 5–10” mapping and will mislead every future rate-tuning attempt.

Secondary structural risks: systems/`App` are not `Send` (zone rebuild must happen on-thread — already documented, still a fleet constraint); `vordar-game` reaches into `engine-physics` internals (`ActivePairs`, narrowphase helpers) rather than only EventBus surfaces; `engine-audio` is an empty stub; the offline sandbox binary does not install any chapter module, so the “fast iteration” path cannot exercise chapter content; and several startup/content failures panic or silently skip without a unified error surface.

Overall grade: **solid custom engine core for the stated MMO shape**, with one scheduler-contract bug that should be fixed before further tick-rate design work, and a few layering seams to harden as content and zone count grow.

## Findings

### F1. [SEVERITY: Critical] `set_phase_rate` claims per-phase fixed rates; scheduler has one app-wide clock that later phases overwrite

- **Where:** `smirk/engine-app/src/scheduler.rs` (`Scheduler::{set_phase_rate, build, run_tick}`), `smirk/engine-app/src/app.rs` (builder docs), `smirk/engine-app/src/lib.rs` / `tick_rate.rs`, DESIGN.md §5
- **What:** `Phase::default_tick_rate` and `App::set_phase_rate` present a per-phase `TickRate`. `build()` stores only a boolean `is_render` per phase and, for every `TickRate::Fixed(hz)`, assigns `self.fixed_dt = 1.0 / hz`. Because phases are walked in `BTreeMap`/`Phase` ord order, **the last fixed phase processed wins**. With defaults, every logic phase is `Fixed(60)`, so overriding only `Phase::Update` to 30 Hz is immediately overwritten when `SpawnFlush`…`PostUpdate` rebuild with their default 60 Hz. `run_tick` then steps **all** non-render phases together on that single accumulator — there is no independent phase cadence.
- **Why it matters:** DESIGN.md explicitly maps “movement at 60 Hz, combat/snapshot phase at 5–10 Hz” onto this API. Server net code already self-gates snapshot work inside `PostUpdate` at `SNAPSHOT_HZ` (`server/vordar-server/src/net/mod.rs`) precisely because the scheduler cannot do it. Anyone calling `set_phase_rate` for load shedding or combat rate will get silent wrong behavior. The app.rs usage comment showing `Fixed(30.0)` on Update alone is actively incorrect.
- **Recommendation:** Either (a) document and rename to an app-wide `set_fixed_hz(f32)` and delete the per-phase Fixed illusion, keeping only `Render` vs `Fixed` membership; or (b) implement true per-phase fixed clocks with explicit rules for multi-rate event/spawn visibility (harder — usually wrong for this combat model). Prefer (a) plus keep snapshot/mechanic throttling as explicit systems (current server pattern). Add a unit test that `set_phase_rate(Update, Fixed(30))` either errors or changes the single app clock only when intended.

### F2. [SEVERITY: High] Event lifetime is “one fixed step,” not “one frame,” and multi-step frames multiply side effects

- **Where:** `smirk/engine-app/src/events.rs` (module docs: “live for exactly one frame”), `ClearEventsSystem` in `flush.rs` / `app.rs` (`Phase::Input`, `First`), `scheduler.rs` `run_tick` multi-step loop
- **What:** On a long frame the scheduler runs the full fixed-phase stack up to 8 times. `ClearEventsSystem` clears the bus at the start of **each** fixed step’s Input phase. Events therefore live one fixed step (good for same-step consumers), but a 8× spiral-of-death catch-up will run AI, movement, collision, death, net receive side-effects, etc. eight times with eight clears — not “once per display frame.”
- **Why it matters:** Correct for deterministic catch-up *if* all gameplay is tick-based (it mostly is). Incorrect documentation causes systems authors to assume frame-coalesced events. Net receive in `Phase::Input` on the server will drain the socket up to 8 times per display-equivalent headless tick budget when the host stalls — usually fine, but death/broadcast and autosave tick counters also advance per fixed step (intended) while any future “once per frame” logic placed in a fixed phase will silently multi-fire.
- **Recommendation:** Rewrite EventBus docs to “one fixed step.” Keep the 8-step cap. Audit fixed-phase systems for accidental frame-rate assumptions. Consider a distinct `Phase::FrameStart` (render-rate) clear only if you ever need display-cadence events; do not mix the two without naming them.

### F3. [SEVERITY: High] `App` / systems are `!Send`; zone-per-thread is correct but rebuild-and-fleet topology is constrained by it

- **Where:** `smirk/engine-app/src/scheduler.rs` (`trait System: 'static` only; `Box<dyn System>`), `server/vordar-server/src/lib.rs` (`build_zone_app` docs: “systems aren't Send”), `server/vordar-server/src/main.rs` + `supervisor.rs`
- **What:** Zone threads build the `App` on-thread, run `run_headless`, and on panic drop the App (closing `NetServer` via Drop) then rebuild via `supervise_zone`. That works. You cannot move a live zone to another thread, steal work, or run a work-stealing parallel scheduler over systems without a large trait redesign (`System: Send`, resource `Send + Sync` bounds, hecs query discipline).
- **Why it matters:** Matches the chosen scale unit (“one zone instance = one thread = one App”) and is the right call for RO-scale zones. It becomes painful if you later want (1) parallel system stages inside a zone, (2) hot zone migration across processes without full rebuild, or (3) embedding multiple zones in one async runtime task. The constraint is acknowledged in code comments — keep it as an explicit architectural invariant, not an accident.
- **Recommendation:** Document `System: !Send` and “App is thread-affine” in the engine-app crate root. Keep zone migration as “drain + ticket + fresh App on target process” (DESIGN.md coordinator path). Do **not** half-add `Send` without also bounding `Resources` (`insert` currently accepts bare `Any`, while `App::insert_resource` requires `Send + Sync` — align these).

### F4. [SEVERITY: High] Game simulation depends on physics crate internals, not only EventBus contracts

- **Where:** `game/vordar-game/src/motion/separation.rs` (`use engine_physics::narrowphase::{shapes_overlap, ActivePairs}`), `game/vordar-game/Cargo.toml` (`engine-physics` dependency), enemy AI using `engine_core::spatial::SpatialGrid` directly
- **What:** Layering rule in engine-app says renderer/physics register via plugins and stay decoupled. `vordar-game` correctly avoids renderer/window/input, but **does** import physics narrowphase types and the spatial grid resource. Separation is a game system in `CollisionResolve` that reads `ActivePairs` produced by the physics plugin.
- **Why it matters:** Swapping or versioning the physics backend requires game changes. Headless tests must manually insert `SpatialGrid`/`ActivePairs` (they do). The EventBus collision events (`CollisionStarted`/`Ended`) exist but gameplay collision response bypasses them for the hot path — fine for perf, but the “all cross-module communication via EventBus” discipline (DESIGN.md §6) is not actually true for physics→game.
- **Recommendation:** Promote a small `engine-physics` public façade module (`pairs`, `overlap` tests, grid resource re-exports) and treat it as a supported API; or move `SeparationSystem` into `engine-physics` as an optional plugin feature and leave only damage/gameplay in `vordar-game`. Stop claiming EventBus-only cross-module communication unless you mean “gameplay intents,” not “collision pairs.”

### F5. [SEVERITY: Medium] Prefab directory load failures are soft; chapter RON load failures are hard — inconsistent content error surface

- **Where:** `smirk/engine-core/src/prefab.rs` (`PrefabLibrary::load_dir` logs and skips bad files; `queue_prefab_spawn` logs spawn errors), `game/vordar-game/src/world/chapter.rs` (`load_chapter` panics), zone/chapter install in server `main.rs` (`unwrap_or_else(|e| panic!(...))`)
- **What:** A corrupt prefab file leaves a hole in the library and only an error log; a corrupt chapter file kills the process; unknown chapter name panics at zone install; missing `ChapterDef` only warns once in `ChapterSetupSystem`.
- **Why it matters:** Content pipelines want fail-fast in CI and recoverable degrade in shipping tools. Today local iteration can boot “successfully” with missing enemy prefabs (spawns fail later per entity) while a typo in `chapter.ron` hard-crashes. Ops/restart supervisor will loop on chapter panic (good) but not on silent prefab skip (bad — zone looks empty).
- **Recommendation:** Introduce a `ContentLoadReport { errors, warnings }` resource filled by `load_dir`/`load_chapter`. Server boot: any error → refuse to mark zone healthy. Sandbox/tools: print report and optionally continue. Keep panic only behind an explicit `strict` path or `debug_assertions`.

### F6. [SEVERITY: Medium] Offline sandbox does not install chapters — “fast iteration” misses the content module path

- **Where:** `client/vordar-client/src/bin/sandbox.rs` vs `client/.../bin/vordar.rs` (`ChapterRegistry::install_all_content`) and `server/.../main.rs` (`chapters().install(...)`)
- **What:** Sandbox wires `RenderPlugin + PhysicsPlugin + PrefabPlugin + CoreGamePlugin + ClientPlugin` and spawns a local `ravager`. It never calls `ChapterRegistry` / chapter plugins, so camps, chapter prefab dirs, and chapter components never load.
- **Why it matters:** README markets sandbox as the fastest iteration loop. Chapter authors must run full server+client (or write custom bins) to see their content. Diverges the two App compositions further and risks “works in sandbox, broken in chapter” gaps (and the reverse).
- **Recommendation:** Sandbox should take an optional `--chapter chapter01` (default one dev chapter) and call `ChapterRegistry::install`, mirroring the server. Keep a `--bare` mode for pure engine smoke if needed.

### F7. [SEVERITY: Medium] `Resources` type-map bounds and panic style are uneven

- **Where:** `smirk/engine-core/src/traits.rs` (`Resources::insert<T: Any>`, `expect`/`expect_mut` panic strings), `smirk/engine-app/src/app.rs` (`insert_resource: Any + Send + Sync`), physics systems using `.expect("SpatialGrid not in resources")` vs `resources.expect_mut::<EventBus>()`
- **What:** Two insertion APIs with different bounds; two failure dialects (`expect` on `Option` with custom strings vs `Resources::expect`). Missing plugins fail at first system tick, not at `build()`/startup validation.
- **Why it matters:** Plugin mis-order or forgotten `PhysicsPlugin` becomes a mid-tick panic rather than a startup diagnostic listing missing resources. `Send + Sync` holes allow putting non-thread-safe junk into resources via `resources.insert` if a system holds `&mut Resources`.
- **Recommendation:** Make `Resources::insert` require `Send + Sync + 'static`. Add optional `App::validate()` / plugin-declared `requires::<T>()` checked once after plugins build. Standardize on `resources.expect::<T>()` everywhere.

### F8. [SEVERITY: Medium] Spatial grid is XZ-hash only; `query_radius` is a cell AABB, not a true radius filter

- **Where:** `smirk/engine-core/src/spatial.rs`, consumers in `engine-physics` broadphase and `vordar-game` enemy AI (`query_radius_into` + manual distance)
- **What:** Grid keys on `(floor(x/cell), floor(z/cell))` — Y ignored (correct for ground-plane MMO). `query_radius` / `query_radius_into` include **all entities in overlapped cells**, not those within Euclidean radius. Cell size is hard-coded `10.0` in `PhysicsPlugin`.
- **Why it matters:** Fine if every caller distance-filters (enemy AI does). Broadphase correctly uses cells as candidate generation. A future interest-management or gameplay caller that trusts the name `query_radius` will over-include (AOI on the server appears to use its own logic in net broadcast — verify any new grid users). Hard-coded cell size couples entity scale to physics plugin.
- **Recommendation:** Rename to `query_cell_approx` / document “superset of radius.” Consider `SpatialGrid::new` from config/RON. Add debug assert stats (entities/cell) to `PhysicsStatsSystem` (partially present) and warn on pathological density.

### F9. [SEVERITY: Medium] Heaps of startup/path I/O assume process cwd == workspace root

- **Where:** README, all binaries’ comments; `add_prefab_dir("content/...")`, `configure("content/config/engine.ron")`, chapter/zone loads, client credentials default path
- **What:** Content resolution is entirely cwd-relative strings. No content root resource, no canonicalization, no pack hash (DESIGN.md content-distribution is future).
- **Why it matters:** Running bins from `target/debug` or an IDE with a different working directory fails in non-obvious ways (empty prefab library, default window config, panic on chapter path). Multi-zone content packs and client patching need a root abstraction eventually.
- **Recommendation:** Insert a `ContentRoot(PathBuf)` early in each binary (env `VORDAR_CONTENT` + searchable defaults). Make `add_prefab_dir` / `load_chapter` / `load_zones` join against it. Cheap now; painful after more path call sites land.

### F10. [SEVERITY: Medium] Scheduler topological sort is registration-time only; duplicate system types and stringly plugin order still matter for startup hooks

- **Where:** `smirk/engine-app/src/scheduler.rs` (`index_of: HashMap<TypeId, usize>` — duplicate `TypeId` silently overwrites), `plugin.rs` (plugin order = startup callback order), `App::on_window_ready` / `on_resize_fn`
- **What:** Ordering constraints key on `TypeId`. Two instances of the same system type in one phase collapse in `index_of` (last wins) while both remain in the systems vec — adjacency can point at the wrong index; cycles/unresolved targets panic clearly (good tests exist). Window init hooks are strictly plugin-registration ordered (necessary for wgpu surface).
- **Why it matters:** Low probability today (systems are distinct types), high confusion if someone registers the same struct twice or uses type-erased wrappers. Startup order bugs (renderer before input bridge, etc.) only show at runtime.
- **Recommendation:** Panic on duplicate `TypeId` in a phase during `build()`. Keep plugin order for hooks; document “Phase+SystemOrder for per-tick, add_plugin order for startup only” at the `Plugin` trait (already partly there — strengthen).

### F11. [SEVERITY: Low] `engine-audio` is a published workspace member with an empty body

- **Where:** `smirk/engine-audio/src/lib.rs` (“empty stub crate; audio not yet built.”), root `Cargo.toml` members
- **What:** Occupies the layer graph and mental model without API, plugin, or feature flags.
- **Why it matters:** Harmless compile cost; risks someone depending on it for “the audio seam” that does not exist. Prefer no crate until the first real `AudioPlugin` lands, or ship a minimal trait + no-op backend so the seam is real.
- **Recommendation:** Remove from default members until implemented, or add `AudioPlugin` no-op with a `Backend` trait and one `log` implementation so game code can emit `PlaySound` events now.

### F12. [SEVERITY: Low] Render components live in `engine-core`, pulling presentation vocabulary into the headless sim

- **Where:** `smirk/engine-core/src/components.rs` (`RenderShape`, `ShapeGroup`, `RenderMesh`, `PointLight`, `AnimationPlayer`), registered in `register_core_components`, used by prefabs on the server
- **What:** Core’s own header says no rendering dependency (true — no wgpu), but core **owns** render-oriented components so prefabs can deserialize them on the dedicated server. Server replicates/`PrefabId` and may carry meshes it never draws.
- **Why it matters:** Pragmatic for one prefab file shared by client and server. Purists will hate server worlds full of `RenderMesh` strings. Memory is usually fine at RO scale; the real cost is conceptual leakage and larger component registry surface on headless.
- **Recommendation:** Accept for now (correct for data-driven shared prefabs). Longer term: split `engine-core` / `engine-prefab-components` or mark render components with a feature `presentation-components` if server memory/cold-start ever matters. Do not move them into `engine-renderer` or headless won’t deserialize prefabs.

### F13. [SEVERITY: Low] Fixed-step PreviousTransform bookkeeping is implied, not engine-enforced

- **Where:** `PreviousTransform` in `engine-core` components; injected by Transform prefab loader; consumed by client `render_position` / renderer sync; **writer** of per-step copies is game/renderer convention
- **What:** Engine documents that PreviousTransform is “saved at the start of each fixed step,” but the scheduler does not itself snapshot transforms. If a movement path forgets to update PreviousTransform, interpolation breaks (camera vibration comments in client show this was already felt).
- **Why it matters:** Classic fixed-timestep footgun. Works while one MovementSystem owns the write; breaks when new motion systems appear.
- **Recommendation:** Add an engine `PreviousTransformSystem` at `Phase::Update`/`First` or end of previous step that copies `Transform → PreviousTransform` for all matched entities, and forbid ad-hoc copies elsewhere.

### F14. [SEVERITY: Low] Panic surfaces are used as control flow for “unrecoverable” config; zone supervisor converts them into restarts

- **Where:** `supervise_zone` (`catch_unwind`), chapter install panics, `NetServer::bind` panics, `run` event loop `expect`s, scheduler cycle panics at build
- **What:** Intentional: bad schedule = don’t boot; bad bind = don’t boot; zone sim panic = restart with strike budget (`MAX_ZONE_RESTARTS = 3`, healthy uptime 60s). Process-level second signal force-exits.
- **Why it matters:** Good operational shape for dedicated servers **if** panics are truly invariant violations. Dangerous if gameplay code `unwrap`s on bad player input or one corrupt entity — one entity could restart the whole zone and drop everyone.
- **Recommendation:** Keep panic for schedule/bind/content-invariant failures. Ban panic in per-entity net/gameplay paths (return/log/despawn). Add a lint/CI grep for `unwrap(` in `vordar-game` and server net receive beyond tests. Consider `AssertUnwindSafe` scope only around `run_headless` inner tick if you ever need finer isolation (usually not worth it with hecs).

### F15. [SEVERITY: Info] Crate layering and dual-binary composition are clean and match the architecture diagram

- **Where:** workspace `Cargo.toml`; `smirk/engine-{core,app,physics,renderer,net,audio}`; `game/vordar-{game,protocol}`; `game/chapter-0{1,2}`; `client/vordar-client`; `server/vordar-server`; `docs/architecture.mmd`
- **What:** Dependency direction holds:
  - `engine-core` ← foundation (hecs/glam/serde/ron), no app/renderer/physics
  - `engine-app` → core only; optional `winit` feature; headless via `default-features = false`
  - `engine-physics` / `engine-renderer` → app + core, register via `Plugin`
  - `engine-net` → pure QUIC/tokio/postcard, **no** ECS dependency (excellent)
  - `vordar-game` → core/app/physics, no winit/wgpu in normal deps (renderer only as dev-dep for content lint)
  - chapters → app + game
  - client → everything presentation + game + chapters + net
  - server → app(headless) + physics + game + chapters + net + rusqlite
- **Why it matters:** This is the structural reason client prediction can share code with authority and the dedicated server stays free of wgpu. Preserve it in CR review (no “just this once” client types in `vordar-game`).
- **Recommendation:** Add a simple `cargo deny` / custom script or `tests` that `vordar-game`’s normal dependency tree does not include `winit`/`wgpu`. Document the graph in-repo next to `architecture.mmd` (crate-level, not only product-level).

### F16. [SEVERITY: Info] Plugin API + chapter registry are the right extension seams for modding and MMO content packs

- **Where:** `smirk/engine-app/src/plugin.rs`, `prefab_plugin.rs`, `App::{register_component, add_prefab_dir, resource_or_default}`, `game/vordar-game/src/world/chapter_registry.rs`, chapter-01/02 `module()` export
- **What:** One `Plugin` trait (`build`, `name`). Chapters export `ChapterModule { name, requires, install, install_content }` with DFS deps, cycle detection, and content-only install for display clients. Prefab compile-once (`OnceLock<Vec<CompiledComponent>>`) makes string spawns cheap after first use. `queue_prefab_spawn` is the network-friendly seam DESIGN.md wanted.
- **Why it matters:** This is how you scale content without forking the engine. The content/sim split is exactly what networked entity display needs. Dependency `requires` already encodes cross-chapter prefab reuse (chapter02 → chapter01).
- **Recommendation:** Keep chapter linking as compile-time registry in the binary for now; when packs go dynamic, replace only `ChapterRegistry::new(vec![...])` construction, not the install algorithm. Stabilize prefab/component string names as a public contract.

### F17. [SEVERITY: Info] Build hygiene is above average for a young workspace

- **Where:** root `Cargo.toml` (`[workspace.lints]`, shared deps, `profile.dev` `debug = 1`), `deny.toml` (licenses, advisories, unknown git/registry deny, Windows target graph), edition `2024`, `publish = false` on members, feature-gated offscreen/bench bins
- **What:** Workspace lints on, deny.toml present, heavy debug info trimmed, optional features keep headless and tools from bloating default builds. `engine-app` winit default-features pattern is correct for server.
- **Why it matters:** Prevents the usual multi-crate dependency drift and accidental copyleft. Supports the “server + client pack” shipping story.
- **Recommendation:** CI should run `cargo deny check` + `cargo clippy --workspace -D warnings` + a headless `vordar-server` smoke. Consider `multiple-versions = "deny"` once warn noise is cleaned (currently warn).

### F18. [SEVERITY: Info] HeCS patterns in use are appropriate; don’t reach for a heavier ECS yet

- **Where:** throughout; notable: deferred `SpawnQueue`/`DespawnQueue`, `EntityBuilder` prefab plans, `hecs::Satisfies` in separation, command-buffer style flush phases, no parallel queries
- **What:** Single-threaded world mutation with explicit flush phases avoids iterator invalidation. Prefab plans clone components into builders. Spatial occupancy stored on entities (`CellOccupant`) with incremental grid diffs — good cache/allocator behavior. No change detection, no archetypal relations, no system pipelining — and at “hundreds per zone” you do not need them yet (DESIGN.md §7 agrees).
- **Why it matters:** Teams often rewrite to Bevy/Shipyard too early. Here the bottlenecks will be net fan-out, persistence, and content tooling — already reflected in server AOI/stagger snapshot design.
- **Recommendation:** Stay on hecs until a profiler shows world query time dominating. If you need relations (inventory, scene graph), prefer explicit components/tables over an ECS migration.

## Strengths worth preserving

1. **Phase stack with interleaved fixed steps** — same-step spawn visibility into collision is a rare, correct choice; tests lock it in (`multi_step_frame_interleaves_phases`).
2. **Headless vs winit feature split on `engine-app`** — dedicated server is a first-class path (`run_headless`, `AppExit`), not a fork.
3. **Prefab string seam + compile-once plans** — modding and net spawn messages stay data-driven without reflection.
4. **Chapter content/sim install split + `requires` graph** — exactly the right shape for authoritative server + display client + multi-zone redirects.
5. **Zone thread = App + supervisor restart budget + shared DB/world clock** — operationally thought-through MMO unit; `NetServer` Drop cleanup makes rebuild safe.
6. **`engine-net` isolated from ECS** — transport stays reusable; protocol crate owns gameplay messages.
7. **Intent/EventBus gameplay boundary in `vordar-game`** — no device/render deps in the shared sim (enforced by crate deps aside from physics internals noted in F4).
8. **Incremental spatial occupancy + broadphase/narrowphase/resolve phase split** — clear physics pipeline with game responses in `CollisionResolve`.
9. **Workspace hygiene** — shared versions, deny.toml, trimmed debug info, feature-gated tools.
10. **Explicit flush ordering for death cosmetics/net** — `DespawnFlush`/`First` consumers (corpses, death broadcast, XP carry) show systems authors understand the phase contract.

## Suggested priority order

1. **Fix or renounce per-phase `TickRate::Fixed` (F1)** — highest design-debt interest rate; blocks honest performance/combat scheduling work.
2. **Align EventBus lifetime docs + audit multi-step side effects (F2)** — cheap, prevents subtle net/game bugs under load spikes.
3. **Sandbox chapter install (F6)** — unlocks the advertised iteration loop for content.
4. **Content load report / strict server boot (F5) + ContentRoot (F9)** — reliability and ergonomics before more chapters land.
5. **Physics façade vs game internal imports (F4) + Resources bounds (F7)** — layering hardens as more authors touch collision.
6. **PreviousTransform engine system (F13) + duplicate TypeId panic (F10)** — small correctness fences.
7. **Document thread-affine App invariant (F3)** — write it down before coordinator/fleet work.
8. **Audio seam decision (F11) and render-component split deferral (F12)** — only when those features are scheduled.
9. **Keep hecs; invest profiling in net/DB (F18, F15, F17)** — do not “upgrade the ECS” as a distraction.

---

*Evidence basis: workspace `Cargo.toml` / `deny.toml` / `README.md` / `docs/architecture.mmd`; `.claude/CLAUDE.md` & `DESIGN.md`; full read of `smirk/engine-core` (components, spatial, prefab, traits), `smirk/engine-app` (app, scheduler, events, plugin, flush, prefab_plugin, tick_rate, app_loop), `smirk/engine-physics` plugin pipeline, `smirk/engine-audio` stub; client/server App bootstrap bins and libs; `vordar-game` plugin/chapter registry/setup; chapter-01/02 modules; server supervisor/zone main.*
