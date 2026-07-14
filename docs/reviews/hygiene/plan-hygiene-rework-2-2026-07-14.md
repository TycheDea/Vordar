# Plan: Decompose server net_plugin.rs (1,791 lines) — 2026-07-14

Source: `docs/reviews/hygiene/reworks-hygiene-2026-07-14.md` finding 2.

## Ideal end state

`server/vordar-server/src/net_plugin.rs` (1,791 lines, one file holding the server's
entire network edge) becomes a `net/` module family whose names predict their
contents: `net/mod.rs` (plugin + `install` wiring + `NetServerState`/`PlayerConn` +
shared constants/helpers), `net/repl_ids.rs`, `net/login.rs`, `net/receive.rs`,
`net/mechanics.rs`, `net/broadcast.rs`, `net/transfer.rs`, `net/autosave.rs`,
`net/shutdown.rs`, `net/bench.rs` (feature-gated). The 12 in-file unit tests sit
beside the module they pin. Behavior is bit-identical: `install()`'s
`insert_resource`/`set_phase_rate`/`add_system` sequence is preserved verbatim
(Phase, `SystemOrder`, and registration order unchanged), `NetReceiveSystem` stays
ONE scheduled system, the external surface (`vordar_server::net::NetServerState`,
`ShutdownFlag`, `MechanicResolveSystem`, `SnapshotBroadcastSystem`, `bench`,
`install`, `NetServerPlugin`) serves every current consumer, and every step leaves
`cargo nextest run -p vordar-server` green at unchanged test counts with the full
workspace gate green at the end.

## Design decisions

- **Module renamed `net_plugin` → `net`, as a `net/mod.rs` dir family.** The finding's
  Ideal offers "a `net/` … family" or flat siblings; flat sibling files
  (`net_receive.rs`, …) are rejected for the same reason hygiene rework 1 rejected
  them on the client: they would force `pub(crate)` on every internal item
  (`PlayerConn`, `validate_intent`, `select_states`, …) and leak the network edge's
  guts to the whole crate, where a dir family keeps them `pub(super)`/private and
  invisible outside the `net` tree. Keeping the old name as `net_plugin/mod.rs` was
  considered (zero import churn) and rejected: the directory holds nine modules, only
  one of which is the plugin — "net_plugin/receive.rs" mispredicts, "net/receive.rs"
  predicts, and it mirrors the client's `net/` family from rework 1. The rename costs
  exactly 7 one-line import edits, all in-workspace (`src/lib.rs` ×3 sites,
  `src/main.rs`, `tests/watchdog.rs`, `tests/shutdown.rs`, `tests/soak.rs`,
  `tests/e2e.rs`, `benchmarks/benches/snapshot.rs`) plus ~9 comment references — no
  public crates.io surface exists.
- **`net/mod.rs` keeps plugin, state, wiring, and the genuinely shared pieces:**
  `NetServerPlugin`, `install()`, `PlayerConn`, `NetServerState` + impl, and the
  items used by 2+ submodules — constants `MAX_REWIND_MICROS` (receive's
  `validate_intent` + mechanics' rewind cap), `AOI_RADIUS` (mod's `aoi_conns` +
  broadcast + bench), `HISTORY_CAP` (receive + bench), `POST_HZ`/`STAGGER`
  (install + mechanics + transfer + broadcast + bench), and helpers
  `cooldown_remainders` (receive/transfer/autosave/shutdown) and `aoi_conns`
  (receive's cast arms + mechanics' HitResult fan-out). Single-consumer constants
  move with their consumer (`PLAYER_PREFAB`, `ARRIVAL_MARGIN_MICROS`,
  `FUTURE_SLACK_MICROS`, `INTENT_QUEUE_CAP`, `spawn_position` → receive;
  `TICK_DT` → mechanics; `MAX_SNAPSHOT_STATES`, `NEAREST_GUARANTEED` → broadcast;
  `AUTOSAVE_TICKS` → autosave; `LOGIN_FAIL_WINDOW_MICROS`, `MAX_LOGIN_FAILURES`
  → login).
- **Privacy via descendants, not getters.** `PlayerConn` and `NetServerState`'s
  fields stay private in `mod.rs`; Rust privacy makes a parent module's private
  items visible to all descendants, so `receive.rs`, `broadcast.rs`, `bench.rs`,
  and every `#[cfg(test)] mod tests` inside the family read/write
  `state.conns`, `pc.queue`, `state.repl_ids` etc. directly — zero accessors added,
  zero field visibility widened. Child-module items consumed by `mod.rs` or a
  sibling get `pub(super)`; items consumed outside the crate stay `pub` in their
  file and are re-exported from `mod.rs` so every existing external path keeps one
  flat segment: `pub use broadcast::SnapshotBroadcastSystem;`,
  `pub use mechanics::MechanicResolveSystem;`, `pub use shutdown::ShutdownFlag;`.
- **The `pub` surface shrinks to actual external users** (same philosophy as rework
  1): `NetServerState` (tests/soak.rs, tests/e2e.rs), `ShutdownFlag` (src/main.rs,
  tests/watchdog.rs, tests/shutdown.rs), `MechanicResolveSystem` +
  `SnapshotBroadcastSystem` + their `new()` (benchmarks/benches/snapshot.rs),
  `bench` (feature-gated, same bench), `NetServerPlugin` + `install` (src/lib.rs)
  stay `pub`. `NetReceiveSystem`, `ZoneTransferSystem`, `DeathBroadcastSystem`,
  `AutosaveSystem`, `ShutdownSystem` are `pub` today with zero users outside
  `install()` (verified by grep across the workspace) and become `pub(super)`.
  `AutosaveSystem`'s private `ticks` field becomes `pub(super)` because `install()`
  (now the parent module) constructs the literal `AutosaveSystem { ticks: 0 }` —
  parent modules do NOT see a child's private fields.
- **`NetReceiveSystem` moves whole, never splits.** The finding's bound is explicit:
  splitting the 475-line system into multiple scheduled systems would change
  scheduling semantics (game-architecture territory). Its private helper fns
  (`validate_intent`, `queue_move_intents`, `spawn_position`) travel with it into
  `receive.rs`; no helper is extracted from the `run` body itself.
- **Two homes the finding's list omits, decided here:** `MechanicResolveSystem` +
  `rewound_position` + `TICK_DT` get `net/mechanics.rs` (the scheduled-mechanic
  resolve pipeline — "resolve.rs" alone was rejected as too vague, and it is not a
  broadcast: its output is damage, the HitResult send is incidental).
  `DeathBroadcastSystem` joins `SnapshotBroadcastSystem` and `select_states` in
  `net/broadcast.rs` — both are pure fan-out systems keyed on the `known` sets and
  `repl_ids`, and the file name predicts both.
- **Moved code keeps its comments verbatim** (the queue orders this rework before
  the comment-cleanup findings 4-7 precisely so cleaning happens once, after
  splitting). Workers fix only references the move itself breaks: comments in other
  files citing "net_plugin.rs" or "net_plugin's X" switch to citing the item by
  name (no file path, no line numbers) so they survive all later steps. New module
  headers state intent + scheduling constraints only, no finding/rework citations.
- **Extraction order: leaves first, receive last.** Step 1 is the pure rename
  (`git mv` + import edits, no code motion) so every later diff is a clean
  within-crate move. Then self-contained types (repl_ids, login), then the three
  small independent systems (shutdown, autosave, transfer), then broadcast, then
  mechanics, then the big receive move, then the bench seam + final mod.rs shape.
  Known single double-touch, accepted: the bench module's wrappers are edited in
  step 4 (when `select_states` moves under `broadcast::`) and moved in step 7 —
  same pattern rework 1 accepted for its apply/bench interaction.
- **Verification per step** (the finding's constraints): (1)
  `cargo nextest run -p vordar-server` green with pass/skip counts identical to the
  baseline captured before the step (12 unit tests move homes — names gain module
  prefixes, the COUNT must not change); (2) `install()`'s body diffs against git
  HEAD only in `use`-path spelling — the `insert_resource`/`set_phase_rate`/
  `add_system` sequence, phases, and `SystemOrder` arguments byte-identical;
  (3) `cargo check -p vordar-benches --benches` compiles (the `bench-internals`
  gate — benchmarks/Cargo.toml enables the feature; `snapshot.rs` is edited ONCE,
  in step 1, then never again); (4) `cargo check -p vordar-server --all-targets`
  with zero new warnings (catches orphaned imports and the bin target, which plain
  nextest may not rebuild); (5) full `cargo nextest run --workspace` at the final
  code step (step 7).
- **No follow-on plan invalidation beyond what rework 1 already handles.**
  `docs/reviews/networking/plan-networking-rework-7-2026-07-14.md` (the only
  pending plan citing `net_plugin.rs` lines: L62, L236-240) already receives a
  "stale — re-run /plan-rework before executing" banner from hygiene rework 1's
  step 10; that re-run happens after this rework lands too, so no additional banner
  is needed. Historical documents (audit reports, completed plans,
  `WEAKPOINTS.md`'s past-tense citation, archive files) keep their `net_plugin.rs`
  citations — they describe the past. Step 8 (docs-only) strikes rework 2 in the
  cross-type queue notes.

## Findings (execution order)

### 1. Rename the module: `net_plugin.rs` → `net/mod.rs`

- **Evidence:** `server/vordar-server/src/net_plugin.rs` is declared
  `pub mod net_plugin;` at `server/vordar-server/src/lib.rs:16` and imported as
  `vordar_server::net_plugin::…` at exactly 7 sites: `src/lib.rs:23`
  (`use net_plugin::NetServerPlugin;`) and `src/lib.rs:76`
  (`net_plugin::install(...)`), `src/main.rs:23`
  (`use vordar_server::net_plugin::ShutdownFlag;`), `tests/watchdog.rs:22` and
  `tests/shutdown.rs:19` (`ShutdownFlag`), `tests/soak.rs:20` and `tests/e2e.rs:26`
  (`NetServerState`), and `benchmarks/benches/snapshot.rs:29`
  (`use vordar_server::net_plugin::{bench as seam, MechanicResolveSystem, SnapshotBroadcastSystem};`).
  Comments in other files cite the old file name: `tests/e2e.rs:1068, 1104, 1202,
  1214, 1233`; `benchmarks/src/lib.rs:5` ("AOI radius 40.0 (net_plugin)") and `:22`
  ("Matches net_plugin's AOI_RADIUS."); `game/vordar-game/src/combat/leap.rs:3`
  ("net_plugin's Leap cast arm"); `server/vordar-server/Cargo.toml:10` ("Exposes
  net_plugin internals to vordar-benches (net_plugin::bench module).").
- **Ideal:** the file lives at `server/vordar-server/src/net/mod.rs`, declared
  `pub mod net;` in lib.rs, contents byte-identical apart from nothing (no code
  moves in this step); every import site says `net` instead of `net_plugin`; the
  comment citations name the module or item instead of the dead file name.
- **Gap:** the module is named for one of its ten tenants ("plugin") and is a
  single flat file; the dir-family conversion and all import churn must land as one
  atomic, trivially-reviewable step before any code moves.
- **Suggestion:** `git mv server/vordar-server/src/net_plugin.rs
  server/vordar-server/src/net/mod.rs`. In `src/lib.rs`: `pub mod net;`,
  `use net::NetServerPlugin;`, `net::install(...)`. Change the 5 test/bin/bench
  import lines to `vordar_server::net::…` (contents of the braces unchanged).
  Comment fixes (cite items, not files, so later steps don't re-break them):
  e2e.rs:1068 "net_plugin.rs's Welcome/HitResult/EntityDied/snapshot-gather sites"
  → "the server net module's Welcome/HitResult/EntityDied/snapshot-gather sites";
  e2e.rs:1104 "net_plugin.rs's old" → "the server's old"; e2e.rs:1202/1233
  "(MAX_SNAPSHOT_STATES, net_plugin.rs)" → "(the server's MAX_SNAPSHOT_STATES)";
  e2e.rs:1214 "(40, net_plugin.rs)" → "(40, the server's AOI_RADIUS)";
  benchmarks/src/lib.rs:5 "(net_plugin)" → "(server net module)" and :22
  "net_plugin's AOI_RADIUS" → "the server net module's AOI_RADIUS"; leap.rs:3
  "net_plugin's Leap cast arm" → "the server's Leap cast arm";
  server/vordar-server/Cargo.toml:10 → "Exposes server net-module internals to
  vordar-benches (net::bench module).". Touch nothing else — no visibility, no
  code order, no other comments.
- **Path:** (1) Baseline: run `cargo nextest run -p vordar-server` and record the
  final "N tests run" counts line; also run `cargo nextest run --workspace` once
  and record its counts (used again at step 7). (2) Make the edits above.
  (3) Verify: `cargo nextest run -p vordar-server` green at identical counts;
  `cargo check -p vordar-benches --benches` compiles;
  `cargo check -p vordar-server --all-targets` zero warnings;
  `cargo nextest run --workspace` green at the recorded workspace counts (this
  step touched files in 3 crates — prove the whole workspace once here);
  `grep -rn "net_plugin" --include="*.rs" .` over the workspace returns zero hits.
  The behavioral gate is the entire vordar-server test suite (e2e/soak/loss/zones/
  shutdown/watchdog drive real QUIC servers through the renamed module).

### 2. Extract `net/repl_ids.rs` and `net/login.rs`

- **Evidence:** in `server/vordar-server/src/net/mod.rs` (all line numbers from the
  pre-step-1 file, unchanged by the rename): `ReplIds` — doc comment + struct
  L183-196, impl (`new`, `id_for`, `sweep`) L198-220 — and its 2 tests
  `repl_ids_assign_stable_monotonic_ids` L1756-1769 and
  `repl_ids_sweep_drops_despawned_and_never_reuses_ids` L1777-1790.
  `LoginFailures` — consts `LOGIN_FAIL_WINDOW_MICROS` L338-341 and
  `MAX_LOGIN_FAILURES` L342-344, struct + doc L346-354, impl (`new`, `record`,
  `is_limited`) L356-384 — and its test
  `login_failures_deny_at_five_and_forget_after_the_window_drains` L1723-1749.
  Users: `NetServerState` holds `repl_ids: ReplIds` (L255) and
  `login_failures: LoginFailures` (L250), both constructed in
  `NetServerState::new` (L285, L288); `ReplIds::id_for` is called from
  `NetReceiveSystem` (L818, L855), `MechanicResolveSystem` (L1048),
  `SnapshotBroadcastSystem` (L1261), `DeathBroadcastSystem` (L1355); `sweep` from
  `SnapshotBroadcastSystem` (L1187); `LoginFailures` only from the Login arm
  (L472, L479, L505, L534) and the BadToken arm (L735) of `NetReceiveSystem`.
  The tests read private fields `ids.by_entity` and `failures.by_ip`.
- **Ideal:** `server/vordar-server/src/net/repl_ids.rs` (declared `mod repl_ids;`
  in mod.rs): the `ReplIds` doc comment, struct, and impl moved verbatim, struct
  and all three fns `pub(super)`, fields private, plus a
  `#[cfg(test)] mod tests` holding the 2 moved tests (`use super::*;` — private
  field access still compiles: the test module is a descendant).
  `server/vordar-server/src/net/login.rs` (declared `mod login;`): the two consts
  (private), the `LoginFailures` doc/struct/impl verbatim, struct + `new` +
  `record` + `is_limited` `pub(super)`, fields private, plus its 1 moved test.
  mod.rs adds `use login::LoginFailures;` and `use repl_ids::ReplIds;` (or spells
  the paths at the two field declarations) — no other mod.rs code changes.
- **Gap:** two self-contained named subsystems the audit calls out explicitly —
  "findable only by reading net_plugin.rs end to end" — still live inline.
- **Suggestion:** pure moves. Imports each new file needs: repl_ids.rs —
  `use engine_core::World; use hecs::Entity; use std::collections::HashMap;`
  (tests add nothing). login.rs —
  `use std::collections::{HashMap, VecDeque}; use std::net::IpAddr;`.
  Remove nothing else from mod.rs's `use` block unless
  `cargo check -p vordar-server --all-targets` flags it unused (IpAddr becomes
  unused in mod.rs only after the receive move in step 6 — leave the compiler to
  arbitrate per step; zero-warning gate decides).
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Create the two files, move the code + 3 tests, wire the two `use` lines.
  (3) Verify: `cargo nextest run -p vordar-server` green at identical counts —
  the 3 moved unit tests (now `net::repl_ids::tests::…`, `net::login::tests::…`)
  drive the real allocator and rate limiter; the e2e suite's login/takeover tests
  and every snapshot-driven test exercise `id_for`/`sweep`/`is_limited` through
  live servers. `cargo check -p vordar-benches --benches` compiles;
  `cargo check -p vordar-server --all-targets` zero warnings.

### 3. Extract `net/shutdown.rs`, `net/autosave.rs`, `net/transfer.rs`

- **Evidence:** in `server/vordar-server/src/net/mod.rs` (pre-split line numbers):
  `ShutdownFlag` doc + struct L1409-1414 and `ShutdownSystem` doc + struct + impl
  L1416-1446; `autosave_due` doc + fn L1370-1378, `AutosaveSystem` doc + struct +
  impl L1380-1407, const `AUTOSAVE_TICKS` L80-81, and the test
  `autosave_spreads_a_crowd_across_the_window_instead_of_bursting` L1672-1700;
  `ZoneTransferSystem` doc + struct + `new` + impl L1073-1136. Users: `install()`
  registers all three (L131, L141, L143 — order: `ShutdownSystem` in
  `Phase::Input`, `ZoneTransferSystem` `before::<SnapshotBroadcastSystem>()` in
  PostUpdate, `AutosaveSystem { ticks: 0 }` PostUpdate Default); external users of
  `ShutdownFlag`: `src/main.rs:23`, `tests/watchdog.rs:22+73`,
  `tests/shutdown.rs:19+32+119` — all via `vordar_server::net::ShutdownFlag` since
  step 1. All three systems read `state.conns`/`state.zone`/`state.db` and call
  `cooldown_remainders` (mod.rs L328); transfer additionally uses
  `portal_hit`/`STAGGER`/`state.directory`; nothing else references them.
- **Ideal:** three sibling files, each with a 1-3-line module header stating
  intent + scheduling constraint, contents moved verbatim:
  - `net/shutdown.rs` (`mod shutdown;` + `pub use shutdown::ShutdownFlag;` in
    mod.rs): `ShutdownFlag` stays `pub` (tuple field stays `pub`);
    `ShutdownSystem` becomes `pub(super)` (only `install()` uses it).
  - `net/autosave.rs` (`mod autosave;`): `AUTOSAVE_TICKS` and `autosave_due`
    private; `AutosaveSystem` `pub(super)` with its `ticks` field `pub(super)`
    (install constructs the literal from the parent module, which cannot see a
    child's private fields); the moved test in `#[cfg(test)] mod tests`.
  - `net/transfer.rs` (`mod transfer;`): `ZoneTransferSystem` + `new` both
    `pub(super)`.
  mod.rs adds `use autosave::AutosaveSystem; use shutdown::ShutdownSystem;
  use transfer::ZoneTransferSystem;` — `install()`'s body otherwise byte-identical.
- **Gap:** three single-purpose systems (process shutdown, periodic persistence,
  portal handoff) are interleaved with broadcast and resolve code in one file.
- **Suggestion:** imports per file: shutdown.rs —
  `engine_app::app::AppExit`, `engine_app::scheduler::System`,
  `engine_core::components::{Health, Transform}`, `engine_core::traits::Resources`,
  `engine_core::World`, `std::sync::atomic::{AtomicBool, Ordering}`,
  `std::sync::Arc`, `crate::db::CharacterRecord`,
  `use super::{cooldown_remainders, NetServerState};`. autosave.rs — `System`,
  `Health`/`Transform`, `Resources`, `World`, `engine_net::ConnId`,
  `CharacterRecord`, `use super::{cooldown_remainders, NetServerState};`.
  transfer.rs — `System`, `Health`/`Transform`, `Resources`, `DespawnQueue`,
  `World`, `engine_net::ConnId`, `vordar_game::zones::portal_hit`,
  `vordar_protocol::{encode, ServerMsg}`, `CharacterRecord`,
  `use super::{cooldown_remainders, NetServerState, STAGGER};`. Do not touch the
  systems' bodies or comments; the compiler's unused-import warnings arbitrate
  what leaves mod.rs's `use` block.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Create the three files, move the code + 1 test, add the re-export and `use`
  lines, adjust the three visibilities named above. (3) Verify:
  `cargo nextest run -p vordar-server` green at identical counts — behavioral
  gates: `tests/shutdown.rs` and `tests/watchdog.rs` construct the re-exported
  `ShutdownFlag` and drive the moved `ShutdownSystem` through a real App (save +
  AppExit path); `tests/zones.rs::phase7_portal_round_trip` walks a player through
  a real portal, exercising the moved `ZoneTransferSystem` end-to-end
  (save-first, Redirect, despawn); the moved autosave test pins `autosave_due`'s
  spread. Diff `install()` against HEAD — identical except `use` paths.
  `cargo check -p vordar-benches --benches`;
  `cargo check -p vordar-server --all-targets` zero warnings.

### 4. Extract `net/broadcast.rs` (select_states + Snapshot + Death fan-out)

- **Evidence:** in `server/vordar-server/src/net/mod.rs` (pre-split line numbers):
  `select_states` doc + fn L1138-1162; consts `MAX_SNAPSHOT_STATES` L60-64 and
  `NEAREST_GUARANTEED` L65-68; `SnapshotBroadcastSystem` struct + `new` + `run`
  L1164-1327 (AOI gather, known-set diff, `AoiDelta` on the stream, `Snapshot` on
  a datagram, the 600-tick WorldClock re-sync + `repl_ids.sweep` + metrics dump);
  `DeathBroadcastSystem` doc + struct + impl L1329-1368; the 4 tests
  `small_crowds_pass_through_untouched` L1520, `nearest_always_included_over_budget`
  L1528, `rotation_refreshes_every_entity` L1540,
  `no_duplicate_indices_in_selection` L1557, and their helper `entries(n)`
  L1515-1518. Users: `install()` registers both systems and two `SystemOrder`
  bounds name `SnapshotBroadcastSystem` (L133-143);
  `benchmarks/benches/snapshot.rs:29` imports `SnapshotBroadcastSystem` and calls
  `SnapshotBroadcastSystem::new()`; the in-mod.rs `bench` module (L1448-1509)
  wraps `select_states` and re-exports `MAX_SNAPSHOT_STATES`/`NEAREST_GUARANTEED`
  as `MAX_STATES`/`NEAREST`.
- **Ideal:** `server/vordar-server/src/net/broadcast.rs` (`mod broadcast;` +
  `pub use broadcast::SnapshotBroadcastSystem;` in mod.rs): module header stating
  the fan-out model (per-conn AOI diff at SNAPSHOT_HZ via STAGGER slices; identity
  on the reliable stream, positions on datagrams; states budget-throttled).
  Contents moved verbatim: `MAX_SNAPSHOT_STATES` + `NEAREST_GUARANTEED`
  `pub(super)` (the bench module — still in mod.rs until step 7, then a sibling —
  needs them), `select_states` `pub(super)` (same reason),
  `SnapshotBroadcastSystem` stays `pub` with `pub fn new` (external bench user,
  reachable via the re-export), `DeathBroadcastSystem` becomes `pub(super)` (only
  `install()` uses it). The 4 tests + `entries` helper move into
  `#[cfg(test)] mod tests` in broadcast.rs. The in-mod.rs bench module's wrapper
  bodies change to `broadcast::select_states(...)` and its consts to
  `pub const MAX_STATES: usize = broadcast::MAX_SNAPSHOT_STATES;` etc. (public
  signatures untouched — `benchmarks/benches/snapshot.rs` is not edited).
- **Gap:** the snapshot pipeline — the server's hottest per-tick fan-out and the
  subject of its own bench file — is buried mid-file between the transfer and
  death systems.
- **Suggestion:** broadcast.rs imports: `engine_app::events::{EventBus,
  HealthDepleted}`, `engine_app::scheduler::System`,
  `engine_core::components::{Health, Transform}`, `engine_core::prefab::PrefabId`,
  `engine_core::spatial::SpatialGrid`, `engine_core::traits::Resources`,
  `engine_core::World`, `engine_net::ConnId`, `glam::Vec3`, `hecs::Entity`,
  `std::collections::HashSet`, `std::sync::atomic::Ordering`,
  `vordar_protocol::{encode, EntityPos, EntityState, ServerMsg, WirePos}`,
  `use super::{NetServerState, AOI_RADIUS, STAGGER};`. Note `state.world_at`,
  `state.tick`, `state.repl_ids`, `state.prefab_table`, `pc.known`, `pc.rr_cursor`,
  `pc.applied_seq` are private mod.rs items — accessible from this descendant, no
  visibility changes.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Create broadcast.rs, move the six items + 4 tests + helper, add the
  re-export, retarget the in-mod.rs bench wrappers, set the visibilities above.
  (3) Verify: `cargo nextest run -p vordar-server` green at identical counts —
  behavioral gates: the 4 moved select_states tests; `tests/e2e.rs`'s crowd test
  (101 entities in AOI proving the MAX_SNAPSHOT_STATES budget + rotation through
  the moved system against a real server) and its EntityDied test (drives the
  moved `DeathBroadcastSystem` known-set filter); `tests/loss.rs` (datagram-lane
  snapshots under impairment). `cargo check -p vordar-benches --benches` compiles
  — proves both the re-exported `SnapshotBroadcastSystem` path and the bench
  wrapper retarget. `cargo check -p vordar-server --all-targets` zero warnings;
  `install()` diff = `use` paths only.

### 5. Extract `net/mechanics.rs` (scheduled-mechanic resolve)

- **Evidence:** in `server/vordar-server/src/net/mod.rs` (pre-split line numbers):
  `MechanicResolveSystem` doc + struct + `new` + `run` L959-1056 (10 Hz self-gate
  on STAGGER, due-mechanic scan, stamp-based rewind through `pc.history`,
  `compute_damage`/`ravager_mods` application, `Provoked` insert, HitResult
  fan-out via `aoi_conns` + `repl_ids.id_for`, mechanic despawn);
  `rewound_position` doc + fn L1058-1071; const `TICK_DT` L69-70 (used only by
  `rewound_position`). Users: `install()` registers it
  `before::<SnapshotBroadcastSystem>()` and `RavagerRageSystem` is ordered
  `after::<MechanicResolveSystem>()` (L135-138);
  `benchmarks/benches/snapshot.rs:29` imports it and calls
  `MechanicResolveSystem::new()`; `bench::fill_histories` (mod.rs) fills the
  histories it rewinds but doesn't reference the system.
- **Ideal:** `server/vordar-server/src/net/mechanics.rs` (`mod mechanics;` +
  `pub use mechanics::MechanicResolveSystem;` in mod.rs): module header stating
  the resolve contract (at the first resolve tick past T, membership is decided AT
  T — players via stamp-based intent rewind, favor-the-defender; rewind capped by
  MAX_REWIND_MICROS). `MechanicResolveSystem` + `new` stay `pub` (external bench
  user via the re-export); `rewound_position` and `TICK_DT` private to the file.
- **Gap:** after step 4 the resolve pipeline is the largest non-receive tenant
  left in mod.rs, and the finding's file list gave it no home — this is it.
- **Suggestion:** mechanics.rs imports: `engine_app::events::EventBus`,
  `engine_app::scheduler::System`, `engine_core::components::{Health, Transform}`,
  `engine_core::traits::Resources`, `engine_core::World`, `glam::{Vec2, Vec3}`,
  `hecs::Entity`, `std::collections::VecDeque`,
  `vordar_game::combat::buff::ravager_mods`,
  `vordar_game::combat::stats::compute_damage`,
  `vordar_game::events::DamageDealt`, `vordar_game::player::movement_velocity`,
  `vordar_game::{CombatStats, Enemy, Mechanic, Player, Provoked}`,
  `vordar_protocol::{encode, ServerMsg}`,
  `use super::{aoi_conns, NetServerState, MAX_REWIND_MICROS, STAGGER};`.
  `pc.history` access is descendant-privileged, unchanged. Move code + comments
  verbatim.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Create mechanics.rs, move the three items, add the re-export.
  (3) Verify: `cargo nextest run -p vordar-server` green at identical counts —
  behavioral gate: `tests/e2e.rs`'s mechanic/HitResult/favor-the-defender tests
  drive the moved system through a real server (cast → telegraph → resolve →
  damage → HitResult), and its rage/leap tests pin the
  `RavagerRageSystem after::<MechanicResolveSystem>()` ordering still resolves.
  `cargo check -p vordar-benches --benches` compiles (bench constructs
  `MechanicResolveSystem::new()` through the re-export);
  `cargo check -p vordar-server --all-targets` zero warnings; `install()` diff =
  `use` paths only.

### 6. Extract `net/receive.rs` (the Input-phase edge, moved whole)

- **Evidence:** in `server/vordar-server/src/net/mod.rs` (pre-split line numbers):
  `NetReceiveSystem` struct + `run` L410-886 — event drain; Disconnected save +
  despawn; the Login arm (rate limit, name validation, token-gated takeover of
  live and in-flight sessions, DB login kickoff); MoveIntents/CastIntent dispatch
  (three cast arms); pending-bolt spawn; DbLoaded completion (deny/redirect/spawn
  + prefab-table build + Welcome/PrefabTable/WorldClock sends); respawn +
  re-Welcome; one-intent-per-tick apply emitting `MoveIntent` events. Its private
  helpers: `spawn_position` L315-319 (login defaults L551, respawn L848),
  `validate_intent` L888-916, `queue_move_intents` L918-957. Its single-consumer
  consts: `PLAYER_PREFAB` L50-53, `ARRIVAL_MARGIN_MICROS` L41-42,
  `FUTURE_SLACK_MICROS` L43-44, `INTENT_QUEUE_CAP` L45-48. Its 3 tests + helper:
  `zero_seq_is_always_rejected` L1565-1594, `fresh_pc` L1596-1610,
  `move_intents_dedupe_silently_without_rejecting` L1612-1647,
  `move_intents_still_rejects_a_genuinely_invalid_entry` L1649-1670. `install()`
  registers the system at L127 (`Phase::Input, SystemOrder::Default`, first
  registration). Nothing else in the crate references any of these items
  (`NetReceiveSystem` is `pub` today with zero external users — verified).
- **Ideal:** `server/vordar-server/src/net/receive.rs` (`mod receive;` in mod.rs):
  module header stating the edge contract (drains ServerEvents once per Input
  tick; a connection enters the game only at DbLoaded-grant; exactly one queued
  intent applies per tick). `NetReceiveSystem` becomes `pub(super)`; everything
  else in the file private; the system's 475-line `run` body moves VERBATIM — no
  helper extraction, no splitting into multiple systems (scheduling semantics are
  out of hygiene's bound). The 3 tests + `fresh_pc` move into
  `#[cfg(test)] mod tests` (they construct `PlayerConn` literals and call
  `validate_intent`/`queue_move_intents` — all reachable: descendants see the
  parent's private `PlayerConn` and this file's private fns).
- **Gap:** the receive system is the file's dominant tenant; once it leaves,
  mod.rs is within sight of the finding's "plugin, state, and wiring" shape.
- **Suggestion:** receive.rs imports (the bulk of mod.rs's current `use` block
  travels here): `crate::db::{CharacterRecord, DbLoaded, DbLoginOutcome}`,
  `engine_app::events::EventBus`, `engine_app::scheduler::System`,
  `engine_core::components::{Health, Transform}`,
  `engine_core::prefab::{spawn_prefab, PrefabId, PrefabLibrary}` — note
  `PrefabId` is NOT used by receive (only broadcast) — take exactly what the
  compiler demands, `engine_core::traits::{DespawnQueue, Resources, SpawnContext}`,
  `engine_core::World`, `engine_net::{ConnId, NetMetrics, ServerEvent}`,
  `glam::{Vec2, Vec3}`, `hecs::Entity`, `std::collections::{HashMap, HashSet,
  VecDeque}`, `std::sync::Arc`, `vordar_game::combat::leap::{leap_velocity,
  LeapImpulse}`, `vordar_game::combat::projectile::spawn_projectile`,
  `vordar_game::events::MoveIntent`, `vordar_game::player::class::{ClassId,
  ClassLibrary, DEFAULT_CLASS}`, `vordar_game::skills::AbilityEffect`,
  `vordar_game::world::WorldTimeRes`, `vordar_game::Mechanic`,
  `vordar_protocol::{decode, encode, AccountToken, ClientMsg, LoginDenyReason,
  MoveIntentEntry, ServerMsg}`, `use super::{aoi_conns, cooldown_remainders,
  NetServerState, PlayerConn, HISTORY_CAP, MAX_REWIND_MICROS};`. After the move,
  prune mod.rs's `use` block to what the zero-warning gate demands (mod.rs keeps
  roughly: db types for state/install, App/Plugin/scheduler types, TickRate,
  Transform (aoi_conns), NetServer/NetLimits/NetMetrics/ConnId, glam, hecs,
  collections, SocketAddr/IpAddr, Instant, Arc, WorldTimeRes, ZoneDef, protocol
  constants). Test-module imports: `use super::*;` plus
  `use engine_net::NetMetrics;`, `use std::sync::atomic::Ordering;`,
  `use vordar_protocol::MoveIntentEntry;` as the compiler demands.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Create receive.rs, move system + 4 consts + 3 helper fns + 3 tests +
  `fresh_pc` verbatim, add `mod receive; use receive::NetReceiveSystem;` to
  mod.rs, prune imports. (3) Verify: `cargo nextest run -p vordar-server` green at
  identical counts — behavioral gates: the 3 moved unit tests (seq-0 rejection,
  batch dedupe, in-batch reject) call the moved validators directly; the entire
  e2e suite (24 tests) logs in, moves, casts, takes over sessions, and respawns
  through the moved system against real QUIC servers — any dispatch or wiring slip
  fails there; `tests/loss.rs` proves the MoveIntents redundancy path.
  `cargo check -p vordar-benches --benches`;
  `cargo check -p vordar-server --all-targets` zero warnings (catches every
  orphaned mod.rs import); `install()` diff = `use` paths only.

### 7. Extract `net/bench.rs`; final `net/mod.rs` shape

- **Evidence:** the last rider in `server/vordar-server/src/net/mod.rs`: the
  feature-gated bench seam (pre-split L1448-1509) — consts
  `MAX_STATES`/`NEAREST`/`AOI`/`STAGGER_TICKS`, the `select_states` wrapper
  (retargeted to `broadcast::` in step 4), `state_with_fake_conns` (constructs
  `NetServerState::new` + `PlayerConn` literals), `fill_histories` (uses
  `HISTORY_CAP`, `Vec2`). Consumer: `benchmarks/benches/snapshot.rs:29`
  (`bench as seam`) uses `seam::select_states`, `seam::MAX_STATES`,
  `seam::NEAREST`, `seam::STAGGER_TICKS`, `seam::state_with_fake_conns`,
  `seam::fill_histories` — it must compile UNCHANGED. Also remaining in mod.rs: a
  `#[cfg(test)] mod tests` now holding only
  `cooldown_remainders_drops_expired_and_subtracts_correctly` (pre-split
  L1708-1721).
- **Ideal:** `server/vordar-server/src/net/bench.rs`, declared in mod.rs as
  `#[cfg(feature = "bench-internals")] #[doc(hidden)] pub mod bench;` — contents
  moved verbatim with paths adjusted for the new depth
  (`use super::*;` plus explicit `use super::broadcast;` if needed:
  `broadcast::select_states`, `broadcast::MAX_SNAPSHOT_STATES`,
  `broadcast::NEAREST_GUARANTEED` are `pub(super)` = visible throughout the `net`
  tree including this descendant; `NetServerState::new`, `PlayerConn`,
  `HISTORY_CAP`, `AOI_RADIUS`, `STAGGER` come from `super`); every `pub` item's
  signature byte-identical. After this step `net/mod.rs` (~450 lines) contains
  ONLY: the module doc header, the `use` block, module decls + the three
  re-exports (`SnapshotBroadcastSystem`, `MechanicResolveSystem`, `ShutdownFlag`)
  + the bench decl, shared consts (`MAX_REWIND_MICROS`, `PLAYER_PREFAB` is gone —
  moved in step 6 — `AOI_RADIUS`, `HISTORY_CAP`, `POST_HZ`, `STAGGER`),
  `NetServerPlugin` + `Plugin` impl, `install()`, `PlayerConn`, `NetServerState` +
  impl, `cooldown_remainders`, `aoi_conns`, and the 1-test `mod tests`.
- **Gap:** an 60-line bench seam still rides inside the plugin/state file; mod.rs
  still carries step-6 leftovers if any import pruning was deferred.
- **Suggestion:** move the bench module verbatim (docs included — its header
  comment explains the fabricated-ConnId trick and stays). Keep the
  `#[cfg(feature = "bench-internals")]` and `#[doc(hidden)]` attributes on the
  `pub mod bench;` declaration in mod.rs (attributes on the decl, not inside the
  file). Give mod.rs its final header: keep the current line-1 comment's intent
  ("NetServerPlugin — the seam between engine-net and the simulation") and extend
  it with a one-line map of the module family (receive = Input edge, broadcast /
  mechanics / transfer / autosave = PostUpdate, shutdown = Input; state lives
  here). No finding citations in the header.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Extract bench.rs; tidy mod.rs's header and `use` block. (3) Verify:
  `cargo check -p vordar-benches --benches` compiles with
  `benchmarks/benches/snapshot.rs` untouched — this IS the step's behavioral
  contract (the seam's fabricated-conn state construction feeds
  `SnapshotBroadcastSystem`/`MechanicResolveSystem` runs in the bench harness);
  `cargo nextest run -p vordar-server` green at identical counts;
  `cargo check -p vordar-server --all-targets` zero warnings; full
  `cargo nextest run --workspace` green at the counts recorded in step 1;
  `cargo build --release -p vordar-server` compiles (proves the bench seam stays
  feature-gated out of shipping builds). (4) Structure checks:
  `server/vordar-server/src/net_plugin.rs` does not exist;
  `grep -c "impl System" server/vordar-server/src/net/mod.rs` returns 0;
  `wc -l server/vordar-server/src/net/*.rs` shows no file above ~600 lines
  (receive.rs, the largest, lands near 550). If receive.rs exceeds ~600 lines,
  record the number in the final report — do NOT split the system to chase a line
  count.

### 8. Close this rework's queue entry (docs-only)

- **Evidence:** the cross-type queue note listing "finding 1 → ~~rework 1~~ →
  rework 2 → rework 3 → …" exists in two places that must stay mirrored verbatim:
  `docs/reviews/hygiene/reworks-hygiene-2026-07-14.md` (L17-28) and
  `docs/reviews/hygiene/audit-hygiene-2026-07-14.md` (L19-30). The only pending
  plan citing `net_plugin.rs` internals,
  `docs/reviews/networking/plan-networking-rework-7-2026-07-14.md` (L62,
  L236-240), already carries (or will carry, from hygiene rework 1's step 10) a
  stale banner requiring a /plan-rework re-run before execution — that re-run
  happens after this rework too, so no second banner is needed.
- **Ideal:** the hygiene queue shows rework 2 done in both mirrored notes; the
  rework-7 plan's staleness is confirmed covered.
- **Gap:** the queue notes still list rework 2 as pending.
- **Suggestion:** strike `rework 2` (`~~rework 2~~`) in the cross-type queue note
  in BOTH files. Then open
  `docs/reviews/networking/plan-networking-rework-7-2026-07-14.md` and check its
  top for the stale banner from hygiene rework 1 step 10: if present, no edit; if
  hygiene rework 1's step 10 has not landed yet, add the banner line yourself
  (italic, directly under the title): "*Stale as of 2026-07-14: hygiene reworks 1
  and 2 decomposed client net.rs and server net_plugin.rs into module families —
  the citations below predate them. Re-run /plan-rework for this finding before
  executing.*"
- **Path:** (1) Make the strike edits; (2) verification: the queue blockquotes in
  the two hygiene files remain byte-identical to each other (diff them), and the
  rework-7 plan carries exactly one stale banner. No code, no test — docs-only.
