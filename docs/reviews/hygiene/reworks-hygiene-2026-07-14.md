# Code Hygiene Reworks — 2026-07-14

Rework-scale companion to `audit-hygiene-2026-07-14.md`: findings that need a design
pass before implementation (live-module splits and a new shared crate). Consumed by
/plan-rework, which turns one rework into a plan of fix-sized steps for
/implement-finding.

## Ideal end state

No source file hides multiple subsystems: the three 1,500+-line units (`net.rs`,
`net_plugin.rs`, the renderer's `lib.rs`/`mesh.rs`) are families of single-purpose
modules whose names predict their contents, and the headless-client test harness exists
exactly once, shared by every test binary that speaks the protocol.

## Findings (implementation order)

> **Cross-type queue** (spans this file and `audit-hygiene-2026-07-14.md`; mirrored
> from there verbatim):
> **finding 1 → rework 1 → rework 2 → rework 3 → finding 2 → finding 3 → rework 4 →
> finding 4 → finding 5 → finding 6 → finding 7 → finding 8 → finding 9 → finding 10 →
> finding 11 → finding 12 → finding 13 → finding 14 → finding 15 → finding 16.**
> Finding 1 first: every comment cleanup (4-7) applies the policy it defines, and every
> worker diff from then on writes under it. Reworks 1-3 before findings 4-6 and 10/12:
> those findings' diffs land inside the files the reworks restructure — splitting first
> means cleaning once. Finding 2 before finding 3: the split decides which binaries the
> consolidated helpers serve. Finding 3 before rework 4: consolidation gives the shared
> test crate one source of truth to lift instead of scattered copies. The rest are
> independent; ordered by impact.

### 1. Decompose client net.rs (2,474 lines) into a net module family

- **Evidence:** `client/vordar-client/src/net.rs` holds at least nine distinct
  responsibilities: reconnect/redirect lifecycle (L442-566), snapshot/AOI apply
  (L616-761), prediction/reconciliation (L763-881) plus input send (L1174-1238),
  interpolation buffer (L273-340 + L1240-1333), and four tenants that are not netcode:
  `WorldTime`/`DayNightSystem` (L883-928), telegraph visuals (L930-1021), the 150-line
  `AbilityCastSystem` (L1023-1172), `NetCameraFollowSystem` (L1335-1348). A 1,039-line
  `mod tests` (L1435-2474, 42% of the file) and an 80-line bench seam (L1354-1433) ride
  along. The full ~20-field `NetClientState` literal is written out 7 times in the tests
  (L1364, 1487, 1608, 1716, 1889, 2055, 2345).
- **Ideal:** a `net/` module family — e.g. `net/lifecycle.rs`, `net/apply.rs`,
  `net/prediction.rs`, `net/interpolate.rs`, `net/mod.rs` (plugin + state) — with the
  non-netcode tenants evicted to their own homes (`world_time.rs`, `telegraph.rs`,
  `cast.rs`) and tests split beside the module they pin. One constructor helper replaces
  the 7 state literals.
- **Gap:** the file every networking diff touches is the workspace's largest; five
  unrelated systems hide inside a file named "net".
- **Suggestion:** design pass first: module boundaries, what stays `pub(crate)` vs
  `pub`, where the bench seam and the two real-server e2e tests land, and how
  `NetClientState`'s fields partition. Structure-only — system registration order in
  `NetClientPlugin::build` must not change (behavior-neutral by definition of this
  audit).
- **Path:** /plan-rework this finding. Constraints for the plan: (1) every step leaves
  `cargo nextest run --workspace` green with unchanged test counts; (2) no scheduling
  changes — `install`/`build` wiring order is preserved verbatim; (3) the
  `bench-internals` seam keeps compiling (`benchmarks/benches/client_netcode.rs` is the
  gate); (4) note: `docs/reviews/networking/plan-networking-rework-7-2026-07-14.md`
  cites net.rs line numbers — once this rework lands, that plan's citations are stale
  and rework 7 of the networking queue needs a /plan-rework re-run before execution.

### 2. Decompose server net_plugin.rs (1,791 lines)

- **Evidence:** `server/vordar-server/src/net_plugin.rs` contains: `ReplIds` wire-id
  allocator (L183-220) and `LoginFailures` rate limiter (L338-384) — two self-contained
  types the filename doesn't predict; the 475-line `NetReceiveSystem` (L410-886) mixing
  login/takeover/rate-limit, move-intent queueing, three cast arms, DB-load completion +
  spawn + Welcome + prefab-table, respawn, and intent apply; then
  `MechanicResolveSystem` (L959-1071), `ZoneTransferSystem` (L1073-1136),
  `SnapshotBroadcastSystem` (L1164-1327), `DeathBroadcastSystem` (L1329-1368),
  `AutosaveSystem` (L1370-1407), `ShutdownSystem` (L1409-1446), bench + test modules
  (L1448-1791).
- **Ideal:** a `net/` (or flat sibling-file) family: `repl_ids.rs`, `login.rs`,
  `receive.rs` (the receive system with its helpers), `broadcast.rs`,
  `transfer.rs`, `autosave.rs`, `shutdown.rs` — with `net_plugin.rs` reduced to the
  plugin, state, and wiring.
- **Gap:** the server's whole network edge is one file; two named subsystems (ReplIds,
  LoginFailures) are findable only by reading it end to end.
- **Suggestion:** design pass first: file boundaries along the section seams above, and
  how `NetReceiveSystem`'s helper fns partition WITHOUT splitting the system itself
  (splitting it into multiple scheduled systems would change scheduling semantics —
  that is game-architecture territory, out of hygiene's behavior-neutral bound; the
  rework moves code between files only).
- **Path:** /plan-rework this finding. Constraints: (1) every step green
  (`cargo nextest run -p vordar-server` + full workspace gate at the end); (2)
  `install()`'s system order preserved verbatim; (3) the bench seam
  (`bench-internals`, consumed by `benchmarks/benches/snapshot.rs` et al.) keeps
  compiling; (4) `tests/` in-file unit tests move beside their subjects.

### 3. Decompose engine-renderer lib.rs (1,526) and mesh.rs (1,396)

- **Evidence:** `smirk/engine-renderer/src/lib.rs` is simultaneously crate root, public
  facade (16 free functions, L396-663), home of `RendererState` (L67-129 + init/resize
  L131-375), five systems including the ~470-line `RenderSystem::run` frame graph
  (L746-1255), the menu-action applier (L1272-1369), and stray type declarations
  (`TextureHandle` L52, `CameraConfig` L56, `ParticleDrawList` L653-657 — mid-file,
  between systems). `mesh.rs` stacks CPU glTF parsing (L107-470), GPU upload/store
  (L472-676), pose/skinning (L678-728), the per-frame `MeshRenderSyncSystem`
  (L769-944), and a 450-line test module with two hand-rolled GLB writers (L946-1396).
- **Ideal:** `lib.rs` = module decls + re-exports + `RenderPlugin`; a `facade.rs` (the
  public API), `state.rs` (RendererState + init/resize), `frame.rs` (RenderSystem),
  `menu_actions.rs`; mesh split into `gltf_import.rs` (CPU), `mesh_store.rs` (GPU +
  store), `mesh_sync.rs` (per-frame system), with tests beside their subjects and the
  GLB writers as a test-support helper.
- **Gap:** the two files a rendering contributor must read first are the crate's two
  biggest, each hiding 3-5 subsystems; types live where they were convenient, not where
  their names point.
- **Suggestion:** design pass first: the facade's surface (what is actually `pub` for
  game/client vs internal), where `ParticleDrawList`/`TextureHandle`/`CameraConfig`
  belong, and the test relocation. Fold the L747-752 TODO's intent (already deleted by
  fixes finding 7) into this design.
- **Path:** /plan-rework this finding. Constraints: (1) every step green including
  `smirk/engine-renderer/tests/offscreen.rs` (the behavioral gate for the frame graph);
  (2) no pass-order changes inside `RenderSystem::run`; (3) public API surface
  unchanged (callers in `client/` and `game/` compile untouched, or via re-exports).

### 4. Shared headless-client test-support crate

- **Evidence:** the protocol-speaking test harness exists twice and drifts: server
  `tests/common/mod.rs` `Bot` (L94-100 `name_token`, L255-304 impaired constructors,
  L319-321 login-on-Connected, L408-416 `wait_for`, L437-449 `send_move` ring) vs
  client `net.rs` tests re-implementing each piece (L1451-1457 `name_token` — its doc
  says "mirrors tests/common/mod.rs"; L2191-2193 `pct` — "mirrors …loss.rs's pct";
  L2347-2352 inline impaired construction; L1926-1955 and L2296-2321 raw pump loops;
  L2229-2249 `mover_tick` duplicating the last-3 ring; server-spawn boilerplate
  copy-pasted at L1879-1882, 2014-2017, 2279-2282). Benchmarks' `src/lib.rs` documents
  the same coupling by comment ("same constants as tests/soak.rs's Wander",
  "Matches net_plugin's AOI_RADIUS").
- **Ideal:** one `test-support` crate (dev-dependency of vordar-server, vordar-client,
  and vordar-benches) owning the Bot/headless-client harness, impairment presets,
  percentile helpers, and server-spawn helper — every test binary constructs the same
  client the same way.
- **Gap:** protocol changes (v9-v15 this month alone) must be hand-mirrored into two
  harnesses; the copies already diverge in capability (Bot has takeover/cast; the
  client side has render-pump).
- **Suggestion:** design pass first: crate placement (workspace member under `game/` or
  a `testing/` dir), what moves from `tests/common` vs what stays server-specific
  (`PopulateSystem`, `walk_into_portal`), how the client's render-world pump
  requirements differ from Bot's headless world, and feature/visibility so the crate
  never ships in release builds.
- **Path:** /plan-rework this finding, after fixes finding 3 has consolidated the
  server-side copies (one source of truth to lift). Constraints: (1) every step green;
  (2) `cargo build --release -p vordar-server -p vordar-client` proves the crate stays
  out of shipping artifacts; (3) end state: `name_token`/percentile/impairment presets
  exist exactly once workspace-wide (grep is the check).

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run of this audit.
