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

# Profiling (flamegraph): samples the real bench process via ETW and opens
# an interactive flame graph in the browser. `[profile.bench] debug = true`
# (root Cargo.toml) is what lets the sampled stacks resolve to function names.
cargo install samply
samply record -- cargo bench -p vordar-benches --bench full_tick -- --profile-time 5
```

Verified: the command above recorded 8 911 stack samples on the `full_tick`
process's main thread — samply's ETW-based sampler ran fine from an
unelevated shell on this box. `cargo flamegraph` (`cargo install flamegraph`,
then `cargo flamegraph --bench full_tick -p vordar-benches -- --bench
--profile-time 10`) is the more commonly documented recipe, but its
blondie/ETW backend errored `NotAnAdmin` here; it needs a shell opened as
Administrator. Either recipe's ETW capture can be swapped for Superluminal's
CLI if you have a license.

Criterion's raw baselines live in `target/criterion` (gitignored); this file is
the durable record. Update it after any change that moves a number.

## Regression gate

`scripts/bench-gate.ps1` wraps the `--baseline main` comparison into a pass/fail
check: it runs one bench target against the saved `main` baseline, reads
criterion's per-bench `change/estimates.json` (`mean.point_estimate` as the
relative change) for every bench instance the run touched, prints a table, and
exits 1 if any bench's mean regressed by more than `-Threshold` (default 0.10).
Both the baseline save and the gate run need a quiet box: measured run-to-run
noise reaches ±25% with a game hogging CPU in the background, which swamps the
10% threshold in either direction.

```powershell
# Save/refresh the baseline before first use, or after an accepted perf change:
cargo bench -p vordar-benches --bench snapshot -- --save-baseline main

# Gate a bench target against it:
powershell scripts/bench-gate.ps1 -Bench snapshot -Threshold 0.10
```

Each run appends one line to `docs/benchmarks/gate-log.txt` (date, bench, max
delta, whether it fired) — a durable history of gate runs, since the criterion
baselines themselves are gitignored.

## Machine

| | |
|---|---|
| CPU | 12th Gen Intel Core i7-12700 (12 cores / 20 threads) |
| RAM | 32 GB |
| OS | Windows 11 Pro |
| rustc | 1.97.1 |
| Date | 2026-07-18 |

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
| 200 enemies + 50 players | 132.1 µs | 0.8 % |
| 1000 enemies + 200 players | 1.586 ms | **9.5 %** |

(was 388 µs / 2.97 ms, 17.8 % pre-fix — the view-idiom, grid-targeting, and
compiled-prefab fixes cut that worst case by ~84 %; the 1000+200 figure has since
climbed from 478.1 µs/2.9 % over the same span `CoreGamePlugin` picked up
`XpGrantSystem` and other `CollisionResolve`/`Update` systems this scenario now
pays for every tick.)

### Collision chain — `physics` (60 Hz)

| Bench | N | Time | Notes |
|---|---|---|---|
| cell_update (full grid rebuild) | 200 / 1000 / 5000 | 5.82 µs / 29.1 µs / 147 µs | linear, cheap |
| cell_update_moving (grid re-bucketing) | 200 / 1000 / 5000 | 15.1 µs / 86.4 µs / 461 µs | every entity changes cell |
| broadphase uniform | 1000 / 5000 | 201 µs / 1.23 ms | realistic density (~2.5/cell) |
| broadphase cluster | 100 / 200 / 500 | 122 µs / 532 µs / 4.41 ms | 4 950 / 19 900 / 124 750 pairs — all-pairs inside the pile (unfixed, see WEAKPOINTS #5) |
| narrowphase cluster | 100 / 200 / 500 | 54.4 µs / 235 µs / 1.53 ms | ≈ **12 ns/pair** (was 104 ns/pair pre-fix — view idiom removed 4 world.get + shape.clone per pair) |
| chain (all three) | uniform-1000 / uniform-5000 / cluster-200 | 280 µs / 1.64 ms / 790 µs | |

### Enemy AI — `enemy_ai` (60 Hz, grid-based targeting)

Idle enemies (aggro_range == 0): below `GRID_PLAYER_MIN` (64) players the O(E×P)
scan fallback still applies (empty result, but every enemy still scans every
player); at or above it, the grid radius query comes back empty in O(1) per enemy:

| E × P | idle |
|---|---|
| 50 × 1 | 387 ns |
| 200 × 1 | 789 ns |
| 200 × 50 | 56.3 µs |
| 200 × 200 | 48.8 µs |
| 1000 × 1 | 2.88 µs |
| 1000 × 50 | 275 µs |
| 1000 × 200 | 376 µs |

Aggressive enemies (grid radius query, nearest by dist², not provoked):

| E × P | aggro |
|---|---|
| 200 × 1 | 897 ns |
| 200 × 50 | 57.1 µs |
| 200 × 200 | 134 µs |
| 1000 × 1 | 3.43 µs |
| 1000 × 50 | 302 µs |
| 1000 × 200 | 880 µs |

Provoked / huge-radius fallback (global scan, unchanged O(E×P) — correct for this path, see plan):

| E × P | engaged |
|---|---|
| 200 × 1 | 1.49 µs |
| 200 × 50 | 57.3 µs |
| 200 × 200 | 237 µs |
| 1000 × 1 | 6.42 µs |
| 1000 × 50 | 298 µs |
| 1000 × 200 | 1.19 ms |

O(E·P) is gone once P reaches `GRID_PLAYER_MIN` (64) — idle and aggro both drop to
a grid radius query at and above that threshold, scaling with grid cell occupancy
rather than total player count. Below the threshold every path (including idle)
falls back to the O(E×P) scan, which is why idle/aggro/engaged converge to nearly
the same cost at P=50. The `engaged` (provoked-only) path keeps the global scan by
design at every P; it's bounded by how many enemies are actively provoked, which is
small in practice.

### Separation — `separation` (60 Hz, per active pair)

| Active pairs | Time | Per pair |
|---|---|---|
| 367 | 17.8 µs | ~48.5 ns |
| 2 217 | 88.0 µs | ~39.7 ns |
| 8 713 | 327 µs | ~37.5 ns |

(was ~200–210 ns/pair pre-fix — view idiom over `(&Transform, &Hitbox, Satisfies<&Solid>)`.)

### Spatial grid — `spatial_grid`

| Bench | Time |
|---|---|
| rebuild 200 / 1000 / 5000 | 6.15 µs / 31.5 µs / 171 µs |
| query r=40 (AOI), allocating | 1.10–1.15 µs |
| query r=40 (AOI), reused buffer (`query_cells_overlapping_into`) | 0.87–0.94 µs |

### Prefab spawn — `prefab_spawn` (compiled spawn plans, gap A)

| Bench | Time | Notes |
|---|---|---|
| spawn/bolt | 733 ns | was 4.0 µs pre-fix — RON parsed once per prefab, not per spawn |
| churn/n8 (spawn+despawn/tick) | 8.52 µs | |
| churn/n32 | 26.6 µs | was 126 µs pre-fix |

### Client netcode — `client_netcode` (gap B)

| Bench | Time | Notes |
|---|---|---|
| apply_snapshot/states_a64 | 222 ns | was ~3.9 µs pre-fix (4×, `mem::take` + view idiom) |
| apply_snapshot/states_a200 | 295 ns | was 10.9 µs pre-fix |
| apply_snapshot/enters_64 | 167 µs | was 264 µs pre-fix (mostly compiled prefab spawns); bolt enters now also build a `VfxTrail` component |
| reconcile/pending60 | 555 ns | |
| reconcile/pending240 | 1.65 µs | |
| reconcile/pending60_statics32 | 4.26 µs | 32-static collision: ~7.7× pend60, replay per intent |
| reconcile/pending240_statics32 | 15.7 µs | 32-static collision: ~9.5× pend240, replay per intent |

### Render CPU — `render_cpu` (VQ-F1 stress figure: 40 rigs × 64 joints)

| Bench | Before | After | Notes |
|---|---|---|---|
| joint_palette_40x64 | 105.03 µs | 108.13 µs | ~3 % up, no `main`-baseline comparison printed (first save under this name) |
| particle_fill_4096 | — | 23.23 µs | per-frame VFX particle buffer fill, 4096 particles |
| frustum_classify_552 | — | 7.77 µs | per-frame cull cost at 40 rigs + 512 statics (rendering rework 5) |

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

### Asset streaming — `asset_load` (rendering rework 2, steps 1–6)

| Asset | Before | After | Notes |
|---|---|---|---|
| statue_vroid (11 MB, embedded textures) upload | 122.11 ms | 24–28 ms | decode off-thread (step 1); upload cost residual, BC compression targeted (rework 4); sidecar decode skip (rework 4): `first_sight/statue_vroid` now 17.7–18.2 ms (lazy per-slot decode, no PNG decode when the DDS sidecar wins, -85.3% vs. the still-decoding-every-embedded-image run) |
| human (9 MB, skinned + clips) upload | 101.63 ms | 24–28 ms | same streaming path as statue; sidecar decode skip (rework 4): `first_sight/human` now 17.0–17.7 ms (-82.9% vs. the still-decoding-every-embedded-image run) |
| zone_ground (3× 2k JPG decode + mesh gen) | 246.27 ms | 6.5–7.3 ms | decode off-thread (step 1); mesh gen on background thread, GPU submission at draw; BC sidecars (rework 4): `decode_and_generate` now 11.6–11.8 ms (3 file reads + header parses, no JPG decode, -95.2% vs. the still-JPG-decoding run) |
| environment bake (per load, synchronous) | ~24 ms → ~20 ms | 6.8–7.3 ms | Baker hoist (step 2, pipeline compile once); single submit (step 3) collects 42 render passes |
| HDRI decode (zone-change path) | 577.9–597.4 ms | 540.7–543.0 ms | off-thread (step 4); frame pays only bake cost at arrival |

The synchronous-frame costs (statue/human/zone-ground uploads, environment bake, HDRI decode)
blocked zone entry and dressing. Rework 2 moves decodes off-frame via spawned background
threads (`load_gltf_data`, `EquirectImage::decode_hdr`); uploads stay on-frame but run under
a `MESH_UPLOADS_PER_FRAME = 1` budget, and environment bakes run only once per zone crossing
(Baker shared since step 2, single encoder through all passes since step 3). The residual
main-thread costs — mesh upload (~25 ms) and bake-on-arrival (~7 ms) — are accepted;
BC-texture compression (rework 4) and potential pre-baked IBL (deferred if residual grows)
are the identified reducers. The ~540 ms HDRI decode and ~250 ms mesh decode no longer land
in frame-critical paths.

### Snapshot path — `snapshot` (server; broadcast = full 6-tick round covering every
conn once, comparable to the old un-staggered number; broadcast_slice = one 60 Hz
tick, i.e. the actual per-tick cost the sim thread pays)

| Bench | Time | Notes |
|---|---|---|
| broadcast (full round) 10 clients | 39.5 µs | |
| broadcast (full round) 50 clients | 386 µs | |
| broadcast (full round) 200 clients | 5.16 ms | was 7.11 ms un-staggered pre-fix |
| broadcast (full round) 50 clients + 500 NPCs | 3.05 ms | |
| **broadcast_slice (per-tick) 200 clients** | **858 µs** | was the whole 7.11 ms landing on one tick pre-fix — this is the number that matters for input-phase jitter |
| broadcast_slice 10 / 50 clients | 6.6 µs / 60.7 µs | |
| broadcast_slice 50 clients + 500 NPCs | 507 µs | |
| select_states A=64 / 200 / 1000 | 43.6 ns / 5.7 µs / 27.8 µs | pass-through under the 64 cap |
| mechanic_resolve 50 / 200 conns | 20.2 µs / 97.3 µs | per due mechanic, unchanged cadence |

### Protocol — `protocol` (postcard)

Snapshot state and AOI enter/leave data now encode as two separate messages
(networking rework 3's datagram split) rather than one combined `Snapshot`:

| Bench | Time |
|---|---|
| encode snapshot_64 (64 states, **627 B**) | 1.82 µs |
| decode same | 1.39 µs |
| encode aoi_delta_8 (8 enters/leaves, **102 B**) | 420 ns |
| decode same | 262 ns |
| encode / decode MoveIntent | 185 ns / 87.9 ns |

627 B + 102 B ≈ 729 B × 10 Hz ≈ **7.1 KB/s per client** for the two per-tick
messages' encoded payload alone (excludes QUIC/datagram framing overhead, so it
undercounts the soak test's on-wire figure below).

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

At the target load (200-player crowd, ~200 NPCs, one zone) **everything still fits
with headroom**: the whole sim tick stays under 10 % of budget even at 1000+200
entities, and the 200-client snapshot fan-out's per-tick cost (0.86 ms) is under 1 %
of its own 100 ms budget.

Ranked by how soon each path becomes the limiter, post-fix:

1. **Dense single-cell crowds (collision chain) — the only remaining measured
   cliff, WEAKPOINTS #5, deliberately out of scope for this pass.** A 500-entity
   pile still costs 4.4 ms (broadphase) + 1.5 ms (narrowphase) ≈ 5.9 ms — down from
   ~23 ms pre-fix (the narrowphase view idiom helped here too) but still the thing
   to watch for chokepoints and boss piles.
2. **Not bottlenecks, confirmed at this baseline:** snapshot fan-out (400-bot soak
   passed at 18.7 ms p99 vs a 25 ms budget as of networking rework 3, not re-probed
   this pass), enemy AI (idle/aggro drop to a grid query at P ≥ 64, unaffected by E
   below it), separation (~40 ns/pair), prefab spawn (733 ns/spawn), client
   apply_snapshot (222 ns at A=64), grid rebuild/query, postcard encode/decode,
   packet loss up to 5 % at both 50 ms and 200 ms RTT (208 ms p99 / 302 ms max
   worst-case datagram-snapshot gap vs a 250/500 ms gate — rework 3, not re-probed
   this pass).

Implication for future work: with structural items #1–#4 fixed, the sim has broad
headroom at the design point (200 players, 200 NPCs). The next real constraint is
content-driven — dense-cell piles (#5) and whatever load real enemy/ability code
adds on top of the now-flat baseline above.

### Texture memory — rework 4

(`docs/reviews/rendering/plan-rendering-rework-4-2026-07-16.md` finding 1)

`ColorTexture::bytes` (`gpu_texture_bytes`, summed across a mip chain) and
`MeshStore::texture_memory_bytes` give the dev overlay's "tex mem (assets)"
line a real number to show. Measured by
`statue_and_human_texture_memory_measurement` streaming both assets through
the real `get_or_request`/`integrate` path:

| Asset pair | Before (all-RGBA8) | After (BC7/BC5 sidecars, steps 3–8) |
|---|---|---|
| statue_vroid.glb + human.glb | 130 MB (137 019 568 B) | 32 MB (34 256 176 B) |

≈4.1× down on the measured pair. This covers only those two assets; the ground
set is tracked by the `zone_ground` bench instead (its BC7/BC5 sidecars land
in step 6). `content_lint.rs`'s `total_texture_memory_within_budget` (step 9)
sums races + zone props + the ground set with the same sidecar-aware
estimate (DDS byte size when a sidecar is bound, RGBA8 + mip-chain estimate
otherwise) and now reports 138.0 MB for the current content set, down from
the ≈300 MB all-RGBA8 estimate recorded when that lint was first written
(step 2, before any sidecar existed).
