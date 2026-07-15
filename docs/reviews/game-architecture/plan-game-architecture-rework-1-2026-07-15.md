# Plan: A multiplayer population & progression model for chapter content — 2026-07-15

Source: `docs/reviews/game-architecture/reworks-game-architecture-2026-07-15.md` finding 1.

## Ideal end state

Chapter content has one population model that is identical for one player and
two hundred: **camps** (`world/camp.rs`, deterministic slots, respawn timers)
are the persistent backbone, and **wave-style pressure is world-event
content** — recurring, capped spawn pulses at authored world positions during
an event window, driven by the shared world clock (`WorldEventsDef`), the
same for every player by construction. The single-player wave machinery
(`WaveSpawnerSystem`'s ring around `query().next()`, `ActiveChapter.elapsed`,
`wave_timers`, `spawn_angle`, `SpawnConfig.max_alive`) is deleted, not
generalized — no live content uses it (both `chapter.ron` files ship
`waves: []`). Progression is per-player-entity state: `Xp` lives in
vordar-game as a core component, survives the server's death→respawn cycle,
and persists in the `characters` table alongside position/health/cooldowns —
so chapter 20's stats are authored against the same model as chapter 1's.

## Design decisions

**Candidate shape: (c) — waves become world events; the open zone is
camps-plus-events.** The finding's three candidates were weighed against the
code and DESIGN.md:

- (a) *per-player wave pressure* is rejected: in a shared zone, enemies
  spawned on a ring around player A immediately interact with player B —
  per-player pressure in shared space is incoherent without instancing, and
  nothing in DESIGN.md asks for roguelite-style personal waves.
- (b) *instanced wave chapters* is rejected for now: per-party instances
  require the (zone, channel) / coordinator machinery DESIGN.md §8
  deliberately sequences at P13–P16 ("you scale a game, not an empty grid").
  Nothing in this design blocks instances later — an instance is just another
  zone App running the same content.
- (c) *world events* is exactly what DESIGN.md §4 already plans: "generalize
  the existing chapter schema — `ChapterDef`/`WaveDef` (`start_time`, prefab,
  interval, …) attached to the world clock instead of chapter elapsed time…
  enable wave table X." The content has already voted the same way: both
  live chapters retired waves (`content/chapters/chapter01/chapter.ron:4-5`
  says "Waves are retired" explicitly; both files ship `waves: []`), and
  `WorldEventsDef` (`game/vordar-game/src/world/mod.rs:27-71`) already solved
  "same for everyone" via pure clock math.

**Where wave state lives:** per-zone system state inside `WorldEventSystem`,
derived from the world clock — the pulse index is a pure function of world
time, so the only mutable state is "pulses already fired today" (the same
shape as the existing `fired: Vec<i64>` one-shot latch). Nothing about wave
progress persists or replicates; a restarting zone re-derives everything from
clock + defs. Alive-count budgeting uses a marker component
(`EventSpawned { event, wave }`) queried from the world — the CampMember
pattern (`world/camp.rs:14-21`), never cached Entity ids.

**`max_alive` budget semantics:** per event-wave, zone-global. World pressure
is a property of the world, not of who is standing in it — a cap that scaled
with player count would reintroduce the per-player coupling this rework
removes. A pulse that finds the cap full is forfeited (the counter still
advances), so pressure is rate-limited with no burst catch-up when players
clear the backlog. The old chapter-level `SpawnConfig.max_alive` is deleted
with the chapter wave machinery.

**Spawn positions are authored, not player-relative.** The player-centered
ring (`wave_spawner.rs:60-65,83-84`) is the single-player residue at the heart
of the finding; its replacement is a `positions: Vec<Vec3>` list per wave —
world places (zone edges, graves), the same authoring model as camps and the
existing one-shot `WorldSpawn.positions`.

**Window end cleans up.** When an event's window closes, entities tagged
`EventSpawned` for that event are pushed to the DespawnQueue — the world
returns to its camp baseline instead of accreting event leftovers day after
day. (The cap already bounds accretion; despawn makes the invariant "events
are transient" true rather than merely bounded.)

**Progression: `Xp`/`XpReward`/`XpGrantSystem` are core game code, not
chapter-01 code.** They are generic mechanics (any chapter's prefab can carry
`XpReward`; the grant consumes the shared `Killed` event, `events.rs:33-37`),
and the server's persistence layer cannot depend on a content crate — so they
move from `game/chapter-01/src/lib.rs:37-69` to a new
`game/vordar-game/src/progression.rs`. Chapter-01 returns to being pure
content (prefabs + chapter RON + enemy behaviors).

**Persistence is one column on the migration ladder, not a generic
progression bag.** `ALTER TABLE characters ADD COLUMN xp INTEGER NOT NULL
DEFAULT 0` as migration entry 3 (`db.rs:33-50` append-only ladder). A
serialized stat-map "for future currencies/quests" was rejected as
speculative — future stats design their own storage when they exist, the same
way `cooldowns` did.

**Death must not launder XP away.** The server despawns a dead player's
entity and respawns a fresh prefab (`receive.rs:511-538`) — once XP is a
component, that cycle would silently zero it (and the next autosave would
persist the zero). The fix is a carry: a `DespawnFlush`-phase capture (before
the flush removes the entity — the same window `DeathBroadcastSystem` uses,
`broadcast.rs:213-218`) stashes the dying player's `Xp` in its `PlayerConn`,
and `respawn_dead` seeds the new body from it. Redesigning death itself
(death as a state instead of a despawn) is explicitly out of scope —
`receive.rs:512` records "no death/respawn design yet".

**Deliberate scope cuts (noted, not blockers):** `Xp` does not replicate to
clients — no UI consumes it, and adding it to the protocol is a wire change
owned by the networking domain; the server log line remains the only
observer. Offline sandbox XP is session-only (no persistence path exists
offline). Neither cut is a product question that needs the user now.

## Findings (execution order)

### 1. Move XP progression out of chapter-01 into vordar-game

- **Evidence:** `game/chapter-01/src/lib.rs:37-69` defines `XpReward`
  (deserializable component referenced by
  `content/chapters/chapter01/prefabs/grunt.ron`'s `"XpReward": (amount: 5)`),
  `Xp(pub u32)` (per-player running total), and `XpGrantSystem` (reads
  `Killed` events from `vordar_game::events`, grants the victim's `XpReward`
  to the killer, inserting `Xp(0)` on first kill). `lib.rs:78` registers the
  `XpReward` loader in `Chapter01ContentPlugin`; `lib.rs:89` registers
  `XpGrantSystem` at `Phase::CollisionResolve,
  SystemOrder::after::<DeathSystem>()`. The server's persistence code
  (`server/vordar-server/src`) depends on `vordar-game` but must never depend
  on a content crate, so XP cannot persist while it lives here.
- **Ideal:** `game/vordar-game/src/progression.rs` owns all three items;
  `GameComponentsPlugin` registers the `XpReward` loader (so every chapter's
  prefabs and every networked client get it); `CoreGamePlugin` registers
  `XpGrantSystem`; chapter-01 is pure content again.
- **Gap:** progression types live in a leaf content crate the server cannot
  (and must not) import.
- **Suggestion:** create `game/vordar-game/src/progression.rs` containing
  `Xp`, `XpReward`, and `XpGrantSystem` moved verbatim from chapter-01
  (imports become crate-relative: `crate::events::Killed`,
  `engine_app::events::EventBus`). Give the file a module-header comment
  stating intent (per-player progression state; grants ride the shared
  `Killed` event). Do not re-export at the crate root — consumers use
  `vordar_game::progression::Xp`.
- **Path:**
  1. Add `progression.rs` to `game/vordar-game/src/`; declare `pub mod
     progression;` in `game/vordar-game/src/lib.rs` (and add the module to
     the header's module list comment).
  2. In `game/vordar-game/src/plugin.rs`: `GameComponentsPlugin::build` adds
     `.register_component::<crate::progression::XpReward>("XpReward")`
     next to the existing registrations (line ~31-39); `CoreGamePlugin::build`
     adds `.add_system(crate::progression::XpGrantSystem,
     Phase::CollisionResolve, SystemOrder::after::<DeathSystem>())` next to
     the existing `DeathSystem` registration (line ~79).
  3. In `game/chapter-01/src/lib.rs`: delete `XpReward`, `Xp`,
     `XpGrantSystem`, the `register_component::<XpReward>` call (keep
     `add_prefab_dir`), the `add_system(XpGrantSystem, …)` call, and the now
     orphaned imports (`EventBus`, `DeathSystem`, `Killed`, `System`,
     `SystemOrder` if unused, `World`, `Resources`). Update the crate header
     comment (it currently claims "one chapter-specific component, and one
     system" — chapter-01 now ships content and enemy behaviors only).
  4. Move the test `only_the_killer_gains_xp`
     (`game/chapter-01/src/lib.rs:105-125`) into `progression.rs`'s test
     module unchanged in behavior: two players and a victim with
     `Health { current: 0, max: 50 }` + `XpReward { amount: 25 }`, a
     `DamageDealt` emitted by the killer, then `DeathSystem::new().run(…)`
     followed by `XpGrantSystem.run(…)`; assert the killer has `Xp == 25` and
     the bystander has no `Xp` component. This is the step's regression test —
     it exercises the moved system through the real death/attribution path.
  5. Green check: `cargo test -p vordar-game -p chapter-01 -p vordar-client
     -p vordar-server` (the networked client registers the loader via
     `GameComponentsPlugin`, `client/vordar-client/src/bin/vordar.rs:47`, and
     the sandbox via `CoreGamePlugin` — both must still spawn `grunt.ron`).
     Then the full workspace suite; zero new warnings.

### 2. XP survives the server's death→respawn cycle

- **Evidence:** the server keeps a connection alive across body death by
  respawning a fresh `PLAYER_PREFAB` and re-Welcoming the client
  (`server/vordar-server/src/net/receive.rs:511-538`, `respawn_dead`,
  called from `NetReceiveSystem` in `Phase::Input`). At that point the dead
  entity is already flushed from the world, so any `Xp` component on it (from
  step 1, `game/vordar-game/src/progression.rs`) is unrecoverable — a death
  silently zeroes the player's XP. `DeathBroadcastSystem`
  (`server/vordar-server/src/net/broadcast.rs:213-251`) demonstrates the
  correct capture window: `Phase::DespawnFlush, SystemOrder::First` runs
  after despawns are queued but before the flush removes entities, and
  `DespawnQueue`'s pending list is a public Vec
  (`smirk/engine-core/src/traits.rs:84`). `PlayerConn` is defined at
  `server/vordar-server/src/net/mod.rs:126-163` and constructed at
  `receive.rs:481-493`.
- **Ideal:** a player's XP is a property of the character-on-this-connection,
  not of a particular body: when the body dies, its `Xp` value carries to the
  respawned entity.
- **Gap:** nothing reads a dying player entity's components before the flush,
  and `respawn_dead` seeds nothing but the prefab defaults on the new body.
- **Suggestion:** add `carried_xp: u32` to `PlayerConn` (doc comment: the XP
  value to seed onto the next body this connection spawns; updated in the
  pre-flush death window). Add a small server system `XpCarrySystem` in
  `receive.rs` (next to `respawn_dead`, which consumes it), registered in
  `net::install` (`mod.rs:96-124`) at `Phase::DespawnFlush,
  SystemOrder::First`: iterate `DespawnQueue.0`; for each pending entity that
  is some `pc.entity` in `NetServerState.conns`, set `pc.carried_xp =
  world.get::<&vordar_game::progression::Xp>(entity)` (keep the old
  `carried_xp` when the component is absent). In `respawn_dead`, after a
  successful spawn, `world.insert_one(entity,
  vordar_game::progression::Xp(pc.carried_xp))` (read the value before
  re-borrowing as needed; `world` and `resources` are independent borrows).
  Initialize `carried_xp: 0` at the `PlayerConn` construction site
  (`receive.rs:481-493`).
- **Path:**
  1. Add the field, the system, the registration, and the respawn seeding as
     above.
  2. Regression test in `server/vordar-server/tests/e2e.rs`, modeled on the
     existing `respawn_after_death` (`e2e.rs:259-307`, which shows the
     injected-system pattern: `KillPlayersSystem` pushes players onto the
     `DespawnQueue` at tick 120): new test `respawn_carries_xp` using
     `spawn_server_with` and three injected systems — (i) at tick 60, insert
     `Xp(25)` on every `Player` entity lacking it; (ii) the existing
     `KillPlayersSystem` shape at tick 120; (iii) from tick 180 on, if a
     `Player` entity carries `Xp`, store its value into a captured
     `Arc<AtomicU32>`. A `Bot` connects, waits for the re-Welcome
     (`player_id` changes), then the test asserts the atomic reads 25.
     Fail-first: without the carry, the respawned body has no `Xp` and the
     atomic stays 0.
  3. Green check: `cargo test -p vordar-server`, then the workspace; zero new
     warnings.

### 3. XP persists with the character

- **Evidence:** `CharacterRecord`
  (`server/vordar-server/src/db.rs:78-88`) carries zone/pos/health/cooldowns
  only; the append-only migration ladder is `MIGRATIONS`
  (`db.rs:33-50`, currently 3 entries → `user_version` 3). Save sites:
  `save_character` (`net/mod.rs:276-284`, used by autosave, disconnect, and
  session takeover) and the portal transfer save
  (`net/transfer.rs:61-69`). Load/hydration site: the login grant spawns
  `PLAYER_PREFAB` and overrides `Health.current` from the record, restores
  cooldown remainders (`net/receive.rs:459-493`); login defaults are built at
  `receive.rs:215-220`. The db worker's save SQL is at `db.rs:265-276`, the
  load SQL at `db.rs:308-325`. After steps 1–2, `Xp` is a vordar-game
  component seeded across respawns but lost on relog and zone transfer.
- **Ideal:** XP round-trips like health: saved on every `save_character` and
  transfer, restored onto the freshly spawned player entity at login, with a
  schema migration old databases adopt losslessly.
- **Gap:** no `xp` column, no field on `CharacterRecord`, no hydration at the
  login grant.
- **Suggestion:** append migration entry 4 to `MIGRATIONS`:
  `ALTER TABLE characters ADD COLUMN xp INTEGER NOT NULL DEFAULT 0;` (plain
  DDL — never edit shipped entries). Add `xp: u32` to `CharacterRecord`.
  Wire the four seams:
  - save SQL: add `xp = ?` to the UPDATE (`db.rs:265-276`); load SQL: add
    `xp` to the SELECT and record construction (`db.rs:308-325`);
    `load_or_create`'s INSERT needs no xp column (the schema default 0
    covers new characters).
  - `save_character` (`net/mod.rs:276-284`): `xp:
    world.get::<&vordar_game::progression::Xp>(pc.entity).map(|x|
    x.0).unwrap_or(pc.carried_xp)`.
  - transfer save (`net/transfer.rs:61-69`): same read on `pc.entity`.
  - login grant (`receive.rs:466-493`): after the `Health.current` override,
    `world.insert_one(entity, vordar_game::progression::Xp(record.xp))` (so
    every logged-in player always carries the component), and initialize
    `carried_xp: record.xp` in the `PlayerConn` literal. Login defaults
    (`receive.rs:215-220`) gain `xp: 0`.
  - update every `CharacterRecord` literal in `db.rs` tests (`defaults()` at
    `db.rs:385-387` and the inline literals) with `xp: 0` or the test's value.
- **Path:**
  1. Apply the schema + record + seam changes above.
  2. Regression test A (db): extend
     `save_then_reload_roundtrips_across_reopen` (`db.rs:434-453`) to save
     `xp: 40` and assert it survives the reopen (the legacy-adoption and
     version-stamp tests pick up the new ladder length automatically via
     `MIGRATIONS.len()`).
  3. Regression test B (server, through the real stack): new e2e test
     `xp_survives_relogin` in `server/vordar-server/tests/e2e.rs` — single
     server run, `:memory:` db is fine because the FIFO guarantee
     (disconnect-save lands before the relogin-load, `db.rs:1-11`) is the
     property under test: bot connects as a fixed name; an injected system
     inserts `Xp(25)` on the player at tick ~60; bot disconnects (drop the
     `Bot` / close its client — the server's Disconnected path calls
     `save_character`); a second `Bot` reconnects with the same name and
     token (check how the harness's `connect_as` builds tokens and reuse
     the exact same credential, otherwise the relogin is denied
     `BadCredentials`); an injected probe system (Arc<AtomicU32>, same
     pattern as step 2) reads the new player entity's `Xp`; assert 25.
     Fail-first: without hydration the probe reads 0. If the harness's Bot
     turns out to mint per-instance random tokens with no way to pin them,
     extend the harness minimally (a `connect_with_token` variant) rather
     than weakening the test.
  4. Green check: `cargo test -p vordar-server`, then the workspace; zero new
     warnings.

### 4. Delete the single-player wave machinery; a chapter is initial spawns plus camps

- **Evidence:** `game/vordar-game/src/world/wave_spawner.rs:55-95` —
  `WaveSpawnerSystem` centers spawn rings on
  `world.query::<(&Transform, &Player)>().iter().next()` (an arbitrary
  player), freezes on a zone-global `max_alive`, and advances
  `ActiveChapter.elapsed`/`wave_timers`/`spawn_angle`
  (`world/chapter.rs:74-81`). No live content uses any of it: both
  `content/chapters/chapter01/chapter.ron` (line 15-18, comment at 4-5:
  "Waves are retired") and `content/chapters/chapter02/chapter.ron`
  (line 15-18) ship `waves: []`; the `spawning` block exists only to feed the
  dead system. `ActiveChapter` is otherwise a pure wrapper around
  `ChapterDef` consumed by `ChapterSetupSystem`
  (`wave_spawner.rs:16-51`, reads `started`) and `CampSystem`
  (`world/camp.rs:48`, reads only `def.camps`). Registration:
  `game/vordar-game/src/plugin.rs:21,66`. Chapter plugins insert the
  resource via `load_chapter` (`game/chapter-01/src/lib.rs:88`,
  `game/chapter-02/src/lib.rs:54`).
- **Ideal:** `ChapterDef { name, initial_spawns, camps }` is itself the
  resource — pure data, no runtime clock, no per-player assumptions; the
  run-once latch lives in `ChapterSetupSystem` (the established pattern:
  its own `warned` field, `CampSystem::initialized`, the sandbox's
  `SpawnPlayerSystem.done`). The dead system, its config types, and its
  content blocks are gone.
- **Gap:** the state layout (`elapsed` as one zone clock, resource-held wave
  timers, `SpawnConfig`) is the single-player residue the finding names, and
  it cannot express the multiplayer model — deletion, not generalization, is
  the design (see Design decisions).
- **Suggestion:**
  - `world/chapter.rs`: delete `SpawnConfig`, `WaveDef`, `default_count`, and
    `ActiveChapter`; `ChapterDef` keeps `name`, `initial_spawns`, `camps`
    (both already `#[serde(default)]`); `load_chapter` returns `ChapterDef`
    and its log line reports camp count instead of wave count.
  - Rename `world/wave_spawner.rs` → `world/setup.rs`, keeping only
    `ChapterSetupSystem` with a new `done: bool` field replacing
    `chapter.started`; it reads `resources.get::<ChapterDef>()`. Update the
    `pub mod` list and the module-header comment in `world/mod.rs:1-14`
    (which currently says "wave spawning (submodules)").
  - `world/camp.rs`: read `ChapterDef` directly (line 48), fix the header
    comment ("No-op without an ActiveChapter resource" → `ChapterDef`), and
    rebuild the test fixture (`camp_resources`, lines 115-138) around a plain
    `ChapterDef` with no `spawning`/`elapsed`/`wave_timers`/`spawn_angle`/
    `started` fields.
  - `plugin.rs`: drop the `WaveSpawnerSystem` registration and import; the
    `ChapterSetupSystem` import path becomes `crate::world::setup`.
  - Content: remove the `spawning: (max_alive: …, waves: [])` block from both
    `chapter.ron` files; rewrite chapter-01's stale header sentence ("Waves
    are retired — the list stays for tooling…") to state the model plainly
    (world is populated by camps; scheduled pressure is world-event content).
  - Tests in `chapter.rs`: replace
    `camps_field_defaults_to_empty_on_old_chapters` (which parses a
    `spawning:` block) with a minimal-chapter test: `(name: "old")` parses
    with empty `initial_spawns` and `camps`.
- **Path:**
  1. Apply the deletions/renames above (chapter-01/02 `lib.rs` lines calling
     `load_chapter` need no code change — the resource type becomes
     `ChapterDef`).
  2. Regression tests: (a) the minimal-chapter parse test above; (b) a new
     behavioral test for `ChapterSetupSystem` in `world/setup.rs` — build
     `Resources` with a `ChapterDef` containing one `initial_spawns` entry, a
     `ComponentRegistry`/`PrefabLibrary`/`SpawnQueue` (mirror `camp.rs`'s
     `camp_resources` fixture), run the system twice, and assert exactly one
     spawn was queued (run-once latch holds without the deleted `started`
     flag). Existing camp tests (updated fixture) pin that camps are
     unaffected.
  3. Green check: full workspace test run — `server/vordar-server/tests`
     (zones.rs, e2e.rs) boot real zone Apps that parse the real `chapter.ron`
     files, so they prove the content migration; zero new warnings.

### 5. World-event waves: recurring, capped pressure spawns during an event window

- **Evidence:** `game/vordar-game/src/world/mod.rs:27-143` — `WorldEventDef`
  supports only one-shot `spawns` fired once per world day on window entry
  (`WorldEventSystem::run`, lines 108-143, with the `fired: Vec<i64>`
  per-day latch); `active_event` (lines 65-71) is the pure window function.
  DESIGN.md §4 plans wave tables attached to the world clock ("enable wave
  table X"). After step 4 the codebase has no mechanism at all for sustained
  wave pressure; camps (`world/camp.rs`) cover only fixed-headcount
  populations. The spawn-with-marker pattern to copy is `camp.rs:88-95`
  (SpawnQueue closure: `spawn_prefab` + `insert_one` of the marker).
- **Ideal:** an event can exert pressure for its whole window: every
  `interval_seconds`, spawn `prefab` at each authored position, capped by a
  zone-global `max_alive` per wave; when the window closes, the event's
  spawns are cleaned up. Pulse timing is a pure function of the world clock,
  so every zone/process that shares the clock and defs agrees on the
  schedule by construction.
- **Gap:** no recurring-spawn schema, no alive-count budgeting, no
  window-end cleanup.
- **Suggestion:** in `world/mod.rs`:
  - Schema: `WorldEventDef` gains `#[serde(default)] pub waves:
    Vec<EventWaveDef>`; new `#[derive(serde::Deserialize)] pub struct
    EventWaveDef { pub prefab: String, pub positions: Vec<Vec3>, pub
    interval_seconds: f64, pub max_alive: usize }`. Old `events.ron` files
    parse unchanged (default = empty), and the client's tint path
    (`client/vordar-client/src/world_time.rs`) ignores the new field.
  - Marker: `pub struct EventSpawned { pub event: u16, pub wave: u16 }` —
    attached in the spawn closure; alive counts and cleanup query it (never
    cached Entity ids).
  - `WorldEventSystem` state: alongside `fired`, add `pulses:
    Vec<Vec<(i64, u64)>>` — per event, per wave: (world day, pulses fired
    that day); re-init like `fired` when def lengths change, reset the count
    when the day advances.
  - Per run (only when `WorldTime` + `WorldEventsDef` exist — same no-op
    guards as today): for each event compute `day`, `day_time`, `in_window`
    exactly as lines 117-124. If `in_window`: for each wave, `due =
    ((day_time - event.start_seconds_of_day) / wave.interval_seconds).floor()
    as u64` (pulse i fires at start + i·interval, i ≥ 1 — window entry
    already has the one-shot `spawns`). For each unfired pulse in
    `fired_count..due`: count alive as live `EventSpawned{event,wave}`
    entities plus spawns queued this run; for each position, if the count is
    below `max_alive`, push a SpawnQueue closure (`spawn_prefab` at the
    position, then `insert_one(EventSpawned { event, wave })` — the
    `camp.rs:88-95` shape) and bump the count. Set the stored count to `due`
    unconditionally (forfeit over-cap pulses — no burst catch-up; see Design
    decisions). If `!in_window`: push every live `EventSpawned` entity of
    this event onto the `DespawnQueue` (they flush the same tick, so the
    steady-state cost is one empty query).
- **Path:**
  1. Implement the schema, marker, and system logic above. Update the
     module-header comment (`world/mod.rs:1-8`) to name waves as part of the
     world-event model.
  2. Regression tests in `world/mod.rs`'s test module, driving the real
     system with a manually inserted `WorldTime` (the system reads the clock,
     not delta) and a fixture mirroring `camp.rs`'s (`ComponentRegistry` +
     `PrefabLibrary` with a `"dummy"` Transform-only prefab + `SpawnQueue` +
     `DespawnQueue`, and the same drain-the-SpawnQueue `tick` helper,
     `camp.rs:144-150`):
     - `event_wave_pulses_spawn_on_interval`: window 30–50 s of a 100 s day,
       interval 5 s, cap 10, 2 positions → at `WorldTime` = 36 s one pulse
       (2 entities), at 46 s three pulses total (6 entities), at 29 s of the
       window nothing extra after it closes.
     - `event_wave_respects_max_alive_cap`: cap 3, 2 positions, advance
       through 3 pulses → exactly 3 tagged entities exist; kill one
       (despawn it), advance one more pulse → back to 3 (capacity freed is
       reused), and the forfeited pulses never burst.
     - `window_end_despawns_event_spawns`: advance past the window end →
       every `EventSpawned` entity of the event lands in the `DespawnQueue`.
     - a parse test: an events RON string **without** `waves` and one
       **with** a wave entry both deserialize (`waves` defaults empty).
  3. Green check: `cargo test -p vordar-game`, then the workspace (the
     server e2e blood-moon test at `server/vordar-server/tests/e2e.rs:210-257`
     inserts a `WorldEventDef` literal — it gains `waves: vec![]`, which the
     compiler will force); zero new warnings.

### 6. Author blood-moon pressure and prove the wave model end-to-end

- **Evidence:** `content/zones/events.ron` — the blood moon carries only
  one-shot `spawns` (6 grunts at fixed points on window entry); after step 5
  the engine supports recurring capped waves
  (`game/vordar-game/src/world/mod.rs`, `EventWaveDef` + `EventSpawned`) but
  no content or end-to-end test exercises them through the real server →
  replication → client stack. The existing e2e test
  `world_clock_and_blood_moon` (`server/vordar-server/tests/e2e.rs:210-257`)
  already proves one-shot event spawns replicate to every bot via the
  `grunt_count` prefab tally.
- **Ideal:** the shipped blood moon exerts sustained pressure (grunts pushing
  in from the zone edges for the whole window, capped), and an e2e test pins
  that wave spawns replicate to connected clients exactly like any other
  server-spawned entity.
- **Gap:** the mechanism has zero content users and no cross-process test.
- **Suggestion:** two small changes:
  - `content/zones/events.ron`: add to the blood_moon event
    `waves: [ (prefab: "grunt", positions: [(16.0, 0.0, 16.0),
    (-16.0, 0.0, 16.0), (16.0, 0.0, -16.0), (-16.0, 0.0, -16.0)],
    interval_seconds: 5.0, max_alive: 8) ]` — edge positions outside every
    chapter-01 camp aggro bubble (`content/chapters/chapter01/chapter.ron`
    layout rules: camps live within |x|,|z| ≤ ~19; the ring at ±16,±16 is
    ~22.6 from origin, clear of the portal corridor along +X at z = 0), and
    update the file's header comment to describe the wave clause.
  - `server/vordar-server/tests/e2e.rs` `world_clock_and_blood_moon`: extend
    the inserted `WorldEventsDef` with one wave (e.g. `interval_seconds:
    2.0`, 2 positions, `max_alive: 20`) and add a wait after the existing
    one-shot assertion: `grunt_count(bot) >= 5` (3 one-shot + at least one
    2-grunt pulse) within the 30 s window — proving pulse spawns ride the
    same AOI/replication path.
- **Path:**
  1. Edit `events.ron` as above; run the client binary's parse path
     indirectly via the step-5 parse tests (the real file is loaded by
     `server/vordar-server/src/main.rs:103` and
     `client/vordar-client/src/bin/vordar.rs:50` at runtime) — additionally
     assert the shipped file parses by pointing a small test at it if one
     does not already exist: prefer extending the step-5 parse test in
     `world/mod.rs` to call `load_world_events("../../content/zones/events.ron")`
     only if the relative-cwd convention allows (the server e2e harness's
     `workspace_root()` shows the repo convention is cwd = workspace root;
     from a unit test cwd is the crate dir, so use the `../../` form and
     verify it resolves — if the path proves brittle, keep the string-literal
     parse test and rely on the e2e boot, which loads the real file via
     `main.rs` only in production; in that case the extended e2e test in the
     next sub-step is the real-content guard and the RON edit must simply
     match the tested schema exactly).
  2. Extend the e2e test as above; run
     `cargo test -p vordar-server world_clock_and_blood_moon` fail-first
     against a build without the wave entry in the inserted def (count stays
     at 3), then green with it.
  3. Green check: full workspace suite; zero new warnings.
