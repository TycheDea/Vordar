# Performance baseline

Measured costs of the paths that bound the two budgets:

- **Sim tick budget: 16.67 ms** (60 Hz) — collision chain, enemy AI, separation, movement.
- **Snapshot tick budget: 100 ms** (10 Hz) — per-client AOI gather + throttle + encode + send.

Together these determine max entities and max clients per zone.

The ranked to-fix list derived from these numbers lives in
[WEAKPOINTS.md](WEAKPOINTS.md).

## How to run

```sh
# Full suite (~6 min), recording a named criterion baseline:
cargo bench -p vordar-benches -- --save-baseline main

# After an optimization, compare against it:
cargo bench -p vordar-benches -- --baseline main

# Fast iteration smoke:
cargo bench -p vordar-benches -- --quick

# Network macro (real QUIC, bot clients), 200 then 400 bots:
cargo test -p vordar-server --release --test soak -- --ignored --nocapture
VORDAR_SOAK_BOTS=400 cargo test -p vordar-server --release --test soak -- --ignored --nocapture
```

Criterion's raw baselines live in `target/criterion` (gitignored); this file is
the durable record. Update it after any change that moves a number.

## Machine

| | |
|---|---|
| CPU | 12th Gen Intel Core i7-12700 (12 cores / 20 threads) |
| RAM | 32 GB |
| OS | Windows 11 Pro |
| rustc | 1.94.0 |
| Date | 2026-07-03 |

## Results (medians)

### Whole sim tick — `full_tick` (Physics + Prefab + CoreGame, no net)

| Scenario | Time/tick | Share of 16.67 ms |
|---|---|---|
| 200 enemies + 50 players | 388 µs | 2.3 % |
| 1000 enemies + 200 players | 2.97 ms | **17.8 %** |

### Collision chain — `physics` (60 Hz)

| Bench | N | Time | Notes |
|---|---|---|---|
| cell_update (full grid rebuild) | 200 / 1000 / 5000 | 15.6 µs / 78 µs / 424 µs | linear, cheap |
| broadphase uniform | 1000 / 5000 | 443 µs / 2.41 ms | realistic density (~2.5/cell) |
| broadphase cluster | 100 / 200 / 500 | 259 µs / 1.18 ms / 9.84 ms | 4 950 / 19 900 / 124 750 pairs — all-pairs inside the pile |
| narrowphase cluster | 100 / 200 / 500 | 545 µs / 2.13 ms / **12.97 ms** | ≈ 104 ns/pair (4× world.get + shape.clone) |
| chain (all three) | uniform-1000 / uniform-5000 / cluster-200 | 1.16 ms / 6.17 ms / 3.22 ms | |

### Enemy AI — `enemy_ai` (60 Hz, O(E·P) scan)

| E × P | idle | engaged (melee chase) |
|---|---|---|
| 200 × 1 | 3.9 µs | 7.6 µs |
| 200 × 200 | 293 µs | 256 µs |
| 1000 × 50 | 286 µs | 402 µs |
| 1000 × 200 | 1.18 ms | **1.56 ms** |

Scan cost ≈ 6 ns per enemy×player pair, plus ~20 ns/enemy fixed (Provoked lookup etc.).

### Separation — `separation` (60 Hz, per active pair)

| Active pairs | Time | Per pair |
|---|---|---|
| 367 | 77 µs | ~210 ns |
| 2 217 | 449 µs | ~202 ns |
| 8 713 | 1.71 ms | ~196 ns |

### Spatial grid — `spatial_grid`

| Bench | Time |
|---|---|
| rebuild 200 / 1000 / 5000 | 6.1 µs / 29 µs / 159 µs |
| query r=40 (AOI), allocating | 1.29–1.64 µs |
| query r=40 (AOI), reused buffer | 0.90–1.06 µs |

### Snapshot path — `snapshot` (10 Hz, server only)

| Bench | Time | Notes |
|---|---|---|
| broadcast 10 clients | 36.8 µs | ~3.7 µs/client |
| broadcast 50 clients | 405 µs | ~8.1 µs/client |
| broadcast 200 clients | **7.11 ms** | ~35.5 µs/client — superlinear: cost is C×A and A grows with the crowd |
| broadcast 50 clients + 500 NPCs | 4.28 ms | ~86 µs/client at A=550 |
| select_states A=64 / 200 / 1000 | 53 ns / 5.9 µs / 28.1 µs | pass-through under the 64 cap |
| mechanic_resolve 50 / 200 conns | 8.9 µs / 37.6 µs | per due mechanic, full 32-tick rewind per target |

### Protocol — `protocol` (postcard)

| Bench | Time |
|---|---|
| encode Snapshot (64 states + 8 enters/leaves, **1 153 B**) | 1.34 µs |
| decode same | 1.51 µs |
| encode / decode MoveIntent | 125 ns / 16 ns |

1 153 B × 10 Hz ≈ **11.3 KB/s per client** steady state — matches the design
estimate in net_plugin.rs and fits the 25 KB/s soak budget.

### Network macro — soak (real QUIC, wandering mutually-in-AOI crowd)

```
soak: bots=200 input_hz=60.0 input_p99_ms=18.66 post_hz=10.0 post_p99_ms=112.75 kb_s_per_client=10.8
soak: bots=400 input_hz=60.0 input_p99_ms=51.25 post_hz=10.0 post_p99_ms=111.66 kb_s_per_client=10.9   (FAILS the 25 ms input-p99 budget)
```

- **200 bots: passes** every budget (walker covered 27.6 of a 30.0 free path).
- **400 bots: the found limit.** Average rates still hold (60 Hz input,
  10 Hz snapshots) and bandwidth stays flat at ~11 KB/s/client (the 64-state
  cap works), but input p99 doubles the 25 ms budget — the 10 Hz PostUpdate
  spike stalls the 60 Hz input phase. That matches the criterion curve:
  broadcast at C=200 mutually visible is 7.1 ms, and C×A extrapolation puts
  C=400 at ~28 ms per snapshot tick, plus the denser 400-crowd collision work.
- So on this machine the current server holds **~200–300 mutually-visible
  clients per zone**; the binding constraint is tick *jitter* from the
  snapshot fan-out sharing the sim thread, not bandwidth or average rate.

## Budget shares & where the limits are

At the target load (200-player crowd, ~200 NPCs, one zone) **everything fits
comfortably**: the whole sim tick is ~18 % of budget even at 1000+200 entities,
and the 200-client snapshot fan-out uses 7 % of its 100 ms budget.

Ranked by how soon each path becomes the limiter:

1. **Dense single-cell crowds (collision chain) — the only measured cliff.**
   Inside one 10×10 cell the broadphase degenerates to all-pairs: a 500-entity
   pile costs 9.8 ms (broadphase) + 13.0 ms (narrowphase) ≈ 23 ms — **over the
   entire 60 Hz budget on its own**. At 200 in a pile (the soak design point)
   the chain is 3.2 ms — survivable but 10× the uniform cost. Chokepoints and
   boss piles are the scenario to watch. Levers when needed: pair budget/cap,
   finer cells, or cheaper narrowphase per pair (it's ~104 ns/pair, dominated
   by 4 random `world.get`s + a `shape.clone`, not the math).
2. **Snapshot fan-out (server, 10 Hz).** C×A scaling measured: 200 mutually
   visible clients → 7.1 ms per snapshot tick on the sim thread. It competes
   with the 60 Hz phases long before its own 100 ms budget runs out — the
   400-bot soak confirms it: input p99 blows to 51 ms while every average
   still holds. Per-client cost at A=550 is ~86 µs, of which encode is only
   1.3 µs — the cost is the AOI gather's per-candidate `world.get`s and the
   known-set diff, not serialization.
3. **Enemy AI O(E·P).** 1.6 ms at 1000×200 (9 % of budget) — fine today, but
   it's the steepest curve in the sim: 5000 NPCs × 500 players would be
   ~15 ms. When NPC counts grow past a few thousand per zone, nearest-player
   selection needs the spatial grid (it currently ignores it).
4. **Separation ~200 ns/pair** — only matters via the same dense-crowd pair
   explosion as #1; at 2 217 pairs it's 0.45 ms.
5. **Not bottlenecks:** grid rebuild (159 µs @ 5000), AOI grid query (~1 µs),
   `query_radius` allocation (~0.4 µs/call — switching the snapshot path to
   `query_radius_into` saves ~0.1 ms/s at 200 clients; do it for hygiene, not
   speed), postcard encode/decode, mechanic resolve (37 µs @ 200 conns),
   select_states under the 64-state cap (53 ns).

Implication for planned work: removing parry3d is justified by dependency
weight and compile time, **not** by these numbers — the AABB test is invisible
inside the 104 ns/pair narrowphase cost. An FxHashMap swap would shave the
HashSet-heavy broadphase/narrowphase constants; re-run `physics/*` and
`snapshot/broadcast/*` against baseline `main` to quantify.
