# Performance baseline

Measured costs of the paths that bound the two budgets:

- **Sim tick budget: 16.67 ms** (60 Hz) — collision chain, enemy AI, separation, movement.
- **Snapshot tick budget: 100 ms** (10 Hz, per client) — AOI gather + throttle + encode + send.
  Server-side, the gather work now runs on the 60 Hz `PostUpdate` phase, staggered across
  6 ticks (see "Architecture" below).

Together these determine max entities and max clients per zone.

The ranked to-fix list derived from these numbers lives in
[WEAKPOINTS.md](WEAKPOINTS.md) — as of this baseline, structural items #1–#4 are fixed;
only #5 (dense-cell broadphase) and gap D (long-run soak) remain open.

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

# Packet-loss probe (real QUIC, below-QUIC datagram drop, 50 ms and 200 ms simulated RTT):
cargo test -p vordar-server --release --test loss -- --ignored --nocapture
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
| Date | 2026-07-04 |

## Architecture note: staggered 60 Hz PostUpdate

`Phase::PostUpdate` now runs at a fixed 60 Hz instead of 10 Hz.
`SnapshotBroadcastSystem` serves 1/6 of connections per tick
(`conn_id % 6 == tick % 6`), so each client still receives exactly 10
snapshots/s, but the AOI-gather cost that used to land as one lump on a single
10 Hz tick is now spread across six 60 Hz ticks. `MechanicResolveSystem` and
`ZoneTransferSystem` self-gate to their original 10 Hz cadence with an internal
tick counter — their behavior is unchanged, only the phase they're driven by is
faster. This is why the soak `post_hz` line below now reads 60.0 instead of
10.0 — it reports the phase rate, not any single client's snapshot rate.

## Results (medians)

### Whole sim tick — `full_tick` (Physics + Prefab + CoreGame, no net)

| Scenario | Time/tick | Share of 16.67 ms |
|---|---|---|
| 200 enemies + 50 players | 157.8 µs | 0.9 % |
| 1000 enemies + 200 players | 478.1 µs | **2.9 %** |

(was 388 µs / 2.97 ms, 17.8 % — the view-idiom, grid-targeting, and compiled-prefab
fixes cut the worst case by ~84 %.)

### Collision chain — `physics` (60 Hz)

| Bench | N | Time | Notes |
|---|---|---|---|
| cell_update (full grid rebuild) | 200 / 1000 / 5000 | 11.7 µs / 57.3 µs / 303 µs | linear, cheap |
| broadphase uniform | 1000 / 5000 | 288 µs / 1.58 ms | realistic density (~2.5/cell) |
| broadphase cluster | 100 / 200 / 500 | 176 µs / 729 µs / 5.78 ms | 4 950 / 19 900 / 124 750 pairs — all-pairs inside the pile (unfixed, see WEAKPOINTS #5) |
| narrowphase cluster | 100 / 200 / 500 | 44.0 µs / 195 µs / 1.50 ms | ≈ **12 ns/pair** (was 104 ns/pair — view idiom removed 4 world.get + shape.clone per pair) |
| chain (all three) | uniform-1000 / uniform-5000 / cluster-200 | 438 µs / 2.52 ms / 1.11 ms | |

### Enemy AI — `enemy_ai` (60 Hz, grid-based targeting)

Idle enemies (aggro_range == 0) never query and cost is independent of player count:

| E | idle |
|---|---|
| 50 | 370 ns |
| 200 | 1.16 µs |
| 1000 | 5.42 µs |

Aggressive enemies (grid radius query, nearest by dist², not provoked):

| E × P | aggro |
|---|---|
| 200 × 1 | 950 ns |
| 200 × 200 | 124 µs |
| 1000 × 1 | 3.09 µs |
| 1000 × 200 | 834 µs |

Provoked / huge-radius fallback (global scan, unchanged O(E×P) — correct for this path, see plan):

| E × P | engaged |
|---|---|
| 200 × 1 | 1.42 µs |
| 200 × 200 | 229 µs |
| 1000 × 1 | 6.30 µs |
| 1000 × 200 | 1.17 ms |

O(E·P) is gone from the common (idle/aggro) case — idle cost no longer depends on P
at all, and aggro scales with grid cell occupancy, not total player count. The
`engaged` (provoked-only) path keeps the global scan by design; it's bounded by how
many enemies are actively provoked, which is small in practice.

### Separation — `separation` (60 Hz, per active pair)

| Active pairs | Time | Per pair |
|---|---|---|
| 367 | 18.2 µs | ~49.5 ns |
| 2 217 | 93.6 µs | ~42.2 ns |
| 8 713 | 335 µs | ~38.4 ns |

(was ~200–210 ns/pair — view idiom over `(&Transform, &Hitbox, Satisfies<&Solid>)`.)

### Spatial grid — `spatial_grid`

| Bench | Time |
|---|---|
| rebuild 200 / 1000 / 5000 | 6.6 µs / 32.0 µs / 169 µs |
| query r=40 (AOI), allocating | 1.19–1.21 µs |
| query r=40 (AOI), reused buffer (`query_radius_into`) | 0.89–0.93 µs |

### Prefab spawn — `prefab_spawn` (compiled spawn plans, gap A)

| Bench | Time | Notes |
|---|---|---|
| spawn/bolt | 677 ns | was 4.0 µs — RON parsed once per prefab, not per spawn |
| churn/n8 (spawn+despawn/tick) | 6.35 µs | |
| churn/n32 | 24.8 µs | was 126 µs |

### Client netcode — `client_netcode` (gap B)

| Bench | Time | Notes |
|---|---|---|
| apply_snapshot/states_a64 | 941 ns | was ~3.9 µs (4×, `mem::take` + view idiom) |
| apply_snapshot/states_a200 | 2.84 µs | was 10.9 µs |
| apply_snapshot/enters_64 | 50.9 µs | was 264 µs (mostly compiled prefab spawns) |
| reconcile/pending60 | 189 ns | |
| reconcile/pending240 | 575 ns | |

### Snapshot path — `snapshot` (server; broadcast = full 6-tick round covering every
conn once, comparable to the old un-staggered number; broadcast_slice = one 60 Hz
tick, i.e. the actual per-tick cost the sim thread pays)

| Bench | Time | Notes |
|---|---|---|
| broadcast (full round) 10 clients | 35.3 µs | |
| broadcast (full round) 50 clients | 326 µs | |
| broadcast (full round) 200 clients | 5.23 ms | was 7.11 ms un-staggered |
| broadcast (full round) 50 clients + 500 NPCs | 3.05 ms | |
| **broadcast_slice (per-tick) 200 clients** | **860 µs** | was the whole 7.11 ms landing on one tick — this is the number that matters for input-phase jitter |
| broadcast_slice 10 / 50 clients | 6.4 µs / 51.1 µs | |
| broadcast_slice 50 clients + 500 NPCs | 492 µs | |
| select_states A=64 / 200 / 1000 | 40.6 ns / 6.2 µs / 30.7 µs | pass-through under the 64 cap |
| mechanic_resolve 50 / 200 conns | 10.0 µs / 41.1 µs | per due mechanic, unchanged cadence |

### Protocol — `protocol` (postcard)

| Bench | Time |
|---|---|
| encode Snapshot (64 states + 8 enters/leaves, **1 153 B**) | 1.09 µs |
| decode same | 1.33 µs |
| encode / decode MoveIntent | 102 ns / 14.6 ns |

1 153 B × 10 Hz ≈ **11.3 KB/s per client** steady state — matches soak's measured
~11 KB/s/client and fits the 25 KB/s soak budget.

### Network macro — soak (real QUIC, wandering mutually-in-AOI crowd)

```
soak: bots=200 input_hz=60.0 input_p99_ms=17.66 post_hz=60.0 post_p99_ms=17.97 kb_s_per_client=10.8
soak: bots=400 input_hz=60.0 input_p99_ms=18.73 post_hz=60.0 post_p99_ms=20.31 kb_s_per_client=11.0
```

- **200 bots: passes** every budget, input p99 well under 25 ms.
- **400 bots: now passes too** — input p99 18.73 ms (was 51.25 ms pre-fix), post p99
  20.31 ms. The stagger fix (Phase 6) directly closed this gap: spreading the AOI
  gather across six 60 Hz ticks instead of one 10 Hz tick removed the spike that
  stalled the input phase.
- Bandwidth stays flat at ~11 KB/s/client at both crowd sizes (64-state cap holds).
- `post_hz` now reads 60.0 (the `PostUpdate` phase rate) rather than 10.0 (the
  per-client snapshot rate) — see the architecture note above.

### Packet-loss probe — `loss` (real QUIC, below-QUIC datagram drop, gap C)

**Downstream (server→client) inter-snapshot gaps**, one wandering mover + one lossy
observer, 30 s window per (RTT, loss) cell:

```
rtt= 50ms loss= 0%  snapshots=300  gap_ms p50=100 p99=116 max=117
rtt= 50ms loss= 1%  snapshots=300  gap_ms p50=98  p99=147 max=155
rtt= 50ms loss= 3%  snapshots=300  gap_ms p50=99  p99=159 max=163
rtt= 50ms loss= 5%  snapshots=300  gap_ms p50=98  p99=163 max=193
rtt=200ms loss= 0%  snapshots=300  gap_ms p50=96  p99=116 max=119
rtt=200ms loss= 1%  snapshots=300  gap_ms p50=97  p99=157 max=165
rtt=200ms loss= 3%  snapshots=300  gap_ms p50=101 p99=160 max=166
rtt=200ms loss= 5%  snapshots=300  gap_ms p50=101 p99=157 max=164
```

The 50 ms rows are the original 2026-07-11 measurement (kept for provenance). The
**200 ms rows are the pre-datagram baseline for rework 3** ("Every message class
rides one reliable ordered stream — head-of-line blocking by design",
`docs/reviews/plan-networking-rework-3-2026-07-13.md`), captured by that plan's
finding 1 before any datagram lane exists. Decision gate for the datagram snapshot
path (p99 > 250 ms or max > 500 ms at 1-5 % loss): at 200 ms RTT the worst observed
is p99=160 ms / max=166 ms (3 % loss) — the gate is **not breached** by this
measurement, which contradicts the plan's arithmetic expectation that one retransmit
cycle at WAN RTT would exceed it. The rework proceeds regardless: the mechanism the
gate approximates — a single lost packet still stalls every later snapshot on the
one reliable stream until the retransmit lands — is visible directly in these rows
(loss pushes max up 40-75 ms over the 0 %-loss floor at both RTTs) even though the
numeric threshold holds. These rows are what rework 3's final after-probe compares
against.

**Upstream (client→server) applied-intent lag**
(`Snapshot::last_processed_seq` vs the bot's own send counter, in ticks),
8 s window per (RTT, loss) cell:

```
rtt= 50ms upstream loss= 0%  lag p50=6  p99=9  max=9
rtt= 50ms upstream loss= 1%  lag p50=6  p99=9  max=9
rtt= 50ms upstream loss= 3%  lag p50=6  p99=9  max=10
rtt= 50ms upstream loss= 5%  lag p50=7  p99=10 max=10
rtt= 50ms upstream loss=60%  lag p50=15 p99=38 max=41
rtt=200ms upstream loss= 0%  lag p50=13 p99=16 max=16
rtt=200ms upstream loss= 1%  lag p50=13 p99=16 max=16
rtt=200ms upstream loss= 3%  lag p50=13 p99=16 max=17
rtt=200ms upstream loss= 5%  lag p50=13 p99=16 max=17
rtt=200ms upstream loss=60%  lag p50=21 p99=43 max=46
```

At realistic WAN loss (0-5 %) applied-intent lag stays within a couple of ticks of
the 0 %-loss baseline at both RTTs — QUIC's retransmission recovers within roughly
one snapshot period at these rates, same conclusion the original 50 ms-only probe
reached. `EXTREME_LOSS` (60 %) exists only to prove the stall mechanism is real:
lag roughly quadruples over baseline at both RTTs (41 vs 9 ticks at 50 ms RTT; 46
vs 16 ticks at 200 ms RTT).

Re-probe if RTT or loss assumptions change materially (e.g. mobile/satellite
clients above 200 ms).

## Budget shares & where the limits are

At the target load (200-player crowd, ~200 NPCs, one zone) **everything fits with
enormous headroom**: the whole sim tick is under 3 % of budget even at 1000+200
entities, and the 200-client snapshot fan-out's per-tick cost (0.86 ms) is under 1 %
of its own 100 ms budget.

Ranked by how soon each path becomes the limiter, post-fix:

1. **Dense single-cell crowds (collision chain) — the only remaining measured
   cliff, WEAKPOINTS #5, deliberately out of scope for this pass.** A 500-entity
   pile still costs 5.8 ms (broadphase) + 1.5 ms (narrowphase) ≈ 7.3 ms — down from
   ~23 ms pre-fix (the narrowphase view idiom helped here too) but still the thing
   to watch for chokepoints and boss piles.
2. **Not bottlenecks, confirmed at this baseline:** snapshot fan-out (400-bot soak
   now passes at 18.7 ms p99 vs a 25 ms budget), enemy AI (idle/aggro paths no
   longer scale with player count), separation (~40 ns/pair), prefab spawn (677 ns/
   spawn), client apply_snapshot (941 ns at A=64), grid rebuild/query, postcard
   encode/decode, packet loss up to 5 % (164 ms worst-case gap vs a 250/500 ms gate).

Implication for future work: with structural items #1–#4 fixed, the sim has broad
headroom at the design point (200 players, 200 NPCs). The next real constraint is
content-driven — dense-cell piles (#5) and whatever load real enemy/ability code
adds on top of the now-flat baseline above.
