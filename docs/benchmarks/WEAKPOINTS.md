# Structural weak points — fix before content builds on them

Companion to [BASELINE.md](BASELINE.md). The game is pre-content (no real
enemies/NPCs yet). Original list (2026-07-03) ranked patterns by how much future
code would copy them, not by current cost. Compare any fix against the saved
criterion baseline: `cargo bench -p vordar-benches -- --baseline main`.

## Fixed (2026-07)

Items #1–#4 below and gaps A/B/C were fixed across a structured pass
(2026-07-03 → 2026-07-04). Full before/after numbers live in
[BASELINE.md](BASELINE.md); summary here.

### 1. `world.get` random access inside hot loops — fixed
Replaced per-entity/per-pair `world.get` fans with a hecs query **view** created
once per system run (`let mut q = world.query::<(...)>(); let view = q.view();`
then `view.get(entity)`), across all four sites:
- `smirk/engine-physics/src/narrowphase.rs` — 104 → **12 ns/pair**
- `game/vordar-game/src/motion/separation.rs` — ~205 → **~40 ns/pair**
- `game/vordar-game/src/enemies/mod.rs` — folded into the #2 fix below
- `server/vordar-server/src/net_plugin.rs` — folded into the #3 fix below

### 2. Enemy targeting ignores the spatial grid — fixed
`EnemyAISystem` now splits into three paths: passive enemies (aggro_range == 0)
idle without querying; aggressive enemies use `grid.query_radius_into` + nearest-
by-dist² over a `(&Transform, &Player)` view; provoked enemies (or aggro_range
above a ~50-unit grid-efficiency threshold) keep the global scan, since that
path is bounded by how many enemies are actively provoked, not total player
count. Idle cost no longer depends on P at all; 1000×200 idle went from 1.18 ms
to 461 µs even before accounting for the fact it's now O(E) not O(E×P).

### 3. Snapshot fan-out shares the sim thread — fixed
`Phase::PostUpdate` runs at a fixed 60 Hz; `SnapshotBroadcastSystem` serves
`conn_id % 6 == tick % 6` each tick, so every client still gets exactly 10
snapshots/s but the AOI-gather cost spreads across six ticks instead of landing
as one lump. `MechanicResolveSystem`/`ZoneTransferSystem` self-gate to their
original 10 Hz cadence via an internal counter — cadence unchanged, only the
driving phase is faster. Combined with the cheap-gather fix (`query_cells_overlapping_into`
+ reused scratch buffers + one `(&Transform, &PrefabId)` view + `mem::swap` on
the known-set): **soak 400-bot input p99 went from 51.25 ms to 18.73 ms**
(budget 25 ms) — the gate that defined this item is now passed.

### 4. Snapshots ride reliable-ordered QUIC streams (head-of-line blocking) — fixed
Gap C's below-QUIC loss probe (see below) measured the actual head-of-line cost at
50 ms RTT: even at 5 % receive-side loss, the worst inter-snapshot gap was 164 ms —
under one retransmit cycle, absorbed by the 100 ms snapshot cadence. Re-probed at
200 ms (WAN) RTT by networking rework 3
(`docs/reviews/networking/plan-networking-rework-3-2026-07-13.md`), the decision gate (p99 >
250 ms or max > 500 ms) still was not numerically breached, but the underlying
mechanism it approximates — a single lost packet stalling every later snapshot on
the one reliable stream until the retransmit lands — was real (loss pushed max up
40-75 ms over the 0 %-loss floor at both RTTs), so the rework built the datagram
snapshot path anyway: `ServerMsg::Snapshot` (states + intent ack) now rides an
unreliable QUIC datagram, latest-wins by a per-connection tick guard, while identity
(`AoiDelta`: enters/leaves) stays on the reliable stream. After-probe numbers
(`docs/benchmarks/BASELINE.md`) show gaps bound to cadence multiples (p99 ≤ 208 ms)
regardless of RTT, and move intents gained matching last-3 redundancy on their own
datagram lane. The `read_frame`/broadcast hygiene fixes noted in the original
writeup (O(len) `buf.remove(0)`, per-connection payload clone) were fixed
regardless — see gap C.

### A. Prefab spawn cost — measured and fixed
`ComponentLoader` now parses each component's RON once per prefab and produces a
`CompiledComponent` closure that clones a pre-parsed value at spawn time, instead
of re-parsing RON on every spawn. `prefab/spawn/bolt`: 4.0 µs → **677 ns**;
`prefab/churn/n32`: 126 µs → **24.8 µs**.

### B. Client-side netcode — measured and fixed
`apply_snapshot` now uses `mem::take` instead of cloning the entire id→pos map,
and a `(&mut NetLerp, &Transform)` view instead of per-entity gets; own-player
state is extracted before the view runs so `reconcile_own` still gets `&mut
World`. `client/apply_snapshot/states_a200`: 10.9 µs → **2.84 µs**;
`enters_64`: 264 µs → **50.9 µs** (mostly gap A's compiled spawns).

### C. Packet-loss behavior — measured (this is what closed #4 above)
Loss is simulated below QUIC — `LossySocket` wraps the client's
`AsyncUdpSocket` and drops received datagrams with probability p via a
deterministic LCG — because dropping frames above QUIC can't reproduce genuine
retransmission stalls. `tests/loss.rs` (`#[ignore]`) probes inter-snapshot gap
p50/p99/max at 0/1/3/5 % loss; results and the resulting #4 decision are in
BASELINE.md. Hygiene fixes landed alongside: `read_frame` no longer does an
O(len) `buf.remove(0)` per message, and broadcast payloads are `Arc<Vec<u8>>`
(one encode, refcount bump per connection, not a clone).

## Localized — real, but nothing builds on top; fix when convenient

### 5. Broadphase all-pairs degeneration in dense cells — still open
One 10×10 cell pile: 200 entities → 19 900 pairs, ~0.9 ms chain; 500 → 124 750
pairs, ~7.3 ms (broadphase 5.8 ms + narrowphase 1.5 ms) — down from ~23 ms
pre-fix (the narrowphase view idiom helped even here) but still the one
scenario that can eat the whole 60 Hz budget on its own. Action-RPG piles (AoE
farming, chokepoints) will hit this. Fix lives entirely inside engine-physics
behind an unchanged interface (pair budget, finer cells, or per-cell sort/sweep).
Deliberately out of scope for the 2026-07 pass. Benches:
`physics/broadphase/cluster/*`, `physics/narrowphase/cluster/*`.

## Not yet measured — gap still open (2026-07-04)

### D. Long-run growth soak
The 30 s soak window can't catch slow leaks (per-conn state, quinn buffers, DB
queue depth under autosave churn). Add an extended headless run (~1 h,
`VORDAR_SOAK_BOTS` reuse) asserting flat RSS and bounded collection sizes —
cheap insurance before content multiplies server state.

Deliberately skipped: GPU/render benches (manual feel-checks by design),
SQLite throughput (off-thread, single UPDATE per save), startup time, and
multi-zone contention (defer until multi-zone content exists).

## Confirmed non-issues (2026-07-04 baseline) — don't spend time here

- postcard encode/decode (1.1 µs per client snapshot; intent decode 15 ns)
- `select_states` under the 64 cap (41 ns pass-through)
- grid rebuild per tick (169 µs at 5000 entities)
- `MechanicResolveSystem` (41 µs at 200 conns per due mechanic, cadence unchanged)
- SQLite persistence (dedicated worker thread, sim never blocks)
- bandwidth (64-state cap holds ~11 KB/s per client flat even at 400 clients)
- packet loss up to 5 % with 50 ms RTT (164 ms worst-case gap vs a 250/500 ms gate)
- parry3d — removal justified by dependency weight/compile time only; the AABB
  math is invisible inside the per-pair fetch cost
