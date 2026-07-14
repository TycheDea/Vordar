# Plan: Decompose client net.rs (2,474 lines) into a net module family — 2026-07-14

Source: `docs/reviews/hygiene/reworks-hygiene-2026-07-14.md` finding 1.

## Ideal end state

`client/vordar-client/src/net.rs` (2,474 lines, nine responsibilities) becomes a
`net/` module family — `net/mod.rs` (plugin + state), `net/lifecycle.rs`,
`net/apply.rs`, `net/prediction.rs`, `net/interpolate.rs`, `net/bench.rs`
(feature-gated), `net/e2e.rs` (test-only) — with the four non-netcode tenants
evicted to homes whose names predict them: `world_time.rs`, `telegraph.rs`,
`cast.rs`, and `NetCameraFollowSystem` beside its sandbox twin in `lib.rs`. Unit
tests sit beside the module they pin; the three real-server e2e tests share one
test module. One `NetClientState::new` constructor replaces the 8 written-out
~20-field literals. Behavior is bit-identical: `NetClientPlugin::build`'s
registration order is preserved verbatim, the external surface
(`vordar_client::net::NetClientPlugin` for `bin/vordar.rs`,
`vordar_client::net::bench` for `benchmarks/benches/client_netcode.rs`) is
unchanged, and every step leaves `cargo nextest run --workspace` green at
unchanged test counts.

## Design decisions

- **`net/mod.rs` family, matching the repo's dir-module convention** (`src/ui/mod.rs`,
  `game/vordar-game/src/world/mod.rs`). `mod.rs` keeps the plugin, `NetClientState`,
  and the two crate-facing accessors (`own_entity`, `reconnect_attempt`); the four
  submodules are private (`mod lifecycle;` etc.), their items `pub(super)` — visible
  throughout the `net` tree (including `bench` and the test modules, which are
  descendants of `net`) but invisible to the rest of the crate. Rejected: flat sibling
  files (`net_lifecycle.rs`, …) — they would leak every internal item to the whole
  crate or force `pub(crate)` on all of it.
- **`NetClientState` stays ONE struct, one resource — no field partition into multiple
  resources.** `teardown_replicated_world` resets nearly every field in one place: the
  struct genuinely is "per-connection session state", a single cohesive thing. Splitting
  it (ConnectionState / PredictionState / PlaybackState) would also break the
  `bench-internals` seam signature (`state_for_bench` returns `NetClientState`), which
  constraint (3) of the finding forbids. The finding's "how the fields partition"
  question is answered by module privacy instead: fields stay private to the struct,
  and child modules of `net` access them directly (Rust privacy: items private in a
  module are visible to its descendants). Zero getters needed inside the family.
- **The evicted tenants consume `NetClientState` only through three small `pub(crate)`
  accessors** added to the impl in `net/mod.rs`: `server_now_micros()` (DayNight,
  TelegraphFill, cast), `predicting()` (cast), and `send_cast_intent(skill, target)`
  (cast — bundles the stamp-seq-encode-send block so `cast.rs` never touches
  `client`/`seq`; the onslaught e2e test's hand-rolled copy of that block switches to
  the same method, removing a duplication). Rejected: `pub(crate)` on the raw fields —
  it would let any module in the crate mutate connection state.
- **Only `NetClientPlugin`, `NetClientState`, and `mod bench` stay `pub`.**
  `NetClientState` must remain `pub` because the public (feature-gated) `bench` module
  leaks it in signatures (E0446 otherwise). Every system currently `pub` in net.rs
  (`NetReceiveSystem`, `NetSendInputSystem`, `NetCorrectionSystem`,
  `NetInterpolateSystem`) has zero users outside the crate (verified: only
  `bin/vordar.rs` uses `net::NetClientPlugin`; only benches use `net::bench`) and
  becomes `pub(super)`. `own_entity` / `reconnect_attempt` stay `pub(crate)` (used by
  `lib.rs`, `ui/action_bar.rs`, `presentation.rs`). Evicted tenant modules
  (`world_time.rs`, `telegraph.rs`, `cast.rs`) follow sibling-module style
  (`pub mod`, `pub` systems, like `react.rs` / `locomotion.rs`); `WorldTime`'s two
  fields become `pub(crate)` since the net receive path and tests write them.
- **The three real-server e2e tests land together in `net/e2e.rs`**
  (`#[cfg(test)] mod e2e;`), not beside individual modules. They pin cross-module
  flows (receive + send + prediction + interpolation against a live headless server),
  and they share helpers (`name_token`, the 10-line prefab-registry block duplicated
  today at net.rs:2034-2044 and 2326-2336, `pct`/`pace_tick`/`mover_tick`). One module
  means each helper exists once and gives hygiene rework 4 (shared test-support crate)
  a single thing to lift later. Unit tests DO move beside their subjects
  (interpolate: 2, prediction: 5, apply: 1). The trivial `const DT: f32 = 1.0/60.0`
  is re-declared per test module (3 one-liners) rather than a `test_util` module —
  rework 4 owns real helper consolidation; a new shared module for one constant is
  over-structure.
- **Constructor over literals, and it is production code**:
  `pub(crate) fn NetClientState::new(client: Option<NetClient>, server_addr: SocketAddr,
  user: String, token: AccountToken, predict: bool, simulated_rtt: Duration) -> Self`,
  defaulting all bookkeeping fields (reconnect None, login_denied false, own_id None,
  empty entities/prefab_names/pending/move_ring, seq 0, correction ZERO,
  latest_state_tick 0, playback None). The plugin's `build`, `bench::state_for_bench`,
  and all 6 test literals call it, then overwrite the few fields they care about
  (allowed: they are all inside the `net` tree). One place lists all ~20 fields; adding
  a field in a future protocol rework touches one site instead of eight. This lands
  FIRST, while everything is one file, so no later move drags a 20-line literal along.
- **Moved code keeps its comments verbatim.** The queue in the reworks file orders this
  rework before the comment-cleanup findings (audit findings 4-7) precisely so cleaning
  happens once, after splitting. Workers must not "improve" comments mid-move — only
  fix references that the move itself breaks (e.g. `mover_tick`'s doc cites
  "net.rs:1203-1212"; that becomes a reference to `NetSendInputSystem` by name, no line
  numbers). New module headers state intent + scheduling constraints only, no
  finding/rework citations.
- **Eviction before family split.** Steps 2-4 move the tenants out while net.rs is
  still one file (a move out of a file is simpler than a move out of a family), then
  steps 5-8 split what remains along the finding's seams in dependency order
  (interpolate → prediction → apply → lifecycle), so each extracted file's imports are
  written once. Known single double-touch: interpolate's unit tests call `apply_states`,
  which moves in step 7 — step 7 updates that one import.
- **Verification per step** (the finding's constraints, applied to every step):
  (1) `cargo nextest run --workspace` green, and the pass/skip counts printed by
  nextest identical before/after the step (capture the "N tests run: N passed,
  M skipped" line before starting); (2) `NetClientPlugin::build`'s `insert_resource` /
  `add_system` sequence byte-identical except for import/type-path spelling;
  (3) `cargo check -p vordar-benches --benches` compiles (the `bench-internals` gate —
  `client_netcode.rs` itself is never edited). Baseline for reference: net.rs today
  contributes 10 running tests + 1 ignored (`remote_render_smoothness_under_loss_probe`)
  to `-p vordar-client`.
- **Follow-on invalidation (product-neutral, no user decision needed):**
  `docs/reviews/networking/plan-networking-rework-7-2026-07-14.md` cites net.rs line
  numbers and registration seams; the finding's Path already prescribes the remedy —
  mark that plan stale and require a /plan-rework re-run before networking rework 7
  executes. Step 10 (docs-only) records this.

## Findings (execution order)

### 1. One `NetClientState::new` constructor replaces the 8 field literals

- **Evidence:** `client/vordar-client/src/net.rs` writes the full ~20-field
  `NetClientState` literal at 8 sites: the plugin's `build` (L120-138), the bench
  seam's `state_for_bench` (L1364-1384), and 6 tests (L1487-1505, L1608-1626,
  L1716-1734, L1889-1907, L2055-2076, L2345-2371). The literals differ only in
  `client`, `server_addr`, `user`, `token`, `predict`, `simulated_rtt` (and the plugin
  additionally sets `reconnect`); every other field is the same empty/zero default at
  every site. The struct's fields are declared at L196-252.
- **Ideal:** one constructor in net.rs —
  `pub(crate) fn new(client: Option<NetClient>, server_addr: SocketAddr, user: String, token: AccountToken, predict: bool, simulated_rtt: Duration) -> Self`
  in `impl NetClientState` (beside the existing `own_entity` method, L254-258) —
  defaulting: `reconnect: None`, `login_denied: false`, `own_id: None`,
  `entities: HashMap::new()`, `prefab_names: Vec::new()`, `seq: 0`,
  `pending: VecDeque::new()`, `move_ring: VecDeque::new()`, `correction: Vec3::ZERO`,
  `latest_state_tick: 0`, `playback: None`. All 8 sites call it; callers that need a
  non-default field set it on the returned value before inserting
  (all callers are inside net.rs today, so private-field assignment compiles).
- **Gap:** adding/renaming a `NetClientState` field today means editing 8 literals in
  lockstep (every protocol rework this month did exactly that); the upcoming module
  split would otherwise drag 20-line literals into the moved test modules.
- **Suggestion:** in `client/vordar-client/src/net.rs` only:
  - Add the constructor to `impl NetClientState`.
  - `NetClientPlugin::build` (L106-138): keep the existing `match NetClient::connect_with_latency(...)`
    producing `(client, reconnect)` verbatim, then
    `let mut state = NetClientState::new(client, self.server_addr, self.user.clone(), self.token, self.predict, self.simulated_rtt); state.reconnect = reconnect; app.insert_resource(state)` —
    the `.add_system(...)` chain after `insert_resource` is untouched.
  - `bench::state_for_bench` (L1362-1385): body becomes
    `let server_addr = "127.0.0.1:9".parse().unwrap();` +
    `let mut state = NetClientState::new(Some(NetClient::connect(server_addr, PROTOCOL_VERSION).expect("bench NetClient")), server_addr, "bench".into(), [0u8; 32], predict, Duration::ZERO); state.own_id = own_id; state`.
    Signature unchanged.
  - Each test literal becomes `NetClientState::new(...)` + explicit assignments for
    only its non-defaults: L1487 (`entities`), L1608 (`entities`), L1716
    (`own_id = Some(2)`, `entities`, `pending = VecDeque::from(vec![intent(48, Vec2::X), intent(49, Vec2::X)])`,
    predict=true via the ctor arg), L1889 (client Some(connect), user/token
    "reconnect-victim"), L2055 (client Some(connect_with_latency 150ms), predict=true,
    simulated_rtt 150ms, user/token "onslaught-dasher"), L2345 (client
    Some(connect_impaired …), simulated_rtt 100ms, user/token "smoothness-observer").
- **Path:** (1) capture the baseline: run `cargo nextest run --workspace` and note the
  final counts line. (2) Make the edits above — no other file changes; do not touch
  comments beyond the edited expressions. (3) The regression proof is behavioral and
  already exists: the full suite exercises the constructor through every path — the
  plugin path via `kicked_connection_reconnects_and_relogs_in` (drives `build`'s state
  shape through a real server round-trip is not applicable, but the test constructs
  via `new` and drives `NetReceiveSystem` end-to-end), the bench path via
  `cargo check -p vordar-benches --benches`, the test path via all 10 net tests.
  Re-run `cargo nextest run --workspace` — green, counts identical to (1). Run
  `cargo check -p vordar-benches --benches` — compiles. If any test fails, the
  constructor's defaults diverge from a literal — diff the failing test's old literal
  against `new`'s defaults and fix the constructor (never the test's assertions).

### 2. Evict `WorldTime`/`DayNightSystem` to `world_time.rs` and `NetCameraFollowSystem` to `lib.rs`

- **Evidence:** `client/vordar-client/src/net.rs` L883-928 holds `WorldTime` (struct,
  2 fields: `offset_micros: i64`, `synced: bool`) and `DayNightSystem` (drives the
  light uniform from world time via `vordar_game::world::{active_event, day_night_light, WorldEventsDef}`
  and `engine_renderer::set_light`) — neither is netcode. L1335-1348 holds
  `NetCameraFollowSystem`, the 14-line networked twin of `CameraFollowSystem`
  (`lib.rs:190-198`); it calls `own_entity`, `crate::render_position`, and
  `crate::orbit_and_follow` — all defined in `lib.rs` or `net`. Cross-references into
  these items: `NetReceiveSystem` writes `wt.offset_micros`/`wt.synced` on
  `ServerMsg::WorldClock` (L402-406); `teardown_replicated_world` sets
  `wt.synced = false` (L481); the plugin inserts `WorldTime { offset_micros: 0, synced: false }`
  (L142) and registers both systems (L165, L167); tests construct the same `WorldTime`
  literal (L1888, L2052, L2344). `DayNightSystem` reads
  `state.client.as_ref().and_then(|c| c.server_now_micros())` (L905) — private
  `NetClientState` internals.
- **Ideal:** `client/vordar-client/src/world_time.rs` (new, `pub mod world_time;` added
  to the lib.rs module list at lib.rs:6-16): module header stating "world-clock mapping
  + day/night lighting, pure functions of the synced server clock", containing
  `pub struct WorldTime { pub(crate) offset_micros: i64, pub(crate) synced: bool }` and
  `pub struct DayNightSystem` + its `System` impl, moved verbatim.
  `NetCameraFollowSystem` + its `System` impl move to `lib.rs`, placed directly after
  `CameraFollowSystem` (lib.rs:198), body changed only to call
  `net::own_entity(resources)` — keep it `pub`. `NetClientState` gains
  `pub(crate) fn server_now_micros(&self) -> Option<u64> { self.client.as_ref().and_then(|c| c.server_now_micros()) }`
  in its impl in net.rs, and `DayNightSystem` uses `state.server_now_micros()` instead
  of reaching into `client`.
- **Gap:** four of net.rs's tenants are not netcode; these are the two smallest — a
  world-clock/lighting module and a camera system hidden in a file named "net".
- **Suggestion:** move the code verbatim (comments included). In net.rs update:
  the plugin `insert_resource(crate::world_time::WorldTime { offset_micros: 0, synced: false })`,
  the two `add_system` lines' type paths
  (`crate::world_time::DayNightSystem`, `crate::NetCameraFollowSystem` — or `use` them;
  the registration ORDER and Phase/SystemOrder arguments stay byte-identical),
  the `WorldClock` arm and `teardown_replicated_world`'s `WorldTime` accesses
  (`resources.get_mut::<crate::world_time::WorldTime>()`; field writes still compile —
  fields are `pub(crate)`), and the three test literals' `WorldTime` imports.
  `DayNightSystem` reads `NetClientState` via `resources.get::<NetClientState>()` —
  `NetClientState` is `pub`, and `server_now_micros` is `pub(crate)`: compiles from
  `world_time.rs`. Remove net.rs's now-unused imports
  (`active_event`, `day_night_light`, `WorldEventsDef` move with the system; check
  `orbit_and_follow` — it was imported at net.rs:15 for the camera system and leaves too).
- **Path:** (1) capture the nextest baseline counts. (2) Create `world_time.rs`, move
  the two items + `NetCameraFollowSystem` as above, add `pub mod world_time;` to
  lib.rs, add the `server_now_micros` accessor, fix all references. (3) Behavioral
  proof: `kicked_connection_reconnects_and_relogs_in` (net.rs tests) constructs
  `WorldTime`, drives the real `NetReceiveSystem` against a live server, and its
  teardown asserts state resets — it exercises the moved struct's writes through
  production code. `cargo nextest run --workspace` green at identical counts;
  `cargo check -p vordar-benches --benches` compiles; visually diff
  `NetClientPlugin::build` against git HEAD to confirm the wiring sequence is
  unchanged except type paths.

### 3. Evict telegraph visuals to `telegraph.rs`

- **Evidence:** `client/vordar-client/src/net.rs` L930-1021: `TelegraphVisual`
  (component struct), `TELEGRAPH_DIM`/`TELEGRAPH_BRIGHT` consts, `spawn_telegraph`
  (called from `NetReceiveSystem`'s `MechanicScheduled` arm, L394-398), and
  `TelegraphFillSystem` (85 lines: fill animation + resolve-moment particle bursts) —
  client-side mechanic presentation, not netcode. Other references:
  `teardown_replicated_world` queries `&TelegraphVisual` to despawn visuals (L447);
  the plugin registers `TelegraphFillSystem` at L166; `TelegraphFillSystem` reads
  `resources.get::<NetClientState>().unwrap().client.as_ref().and_then(|c| c.server_now_micros())`
  (L970) — replaced in step 2's pattern by the `server_now_micros()` accessor.
- **Ideal:** `client/vordar-client/src/telegraph.rs` (new, `pub mod telegraph;` in
  lib.rs): module header stating the design constraint (fill is a pure function of
  synced server time vs `resolve_at` — zero per-frame network updates), containing
  `pub(crate) struct TelegraphVisual { resolve_at_micros: u64, duration_micros: u64 }`,
  the two color consts (private), `pub(crate) fn spawn_telegraph(...)` (same signature
  as today), and `pub struct TelegraphFillSystem` + impl using
  `state.server_now_micros()`.
- **Gap:** telegraph presentation is findable only by reading net.rs end to end; its
  natural home is a file named for it.
- **Suggestion:** move verbatim (comments included). In net.rs update: the
  `MechanicScheduled` arm calls `crate::telegraph::spawn_telegraph(...)`;
  `teardown_replicated_world`'s query names `crate::telegraph::TelegraphVisual`
  (visible: `pub(crate)`); the plugin's `add_system` line's type path (order
  preserved). Move the now-unused imports (`RenderShape` — check: net.rs:21 imports it
  only for `TelegraphFillSystem`'s query; `vordar_game::vfx::{BurstDef, ParticleBlend}`
  are referenced by full path already and move with the system).
- **Path:** (1) capture the nextest baseline counts. (2) Create `telegraph.rs`, move
  the four items, wire the three reference sites, prune imports. (3) No unit test pins
  telegraphs today (their gate is compile + the e2e suite's server round-trips), so
  the proof is: `cargo nextest run --workspace` green at identical counts,
  `cargo check -p vordar-benches --benches` compiles, `cargo build -p vordar-client`
  produces zero new warnings (an orphaned import is the likely failure mode), and
  `NetClientPlugin::build` diffs against HEAD only in type paths.

### 4. Evict `AbilityCastSystem` to `cast.rs` behind a `send_cast_intent` seam

- **Evidence:** `client/vordar-client/src/net.rs` L1023-1172: `SLOT_KEYS` const and the
  150-line `AbilityCastSystem` — input reading (LMB + Q/E edge triggers), cooldown
  sync against `crate::CastState`, range clamping, and per-cast side effects. Its only
  netcode is the block at L1135-1150: read `server_now_micros` (bail if unsynced),
  `state.seq += 1`, `client.send(encode(&ClientMsg::CastIntent { seq, t_server_micros, skill, target }))`,
  then return `(state.own_entity(), state.predict)`. The same stamp-seq-send block is
  hand-rolled in the `onslaught_dash_replay_never_snaps_at_150ms_rtt` test
  (L2155-2166). `AbilityCastSystem` also calls `start_predicted_leap`
  (net.rs:1179-1186, inserts the optimistic `LeapImpulse` and retags this tick's
  `PendingIntent`) and `own_entity(resources)`. The plugin registers it at L141
  (`Phase::Input, SystemOrder::after::<NetSendInputSystem>()`).
- **Ideal:** `client/vordar-client/src/cast.rs` (new, `pub mod cast;` in lib.rs):
  `SLOT_KEYS` + `pub struct AbilityCastSystem` (+ `pub fn new`) moved verbatim, with
  the L1135-1150 block replaced by production seams on `NetClientState` (in net.rs):
  - `pub(crate) fn send_cast_intent(&mut self, skill: String, target: Vec2) -> bool` —
    returns false (no send, no seq bump) when `server_now_micros()` is None; otherwise
    bumps `seq`, encodes and sends `ClientMsg::CastIntent`, returns true. (When
    `server_now_micros()` is Some, `client` is necessarily Some — same invariant the
    current code relies on.)
  - `pub(crate) fn predicting(&self) -> bool { self.predict }`
  `cast.rs`'s per-slot loop becomes: `if !state.send_cast_intent(id.clone(), target) { return; }`
  (matching today's early `return` from the whole run when the clock is unsynced),
  `let own = crate::net::own_entity(resources); let predict = ...predicting()`.
  `start_predicted_leap` stays in net.rs, visibility raised to `pub(crate)`. The
  onslaught e2e test's hand-rolled block (L2155-2166) becomes
  `assert!(resources.get_mut::<NetClientState>().unwrap().send_cast_intent("onslaught".into(), cast_target))` —
  the test now exercises the same production send path `cast.rs` uses.
- **Gap:** a 150-line input/UI system lives in "net" because it needs a 10-line send;
  the send block exists twice (system + test).
- **Suggestion:** as above. Preserve behavior precisely: today the unsynced-clock case
  `return`s out of the entire system run mid-loop (skipping later triggered slots) —
  keep that (`return` on `false`). `crate::CastState`, `crate::local_class`,
  `engine_renderer::screen_to_ground`, `crate::pose`/`crate::locomotion`/`crate::vfx`
  calls all remain valid from `cast.rs` (all `pub` in-crate). The plugin's
  `add_system(crate::cast::AbilityCastSystem::new(), Phase::Input, SystemOrder::after::<NetSendInputSystem>())`
  keeps its position (line 141, between `NetSendInputSystem` and the
  `insert_resource(WorldTime …)` line). Prune net.rs's now-unused imports (`winit`
  keycode/mouse types move; check `Vec2` stays — `send_cast_intent` uses it).
- **Path:** (1) capture the nextest baseline counts. (2) Add the two accessors +
  visibility change in net.rs; create `cast.rs`; move the system; switch the onslaught
  test to `send_cast_intent`. (3) Behavioral proof:
  `onslaught_dash_replay_never_snaps_at_150ms_rtt` — a real headless server, a real
  predicting client at 150 ms simulated RTT, casting through the NEW production
  `send_cast_intent` seam and asserting no reconciliation snap: if the seam drops the
  seq bump, mis-stamps time, or sends the wrong shape, the server rejects/mis-orders
  the cast and the dash snaps past `SNAP_DISTANCE`, failing the test.
  `cargo nextest run --workspace` green at identical counts;
  `cargo check -p vordar-benches --benches` compiles; `build` wiring diff = type paths
  only.

### 5. Convert to `net/mod.rs` and extract `net/interpolate.rs`

- **Evidence:** after steps 1-4, `client/vordar-client/src/net.rs` (~2,050 lines)
  holds only netcode. The interpolation slice — `NET_BUFFER_CAP`,
  `INTERP_DELAY_TICKS`, `MAX_SLEW_FRACTION`, `RESYNC_TICKS`, `EXTRAP_CAP_TICKS`
  (today's net.rs L276-301), `NetBuffer` + impl (L313-340), `NetInterpolateSystem`
  (L1248-1272), `advance_playback` (L1280-1290), `sample_buffer` (L1300-1333) — plus
  its two unit tests `fixed_delay_playback_rides_through_jittered_arrivals` (L1471)
  and `extrapolation_bridges_lost_snapshots_then_caps` (L1588). Cross-references that
  stay behind: `apply_aoi_delta` seeds `NetBuffer::seeded` (L648), `apply_states`
  reads `buffer.samples.back()` and calls `buffer.push` (L746-750), the plugin
  registers `NetInterpolateSystem` (L147). The two tests call `apply_states` (still in
  the parent module after this step) and construct `NetClientState` via `new` (step 1).
- **Ideal:** `net.rs` becomes `client/vordar-client/src/net/mod.rs` (`git mv`, content
  initially unchanged — `pub mod net;` in lib.rs already resolves to the dir form),
  then the interpolation slice moves to `client/vordar-client/src/net/interpolate.rs`
  (declared `mod interpolate;` in mod.rs): module header stating the fixed-delay
  playback model (render `INTERP_DELAY_TICKS` behind the newest snapshot tick; slewed
  cursor; capped extrapolation). Visibilities: `pub(super)` on `NetBuffer` (and its
  `samples` field, `seeded`, `push`), `NetInterpolateSystem`, and any const the parent
  or sibling tests use (`INTERP_DELAY_TICKS` is referenced only within the slice and
  its tests today — keep consts private unless the compiler says otherwise;
  `EXTRAP_CAP_TICKS` is used by the extrapolation test, same file). `advance_playback`
  and `sample_buffer` stay private to `interpolate.rs`. The two unit tests move into
  `#[cfg(test)] mod tests` at the bottom of `interpolate.rs`, with
  `const DT: f32 = 1.0 / 60.0;` declared locally and imports:
  `use super::*;` plus `use crate::net::{apply_states, NetClientState};` (both visible
  here — descendants of `net` see `net`'s private items),
  `use vordar_protocol::WirePos;` etc.
- **Gap:** the interpolation buffer is split across two distant regions of one huge
  file (L273-340 and L1240-1333, per the finding's evidence); its tests sit 1,100
  lines from the code they pin.
- **Suggestion:** two commits-worth in one step: (a) `git mv src/net.rs src/net/mod.rs`
  (build must already pass here); (b) extract the slice + tests. In mod.rs:
  `mod interpolate;` + `use interpolate::{NetBuffer, NetInterpolateSystem};` — the
  plugin registration line and `apply_*` fns keep compiling unchanged. Move the
  `NetMotion` reference note: `NetInterpolateSystem` writes
  `crate::locomotion::NetMotion` — path already absolute, moves cleanly.
- **Path:** (1) capture the nextest baseline counts. (2) `git mv`, build, then extract
  as above. (3) Behavioral proof: the two moved tests themselves — they drive the real
  `apply_states` → `NetBuffer` → `NetInterpolateSystem` pipeline tick-by-tick and
  assert freeze/warble/cap/pop bounds; if the move breaks a visibility or drops a
  const, they fail to compile or run. `cargo nextest run --workspace` green at
  identical counts (the 2 tests now report under `net::interpolate::tests::…` — name
  prefix changes are fine, the COUNT must match);
  `cargo check -p vordar-benches --benches` compiles.

### 6. Extract `net/prediction.rs`

- **Evidence:** in `net/mod.rs` (was net.rs) the prediction/reconciliation slice:
  `TRUST_DISTANCE`, `SNAP_DISTANCE`, `CORRECTION_HALF_LIFE`, `MAX_PENDING_INTENTS`,
  `MOVE_RING_LEN` (today's net.rs L41-60), `PendingIntent` (L189-194), `reconcile_own`
  (L768-813), `replay_position` (L820-829), `Correction` enum + `classify_error`
  (L831-851), `correction_step` (L854-856), `NetCorrectionSystem` (L863-881),
  `NetSendInputSystem` (L1192-1238), `start_predicted_leap` (L1179-1186), and 5 unit
  tests: `replay_applies_unacked_intents` (L1787), `replay_normalizes_direction_like_the_simulation`
  (L1796), `replay_reconstructs_a_dash_leap_instead_of_dead_reckoning_wasd` (L1814),
  `error_classification_bands` (L1838), `correction_decays_smoothly_to_zero` (L1846),
  plus their `intent(seq, dir)` helper (L1442-1444). Cross-references: `apply_states`
  calls `reconcile_own` (L756); `NetClientState` fields hold `VecDeque<PendingIntent>`
  and use `MOVE_RING_LEN`-related docs; the plugin registers `NetSendInputSystem` and
  `NetCorrectionSystem`; `cast.rs` (step 4) calls `start_predicted_leap`; the bench
  module's `push_pending` constructs `PendingIntent`; the e2e tests use
  `NetSendInputSystem`/`NetCorrectionSystem`/`start_predicted_leap`/`SNAP_DISTANCE`/`MOVE_RING_LEN`.
- **Ideal:** `client/vordar-client/src/net/prediction.rs` (`mod prediction;` in
  mod.rs): module header stating the predict-and-reconcile contract (send intent +
  emit locally each Input tick; rebase onto server pos + replay unacked intents;
  trust/smooth/snap bands). Visibilities: `pub(super)` on `PendingIntent` (struct +
  all 4 fields — bench and tests construct it), `NetSendInputSystem`,
  `NetCorrectionSystem`, `reconcile_own`, `SNAP_DISTANCE`, `MOVE_RING_LEN`,
  `MAX_PENDING_INTENTS` (if mod.rs docs/code reference it — compiler decides);
  `pub(crate)` on `start_predicted_leap` (called from `cast.rs`, outside `net`).
  `replay_position`, `Correction`, `classify_error`, `correction_step` and the other
  consts stay private to `prediction.rs`. The 5 tests + `intent` helper +
  `const DT: f32 = 1.0 / 60.0;` move into `#[cfg(test)] mod tests` in
  `prediction.rs`.
- **Gap:** prediction is the client's most behavior-dense netcode, currently
  interleaved with receive dispatch and input send across three regions of the file.
- **Suggestion:** move verbatim; in mod.rs add
  `use prediction::{NetCorrectionSystem, NetSendInputSystem, PendingIntent};` (state
  field type + plugin registrations), and leave the still-in-mod.rs `apply_states`
  calling `prediction::reconcile_own` (or via the `use`). The remaining mod.rs tests
  (`apply_states_drops_a_stale_snapshot_tick` uses `intent(...)` — it moves in step 7;
  for THIS step it stays in mod.rs's tests and needs the helper: give the moved
  `intent` helper `pub(super)` visibility inside `prediction::tests`? No — test
  modules can't export cleanly across files. Instead: `apply_states_drops_a_stale_snapshot_tick`
  builds its two `PendingIntent`s inline (`PendingIntent { seq: 48, dir: Vec2::X, dt: DT, leap: None }`)
  for the one step it remains in mod.rs; step 7 moves it beside `apply.rs` where it
  keeps that inline form (2 uses total — no helper needed there). The e2e tests
  (still in mod.rs's tests) reference `NetSendInputSystem` etc. via `super::*` →
  update to `use super::prediction::…` or rely on mod.rs's `use` re-exposure (the
  `use` in mod.rs is private-by-default but visible to mod.rs's own test module via
  `use super::*` — it is: `use super::*` imports mod.rs's `use`d names).
- **Path:** (1) capture the nextest baseline counts. (2) Extract as above; fix the
  stale-snapshot test's `intent` uses inline; confirm the e2e tests compile via
  `use super::*`. (3) Behavioral proof: the 5 moved unit tests (replay math, error
  bands, correction decay through real production fns) plus
  `onslaught_dash_replay_never_snaps_at_150ms_rtt` (drives the real
  `NetSendInputSystem` + `NetCorrectionSystem` + `start_predicted_leap` against a live
  server — any visibility or wiring slip fails it). `cargo nextest run --workspace`
  green at identical counts; `cargo check -p vordar-benches --benches` compiles
  (bench's `push_pending` now constructs `prediction::PendingIntent`).

### 7. Extract `net/apply.rs`

- **Evidence:** in `net/mod.rs` the snapshot-apply slice: `apply_aoi_delta` (today's
  net.rs L623-672), `apply_states` (L680-761), `handle_entity_died` (L572-614 — a
  server-message applier: death burst + corpse + despawn), and the unit test
  `apply_states_drops_a_stale_snapshot_tick` (L1703-1784). Cross-references:
  `NetReceiveSystem`'s match arms call all three (L388-393, L407-409); the bench
  module wraps `apply_aoi_delta`/`apply_states`; interpolate's two unit tests (step 5)
  call `apply_states`; `apply_states` calls `prediction::reconcile_own` and touches
  `NetBuffer` (`interpolate::NetBuffer`, `pub(super)` since step 5) and
  `state.playback`/`state.latest_state_tick`/`state.entities` (private fields of the
  parent-module struct — accessible from a child module).
- **Ideal:** `client/vordar-client/src/net/apply.rs` (`mod apply;` in mod.rs): module
  header stating the two-lane snapshot contract (AoiDelta = reliable stream, identity
  once; Snapshot = unreliable datagram, tick-guarded). `pub(super)` on
  `apply_aoi_delta`, `apply_states`, `handle_entity_died`. The stale-snapshot test
  moves into `#[cfg(test)] mod tests` in `apply.rs` (with inline `PendingIntent`
  literals from step 6, local `const DT`, and
  `use crate::net::{NetClientState, prediction::PendingIntent, …}`).
- **Gap:** the receive dispatch and the appliers it dispatches to are one undivided
  600-line region; the applier tests sit 1,000 lines away.
- **Suggestion:** move verbatim. Update: mod.rs's `NetReceiveSystem` arms call
  `apply::apply_aoi_delta(...)` etc. (or `use apply::…`); `net/bench.rs` is not yet
  extracted — the in-mod.rs bench module's wrappers change their bodies to
  `super::apply::apply_aoi_delta(...)` (public signatures untouched);
  `net/interpolate.rs`'s two tests update their import to
  `use crate::net::apply::apply_states;` (the single planned double-touch).
- **Path:** (1) capture the nextest baseline counts. (2) Extract as above.
  (3) Behavioral proof: `apply_states_drops_a_stale_snapshot_tick` (drives the real
  tick guard + ack trim + buffer write through the moved fns) and interpolate's
  `fixed_delay_playback_rides_through_jittered_arrivals` (feeds the moved
  `apply_states` from a different module — proves the `pub(super)` seam).
  `cargo nextest run --workspace` green at identical counts;
  `cargo check -p vordar-benches --benches` compiles.

### 8. Extract `net/lifecycle.rs`

- **Evidence:** in `net/mod.rs` the connection-lifecycle slice: `RECONNECT_INITIAL_BACKOFF`,
  `RECONNECT_MAX_BACKOFF`, `RECONNECT_ATTEMPT_GRACE`, `reconnect_backoff` (today's
  net.rs L62-79), `Reconnect` (L86-89), `NetReceiveSystem` + its `System` impl — the
  event drain + message dispatch loop (L342-440) — `teardown_replicated_world`
  (L446-482), `handle_redirect` (L487-513), `handle_disconnected` (L520-533),
  `maybe_reconnect` (L539-566). Cross-references: the plugin's `build` constructs
  `Reconnect { attempt: 1, retry_at: … }` and calls `reconnect_backoff(1)` on initial
  connect failure (L117) and registers `NetReceiveSystem` (L139, and
  `NetSendInputSystem` is ordered `after::<NetReceiveSystem>()`); `NetClientState`
  holds `reconnect: Option<Reconnect>` and `reconnect_attempt()` reads `r.attempt`
  (L269-271); the dispatch loop calls `apply::…` (step 7), `crate::telegraph::spawn_telegraph`
  (step 3), writes `crate::world_time::WorldTime` (step 2); the kicked e2e test drives
  `NetReceiveSystem` directly.
- **Ideal:** `client/vordar-client/src/net/lifecycle.rs` (`mod lifecycle;` in mod.rs):
  module header stating the lifecycle contract (drain events every Input tick; a due
  redial fires on its own clock; teardown is shared by redirect and disconnect;
  LoginDenied stops redials). Visibilities: `pub(super)` on `NetReceiveSystem`,
  `Reconnect` (struct + both fields — mod.rs constructs it), `reconnect_backoff`;
  the rest (`teardown_replicated_world`, `handle_redirect`, `handle_disconnected`,
  `maybe_reconnect`, the three consts) private to `lifecycle.rs`.
- **Gap:** after steps 5-7 this is the last multi-concern region in mod.rs; extracting
  it leaves mod.rs as pure plugin + state, the finding's target shape.
- **Suggestion:** move verbatim. mod.rs adds `use lifecycle::{NetReceiveSystem, Reconnect};`
  (state field type, plugin build, and the `SystemOrder::after::<NetReceiveSystem>()`
  token) plus `use lifecycle::reconnect_backoff;` for the build's error arm. The e2e
  tests still in mod.rs's `#[cfg(test)] mod tests` reach `NetReceiveSystem` through
  `use super::*` (which now picks up the `use`d name). No signature changes anywhere.
- **Path:** (1) capture the nextest baseline counts. (2) Extract as above.
  (3) Behavioral proof: `kicked_connection_reconnects_and_relogs_in` — a real server
  kick driving the moved `NetReceiveSystem` → `handle_disconnected` →
  `teardown_replicated_world` → `maybe_reconnect` → relogin chain end-to-end; any slip
  in the moved dispatch or backoff wiring fails it.
  `cargo nextest run --workspace` green at identical counts;
  `cargo check -p vordar-benches --benches` compiles.

### 9. Extract `net/bench.rs` and `net/e2e.rs`; final `net/mod.rs` shape

- **Evidence:** in `net/mod.rs` two riders remain: the feature-gated bench seam
  (today's net.rs L1354-1433: `state_for_bench`, `map_entity`, `set_prefab_table`,
  `push_pending`, `apply_aoi_delta`/`apply_states`/`reconcile_own` wrappers —
  `benchmarks/benches/client_netcode.rs:19` imports it as
  `use vordar_client::net::bench as seam;` and must compile UNCHANGED), and the
  remaining `#[cfg(test)] mod tests` holding the three real-server e2e tests —
  `kicked_connection_reconnects_and_relogs_in` (L1874), `onslaught_dash_replay_never_snaps_at_150ms_rtt`
  (L2009), `remote_render_smoothness_under_loss_probe` (L2267, `#[ignore]`) — plus
  their helpers `name_token` (L1451-1457), `pct` (L2191-2193), `pace_tick`
  (L2203-2211), `mover_tick` (L2229-2249), local `DT`, and the 10-line prefab-registry
  block duplicated verbatim inside the onslaught test (L2034-2044) and the probe
  (L2326-2336).
- **Ideal:** `client/vordar-client/src/net/bench.rs`, declared in mod.rs as
  `#[cfg(feature = "bench-internals")] #[doc(hidden)] pub mod bench;` — contents moved
  verbatim, wrapper bodies calling `super::apply::…` / `super::prediction::reconcile_own`,
  `state_for_bench` still calling `NetClientState::new` (all reachable: `bench` is a
  descendant of `net`); every `pub fn` signature byte-identical.
  `client/vordar-client/src/net/e2e.rs`, declared `#[cfg(test)] mod e2e;` — the three
  tests + `name_token`/`pct`/`pace_tick`/`mover_tick`/`DT`, with the duplicated
  registry block factored into one local
  `fn insert_game_prefabs(resources: &mut Resources)` (identical registration list:
  core components + Player, Enemy, ContactDamage, CombatStats, Class, Race, VfxTrail +
  `PrefabLibrary::load_dir("content/prefabs")`) called by both the onslaught test and
  the probe. `mover_tick`'s doc comment reference "(net.rs:1203-1212)" becomes a
  reference to `NetSendInputSystem` by name (no line numbers). After this step
  `net/mod.rs` contains ONLY: the module doc header (the Phase-2 model comment),
  module declarations, `NetClientPlugin` + `build`, `NetClientState` (+ `new`,
  `own_entity` method, `server_now_micros`, `send_cast_intent`, `predicting`), and the
  `own_entity`/`reconnect_attempt` free fns — roughly 300 lines.
- **Gap:** a 1,039-line test module and an 80-line bench seam still ride inside the
  plugin/state file; the registry block exists twice.
- **Suggestion:** move verbatim except the two prescribed edits (registry dedupe,
  `mover_tick` comment reference). e2e imports:
  `use super::*;` won't exist as before — write explicit
  `use crate::net::{lifecycle::NetReceiveSystem, prediction::{NetCorrectionSystem, NetSendInputSystem, start_predicted_leap, MOVE_RING_LEN, SNAP_DISTANCE}, NetClientState, own_entity, reconnect_attempt};`
  plus the external imports the tests already use (`vordar_server::build_server_app`,
  `engine_net::Impairment`, protocol types, `crate::world_time::WorldTime`, …). All
  `pub(super)` items are visible (e2e is a descendant of `net`). Verify the final
  mod.rs has no leftover `use`s (zero-warning gate).
- **Path:** (1) capture the nextest baseline counts. (2) Extract both modules; tidy
  mod.rs. (3) Behavioral proof: run the full gate suite —
  `cargo nextest run --workspace` green at identical counts (the two runnable e2e
  tests execute against real servers; the probe stays ignored/compiled);
  `cargo check -p vordar-benches --benches` proves the bench seam relocation
  (client_netcode.rs untouched); `cargo build -p vordar-client` with zero new
  warnings. (4) Structure check (grep):
  `grep -c "impl System" client/vordar-client/src/net/mod.rs` must be 0, and
  `client/vordar-client/src/net.rs` must not exist. If the ignored probe is cheap to
  spot-check locally, optionally `cargo test -p vordar-client --release extrapolation -- --nocapture`
  style is NOT required — compile coverage suffices; do not run the probe.

### 10. Mark the networking rework-7 plan stale and close this rework's queue entry (docs-only)

- **Evidence:** `docs/reviews/networking/plan-networking-rework-7-2026-07-14.md` cites
  net.rs internals by line/seam (e.g. L199, L220-251: `NetClientState` mirrors, system
  registration relative to `NetCorrectionSystem` in `NetClientPlugin::build`) — all
  stale once the family split lands, as this rework's source finding's Path note (4)
  states. The cross-type queue note listing
  "finding 1 → rework 1 → rework 2 → …" exists in both
  `docs/reviews/hygiene/reworks-hygiene-2026-07-14.md` (L17-28) and
  `docs/reviews/hygiene/audit-hygiene-2026-07-14.md` (L19-30), mirrored verbatim.
- **Ideal:** rework 7 of the networking queue cannot be executed against a stale plan
  by accident, and the hygiene queue shows rework 1 done.
- **Gap:** nothing records the invalidation; the queue note still lists rework 1 as
  pending.
- **Suggestion:** (a) add one italic line directly under the title of
  `docs/reviews/networking/plan-networking-rework-7-2026-07-14.md`:
  "*Stale as of 2026-07-14: hygiene rework 1 decomposed client net.rs into the
  `net/` module family — every net.rs citation below predates it. Re-run /plan-rework
  for this finding before executing.*"; (b) strike `rework 1` (`~~rework 1~~`) in the
  cross-type queue note in BOTH hygiene files so the mirror stays verbatim-identical.
- **Path:** (1) make the two edits above; (2) verification: the queue notes in the two
  hygiene files remain byte-identical to each other (diff the two blockquotes), and
  the rework-7 plan's stale banner names the re-plan requirement. No code, no test —
  docs-only.
