# Plan: Shared headless-client test-support crate — 2026-07-14

Source: `docs/reviews/hygiene/reworks-hygiene-2026-07-14.md` finding 4.

## Ideal end state

One workspace crate, `testing/test-support` (package `test-support`, lib `test_support`),
owns the protocol-speaking headless-client harness (`Bot` and its impaired constructors,
`name_token`), the server-side test systems (`PopulateSystem`, `MetricMirror`), the
multi-zone and single-zone server bring-up helpers, and the shared utilities
(`percentile`, `Lcg`, `workspace_root`, `join_with_deadline`, `temp_db`, `test_zones`).
`server/vordar-server/tests/common/mod.rs` is gone; the client's `net/e2e.rs` drives its
auxiliary raw peers (kicker, mover) through `Bot` instead of hand-rolled poll loops; the
benchmarks crate's drift-by-comment couplings (`Lcg` "same constants as soak's Wander",
`AOI_RADIUS` "matches the server net module") are replaced by a single definition each.
`cargo build --release -p vordar-server -p vordar-client` never builds the crate, and
`name_token`/`percentile`/the impairment-preset constructors each exist exactly once
workspace-wide.

## Design decisions

- **The whole of `tests/common/mod.rs` moves; nothing stays behind.** The finding's
  suggestion anticipated keeping some pieces server-specific (`PopulateSystem`,
  `walk_into_portal`), but studying the deps shows the crate must depend on
  `vordar-server` anyway (the server-spawn helper calls `vordar_server::build_server_app`;
  `MetricMirror` reads `vordar_server::net::NetServerState`; `spawn_zones` uses
  `vordar_server::db::DbWorker`) — so a rump `tests/common` would buy nothing except a
  second home and per-binary recompilation. Single home wins; the file is deleted, not
  shimmed (`pub use test_support::*` re-export shims are explicitly rejected).
- **Dev-dependency cycle `vordar-server` ⇄ `test-support` is deliberate and legal.**
  Cargo permits a package's dev-dependency to depend on the package itself (the standard
  self-testing-helper pattern). All crates are `publish = false`, so the one thing the
  cycle breaks (`cargo publish`) is irrelevant.
- **Crate placement: new top-level `testing/` directory, not under `game/`.** The crate
  depends on `vordar-server`, which sits above `game/` in the layering; parking it under
  `game/` would misstate the dependency direction to readers. Workspace member
  `"testing/test-support"`.
- **Dependency kinds:** dev-dependency of `vordar-server` and `vordar-client`; **normal**
  dependency of `vordar-benches` — diverging from the finding's "dev-dependency of ...
  vordar-benches" because `benchmarks/src/lib.rs` (the scenario-builder lib that bench
  targets import) uses `Lcg`, and a lib does not see dev-dependencies. `vordar-benches`
  never ships, so the release guard (constraint 2) is untouched: it covers only
  `vordar-server`/`vordar-client`, whose link to `test-support` is dev-only.
- **`engine-app` is depended on with `default-features = false`** (matching
  `vordar-server` and `vordar-benches`) so the crate never drags windowing features into
  server test builds; the client's own full-featured dep unifies upward where needed.
- **Internal layout is three single-purpose modules** (per the hygiene audit's own
  standard): `bot.rs` (client-of-the-protocol: `name_token`, `Bot`, `settle`,
  `walk_into_portal`, `raw_login_probe`), `server.rs` (server-side bring-up:
  `PopulateSystem`, `MetricMirror`, `test_zones`, `temp_db`, `spawn_zones`, later
  `spawn_server`/`spawn_server_with`), `util.rs` (`workspace_root`, `percentile`,
  `join_with_deadline`, later `Lcg`). `lib.rs` is crate doc + `mod` decls + flat
  `pub use` re-exports, so call sites read `test_support::Bot`.
- **"Impairment presets" = `Bot`'s impaired constructors** (`connect_with_latency_as`,
  `connect_impaired_as`, `connect_upstream_impaired_as`, `connect_full_as`), which exist
  once in the crate after the move. Probe-specific `Impairment { .. }` literals (the
  smoothness probe's 100 ms/30 ms/3 % WAN point, loss.rs's swept rates) are test
  parameterization, not harness copies — they stay local.
- **`Bot` replaces only the client e2e's auxiliary raw peers** (the takeover kicker, the
  smoothness mover). The observer/predicting worlds keep driving the real client systems
  (`NetReceiveSystem` etc.) — that is the point of those tests and is out of `Bot`'s
  job. `insert_game_prefabs` stays in `net/e2e.rs`: it is client-world setup, and the
  superficially similar registration in `benchmarks/benches/client_netcode.rs` is
  intentionally minimal (core components + one bolt prefab), not a duplicate.
- **`Lcg` gains a `next_u64` so `soak.rs`'s `Wander` refactors onto it bit-identically**
  (same seed scramble, same step constants, same 31-bit angle recipe) — deterministic
  soak trajectories do not change. Unifying the move-ring depth (Bot's `3` vs the
  client's `MOVE_RING_LEN`) into a protocol constant was considered and rejected: it
  would be a protocol-crate change for a test nicety; the existing cross-reference
  comments suffice.
- **The server-spawn helper has two shapes**: `spawn_server(addr, db_path, max_ticks)`
  for the plain fire-and-forget case and `spawn_server_with(addr, db_path, max_ticks,
  configure)` where `configure: impl FnOnce(&mut engine_app::app::App) + Send + 'static`
  runs between `build_server_app` and `run_headless(60.0, Some(max_ticks))`. Both end
  with the 300 ms settle sleep every current call site performs. Sites using
  `build_server_app_with_limits` (soak) or a non-300 ms follow-up stay explicit —
  they are genuinely different bring-ups, not boilerplate.

## Findings (execution order)

### 1. Create `testing/test-support` and move the entire server test harness into it

- **Evidence:** `server/vordar-server/tests/common/mod.rs` (605 lines) holds the whole
  harness: `workspace_root` (L21-25), `temp_db` (L28-32), `test_zones` (L39-66),
  `walk_into_portal` (L70-87), `name_token` (L98-104), `Bot` + constructors + `pump` +
  `wait_for`/`walk_until`/`send_move`/`send_cast` (L106-467), `settle` (L470-476),
  `PopulateSystem` (L479-499), `MetricMirror` (L505-516), `percentile` (L519-523),
  `join_with_deadline` (L528-536), `raw_login_probe` (L544-574), `spawn_zones`
  (L585-605). Ten test binaries consume it via `mod common;`:
  `tests/{e2e,e2e_combat,e2e_persistence,e2e_security,e2e_wireformat,loss,shutdown,soak,watchdog,zones}.rs`
  (plus qualified calls: `common::workspace_root()` at soak.rs:101, loss.rs:32/141;
  `common::settle(...)` at loss.rs:55/161). Root `Cargo.toml` members list ends at
  `"benchmarks"` (L15).
- **Ideal:** the harness lives once in `testing/test-support` as three modules —
  `src/bot.rs` (`name_token`, `Bot` with all fields/constructors/methods, `settle`,
  `walk_into_portal`, `raw_login_probe`), `src/server.rs` (`PopulateSystem`,
  `MetricMirror`, `test_zones`, `temp_db`, `spawn_zones`), `src/util.rs`
  (`workspace_root`, `percentile`, `join_with_deadline`) — with `src/lib.rs` carrying
  the crate doc (adapted from common/mod.rs's header) plus `mod` decls and flat
  `pub use bot::*; pub use server::*; pub use util::*;`. `tests/common/` is deleted and
  every server test binary imports `test_support::{...}` instead.
- **Gap:** the harness is a per-binary `mod common;` inside one crate; the client cannot
  reach it, so it re-implements pieces (fixed in steps 2-3).
- **Suggestion:** pure code motion — no signature, no behavior, no comment changes
  beyond the crate header. Details that need care: (a) every moved item becomes `pub`
  (they already are) and the file-level `#![allow(dead_code)]` is dropped — unused pub
  items in a lib don't warn; (b) `workspace_root` keeps working because `env!`
  expands in the defining crate: from `testing/test-support` the workspace root is
  `join("../..")` — same expression as today, new anchor, verify the depth; (c) the
  `NEXT_BOT` static goes from per-test-binary to per-process — identical in practice
  since each test binary is its own process.
- **Path:**
  1. Create `testing/test-support/Cargo.toml`: package `test-support`, edition 2021,
     `publish = false`, dependencies `engine-app = { path = "../../smirk/engine-app", default-features = false }`,
     `engine-core`, `engine-net` (same path style), `vordar-game`, `vordar-protocol`
     (`../../game/...`), `vordar-server = { path = "../../server/vordar-server" }`,
     `glam = { workspace = true }`. If the compiler demands another workspace crate,
     add it with the same path pattern — do not add external crates.
  2. Add `"testing/test-support"` to the root `Cargo.toml` `[workspace] members`.
  3. Move `tests/common/mod.rs`'s items verbatim into `src/bot.rs` / `src/server.rs` /
     `src/util.rs` per the split above (imports partitioned to match); write `src/lib.rs`
     with the crate doc + re-exports. Delete `server/vordar-server/tests/common/`.
  4. Add `test-support = { path = "../../testing/test-support" }` to
     `server/vordar-server/Cargo.toml` `[dev-dependencies]` (the dev-cycle is
     intentional — see the plan's Design decisions).
  5. In all ten server test binaries: delete `mod common;`, rewrite
     `use common::{...}` → `use test_support::{...}`, and the qualified
     `common::workspace_root()` / `common::settle(...)` calls (soak.rs:101,
     loss.rs:32/55/141/161) → `test_support::...`.
  6. Prove: `cargo nextest run --workspace` — green, test count unchanged from a
     pre-change `cargo nextest list -p vordar-server | tail -1` baseline (record both
     counts in the commit message). This compiles every test binary including the
     ignored probes, and the passing e2e suites exercise the moved `Bot` end-to-end
     against real servers. Also confirm the release guard early:
     `cargo tree -p vordar-server -p vordar-client -e normal | grep -c test-support`
     must print 0 (dev-only linkage).

### 2. Client `net/e2e.rs` adopts the crate: `Bot` peers, `name_token`, `percentile`, `workspace_root`

- **Evidence:** `client/vordar-client/src/net/e2e.rs` re-implements the harness:
  `name_token` (L33-39, doc says "mirrors tests/common/mod.rs"); the kicker's raw
  poll/Login/Welcome loop (L114-146); the mover's raw Welcome loop (L449-476) and
  `mover_tick` with a hand-kept last-3 `MoveIntentEntry` ring (L384-404); `pct`
  (L346-348, "mirrors server/vordar-server/tests/loss.rs's pct"); three copies of the
  workspace-root cwd fix (L76-77, L200-201, L423-424). `vordar-client/Cargo.toml`
  dev-deps already include `vordar-server` (L46) but not the new crate. The e2e module
  is `#[cfg(test)] mod e2e;` in the lib (net/mod.rs L269-270), so dev-dependencies are
  visible to it.
- **Ideal:** the file imports `test_support::{name_token, percentile, workspace_root, Bot}`;
  the kicker and mover are `Bot`s; `pct`, `mover_tick`, and the local `name_token` are
  deleted; the three cwd stanzas are `workspace_root();` calls. The observer and
  predicting worlds still drive the real client systems — untouched.
- **Gap:** every protocol change must be hand-mirrored into these copies; they already
  lag `Bot` in capability.
- **Suggestion:** mechanical swap with three precise substitutions:
  - **Kicker** (in `kicked_connection_reconnects_and_relogs_in`): replace L114-146 with
    `let mut kicker = Bot::connect_as(addr, "reconnect-victim");` +
    `kicker.wait_for("kicker Welcome", Duration::from_secs(5), |b| b.player_id.is_some());`
    then the existing `drop(kicker);`. `Bot` derives the same `name_token` internally,
    so the token-match comment (L121-125) stays true — condense it onto the new lines.
  - **Mover** (in `remote_render_smoothness_under_loss_probe`): replace the raw
    `NetClient` + Welcome loop (L442-476) with `let mut mover = Bot::connect_as(addr, "smoothness-mover");`
    + `mover.wait_for("mover Welcome", Duration::from_secs(5), |b| b.player_id.is_some());`
    + `let mover_id = mover.player_id.unwrap();`. Replace `mover_tick` with a local
    `fn drive_mover(mover: &mut Bot, dir: &mut Vec2, last_reverse: &mut Instant)` that
    keeps the reversal logic verbatim (`REVERSE_INTERVAL` 2170 ms, flip `dir.x`) and
    then calls `mover.send_move(*dir); mover.pump();` — `Bot::send_move` is the same
    clock-gated last-3 ring `mover_tick` hand-rolls. Keep `mover_tick`'s doc comment
    (the 2170 ms phase-drift and Z-drift rationale) on `drive_mover`. Replace the three
    per-loop `mover_tick(...); let _ = mover.poll();` pairs (AOI wait L529-530, settle
    L544-545, window L559-560) with `drive_mover(&mut mover, &mut mover_dir, &mut last_reverse);`.
  - **Percentiles:** delete `pct`; after the window loop, build
    `let mut steps64: Vec<f64> = steps.iter().map(|&s| s as f64).collect();`, drop the
    manual `steps.sort_by` (L580 — `percentile` sorts in place), compute
    `p50`/`p99` via `percentile(&mut steps64, ..)` and `max` as `*steps64.last().unwrap()`
    after the last percentile call; cast `nominal` to `f64` in the gates. Assertions'
    thresholds and messages unchanged.
  - Remove imports orphaned by the swap (`VecDeque`, `MoveIntentEntry`, `encode`,
    `decode`, `ClientMsg`, `ServerMsg`, `ClientEvent`, `AccountToken`, `MOVE_RING_LEN`,
    `NetClient` where now unused — let the compiler enumerate).
- **Path:**
  1. Add `test-support = { path = "../../testing/test-support" }` to
     `client/vordar-client/Cargo.toml` `[dev-dependencies]`.
  2. Apply the substitutions above; replace the three cwd stanzas with
     `test_support::workspace_root();` (imported).
  3. Prove (behavioral, real servers): `cargo nextest run -p vordar-client` — the two
     always-on e2e tests (`kicked_connection_reconnects_and_relogs_in`,
     `onslaught_dash_replay_never_snaps_at_150ms_rtt`) pass through the `Bot` kicker.
  4. Prove the mover swap:
     `cargo test -p vordar-client --release -- --ignored --nocapture net::e2e::remote_render_smoothness_under_loss_probe`
     — both gates (`max_zero_run <= 5`, `p99 <= 1.5x nominal`) must pass. The probe is
     statistical with ≥2x margins: if a gate fails, re-run once; if it fails twice,
     do NOT loosen the gates — revert nothing, park the step and report the measured
     numbers (a real behavior change in the mover would be a finding, not a tweak).
     Record the printed p50/p99/max/zero-run line in the commit message.

### 3. Server-spawn helpers in `test-support`; client e2e adopts them

- **Evidence:** the fire-and-forget server bring-up is copy-pasted:
  `client/vordar-client/src/net/e2e.rs` L79-83 (port 25400, 1800 ticks), L203-207
  (25402, 2400), L433-437 (25404, 60*60) — each `std::thread::spawn(move || { vordar_server::build_server_app(addr, ":memory:").run_headless(60.0, Some(N)); })`
  followed by `std::thread::sleep(Duration::from_millis(300))`. The identical shape
  recurs ~12 times across server test binaries (converted in step 4).
  `engine_app::app::App::run_headless(&mut self, hz: f64, max_ticks: Option<u64>)`
  (smirk/engine-app/src/app.rs L263); `build_server_app(addr: SocketAddr, db_path: &str) -> App`
  (server/vordar-server/src/lib.rs L41).
- **Ideal:** `test_support::server` owns
  `pub fn spawn_server(addr: SocketAddr, db_path: &str, max_ticks: u64)` and
  `pub fn spawn_server_with(addr: SocketAddr, db_path: &str, max_ticks: u64, configure: impl FnOnce(&mut engine_app::app::App) + Send + 'static)`;
  `spawn_server` delegates to `spawn_server_with(.., |_| {})`. `spawn_server_with`
  clones `db_path` to an owned `String`, spawns the thread
  (`build_server_app` → `configure(&mut app)` → `app.run_headless(60.0, Some(max_ticks))`),
  then sleeps 300 ms before returning. The client's three sites become one-liners:
  `spawn_server(addr, ":memory:", 1800);` etc.
- **Gap:** the bring-up idiom (including the load-bearing 300 ms settle) has no single
  owner, so its copies can drift independently.
- **Suggestion:** add the two functions to `testing/test-support/src/server.rs` with a
  doc comment stating the 300 ms settle is included; convert only the client this step
  (small, verifiable diff — the server sweep is step 4).
- **Path:**
  1. Add `spawn_server`/`spawn_server_with` to `testing/test-support/src/server.rs`
     (re-exported via lib.rs like everything else).
  2. In `client/vordar-client/src/net/e2e.rs`, replace the three spawn+sleep stanzas
     with `spawn_server(addr, ":memory:", 1800 | 2400 | 60 * 60);` (import it), keeping
     each site's surrounding comments.
  3. Prove: `cargo nextest run -p vordar-client` — the two real-server e2e tests pass,
     which exercises `spawn_server` end-to-end (a server that failed to come up would
     time out the Welcome waits). No new test needed: the helper's behavior IS the
     bring-up these tests depend on.

### 4. Server test binaries adopt `spawn_server`/`spawn_server_with`

- **Evidence:** exact-shape spawn sites in server tests — plain
  (`spawn` + `run_headless` + 300 ms sleep): `tests/e2e.rs` L32-35, L88-91, L129-132,
  L189-192; `tests/e2e_combat.rs` L19-22, L192-195; `tests/e2e_persistence.rs` L18-21,
  L61-64, L116-119 (first two pass `&server_db`, third `":memory:"`);
  `tests/e2e_security.rs` L79-82, L121-124; `tests/e2e_wireformat.rs` L114-117.
  Configure-shape (`let mut app = build_server_app(..); app.add_plugin/add_system(..); app.run_headless(..)` + 300 ms sleep):
  `tests/e2e.rs` L164-169, L232-256, L312-317, L350-355; `tests/e2e_combat.rs` L114-…;
  `tests/e2e_security.rs` L27-…; `tests/e2e_wireformat.rs` L28-32, L65-…, L171-…;
  `tests/loss.rs` L37-41, L146-150 (configure-shape with only a tick-budget comment
  inside — convert to plain `spawn_server(addr, ":memory:", 60 * 600 | 60 * 400)` and
  move the comment above the call). NOT to convert: `tests/soak.rs` L117-140 (uses
  `build_server_app_with_limits` — a genuinely different bring-up), anything in
  `shutdown.rs`/`watchdog.rs`/`zones.rs` (they use `spawn_zones`), and any site not
  immediately followed by a 300 ms sleep (check `tests/e2e_persistence.rs` L159-162,
  the 600-tick restart server — if its follow-up differs from the plain 300 ms shape,
  leave it exactly as is).
- **Ideal:** every exact-shape site is one call — plain sites
  `spawn_server(addr, db, ticks);`, configure sites
  `spawn_server_with(addr, db, ticks, |app| { ...verbatim registrations... });` — with
  every existing comment preserved (moved into the closure or above the call).
- **Gap:** ~20 hand-rolled copies of the bring-up idiom remain after step 3 owns it.
- **Suggestion:** purely mechanical, rule-driven conversion: a site converts iff its
  thread body is exactly `build_server_app` → zero-or-more `app.add_plugin`/`app.add_system`
  lines (+ comments) → `run_headless(60.0, Some(N))`, and the next statement is
  `sleep(300ms)`. Anything else stays untouched — the helper exists to kill the common
  copy-paste, not to absorb every bring-up. Delete the call-site `sleep(300ms)` when
  converting (the helper sleeps). `PopulateSystem`/`MetricMirror`/`Phase`/`SystemOrder`
  imports at converted files already exist; remove `Duration` or other imports only if
  YOUR conversion orphaned them.
- **Path:**
  1. Sweep `server/vordar-server/tests/*.rs` with the rule above; convert each matching
     site (`use test_support::{spawn_server, spawn_server_with}` additions as needed).
  2. Prove: `cargo nextest run -p vordar-server` — green, unchanged test count (the
     converted bring-ups back every e2e test in the suite, so a wrong ticks/db/configure
     translation fails loudly here). Also `cargo check -p vordar-server --tests` is
     implied by nextest's build of the ignored probes (loss.rs converted sites).

### 5. Benchmarks dedup: `Lcg` to `test-support`, soak's `Wander` on it, `AOI_RADIUS` from the bench seam

- **Evidence:** `benchmarks/src/lib.rs` L32-45 defines `Lcg` with the comment
  "same constants as tests/soak.rs's Wander"; `server/vordar-server/tests/soak.rs`
  L56-81 `Wander` inlines those constants (`2862933555777941757`/`3037000493` seed
  scramble at L64; step constants and 31-bit angle recipe at L76-77).
  `benchmarks/src/lib.rs` L22-23 `pub const AOI_RADIUS: f32 = 40.0;` with "Matches the
  server net module's AOI_RADIUS", while `server/vordar-server/src/net/bench.rs` L9
  already exports `pub const AOI: f32 = AOI_RADIUS;` and `benchmarks/Cargo.toml` L19
  already depends on `vordar-server` with `features = ["bench-internals"]`.
  `benchmarks/src/lib.rs` L27-30 `workspace_root` duplicates the cwd fix ("same trick
  as server/vordar-server/tests/common"). `benchmarks/benches/snapshot.rs` L24 imports
  `Lcg` from `vordar_benches`.
- **Ideal:** `test_support::util::Lcg` is the one LCG, with `pub fn next_u64(&mut self) -> u64`
  (advance state, return it) and `next_f32` built on it (`((self.next_u64() >> 33) as u32) as f32 / (u32::MAX as f32 + 1.0)`
  — bit-identical to today). `benchmarks/src/lib.rs` re-exports it
  (`pub use test_support::{Lcg, workspace_root};`) so `vordar_benches::{Lcg, workspace_root}`
  call sites in bench targets compile unchanged, and defines
  `pub const AOI_RADIUS: f32 = vordar_server::net::bench::AOI;` (comment updated: taken
  from the seam, cannot drift). `soak.rs`'s `Wander` holds `rng: Lcg`
  (`Lcg::new(seed)` performs the identical seed scramble) and derives the angle as
  `(self.rng.next_u64() >> 33) as f32 / (u32::MAX >> 1) as f32 * std::f32::consts::TAU`
  — bit-identical trajectories.
- **Gap:** two LCGs and one constant are kept equal by comment discipline alone; the
  benchmarks crate documents its own coupling.
- **Suggestion:** move `Lcg` (adding `next_u64` in the same motion), re-export rather
  than re-house `workspace_root`, and take `AOI_RADIUS` from the already-existing bench
  seam. `CELL_SIZE`'s "matches PhysicsPlugin" comment is analogous but was not cited by
  the finding and PhysicsPlugin exposes no seam — leave it.
- **Path:**
  1. Add `Lcg` (with `new`, `next_u64`, `next_f32`, its doc comment) to
     `testing/test-support/src/util.rs`.
  2. `benchmarks/Cargo.toml`: add `test-support = { path = "../testing/test-support" }`
     under `[dependencies]` (normal dep — the lib uses it; see Design decisions).
  3. `benchmarks/src/lib.rs`: delete the `Lcg` and `workspace_root` definitions,
     `pub use test_support::{Lcg, workspace_root};`; change `AOI_RADIUS` to
     `= vordar_server::net::bench::AOI;` and update its comment.
  4. `server/vordar-server/tests/soak.rs`: refactor `Wander` onto `test_support::Lcg`
     exactly as specified in Ideal (keep `dir`/`sends` fields and all logic verbatim).
  5. Prove: `cargo check -p vordar-benches --benches` (all ten bench targets compile);
     `cargo bench -p vordar-benches --bench snapshot -- --test` (criterion test mode
     runs each snapshot bench once — exercises `Lcg` and `AOI_RADIUS` through real
     bench code); `cargo nextest run -p vordar-server` (compiles soak, runs the
     non-ignored suite). The full 200-bot soak is NOT run (heavy, and the refactor is
     bit-identical by construction); if the worker has any doubt about bit-identity,
     print the first 5 `next_dir` angles for seed 1000 before and after the change in a
     scratch binary and compare — they must be equal to the last bit.

### 6. Final verification, release guard, and queue bookkeeping (docs-only)

- **Evidence:** finding 4's own end-state checks: constraint (2) —
  `cargo build --release -p vordar-server -p vordar-client` must not build the crate;
  constraint (3) — `name_token`/percentile/impairment presets exist exactly once
  workspace-wide, "grep is the check". The cross-type queue note lives in
  `docs/reviews/hygiene/reworks-hygiene-2026-07-14.md` L17-28 and is mirrored in
  `docs/reviews/hygiene/audit-hygiene-2026-07-14.md`.
- **Ideal:** all three checks pass and rework 4 is struck through in both queue notes,
  matching how reworks 1-3 were marked.
- **Gap:** unverified end state; queue notes still show rework 4 pending.
- **Suggestion:** run the checks; if any grep finds a second definition, that is a
  missed site from steps 1-5 — report it with the file:line rather than patching here.
- **Path:**
  1. `cargo nextest run --workspace` — green.
  2. `cargo build --release -p vordar-server -p vordar-client`, then
     `cargo tree -p vordar-server -p vordar-client -e normal | grep -c test-support`
     → must print 0 (test-support absent from normal dependency graphs, hence from
     shipping artifacts).
  3. Greps (each must yield exactly one defining hit, in `testing/test-support/src/`):
     `grep -rn "fn name_token" --include=*.rs .`;
     `grep -rn "fn percentile\|fn pct" --include=*.rs .`;
     `grep -rn "fn connect_impaired_as\|fn connect_upstream_impaired_as\|fn connect_full_as" --include=*.rs .`;
     `grep -rn "struct Lcg" --include=*.rs .`;
     `grep -rn "mod common" server/ client/` → no hits.
     If a grep shows a stray copy, stop and report file:line — do not fix silently.
  4. Strike rework 4 in the queue note of `reworks-hygiene-2026-07-14.md`
     (`rework 4` → `~~rework 4~~`) and in the mirrored queue in
     `audit-hygiene-2026-07-14.md`.
