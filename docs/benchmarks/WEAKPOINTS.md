# Structural weak points — fix before content builds on them

Companion to [BASELINE.md](BASELINE.md) (2026-07-03 numbers). The game is
pre-content (no real enemies/NPCs yet); this list is ranked by **how much
future code will sit on top of each pattern**, not by current cost. Compare
any fix against the saved criterion baseline:
`cargo bench -p vordar-benches -- --baseline main`.

## Fix now — patterns future code will copy

### 1. `world.get` random access inside hot loops
The codebase's dominant idiom: iterate a query, then do per-entity/per-pair
random component fetches inside the loop. Not one bottleneck — a habit every
future combat/AI/replication system will inherit by imitation.

Where it lives today:
- `smirk/engine-physics/src/narrowphase.rs` — 4 gets + `shape.clone()` per pair (~104 ns/pair measured)
- `game/vordar-game/src/motion/separation.rs` — 6 gets per pair (~200 ns/pair)
- `game/vordar-game/src/enemies/mod.rs:98` — per-enemy `Provoked` get
- `server/vordar-server/src/net_plugin.rs:732-735` — 2–3 gets per AOI candidate per client

Fix shape: batch/query-based access (archetype iteration, `Satisfies`/optional
query terms, or prefetch into scratch buffers). Fixing these four sets the
idiom. Benches: `physics/narrowphase/*`, `separation/*`, `enemy_ai/*`,
`snapshot/broadcast/*`.

### 2. Enemy targeting ignores the spatial grid — O(E·P)
`EnemyAISystem` (`game/vordar-game/src/enemies/mod.rs:68`) collects all player
positions and does a linear nearest-player scan per enemy: ~6 ns per
enemy×player pair → 1.6 ms at 1000×200, ~15 ms at 5000×500. Steepest curve in
the sim.

Why now: real enemies mean per-archetype behavior code written against
`BehaviorCtx`, fed by this targeting loop. Grid-based target selection before
that code exists means no behavior ever needs revisiting; retrofitting later
touches every archetype. Bench: `enemy_ai/*`.

### 3. Snapshot fan-out shares the sim thread — the clients-per-zone limiter
`SnapshotBroadcastSystem` (`server/vordar-server/src/net_plugin.rs:699`), O(C×A)
at 10 Hz. Measured 7.1 ms at 200 mutually-visible clients; soak at 400 bots
holds every average but input p99 hits 51 ms (budget 25) — the 10 Hz spike
stalls the 60 Hz phases. Ceiling today: ~200–300 mutually visible clients.

Cost is the AOI gather (per-candidate gets, known-set HashSet diff), not
serialization (encode = 1.3 µs/client). Options, cheapest structural first:
- **Stagger**: spread clients across the six 60 Hz ticks per snapshot period
  (turns one 7–28 ms spike into 6 slices) — likely first move.
- Make the gather cheaper (grid query with reused buffer, batched fetches) —
  overlaps with #1.
- Off-thread snapshot building (copy minimal state out per tick).

Decide while replication is one system; after more message types/replication
features exist this becomes a subsystem rewrite. Benches:
`snapshot/broadcast/*` + soak (`VORDAR_SOAK_BOTS`).

### 4. Snapshots ride reliable-ordered QUIC streams (head-of-line blocking)
`smirk/engine-net` uses one bi-directional stream for everything; a dropped
packet stalls all later snapshots for that client until retransmit. Position
updates are the classic case for unreliable datagrams. Client prediction/
interpolation code (`client/vordar-client/src/net.rs`) shapes itself around
delivery semantics — decide before more client netcode accumulates. Also noted
in exploration: `read_frame` does `buf.remove(0)` (O(len) memmove per message)
and broadcast clones the payload per connection (`engine-net/src/server.rs:149`).

## Localized — real, but nothing builds on top; fix when convenient

### 5. Broadphase all-pairs degeneration in dense cells
One 10×10 cell pile: 200 entities → 19 900 pairs, 3.2 ms chain; 500 → 124 750
pairs, ~23 ms (broadphase 9.8 + narrowphase 13) — over the whole 60 Hz budget
alone. Action-RPG piles (AoE farming, chokepoints) will hit this. Fix lives
entirely inside engine-physics behind an unchanged interface (pair budget,
finer cells, or per-cell sort/sweep). Benches: `physics/broadphase/cluster/*`,
`physics/narrowphase/cluster/*`.

## Not yet measured — gaps to close (2026-07-03)

### A. Prefab spawn cost — suspected hidden foul, bench first
`spawn_prefab` re-parses RON per component per spawn; `engine-core/src/prefab.rs`
justifies it with "never on a per-frame path" — but `spawn_projectile`
(`game/vordar-game/src/combat/projectile.rs:49`) goes straight through it, so
every bolt from every player and ranged enemy is a multi-component RON parse
at combat rate. Add to the suite: spawn+despawn churn of the `bolt` prefab
(N spawns/tick), plus the despawn side (`SpatialGrid::remove` is O(cell
occupancy) — death waves in a dense pile hit both). Seam-free. If confirmed,
the fix is caching parsed component builders per prefab.

### B. Client-side netcode — zero bench coverage today
The suite is all server/engine. Client hot spots, all headless-benchable as
plain systems fed fake `ServerMsg::Snapshot`s (A ∈ {64, 200} entities):
- `apply_snapshot` clones the entire id→pos map every snapshot
  (`client/vordar-client/src/net.rs:252`)
- reconciliation replay of up to 240 pending intents (`reconcile_own`,
  `net.rs:305`)
- `NetLerp` restart per remote entity per snapshot
Matters because the client runs on the weakest hardware in the system and
prediction is foundation code more netcode will layer onto.

### C. Packet-loss behavior — a check, not a bench (prerequisite for #4)
All existing measurements run on loopback QUIC, which never drops packets, so
the head-of-line blocking from reliable-stream snapshots (#4 above) is
invisible in every number we have. engine-net's client already simulates
latency (`connect_with_latency`); extend it to drop a percentage of frames,
then measure snapshot-age p99 / freeze duration at 1–5 % loss. The streams-vs-
datagrams decision should not be made without this evidence.

### D. Long-run growth soak
The 30 s soak window can't catch slow leaks (per-conn state, quinn buffers, DB
queue depth under autosave churn). Add an extended headless run (~1 h,
`VORDAR_SOAK_BOTS` reuse) asserting flat RSS and bounded collection sizes —
cheap insurance before content multiplies server state.

Deliberately skipped: GPU/render benches (manual feel-checks by design),
SQLite throughput (off-thread, single UPDATE per save), startup time, and
multi-zone contention (defer until multi-zone content exists).

## Confirmed non-issues (2026-07 baseline) — don't spend time here

- postcard encode/decode (1.3 µs per client snapshot; intent decode 16 ns)
- `select_states` under the 64 cap (53 ns pass-through)
- grid rebuild per tick (159 µs at 5000 entities)
- `MechanicResolveSystem` (37 µs at 200 conns per due mechanic)
- SQLite persistence (dedicated worker thread, sim never blocks)
- bandwidth (64-state cap holds ~11 KB/s per client flat even at 400 clients)
- `query_radius` allocation (~0.4 µs/call — switch the snapshot path to
  `query_radius_into` as hygiene while touching #3, not for speed)
- parry3d — removal justified by dependency weight/compile time only; the AABB
  math is invisible inside the per-pair fetch cost
