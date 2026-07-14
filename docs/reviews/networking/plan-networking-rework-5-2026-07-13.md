# Plan: Wire format waste: 5-byte-minimum entity ids, repeated prefab strings, unquantized absolute states — 2026-07-13

Source: docs/reviews/networking/reworks-networking-2026-07-11.md finding 5.

## Ideal end state

Snapshot frames stop paying for representation instead of information: every
wire entity reference is a small zone-local `u32` (1–2 varint bytes instead of
the 5+ bytes hecs entity bits cost today), positions ride as 1/256-meter fixed
point in zigzag varints instead of raw `3×f32`, prefab identity is a `u16`
index into a per-zone table the server sends once per connection right after
`Welcome`, and hp is an explicit `Option<i32>` so `0` no longer conflates "no
Health component" with "dead". A steady-state full snapshot (the 64-entry
`MAX_SNAPSHOT_STATES` budget) lands around ~800 bytes — proven by a crowd e2e
gate at ≤ 1100 bytes — comfortably under the ~1.2 KB QUIC datagram capacity
that rework 3 (snapshots on datagrams) is physically blocked on today.

## Design decisions

- **Zone-local `u32` replication ids, shared across connections, allocated by
  a monotonic per-zone allocator.** Wire ids today are hecs entity bits
  (`entity.to_bits().get()`, e.g. `net_plugin.rs:1134`), always ≥ 2³² because
  of generation bits, hence ≥ 5-byte varints. A `ReplIds` map on
  `NetServerState` (`HashMap<Entity, u32>` + `next: u32`, starting at 1)
  assigns an id the first time any wire message references an entity and is
  swept periodically against `world.contains` so despawned entities (bolts,
  dead enemies) don't leak entries. Ids are shared zone-wide, not
  per-connection, because `HitResult` and `EntityDied` frames are encoded once
  and cloned to many connections (`net_plugin.rs:941`, `net_plugin.rs:1208`) —
  per-connection ids would force per-recipient re-encoding for zero size win.
  `u32` over `u16` because `u16` would force id recycling (65 k spawns per
  zone lifetime is easily reached with projectiles) while postcard varints
  make small `u32` values just as cheap; ids are never reused (u32 headroom is
  years of uptime), so a stale id can never alias a new entity. hecs
  generations mean a reused `Entity` slot compares unequal to the old value,
  so the map itself can never alias across the sweep either.
- **Quantization as a serde boundary type, not a call-site rewrite.**
  `WirePos(pub Vec3)` in vordar-protocol serializes as three `i32`s at 256
  units per meter (zigzag varints under postcard) and deserializes back to
  `Vec3` — all Rust-side code keeps working in `Vec3`, and the precision loss
  (≤ ~2 mm per axis, half a quantum) happens exactly once at encode. 2 mm is
  two orders of magnitude below the client's `TRUST_DISTANCE = 0.3`
  reconciliation band (`client/net.rs:39`), so gameplay/prediction is
  unaffected. **Zone-local origins rejected:** zigzag varints are already
  1 byte near zero and 3 bytes out to ±128 m — rebasing per zone would add a
  per-zone origin contract to the protocol for no size win at current zone
  scales (`i32/256` covers ±8 388 km as-is).
- **Prefab identity: a per-zone name table sent once per connection right
  after `Welcome`, not a hash-checked registry.** The finding's suggestion
  (registry pinned by content hash, checked at login) hits a wall in this
  codebase: the client deliberately installs EVERY chapter's content
  (`client/vordar-client/src/bin/vordar.rs:54`, `install_all_content` — a
  Redirect can land it in any zone) while each zone server installs only its
  own chapter's prefab dir (`server/vordar-server/src/main.rs:96-103`,
  `game/chapter-01/src/lib.rs:75`), so independently derived registries
  mismatch by construction and a hash check would deny every login. Instead
  the server is authoritative: a new `ServerMsg::PrefabTable { names }`
  (index = u16 id) built lazily from the zone's fully-populated
  `PrefabLibrary` (sorted names, deterministic) is sent immediately after
  `Welcome` on the same ordered stream, so it always arrives before the first
  snapshot's `enters`. Redirects/reconnects get the new zone's table
  automatically because the client clears it in `teardown_replicated_world`.
  Content skew still surfaces loudly: an unknown name fails `spawn_prefab`
  with `UnknownPrefab` and is logged (`client/net.rs:525`); a full
  content-version handshake belongs to a future deployment story (rework 6
  territory), not to this wire-size fix. **Per-connection incremental binding
  (defs riding in each snapshot's `enters`) rejected:** same bytes, strictly
  more bookkeeping (per-conn announced-sets on the server, partial tables on
  the client).
- **`hp: Option<i32>`** on both `EntityState` and `EntityPos`: `None` = the
  entity has no `Health` component, `Some(v)` = a real reading (which may be
  ≤ 0 momentarily). Costs 1 extra byte per present hp; removes the standing
  lie at `vordar-protocol/src/lib.rs:98-99` / `net_plugin.rs:1133`.
- **`MechanicScheduled.telegraph_prefab` (String) and `EntityDied.pos`
  (f32 Vec3) deliberately stay as-is:** both are rare, event-scale messages;
  the finding targets the per-tick snapshot cost. Widening scope buys bytes
  nobody is spending.
- **Delta-vs-baseline compression deliberately omitted** (finding path step 5,
  gated by its own Ideal: "only if numbers still bind after"). Arithmetic
  after compaction: `EntityPos` ≈ id 1–2 B + pos ~5–7 B + hp 1–3 B ≈ 8–12 B;
  64 states + envelope ≈ 800 B < 1.2 KB datagram budget. Step 5's measurement
  gate proves it; if that gate cannot be met, delta compression gets its own
  design pass then — not speculatively now.
- **`PROTOCOL_VERSION` bumps by one in every wire-changing step (v10…v13).**
  engine-net rejects mismatched versions at handshake; client and server ship
  from one workspace, so intermediate versions never coexist. Keeping the
  invariant "any wire change = version change" per commit is a one-line cost.
- **`MAX_FRAME_OUT` stays 64 KiB** (`smirk/engine-net/src/common.rs:13`): a
  first-join snapshot in a crowd legitimately carries ~100 `enters` and may
  exceed the datagram budget; only steady-state `states` traffic must fit
  under it (rework 3 keeps `enters`/`leaves` on the reliable stream).
- **online-play.mmd:** the only flow-visible change is the prefab table after
  `Welcome`, so the diagram + SVG update rides inside step 4 per the queue
  note in the reworks file.

## Findings (execution order)

### 1. Zone-local u32 replication ids on every wire entity reference (protocol v10)

- **Evidence:** Every wire entity id is hecs entity bits, ≥ 2³² and therefore
  a 5+ byte postcard varint: `game/vordar-protocol/src/lib.rs` —
  `Welcome { player_id: u64 }` (line 48), `Snapshot.leaves: Vec<u64>` (60),
  `EntityState.id: u64` (120), `EntityPos.id: u64` (131),
  `EntityDied { id: u64 }` (97), `HitResult { hits: Vec<u64> }` (80).
  Producers in `server/vordar-server/src/net_plugin.rs`:
  `entity.to_bits().get()` at 759 and 788 (Welcome/re-Welcome), 898
  (HitResult hits), 1134 (snapshot gather), 1198 (DeathBroadcastSystem);
  `PlayerConn.known: HashSet<u64>` (164) and the `current_ids`/`leaves`/
  `enters` diff (1144-1166) plus `select_states(entries: &[(u64, f32)], …)`
  (1034). Consumers: `client/vordar-client/src/net.rs` —
  `NetClientState.own_id: Option<u64>` (198), `entities: HashMap<u64, Entity>`
  (200), `handle_entity_died(…, id: u64, …)` (447), bench mod `map_entity`
  (1121); `server/vordar-server/tests/common/mod.rs` — `Bot.player_id`,
  `last_snapshot`, `prefabs`, `hit_results`, `last_hp`, `deaths` (109-137);
  `benchmarks/benches/client_netcode.rs:55-96` and
  `benchmarks/benches/protocol.rs:12-24` build ids as small u64 literals.
- **Ideal:** All six wire fields are `u32`. The server owns a per-zone
  `ReplIds` allocator on `NetServerState`: `id_for(entity)` returns the
  existing id or assigns `next` (monotonic, starts at 1, never reused);
  `sweep(&World)` retains only entries whose entity is still alive, called
  from `SnapshotBroadcastSystem`'s existing periodic block
  (`state.tick % 600 == 0`, net_plugin.rs:1076). Client and bot id types
  follow mechanically. `PROTOCOL_VERSION` = 10.
- **Gap:** No allocator exists; every producer converts `Entity` → bits
  inline; the id type is u64 end to end.
- **Suggestion:** Add `struct ReplIds { by_entity: HashMap<Entity, u32>, next: u32 }`
  with `id_for`/`sweep` as a private type in net_plugin.rs (unit-testable in
  its existing tests mod, which already spawns hecs `World`s). Field
  `repl_ids: ReplIds` on `NetServerState`. Convert at the state-borrow sites:
  in `SnapshotBroadcastSystem` drop the precomputed u64 from the gather tuple
  (make it `(Entity, Vec3, i32, f32)`) and call `state.repl_ids.id_for(entity)`
  inside the per-conn loop where `state` is already `&mut` (1140+), changing
  `select_states` entries and `PlayerConn.known`/`rr` bookkeeping to `u32`;
  in `MechanicResolveSystem` keep collecting `hit_entities: Vec<Entity>` and
  build `hits` at frame-encode time under a `get_mut` borrow (today lines
  898/940 use an immutable borrow); in `DeathBroadcastSystem` collect
  `(Entity, Vec3)` from the events and map to ids under the existing
  `get_mut` (1206); Welcome/re-Welcome send `id_for(entity)`. Client:
  `own_id: Option<u32>`, `entities: HashMap<u32, Entity>`, u32 through
  `apply_snapshot`/`handle_entity_died`/bench mod. Bot: same u32 key changes.
  Benches: id literals become u32.
- **Path:**
  1. Write the failing test first: new e2e in
     `server/vordar-server/tests/e2e.rs` — `build_server_app(addr, ":memory:")`
     plus `PopulateSystem` spawning ~10 `"player"` NPCs inside AOI (mirror the
     pattern at e2e.rs:447-448), a `Bot` connects and waits for a snapshot
     with non-empty `enters`; assert `bot.player_id` and every id seen in
     `enters`/`states`/`leaves` is `< 100_000`. Against today's code this
     fails (hecs bits are ≥ 2³²); after the change the assert holds by
     construction and documents the compactness contract.
  2. Change the six protocol fields to `u32`, bump `PROTOCOL_VERSION` to 10,
     update the protocol roundtrip tests in `vordar-protocol/src/lib.rs`.
  3. Add `ReplIds` + `NetServerState.repl_ids`, convert the five server
     producer sites and `PlayerConn.known`/`select_states`/`bench` seam
     (`state_with_fake_conns` needs no change; `select_states` signature and
     its four unit tests move to `u32`).
  4. Unit-test `ReplIds` in net_plugin's tests mod: same entity → same id;
     two entities → distinct monotonic ids; after despawning one and
     `sweep(&world)`, its entry is gone and a fresh entity gets a NEW id
     (never a reused one).
  5. Mechanically follow the type through client (`net.rs` state, bench mod,
     the two integration tests at net.rs:1262/1393 compile untouched — they
     only compare `own_id` values), Bot, and both benches.
  6. Green gate: `cargo test --workspace` and `cargo check --benches` pass
     with zero new warnings.

### 2. Quantized snapshot positions via WirePos (protocol v11)

- **Evidence:** `EntityState.pos: Vec3` and `EntityPos.pos: Vec3`
  (`game/vordar-protocol/src/lib.rs:122,132`) serialize as raw `3×f32` =
  12 bytes each, per AOI entry per snapshot. Producers:
  `net_plugin.rs:1151-1153` (enters) and 1160-1166 (states) pass
  `t.position`/`pos` straight through. Consumers read `.pos` as `Vec3`:
  `client/net.rs` `apply_snapshot` (512, 516, 539, 571-574),
  `tests/common/mod.rs` Bot.pump (311, 322), benches
  (`protocol.rs:14`, `client_netcode.rs:58,65,95`).
- **Ideal:** vordar-protocol gains
  `pub struct WirePos(pub Vec3)` with hand-written `Serialize`/`Deserialize`
  encoding `((p.x*256).round() as i32, y…, z…)` and decoding `i32 as f32/256`,
  plus `pub const POS_UNITS_PER_METER: f32 = 256.0;`. `EntityState.pos` and
  `EntityPos.pos` become `WirePos`. Wire cost per position drops from a fixed
  12 B to ~3–7 B for in-zone coordinates (zigzag varints; y ≈ 0 encodes in
  1 byte). Rounding error ≤ 1/512 m per axis — invisible under the client's
  0.3-unit trust band. `PROTOCOL_VERSION` = 11.
- **Gap:** No quantized type exists; positions are raw f32 on the wire.
- **Suggestion:** Keep every call site in `Vec3`: server wraps
  (`pos: WirePos(pos)`) at the two construction sites; client/bot/benches
  unwrap with `.0`. Do NOT quantize `MechanicScheduled.pos` or
  `EntityDied.pos` (rare event messages — out of scope by design decision).
- **Path:**
  1. Fail-first unit tests in `vordar-protocol/src/lib.rs`: (a) round-trip a
     `ServerMsg::Snapshot` through the real `encode`/`decode` with states at
     awkward coordinates (negative, fractional, e.g. `(-37.123, 0.0, 81.987)`)
     and assert every decoded position is within `1.0/512.0 + 1e-4` per axis
     of the original; (b) assert `encode` of a single `EntityPos { id: 500,
     pos: WirePos(Vec3::new(12.34, 0.0, -7.89)), hp: 100 }` is at most
     12 bytes (raw f32 made it ≥ 17). Written against the new type, these
     compile only with the change — the size assertion is the step's
     behavioral teeth.
  2. Add `WirePos` + const with doc comment (quantum, max error, why 256).
  3. Switch the two struct fields, bump `PROTOCOL_VERSION` to 11, update the
     protocol roundtrip tests to approximate position comparison (±1/256).
  4. Wrap at net_plugin.rs enters/states construction; unwrap at
     client `apply_snapshot` (spawn pos, NetLerp from/to, own reconcile pos,
     NetMotion velocity math), Bot.pump, and both benches.
  5. Run the FULL workspace test suite: existing e2e position assertions all
     use tolerances far above 2 mm (walk checks, trust-band checks), so any
     failure is a real bug in the wrapper, not a tolerance to widen. Green
     gate: `cargo test --workspace`, zero new warnings.

### 3. hp as an explicit Option — 0 stops meaning "no Health" (protocol v12)

- **Evidence:** `EntityState.hp: i32` and `EntityPos.hp: i32` with the doc
  "0 for entities without a Health component"
  (`game/vordar-protocol/src/lib.rs:124-126,133-134`). The server flattens at
  `net_plugin.rs:1133` — `hp.map(|h| h.current).unwrap_or(0)` — so a
  replicated entity with no `Health` (e.g. the `bolt` prefab,
  `content/prefabs/bolt.ron`, which has `Transform`+`Hitbox` but no Health)
  is indistinguishable from a dying one at hp 0. Client seeds/overwrites
  `Health.current` unconditionally (`client/net.rs:520-522, 545-554`); Bot
  stores every hp (`tests/common/mod.rs:312, 323`).
- **Ideal:** Both wire fields are `Option<i32>`: `None` = entity has no
  Health component; `Some(v)` = authoritative reading. The server passes the
  `Option` straight through from the gather (`hp.map(|h| h.current)`); the
  client touches `Health.current` only on `Some`; the Bot records only `Some`
  into `last_hp`. `PROTOCOL_VERSION` = 12.
- **Gap:** The `unwrap_or(0)` flattening and the i32 wire type.
- **Suggestion:** Change the gather tuple in `SnapshotBroadcastSystem` to
  carry `Option<i32>` (drop the `unwrap_or(0)` at 1133), thread it into
  `enters` and `states`. Client `apply_snapshot`: `if let Some(hp) = enter.hp`
  around the Health seed, and skip `None` in the states hp view loop. Bot:
  `if let Some(hp) = …` before `last_hp.insert`. Benches: `hp: Some(40)` /
  `hp: None` for the bolt fixtures in `client_netcode.rs:58,65,95` and
  `protocol.rs:19,22`.
- **Path:**
  1. New e2e in `server/vordar-server/tests/e2e.rs`:
     `build_server_app(addr, ":memory:")` + `PopulateSystem` spawning one
     `"bolt"` (Health-less, replicated: it has Transform+Hitbox+PrefabId)
     near the spawn ring plus the bot itself. Wait until `bot.prefabs`
     contains a `"bolt"` entry (prefab names are still Strings at this step);
     assert its id is present in `bot.last_snapshot` but ABSENT from
     `bot.last_hp` (wire `None`), while `bot.last_hp[player_id] == 100`
     (wire `Some`). This is the behavioral distinction the old format could
     not express.
  2. Change the two fields to `Option<i32>`, bump `PROTOCOL_VERSION` to 12,
     update protocol roundtrip tests (`hp: Some(100)` and a `None` case).
  3. Server gather + enters/states construction; client apply sites; Bot
     pump; both benches.
  4. Green gate: `cargo test --workspace` (the grunt hp e2e at e2e.rs:484-549
     keeps passing — grunts have Health, so their readings still flow), zero
     new warnings.

### 4. Per-zone prefab table after Welcome; u16 prefab refs in enters (protocol v13)

- **Evidence:** `EntityState.prefab: String`
  (`game/vordar-protocol/src/lib.rs:121-122`) repeats the full prefab name in
  every AOI enter, built per enter at `net_plugin.rs:1151`
  (`world.get::<&PrefabId>(entity)…0.clone()`). Client spawns by that string
  (`client/net.rs:512`); Bot stores it (`tests/common/mod.rs:313`); e2e
  tests assert on names (`e2e.rs:176,354,503-506`, `zones.rs:186-195`). The
  zone's prefab set is static after App build: `vordar-game` adds
  `content/prefabs` (`game/vordar-game/src/plugin.rs:38`) and the zone's
  chapter adds its own dir (`game/chapter-01/src/lib.rs:75`) — but the CLIENT
  installs all chapters (`client/vordar-client/src/bin/vordar.rs:54`), so
  client and zone libraries differ by design (see Design decisions: this is
  why a login hash check is the wrong shape).
- **Ideal:** New `ServerMsg::PrefabTable { names: Vec<String> }` — index is
  the u16 prefab id — sent once per connection immediately after the initial
  `Welcome` (net_plugin.rs:759-762, alongside the WorldClock send; NOT resent
  on the respawn re-Welcome at 788, the connection keeps its table).
  `EntityState.prefab` becomes `u16`. The server builds the table lazily on
  first login grant (the App is fully built by then, so every chapter prefab
  dir has loaded) as the SORTED names of the zone's `PrefabLibrary`, cached
  on `NetServerState` as `Vec<String>` + `HashMap<String, u16>`. Stream
  ordering guarantees the table precedes the first snapshot. The client
  caches `prefab_names: Vec<String>` on `NetClientState`, cleared in
  `teardown_replicated_world` (net.rs:339-357) so redirects/reconnects adopt
  the new zone's table. docs/online-play.mmd + SVG updated (the queue-note
  requirement — this step changes the login/welcome flow).
  `PROTOCOL_VERSION` = 13.
- **Gap:** No table type, no send, no client/bot cache; prefab is a String
  per enter.
- **Suggestion:** Server: `NetServerState` gains
  `prefab_table: Option<(Arc<Vec<String>>, HashMap<String, u16>)>`; a
  `fn prefab_table(&mut self, resources)`-style helper is awkward under the
  borrow patterns here, so build it inline in the DbLoaded-granted branch
  (net_plugin.rs:727-763) where `resources` is in hand:
  `resources.get::<PrefabLibrary>()` → sorted name list (guard
  `names.len() <= u16::MAX as usize + 1` with an `expect` — content-scale
  counts are single digits today). In `SnapshotBroadcastSystem`'s enters
  construction (1147-1154), map `PrefabId.0` through the name→u16 map; a miss
  (impossible for a spawned prefab — `spawn_prefab` attaches `PrefabId` from
  the same library) logs an error and skips the entity. Client: on
  `ServerMsg::PrefabTable` store the names; in `apply_snapshot` resolve
  `enter.prefab as usize` via `prefab_names.get(…)`, log + skip on a miss.
  Bot: store the table in a new `prefab_names: Vec<String>` field and keep
  `Bot.prefabs: HashMap<u32, String>` resolving indices to names in `pump`
  (panic on an unresolvable index — test hygiene), so every existing
  name-based e2e assertion keeps working unchanged and doubles as proof the
  binding resolves. Benches: `client_netcode.rs` builds enters with `prefab:
  <u16>` and seeds the client table via a new bench-mod helper
  (`pub fn set_prefab_table(state: &mut NetClientState, names: Vec<String>)`).
- **Path:**
  1. Fail-first e2e in `server/vordar-server/tests/e2e.rs`
     (`prefab_table_binds_u16_refs`): bot connects to
     `build_server_app(addr, ":memory:")`; assert the bot receives a
     `PrefabTable` before its first snapshot with non-empty `enters`
     (bot records arrival order), that every enter's index is in range, and
     that its own enter resolves to `"ravager"` (the `PLAYER_PREFAB`).
  2. Protocol: `EntityState.prefab: u16`, new `PrefabTable` variant with doc
     comment (sent once per connection after Welcome, index = id, per-zone),
     bump `PROTOCOL_VERSION` to 13, roundtrip tests.
  3. Server: lazy table build at grant + send after Welcome; enters mapping;
     u16::MAX guard.
  4. Client: `NetClientState.prefab_names` (+ clear in teardown), receive arm,
     resolve in `apply_snapshot`; bench mod helper; the two real-server
     integration tests in net.rs (1262, 1393) now exercise the table
     end-to-end without modification — they spawn via real enters.
  5. Bot: `prefab_names` field, resolve in pump. Full suite green — the
     zones.rs chapter tests (`grunt`, `npc_villager`, cross-zone redirect →
     fresh table from the other zone) are the multi-zone regression proof.
  6. docs/online-play.mmd: extend the `SW` node label ("spawn + Welcome +
     zone prefab table (u16 → prefab name)") and the `SB ==> CRECV` edge
     label to mention the prefab table; regenerate docs/online-play.svg (use
     the mermaid-diagrams skill). Green gate: `cargo test --workspace`, zero
     new warnings.

### 5. Crowd-snapshot size gate: steady-state full snapshot ≤ 1100 bytes

- **Evidence:** The rework's acceptance criterion lives in the queue note of
  `docs/reviews/networking/reworks-networking-2026-07-11.md`: "QUIC datagrams carry
  ~1.2 KB and crowd snapshots are ~2.2 KB today, so [rework] 3's
  snapshots-on-datagrams step is physically impossible until 5's compaction
  shrinks them under the MTU." The measurement hook exists —
  `Bot.bytes` counts app-message bytes (`tests/common/mod.rs:301`) — but
  nothing measures per-frame size, and no test would catch a regression that
  pushes snapshots back over the datagram budget. The steady-state worst case
  is a full `states` list: `MAX_SNAPSHOT_STATES = 64`
  (`net_plugin.rs:64`).
- **Ideal:** A permanent e2e gate: a bot inside a 100-entity crowd, past the
  initial `enters` wave, sees every snapshot frame ≤ 1100 bytes while its
  `states` list is pinned at the full 64-entry budget. (1100 leaves envelope
  margin under the ~1.2 KB datagram capacity; post-compaction arithmetic puts
  the frame near 800 B, so the gate has slack against noise but fails the old
  ~1.25–2.2 KB format decisively.) First-join snapshots may exceed the gate —
  they carry the full `enters` wave and stay on the reliable stream in
  rework 3 — so only steady-state frames are measured.
- **Gap:** No per-frame size tracking in the Bot; no crowd-scale size test.
- **Suggestion:** Add `pub snapshot_bytes: Vec<usize>` to `Bot`
  (`tests/common/mod.rs`), pushed with `data.len()` in the `Snapshot` arm of
  `pump` (alongside the existing `self.bytes += data.len()` at line 301).
  New e2e in `server/vordar-server/tests/e2e.rs`
  (`crowd_snapshot_fits_datagram_budget`).
- **Path:**
  1. Add `Bot.snapshot_bytes` (initialize in all three Bot constructors).
  2. The test: `build_server_app(addr, ":memory:")` +
     `PopulateSystem` (pattern of e2e.rs:447-448) spawning 100 `"player"`
     NPCs on rings of radius 5–25 around the origin (all inside the bot's
     `AOI_RADIUS = 40`); bot connects and waits until
     `bot.last_states.len() == 64` (the crowd-throttle budget — cite
     `MAX_SNAPSHOT_STATES` in a comment); `settle` ~1 s so the enters wave
     completes; clear `snapshot_bytes`; `settle` ~1 s more (≈ 10 snapshots);
     assert `snapshot_bytes` is non-empty, `bot.last_states.len()` is still
     64 (the worst case really was measured), and
     `snapshot_bytes.iter().max() ≤ 1100`.
  3. Note the fail-first property in the test doc comment: against the
     pre-rework wire format this exact scenario measures ~1.25 KB+ per frame
     and fails; it passes only because steps 1–4 landed. Green gate:
     `cargo test --workspace`, zero new warnings.
