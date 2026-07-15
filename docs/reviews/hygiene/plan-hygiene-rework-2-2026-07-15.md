# Plan: receive.rs — promote the five seams inside NetReceiveSystem::run — 2026-07-15

Source: `docs/reviews/hygiene/reworks-hygiene-2026-07-15.md` finding 2.

## Ideal end state

`server/vordar-server/src/net/receive.rs` keeps exactly one scheduled system, but
`NetReceiveSystem::run` (today ~465 lines, L53-519) shrinks to a short pipeline a
reader scans in one screen: preamble (ClassLibrary clone, WorldTime publish, event
poll) → an event match whose arms delegate to `handle_disconnect`, `handle_login`,
`queue_move_intents` (already extracted), and `dispatch_cast` → the pending-bolt
spawn loop → `complete_db_load` per DB grant → `respawn_dead` → `drain_intents`.
Each extracted function is a private free function in the same file with the borrow
scope it actually needs, its seam's block comment promoted to its rustdoc. The
duplicated 5-line "persist Transform/Health/cooldowns" block (4 identical sites
across the net family) exists once, as `save_character` in `net/mod.rs` beside
`cooldown_remainders`. Behavior is bit-identical: same event ordering, same sends,
same log lines, `install()` and the `bench-internals` seam untouched, every step
green at unchanged `cargo nextest` counts.

## Design decisions

- **Same file, no `receive/` directory family.** The five seams are one system's
  tick pipeline with one consumer each; named private functions make them findable
  (the file's own `validate_intent`/`queue_move_intents` set the precedent). A
  `receive/login.rs` would collide conceptually with the existing `net/login.rs`
  (the `LoginFailures` ledger), and a dir family for six private functions is
  module ceremony with no navigability gain. The file lands around 730 lines —
  acceptable; do NOT split to chase a line count.
- **Free functions, not methods.** `NetReceiveSystem` is a stateless unit struct;
  methods would imply state that isn't there. Free functions match the existing
  extracted seams.
- **Signatures follow the one-borrow-at-a-time constraint.** `Resources::get_mut`
  is `fn get_mut<T>(&mut self) -> Option<&mut T>` (`smirk/engine-core/src/traits.rs:30`)
  — only one resource borrow can be live at a time, which is why today's body
  constantly re-fetches `NetServerState`. Consequences, seam by seam:
  - Seams that must touch a *second* resource mid-body take
    `(world: &mut World, resources: &mut Resources, …)` and re-acquire state
    internally exactly where today's code does: `handle_login` (DespawnQueue push
    mid-takeover, then re-acquire at today's L162), `handle_disconnect`
    (DespawnQueue), `complete_db_load` (PrefabLibrary read + `spawn_prefab`'s
    `SpawnContext`), `respawn_dead` (`spawn_prefab`), `drain_intents`
    (NetServerState then EventBus).
  - `dispatch_cast` never needs a second resource, so it takes
    `state: &mut NetServerState` directly (plus `world`, the cloned
    `&ClassLibrary`, and `&mut Vec<PendingBolt>`). It re-fetches `pc` from
    `state.conns` internally — passing `&mut PlayerConn` alongside
    `&mut NetServerState` (or alongside `&state.conns` for `aoi_conns`) is
    un-callable; today it only compiles because NLL ends the `pc` borrow before
    each `aoi_conns` call inside one function body, and the verbatim move
    preserves exactly that.
  - `save_character` needs only shared borrows (`DbHandle::save`, `peer`-free
    reads are all `&self` — `server/vordar-server/src/db.rs:208`), so it takes
    `(world: &World, state: &NetServerState, pc: &PlayerConn)` and is callable
    while iterating `state.conns` (autosave/shutdown) and on owned removed
    `PlayerConn`s (receive).
- **One by-value `match msg` replaces the `&msg` pre-match + shared guard + dead
  arm.** Today Login is peeled off with `if let … = &msg` before the shared
  `rtt`/`PlayerConn` guard, and the later match carries a dead
  `ClientMsg::Login { .. } => {}` arm (receive.rs:103, 198-199, 341). After the
  restructure each arm owns its 2-line prelude; the drop-unknown-conn guard is
  preserved per-arm (MoveIntents inline, CastIntent inside `dispatch_cast`).
  Behavior identical — `Resources::get_mut` is a side-effect-free map lookup, so
  per-arm re-fetching changes nothing observable. The two comments this orphans
  (L101-102 "handle it before the guard below", L340 "Handled before the
  PlayerConn guard above.") are deleted — our restructure makes them false.
- **`save_character` is promoted to `net/mod.rs` and converts all four identical
  sites** — receive.rs Disconnected arm (L86-90), receive.rs takeover block
  (L153-157), `autosave.rs:35-41`, `shutdown.rs:37-43`. The finding's letter is
  receive.rs-only, but the byte-identical block existing 4× across the family is
  exactly the seam-as-copy-paste disease the audit targets, and `mod.rs` is the
  established home for cross-sibling helpers (`cooldown_remainders`, `aoi_conns`).
  `transfer.rs` is deliberately NOT converted: its save writes a *different*
  record (target zone, portal arrival position, health with a 100 fallback) — it
  is not the same seam.
- **`PendingBolt` struct replaces the 8-field tuple**
  (`Vec<(String, Vec3, Vec3, f32, i32, DamageType, f32, Entity)>`, receive.rs:70)
  so `dispatch_cast`'s signature and the spawn loop are readable. Pure data-shape
  rename, zero behavior.
- **The three cast arms stay verbatim inside `dispatch_cast` — no Scheduled/Leap
  dedup.** The finding calls the arms "near-identical" as evidence of length, but
  its Suggestion is extraction-only, zero behavior change; folding the shared
  mechanic-scheduling block would be a logic refactor on behavior-critical combat
  code. If wanted, that is a future fixes-scale finding, not this rework.
- **Comments move verbatim; seam block comments become fn rustdoc.** The queue
  orders fixes finding 4 (the file's one comment straggler, receive.rs:470-473)
  before this rework, so comments should already be clean; if the straggler is
  still present when a step runs, move it verbatim anyway — this rework never
  edits comment text except the two deletions the restructure forces (above).
- **Test strategy: pinned counts, existing behavioral gates, no new tests.**
  Every seam is already driven end-to-end through real QUIC servers:
  e2e_security (login rate limit, token gates, reject counter), e2e_persistence
  (disconnect save, takeover, cooldown remainders, load completion),
  zones (redirect, world clock), e2e_combat (Scheduled + Leap cast arms),
  e2e (respawn, movement drain), e2e_wireformat (prefab table, MoveIntents
  redundancy), loss (intent drain under impairment). Adding unit tests would
  break the "unchanged counts" gate that proves moves are moves. **Known
  coverage gap, recorded:** the Projectile cast arm has no e2e — the server
  spawns the `ravager` prefab and `ravager.ron` has only Scheduled/Leap
  abilities (`bolt` lives in `human.ron`). That arm moves verbatim under
  compile + diff-review only; workers must not invent a test for it.
- **Untouched by constraint:** `install()`/`SystemOrder` registrations
  (`net/mod.rs:110-125`), `net/bench.rs`, `validate_intent`,
  `queue_move_intents`, and the three in-file unit tests (which keep passing at
  the same names).

Line numbers in the steps below refer to the files as of this plan's date; steps
2-5 shift them — locate by the quoted anchor text, not by number.

## Findings (execution order)

### 1. `save_character` in net/mod.rs; extract `handle_disconnect`; convert the four save sites

- **Evidence:** the identical persist block —
  `if let (Ok(tr), Ok(hp)) = (world.get::<&Transform>(pc.entity), world.get::<&Health>(pc.entity)) { … cooldown_remainders(&pc.cooldown_ready, state.server.now_micros()) … state.db.save(pc.name.clone(), CharacterRecord { zone: <this zone>, pos: tr.position, health: hp.current, cooldowns }); }`
  — exists at four sites: `server/vordar-server/src/net/receive.rs:86-90`
  (Disconnected arm), `receive.rs:153-157` (session-takeover block inside the
  Login arm), `server/vordar-server/src/net/autosave.rs:35-41`,
  `server/vordar-server/src/net/shutdown.rs:37-43`. The Disconnected arm
  (receive.rs:80-94) is itself a seam inline in `run`'s event match.
  `net/mod.rs` already hosts the family's shared helpers `cooldown_remainders`
  (mod.rs:263) and `aoi_conns` (mod.rs:284). `DbHandle::save` is `&self`
  (db.rs:208); `NetServer::now_micros` is `&self` (engine-net server.rs:248).
  `transfer.rs:61-69` saves a *different* record (target zone/pos, health
  fallback 100) and is out of scope.
- **Ideal:** one private fn in `net/mod.rs`, placed beside `cooldown_remainders`:
  ```rust
  /// Persist a connected player's live state (position, health, cooldown
  /// remainders) under this zone's name. A player whose entity is already
  /// gone from the world has nothing to save — silently skipped.
  fn save_character(world: &World, state: &NetServerState, pc: &PlayerConn) {
      if let (Ok(tr), Ok(hp)) = (world.get::<&Transform>(pc.entity), world.get::<&Health>(pc.entity)) {
          let cooldowns = cooldown_remainders(&pc.cooldown_ready, state.server.now_micros());
          state.db.save(
              pc.name.clone(),
              CharacterRecord { zone: state.zone.name.clone(), pos: tr.position, health: hp.current, cooldowns },
          );
      }
  }
  ```
  All four sites call it. The Disconnected arm becomes a one-line delegation to a
  new private free fn in receive.rs:
  ```rust
  fn handle_disconnect(world: &mut World, resources: &mut Resources, conn: ConnId) {
      let state = resources.get_mut::<NetServerState>().unwrap();
      state.loading.remove(&conn);
      if let Some(pc) = state.conns.remove(&conn) {
          // Persist before queuing the despawn — DespawnFlush
          // runs later in the frame, the entity is still alive.
          save_character(world, state, &pc);
          resources.get_mut::<DespawnQueue>().unwrap().push(pc.entity, None);
          log::info!("conn {conn}: disconnected, despawning {:?}", pc.entity);
      }
  }
  ```
  (`pc` is owned after `remove`; the `state` borrow ends at the `save_character`
  call, so the DespawnQueue fetch compiles — same NLL shape as today.)
- **Gap:** the persist block is copy-pasted 4×, and the disconnect seam is inline
  in the 465-line `run`.
- **Suggestion:** exact edits:
  - `net/mod.rs`: add `save_character` as above (private — descendants see it);
    extend imports to `use crate::db::{CharacterRecord, DbHandle, DbWorker};`
    and `use engine_core::components::{Health, Transform};`.
  - `receive.rs`: add `handle_disconnect` (place directly after the `impl System`
    block); the `ServerEvent::Disconnected(conn)` arm body becomes
    `handle_disconnect(world, resources, conn);`. In the takeover block, replace
    lines 153-157 (the `if let (Ok(tr), Ok(hp)) …` save, including the
    `let zone` / `let cooldowns` lines) with `save_character(world, state, &pc);`
    keeping the preceding "Same save-then-despawn as a real disconnect…" comment.
    Update `use super::…` to add `save_character`; `cooldown_remainders` becomes
    unused in receive.rs — remove it from the import (the zero-warning gate
    arbitrates).
  - `autosave.rs`: loop body becomes
    `if !autosave_due(conn, tick) { continue; } save_character(world, state, pc);`
    — imports shrink to
    `use engine_core::traits::Resources; use engine_core::World; use engine_net::ConnId;`
    plus `use super::{save_character, NetServerState};` and the `System` import
    (drop `Health`/`Transform`/`CharacterRecord`/`cooldown_remainders`; keep
    whatever the compiler still demands).
  - `shutdown.rs`: the `for pc in state.conns.values()` body becomes the existing
    "Players still in `state.loading` have no entity yet — nothing to save."
    comment followed by `save_character(world, state, pc);` — same import shrink.
    (Shared reborrows: iterating `&state.conns` while passing `&*state` is two
    shared borrows from one `&mut` — compiles.)
  - Do not touch `transfer.rs`, `install()`, or any comment text beyond the lines
    the replaced code carried.
- **Path:** (1) Baseline: run `cargo nextest run -p vordar-server` and record the
  final counts line. (2) Make the edits above. (3) Verify:
  `cargo nextest run -p vordar-server` green at identical counts — behavioral
  gates that drive the moved save through real servers:
  `e2e_persistence::reconnect_restores_position` (disconnect-save → relogin
  restores position), `e2e_persistence::relog_restores_exact_cooldown_remainder`
  (cooldown remainders through the helper), `e2e_persistence::login_takeover`
  (takeover-site save), `shutdown::shutdown_flag_saves_all_players_and_returns_from_run_headless`
  and `shutdown::shared_flag_drains_both_zones_and_worker_drop_returns`
  (shutdown site), plus the autosave unit test.
  `cargo check -p vordar-server --all-targets` zero warnings (catches orphaned
  imports); `cargo check -p vordar-benches --benches` compiles (bench.rs does
  `use super::*` on mod.rs — prove the new imports don't collide).

### 2. Restructure the Message arm into one by-value match; extract `handle_login`

- **Evidence:** `server/vordar-server/src/net/receive.rs` Message arm: L96
  fetches state, L97-100 decodes, L103-197 peels Login off with
  `if let ClientMsg::Login { name, token } = &msg { …; continue; }` (rate limit →
  name validation → duplicate-login guard → token-gated takeover → token-gated
  stale-load eviction → `loading.insert` + `db.login`; every internal exit is
  `continue`), L198-199 the shared prelude
  `let rtt = state.server.rtt_micros(conn).unwrap_or(0); let Some(pc) = state.conns.get_mut(&conn) else { continue };`,
  L201-342 the match with MoveIntents, CastIntent, and a dead
  `ClientMsg::Login { .. } => {}` arm (L340-341). After step 1 the takeover block
  already calls `save_character`. Wire shape: `ClientMsg::Login { name: String,
  token: AccountToken }` (game/vordar-protocol/src/lib.rs:54).
- **Ideal:** the Message arm reads:
  ```rust
  ServerEvent::Message { conn, data, recv_micros } => {
      let Some(msg) = decode::<ClientMsg>(&data) else {
          log::warn!("conn {conn}: undecodable message ({} bytes)", data.len());
          continue;
      };
      match msg {
          ClientMsg::Login { name, token } => handle_login(world, resources, conn, name, token),
          ClientMsg::MoveIntents { intents } => {
              let state = resources.get_mut::<NetServerState>().unwrap();
              let rtt = state.server.rtt_micros(conn).unwrap_or(0);
              let Some(pc) = state.conns.get_mut(&conn) else { continue };
              queue_move_intents(pc, &intents, recv_micros, rtt, &state.server.metrics());
          }
          ClientMsg::CastIntent { seq, t_server_micros, skill, target } => {
              // <today's L206-338 cast body, verbatim, prefixed by its own
              //  state/rtt/pc-guard prelude — extracted in step 3>
          }
      }
  }
  ```
  and `handle_login` is a private free fn in receive.rs:
  `fn handle_login(world: &mut World, resources: &mut Resources, conn: ConnId, name: String, token: AccountToken)`
  whose body is today's L105-196 verbatim (comments included) with the mechanical
  adjustments listed below. Its rustdoc absorbs the constraint from the old
  in-arm comment: login arrives from a connection that has no `PlayerConn` yet,
  and grant/spawn happens only when the DB load completes.
- **Gap:** login — rate limiting, session takeover, and eviction security logic —
  is findable only by scrolling a 465-line function; the Login peel-off and dead
  match arm are control-flow noise.
- **Suggestion:** mechanical adjustments inside the moved body (everything else
  byte-identical):
  - delete `let token = *token;` (L104) — `token` is owned by value;
  - every `continue` becomes `return`;
  - `pc.name == *name` (L140) → `pc.name == name`;
  - `.find(|(_, (n, _))| n == name)` (L169) → `n == &name`;
  - `state.loading.insert(conn, (name.clone(), token));` (L182) stays;
    the final `state.db.login(conn, name.clone(), token, defaults);` (L195)
    passes `name` by move (last use);
  - the mid-body `let state = resources.get_mut::<NetServerState>().unwrap();`
    re-acquire (L162, after the takeover's DespawnQueue push) stays verbatim —
    it is what makes the borrow structure legal;
  - the fn's first line is `let state = resources.get_mut::<NetServerState>().unwrap();`.
  In `run`: replace the `if let … Login` peel-off + guard + match with the
  by-value match above; the CastIntent arm keeps today's body verbatim but gains
  the 3-line prelude (state fetch, rtt, pc guard — with `continue`, since it is
  still inside the event loop) and binds `skill` (do NOT rename the wire field;
  the body's `skill_id` uses become `skill`, or bind `skill: skill_id` in the
  pattern to keep the body untouched — prefer `skill: skill_id`, zero body
  edits). Delete the two orphaned comments (L101-102, L340) and the dead Login
  arm. Add `AccountToken` to the `vordar_protocol` import in receive.rs. Place
  `handle_login` after `handle_disconnect`.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Make the edits. (3) Verify: `cargo nextest run -p vordar-server` green at
  identical counts — behavioral gates through real servers:
  `e2e_security::login_failures_are_rate_limited` (rate-limit + failure
  recording), `e2e_security::wrong_token_cannot_kick_or_impersonate` (both
  token gates), `e2e_security::invalid_intent_increments_reject_counter`
  (CastIntent arm's new prelude + reject path),
  `e2e_persistence::login_takeover` (takeover save/disconnect/despawn order),
  `zones::login_routes_to_saved_zone` (login → load kickoff), and
  `e2e::login_move_sync_roundtrip` (MoveIntents arm's new prelude).
  `cargo check -p vordar-server --all-targets` zero warnings.

### 3. `PendingBolt` struct; extract `dispatch_cast`

- **Evidence:** `server/vordar-server/src/net/receive.rs`: the pending-bolt
  buffer `let mut pending_bolts: Vec<(String, Vec3, Vec3, f32, i32, DamageType, f32, Entity)> = Vec::new();`
  (L68-70, with its "spawned after the event loop releases the NetServerState
  borrow" comment), the CastIntent body (post-step-2 shape; originally
  L206-338): validate_intent + `record_reject`, `last_seq`/`last_t` update,
  ClassId lookup → `class_library.get`, cooldown gate, caster position, target
  finite check, then three arms — `AbilityEffect::Scheduled` (range gate,
  cooldown insert, `next_mechanic_id`, `world.spawn` Mechanic,
  `MechanicScheduled` fan-out via `aoi_conns`), `AbilityEffect::Projectile`
  (direction, cooldown insert, push onto `pending_bolts`),
  `AbilityEffect::Leap` (Scheduled plus `LeapImpulse` insert) — and the spawn
  loop `for (prefab, origin, dir, speed, damage, damage_type, ttl, caster) in pending_bolts { spawn_projectile(…, false); }`
  (L347-350). Borrow fact: `pc` (`&mut` into `state.conns`) is last used before
  each `aoi_conns(&state.conns, …)` call (Scheduled: cooldown insert at L241 vs
  aoi at L266; Leap: L302 vs L333), which is the only reason the body borrows —
  the extracted fn must preserve that statement order. Wire shape:
  `CastIntent { seq: u32, t_server_micros: u64, skill: String, target: Vec2 }`
  (vordar-protocol lib.rs:48). No e2e drives the Projectile arm (server class is
  `ravager`; see Design decisions) — verbatim move, compile-only coverage there.
- **Ideal:** in receive.rs:
  ```rust
  /// Projectile casts accepted this tick — spawned after the event loop
  /// releases the NetServerState borrow (spawn_projectile needs resources).
  struct PendingBolt {
      prefab: String,
      origin: Vec3,
      dir: Vec3,
      speed: f32,
      damage: i32,
      damage_type: DamageType,
      ttl_secs: f32,
      caster: Entity,
  }

  fn dispatch_cast(
      world: &mut World,
      state: &mut NetServerState,
      class_library: &ClassLibrary,
      pending_bolts: &mut Vec<PendingBolt>,
      conn: ConnId,
      seq: u32,
      t: u64,
      recv_micros: u64,
      skill_id: String,
      target: Vec2,
  )
  ```
  with rustdoc naming the contract (validation → class/cooldown gates → one of
  three effects; bolts deferred to the caller's spawn loop). Body = prelude
  (`rtt` fetch, `pc` guard with `return`) + today's cast body verbatim, every
  `continue` → `return`, the tuple push becoming a `PendingBolt { … }` literal.
  `run`'s CastIntent arm becomes:
  ```rust
  ClientMsg::CastIntent { seq, t_server_micros, skill, target } => {
      let state = resources.get_mut::<NetServerState>().unwrap();
      dispatch_cast(world, state, &class_library, &mut pending_bolts, conn, seq, t_server_micros, recv_micros, skill, target);
  }
  ```
  and the spawn loop destructures the struct:
  `for b in pending_bolts { spawn_projectile(world, resources, &b.prefab, b.origin, b.dir, b.speed, b.damage, b.damage_type, b.ttl_secs, b.caster, false); }`.
- **Gap:** the largest seam (~130 lines, three effect arms) still sits inline;
  the bolt buffer is an anonymous 8-tuple.
- **Suggestion:** the moved body keeps its exact statement order (the NLL note in
  Evidence), all comments verbatim including the "No range gate" and "Schedule in
  ABSOLUTE server time" blocks, and the `let target = Vec3::new(target.x, 0.0, target.y);`
  conversion stays inside `dispatch_cast`. Inside the fn the pattern-bound wire
  fields arrive as plain params, so today's in-pattern renames
  (`t_server_micros: t`, `skill: skill_id`) become param names `t` and
  `skill_id` — body untouched. Place `dispatch_cast` after `handle_login`.
  `pending_bolts`' decl in `run` becomes `let mut pending_bolts: Vec<PendingBolt> = Vec::new();`
  (its comment moved onto the struct). No other file changes.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Make the edits. (3) Verify: `cargo nextest run -p vordar-server` green at
  identical counts — behavioral gates: `e2e_combat::scheduled_aoe` (Scheduled
  arm end-to-end: range gate, cooldown, broadcast, caster exclusion, backdated
  reject), `e2e_combat::rend_kills_camped_enemy` (Scheduled under load),
  `e2e_combat::ravager_onslaught_dashes_and_resolves` (Leap arm: Mechanic +
  LeapImpulse + broadcast), `e2e::far_bot_never_sees_out_of_aoi_mechanic`
  (aoi_conns fan-out from inside the extracted fn). The Projectile arm has no
  e2e (recorded in Design decisions): verify it by diffing the moved arm against
  git HEAD — it must be byte-identical apart from `continue`→`return` and the
  `PendingBolt` literal; do not write a new test.
  `cargo check -p vordar-server --all-targets` zero warnings.

### 4. Extract `complete_db_load`

- **Evidence:** `server/vordar-server/src/net/receive.rs` L352-468 (anchor: the
  comment "Finished character loads → spawn + Welcome (or a denial)."): the
  `db.poll()` fetch, then per `DbLoaded { conn, name, outcome }`: `loading`
  removal (token capture; `continue` if the conn dropped meanwhile), BadToken
  denial (+ failure recording), zone-ownership routing (Redirect or disconnect,
  in its own block so the state borrow ends), lazy one-time prefab-table build
  (reads `PrefabLibrary` while no state borrow is live, asserts the u16 wire
  bound), `spawn_prefab(PLAYER_PREFAB, record.pos, &mut SpawnContext { world, resources })`,
  table install, health override, cooldown-remainder restore, `PlayerConn`
  insert, and the Welcome → PrefabTable → WorldClock send sequence. Every
  early exit is a `continue` of the `for DbLoaded … in loaded` loop.
- **Ideal:** a private free fn in receive.rs:
  `fn complete_db_load(world: &mut World, resources: &mut Resources, loaded: DbLoaded)`
  — body starts `let DbLoaded { conn, name, outcome } = loaded;` then today's
  loop body verbatim with `continue` → `return`; rustdoc absorbs the seam
  comment ("The connection enters the game only now; anything it sent earlier
  was dropped by the PlayerConn guard.") plus the grant sequence
  (redirect-or-spawn, Welcome/PrefabTable/WorldClock on the ordered stream).
  `run` keeps the pipeline visible:
  ```rust
  let loaded = resources.get_mut::<NetServerState>().unwrap().db.poll();
  for l in loaded {
      complete_db_load(world, resources, l);
  }
  ```
- **Gap:** grant completion — the only place a connection enters the game — is
  buried after the event loop instead of being a named stage.
- **Suggestion:** pure move; the interleaved state re-fetches (before BadToken
  send, around the routing block, the `new_prefab_table` read, after
  `spawn_prefab`) stay exactly where they are — they encode the
  one-resource-borrow-at-a-time constraint. `DbLoaded` is already imported.
  Comments verbatim, including the prefab-table lazy-build block and the
  "NOT resent on the respawn re-Welcome below" note (whose "below" now means
  `respawn_dead` — still true, both live in this file). Place after
  `dispatch_cast`.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Make the edits. (3) Verify: `cargo nextest run -p vordar-server` green at
  identical counts — behavioral gates: `e2e_persistence::reconnect_restores_position`
  and `e2e_persistence::restart_durability` (grant → spawn with DB overrides),
  `e2e_persistence::relog_restores_exact_cooldown_remainder` (remainder → ready_at
  restore), `zones::login_routes_to_saved_zone` (Redirect branch),
  `e2e_security::wrong_token_cannot_kick_or_impersonate` (BadToken branch),
  `e2e_wireformat::prefab_table_binds_u16_refs` (lazy table build + send order),
  `zones::world_clock_shared_across_zones` (WorldClock at join).
  `cargo check -p vordar-server --all-targets` zero warnings.

### 5. Extract `respawn_dead` and `drain_intents`; final `run` shape

- **Evidence:** `server/vordar-server/src/net/receive.rs` L470-495 (anchor: "A
  connection must always own a live player"): the dead-conn scan
  (`!world.contains(pc.entity)` under a shared state borrow, collected to end
  the borrow), then per dead conn `spawn_prefab` + rebind (`pc.entity`,
  `pc.queue.clear()`) + re-Welcome. L497-518 (anchor: "Apply exactly one queued
  intent per connection per tick"): pop one intent per conn, advance
  `applied_seq`, push `history` capped at `HISTORY_CAP`, collect
  `(Entity, Vec2)`, then emit `MoveIntent` on the `EventBus`. Both blocks are
  the tail of `run`.
- **Ideal:** two private free fns in receive.rs, placed after `complete_db_load`:
  `fn respawn_dead(world: &mut World, resources: &mut Resources)` and
  `fn drain_intents(resources: &mut Resources)` — bodies verbatim (the intent
  block never touches `world`), each block's leading comment promoted to the
  fn's rustdoc (the respawn comment may already be reworded by fixes finding 4 —
  move whatever text is there, verbatim). `run` ends:
  ```rust
  respawn_dead(world, resources);
  drain_intents(resources);
  ```
  After this step `run` is ~60 lines and reads as the tick pipeline: ClassLibrary
  clone + WorldTime publish + poll → event match (Connected log /
  `handle_disconnect` / decode + three-arm message match) → bolt spawn loop →
  db-load loop → `respawn_dead` → `drain_intents`.
- **Gap:** the last two inline seams; until they move, `run` still ends in 50
  lines of block-comment-delimited logic.
- **Suggestion:** pure moves; `respawn_dead` keeps the scan-then-act split and
  the `let Some(pc) = state.conns.get_mut(&conn) else { continue };` guard
  (the conn may disconnect between scan and act — that `continue` stays a
  `continue`, it is inside the fn's own loop). `drain_intents`' final shape:
  state borrow → collect, then `EventBus` borrow → emit, exactly as today.
  No import changes expected; the zero-warning gate arbitrates.
- **Path:** (1) Baseline: record `cargo nextest run -p vordar-server` counts.
  (2) Make the edits. (3) Verify: `cargo nextest run -p vordar-server` green at
  identical counts — behavioral gates: `e2e::respawn_after_death` (kill →
  respawn at ring point → re-Welcome rebind), `e2e::login_move_sync_roundtrip`
  and `e2e::simulated_latency` (one-intent-per-tick drain = bit-exact
  prediction), `e2e_wireformat::move_intents_redundancy_survives_upstream_loss`
  and `loss::loss_probe_upstream_intent_lag` (drain under loss),
  `e2e_combat::rend_kills_camped_enemy` (history push feeding mechanic rewind).
  (4) Full-suite gates, this being the last code step:
  `cargo nextest run --workspace` green at the pre-rework workspace counts;
  `cargo check -p vordar-server --all-targets` zero warnings;
  `cargo check -p vordar-benches --benches` compiles. (5) Shape check: read
  `NetReceiveSystem::run` — it must contain no `db.poll`-result handling, no
  spawn logic, and no queue-pop logic inline; if any seam still exceeds a
  delegation call, report it rather than improvising further extraction.

### 6. Close this rework's queue entry (docs-only)

- **Evidence:** the cross-type queue note listing
  "… finding 18 → ~~rework 1~~ → rework 2." exists mirrored in two files that
  must stay byte-identical to each other:
  `docs/reviews/hygiene/reworks-hygiene-2026-07-15.md` (L16-31) and
  `docs/reviews/hygiene/audit-hygiene-2026-07-15.md` (L24-39). Rework 2 is the
  queue's final entry.
- **Ideal:** both mirrored notes show `~~rework 2~~`.
- **Gap:** the queue still lists rework 2 as pending.
- **Suggestion:** strike `rework 2` → `~~rework 2~~` in the queue blockquote of
  BOTH files; change nothing else.
- **Path:** (1) Make the two edits. (2) Verify the two queue blockquotes are
  byte-identical to each other (diff the quoted sections). No code, no test —
  docs-only.
