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

# Remote render smoothness probe (real QUIC, WAN-impaired observer):
cargo test -p vordar-client --release -- --ignored --nocapture remote_render_smoothness_under_loss_probe
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
| reconcile/pending60_statics32 | 20.7 µs | 32-static collision: ~110× pend60, replay per intent |
| reconcile/pending240_statics32 | 81.7 µs | 32-static collision: ~142× pend240, replay per intent |

### Render CPU — `render_cpu` (VQ-F1 stress figure: 40 rigs × 64 joints)

| Bench | Before | After | Notes |
|---|---|---|---|
| joint_palette_40x64 | 105.03 µs | 105.44 µs | unchanged (noise, p = 0.23) |

Rendering finding 10: `pose_player`, the per-entity posing path `MeshRenderSyncSystem`
runs every display frame, allocated five fresh `Vec`s per skinned instance (sample,
blend, the two buffers inside `global_transforms`, the palette) plus a `String` clone
for an active crossfade — ~200 heap allocations/frame at this stress figure.
`sample_pose`/`global_transforms` gained `_into` out-parameter twins and kept their
old allocating signatures as thin wrappers (this bench calls those wrappers directly,
which is why the number above doesn't move — the bench never exercised the per-frame
hot path). `pose_player` was rewired into `pose_player_into`, fed by a `PoseScratch`
owned by `MeshRenderSyncSystem` and reused across entities and frames; the prev-clip
lookup for crossfades now borrows `player.prev`'s name instead of cloning it. Verified
by `pose_player_into_stops_growing_scratch_buffers_after_warmup`
(`engine-renderer`, `mesh::sync`): scratch buffer capacity is unchanged across five
repeated calls once warmed, i.e. zero further allocations in steady state.

### Asset streaming — `asset_load` (rendering rework 2)

| Bench | Before |
|---|---|
| first_sight/statue_vroid (11 MB, embedded textures) | 122.11 ms |
| first_sight/human (9 MB, skinned + clips) | 101.63 ms |
| zone_ground/decode_and_generate (3× 2k JPG decode + mesh gen) | 246.27 ms |

These are the synchronous costs blocking the frame during zone entry and dressing.
Rework 2 moves these off-frame via streaming: `load_gltf_data` and
`load_ground_material` become async tasks, uploads (`upload_mesh`, indirect GPU
setup) stay synchronous but run on a small pool instead of the frame's main thread.
The environment load baseline (rework 7 finding 7): ~24 ms per `set_uniform_environment`,
241 ms/10 loads in the zone-change path (steps 2–3 re-measure this after the refactor).
Rendering rework 2 finding 2 hoists `Baker::new` (shader module + all four bake
pipelines) to compile once per device instead of once per environment load, sharing
it the way `bake_brdf_lut`'s LUT already was. Re-measured after the hoist: ~20 ms per
`set_uniform_environment` (187–230 ms/10 loads across three runs) — a smaller drop
than the ~9–10 ms `Baker::new` isolation predicted (expected ~14 ms/load). The hoist
is verified correct regardless (`repeated_environment_loads_skip_redundant_baker_construction`
asserts zero `Baker` reconstructions across 3 reloads); the remaining ~20 ms per load
is the cubemap/irradiance/prefilter bake work itself (216 render passes), not pipeline
compilation.

Rendering rework 2 finding 3 threads one `CommandEncoder` through all 42 bake passes
(6 equirect + 6 irradiance + 30 prefilter faces) per environment load, replacing the
per-face encoder/submit round trip with a single `queue.submit` at the end — the
same-encoder write-then-sample ordering (equirect passes write the base cubemap,
irradiance/prefilter passes sample it) needed no fallback: wgpu's automatic usage
transitions between render passes made it legal, and the full offscreen suite (furnace,
reflection, sky) stayed green. Re-measured: ~6.5-7.3 ms per `set_uniform_environment`
(three runs), down from ~20 ms — the per-submit overhead was indeed the dominant cost.

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

### Packet-loss probe — `loss` (real QUIC, below-QUIC datagram drop, gap C;
before/after networking rework 3, `docs/reviews/networking/plan-networking-rework-3-2026-07-13.md`)

**Downstream (server→client) inter-snapshot gaps**, one wandering mover + one lossy
observer, 30 s window per (RTT, loss) cell.

**Before** (`ServerMsg::Snapshot` on the one reliable stream — rework 3's finding 1,
captured before any datagram lane exists; 50 ms rows are the original 2026-07-11
measurement, kept for provenance):

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

Decision gate for the datagram snapshot path (p99 > 250 ms or max > 500 ms at
1-5 % loss): at 200 ms RTT the worst observed was p99=160 ms / max=166 ms (3 % loss)
— the gate was **not breached** by this before-measurement, which contradicted the
plan's arithmetic expectation that one retransmit cycle at WAN RTT would exceed it.
The rework proceeded regardless: the mechanism the gate approximates — a single lost
packet stalling every later snapshot on the one reliable stream until the retransmit
lands — was visible directly in these rows (loss pushed max up 40-75 ms over the
0 %-loss floor at both RTTs) even though the numeric threshold held.

**After** (`ServerMsg::Snapshot` on a datagram, latest-wins by tick — rework 3
findings 2-5 landed; `loss_probe_inter_snapshot_gaps` now asserts `p99 <= 250 ms` at
every cell as a permanent regression gate):

```
rtt= 50ms loss= 0%  snapshots=300  gap_ms p50=99  p99=113 max=115
rtt= 50ms loss= 1%  snapshots=297  gap_ms p50=96  p99=200 max=203
rtt= 50ms loss= 3%  snapshots=284  gap_ms p50=100 p99=207 max=300
rtt= 50ms loss= 5%  snapshots=277  gap_ms p50=97  p99=204 max=298
rtt=200ms loss= 0%  snapshots=300  gap_ms p50=101 p99=115 max=119
rtt=200ms loss= 1%  snapshots=296  gap_ms p50=99  p99=197 max=202
rtt=200ms loss= 3%  snapshots=284  gap_ms p50=100 p99=208 max=302
rtt=200ms loss= 5%  snapshots=278  gap_ms p50=99  p99=204 max=294
```

p99 stays comfortably inside the 250 ms gate at both RTTs (worst 208 ms), and — the
point of the rework — it no longer depends on RTT at all: a lost datagram is simply
skipped, so gaps settle at cadence multiples (~100 ms with no consecutive loss,
~200 ms with one, ~300 ms with two in a row) regardless of how long a retransmit
would have taken. Max is *higher* than the before numbers at 3-5 % loss (298-302 ms
vs 163-193 ms) — expected and correct: the old stream redelivered a lost snapshot
after one retransmit cycle (bounded by RTT) and every later snapshot queued behind
it; the new datagram path never redelivers, so a rare run of 2-3 consecutive losses
costs 2-3 cadence periods outright instead of one bounded stall. Both stay far under
the 500 ms max gate.

**Upstream (client→server) applied-intent lag**
(`Snapshot::last_processed_seq` vs the bot's own send counter, in ticks),
8 s window per (RTT, loss) cell.

**Before** (`ClientMsg::MoveIntent`, one per tick on the one reliable stream —
rework 3's finding 1):

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

At realistic WAN loss (0-5 %) applied-intent lag stayed within a couple of ticks of
the 0 %-loss baseline at both RTTs — QUIC's retransmission recovers within roughly
one snapshot period at these rates. `EXTREME_LOSS` (60 %) proved the stall mechanism
was real: lag roughly quadrupled over baseline at both RTTs (41 vs 9 ticks at 50 ms
RTT; 46 vs 16 ticks at 200 ms RTT).

**After** (`ClientMsg::MoveIntents`, last-3 redundancy on a datagram — rework 3
finding 5 landed; `loss_probe_upstream_intent_lag` now asserts `EXTREME_LOSS` lag
stays `> baseline` (impairment still reaches the transport) but `< baseline * 3`
(redundancy bounds the damage) as a permanent regression gate):

```
rtt= 50ms upstream loss= 0%  lag p50=6  p99=9  max=9
rtt= 50ms upstream loss= 1%  lag p50=6  p99=9  max=9
rtt= 50ms upstream loss= 3%  lag p50=6  p99=9  max=10
rtt= 50ms upstream loss= 5%  lag p50=6  p99=9  max=10
rtt= 50ms upstream loss=60%  lag p50=8  p99=14 max=14
rtt=200ms upstream loss= 0%  lag p50=13 p99=16 max=17
rtt=200ms upstream loss= 1%  lag p50=13 p99=16 max=17
rtt=200ms upstream loss= 3%  lag p50=13 p99=16 max=16
rtt=200ms upstream loss= 5%  lag p50=13 p99=16 max=17
rtt=200ms upstream loss=60%  lag p50=15 p99=20 max=22
```

The improvement is the point of finding 5: at `EXTREME_LOSS` (60 %), applied-intent
lag now stays within ~1.5x of the 0 %-loss baseline at both RTTs (14 vs 9 ticks at
50 ms RTT; 22 vs 17 ticks at 200 ms RTT) instead of roughly quadrupling — last-3
redundancy means a seq is only truly lost if all three datagrams carrying it drop,
so the applied ack keeps pace with the send counter instead of queuing behind a
stalled reliable stream.

Re-probe if RTT or loss assumptions change materially (e.g. mobile/satellite
clients above 200 ms).

### Remote render smoothness probe — `remote_render_smoothness_under_loss_probe`
(client, real QUIC; networking rework 4,
`docs/reviews/networking/plan-networking-rework-4-2026-07-14.md` finding 3)

The gap-C loss probes above measure arrival gaps and intent-ack lag only —
neither says what a player actually SEES. This probe drives a real headless
server, a real unimpaired "mover" bot walking ±X at 6 u/s (reversing every
~2.17 s to stay in the 40-unit AOI), and a real WAN-impaired "observer"
(100 ms RTT, 30 ms jitter, 3 % downstream loss) running the actual client
systems (`NetReceiveSystem` + `NetInterpolateSystem`, `predict: false`) — it
records the mover entity's rendered `Transform.position` after every Update
tick over a 20 s window and asserts two permanent regression gates:

- the longest run of consecutive zero-motion ticks (step < 1e-4) is `<= 5`
  ticks (~83 ms — the pre-rework-4 client froze 10-18 ticks at every
  late/lost snapshot instead);
- p99 per-tick step is `<= 1.5x` nominal (0.15 u at 6 u/s/60 Hz — the
  pre-rework-4 client's catch-up steps ran ~2x).

```
remote render smoothness: ticks=1200 step_u p50=0.1094 p99=0.1102 max=0.1467 longest_zero_run=0
```

Both gates pass with wide margin (p50/p99 sit almost exactly on the 0.1 u
nominal step, and the window recorded zero freeze ticks at all in this run —
the tick-indexed playback buffer (finding 1) and capped extrapolation
(finding 2) absorb the WAN jitter/loss entirely inside interpolation). Sanity
checked by temporarily zeroing `INTERP_DELAY_TICKS` (no buffer slack at all):
the probe fails hard (p99 rises to ~0.30 u, close to 3x nominal), confirming
the gates actually detect the freeze/warble regime they're meant to catch.

Run: `cargo test -p vordar-client --release -- --ignored --nocapture
remote_render_smoothness_under_loss_probe`.

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
   encode/decode, packet loss up to 5 % at both 50 ms and 200 ms RTT (208 ms p99 /
   302 ms max worst-case datagram-snapshot gap vs a 250/500 ms gate — rework 3).

Implication for future work: with structural items #1–#4 fixed, the sim has broad
headroom at the design point (200 players, 200 NPCs). The next real constraint is
content-driven — dense-cell piles (#5) and whatever load real enemy/ability code
adds on top of the now-flat baseline above.
