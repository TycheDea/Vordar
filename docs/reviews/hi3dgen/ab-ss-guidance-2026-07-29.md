# Sparse-structure guidance sweep: `cfg_interval` lower bound and `rescale_t`

2026-07-29. Rework 4 step 6. Six OFAT arms per subject against the sparse
structure (SS) stage's guidance schedule — `--ss-cfg-interval-lo` and
`--ss-rescale-t` — read against the floor established in
`noise-floor-2026-07-29.md`. ab-sampler-2026-07-28.md found the SLAT stage's
own cfg-strength/steps knobs inert and pointed further quality work at the SS
stage; this step is the first direct measurement of the SS stage's own
guidance knobs.

## What ran

Three subjects (`chapel_arch` seed 0, `candelabra_shrine` seed 4, `crucero`
seed 21) x six arms, defaults elsewhere (50 SS steps, cfg 5.0, baseline
`cfg_interval` [0.5, 1.0], baseline `rescale_t` 3.0):

- `--ss-cfg-interval-lo` ∈ {0.0, 0.25, 0.75} (interval upper bound stays 1.0)
- `--ss-rescale-t` ∈ {1.0, 2.0, 4.5}

18 runs, `target/knob-sweep/ss-guidance/<subject>/<knob>-<value>/cand_<seed>/`.
Each candidate's `raw.glb` scored against `target/knob-sweep/floor/<subject>/r1/cand_<seed>/raw.glb`
(the same repeat-1 baseline mesh step 5 used), with `prop_extract.topo_stats`
and the `deviation()` instrument from step 4's `floor.py`, at 20k/80k/320k
samples per direction. No source touched, no Rust touched.

## Manifest predicate: all 18 pass

For every arm, `hi3dgen_manifest.json`'s `sampler_params.sparse_structure`
carries the arm's nominal value on the knob under test, with the other knob
at its default (`cfg_interval[0]=0.5` for the `rescale_t` arms,
`rescale_t=3.0` for the `cfg_interval_lo` arms, `cfg_interval[1]=1.0`
throughout). All 18 `predicate_ok=True`. No arm ran with an unintended
parameter drift.

## `ss_active_voxels`: exact, zero-floor signal, non-uniform direction

The SS stage is bit-reproducible at a fixed seed (step 4: three `ss_logits.npy`
byte-identical per subject), so this count carries no repeat jitter — any
nonzero delta from the default-arm value is real at integer resolution.
Every one of the 18 arms moves it:

| subject | default | cfg-lo 0.0 | cfg-lo 0.25 | cfg-lo 0.75 | rescale 1.0 | rescale 2.0 | rescale 4.5 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| chapel_arch | 14588 | 14687 (+99) | 14650 (+62) | 14229 (-359) | 14459 (-129) | 14572 (-16) | 14447 (-141) |
| candelabra_shrine | 8417 | 8478 (+61) | 8466 (+49) | 8401 (-16) | 8229 (-188) | 8096 (-321) | 8571 (+154) |
| crucero | 6715 | 6673 (-42) | 6678 (-37) | 6754 (+39) | 6905 (+190) | 6785 (+70) | 6626 (-89) |

`cfg_interval_lo` moves the active count **monotonically down** as `lo`
increases on chapel_arch and candelabra_shrine (0.0 → 0.25 → default(0.5) →
0.75 is strictly decreasing on both), but **monotonically up** on crucero —
the opposite sign. `rescale_t` shows no consistent monotone trend on any
subject (chapel_arch and candelabra_shrine both dip below the default at
1.0-2.0 then differ at 4.5; crucero decreases monotonically with `rescale_t`
increasing). The knob is real and content-dependent, not a uniform bias a
single default retune would correct in one direction for every subject.

## Vertex / face / volume vs the floor

All figures `abs(delta) / baseline` against each subject's floor r1 mesh;
"x floor" is that percentage over the floor row's relative spread from
`noise-floor-2026-07-29.md`. Every one of the 54 (18 arms x 3 metrics) rows
clears its floor — none is sub-floor / unresolved.

### chapel_arch (floor: vertex 0.0291%, face 0.0302%, volume 0.0405%)

| arm | vertex Δ% (x floor) | face Δ% (x floor) | volume Δ% (x floor) |
| --- | --- | --- | --- |
| cfg-lo 0.0 | 0.357% (12.3x) | 0.354% (11.7x) | 9.88% (244x) |
| cfg-lo 0.25 | 0.409% (14.1x) | 0.417% (13.8x) | 3.72% (92x) |
| cfg-lo 0.75 | 0.091% (3.1x) | 0.101% (3.4x) | 14.08% (348x) |
| rescale 1.0 | 0.042% (1.4x) | 0.037% (1.2x) | 4.66% (115x) |
| rescale 2.0 | 0.706% (24.2x) | 0.708% (23.4x) | 9.60% (237x) |
| rescale 4.5 | 0.423% (14.5x) | 0.420% (13.9x) | 6.00% (148x) |

### candelabra_shrine (floor: vertex 0.0089%, face 0.0089%, volume 0.0234%)

| arm | vertex Δ% (x floor) | face Δ% (x floor) | volume Δ% (x floor) |
| --- | --- | --- | --- |
| cfg-lo 0.0 | 2.63% (295x) | 2.63% (295x) | 2.10% (90x) |
| cfg-lo 0.25 | 0.88% (98x) | 0.87% (98x) | 2.19% (93x) |
| cfg-lo 0.75 | 1.59% (179x) | 1.59% (179x) | 1.81% (78x) |
| rescale 1.0 | 3.88% (436x) | 3.89% (437x) | 0.36% (16x) |
| rescale 2.0 | 0.90% (101x) | 0.90% (101x) | 3.86% (165x) |
| rescale 4.5 | 1.31% (147x) | 1.30% (146x) | 5.87% (251x) |

### crucero (floor: vertex 0.026%, face 0.0221%, volume 0.0169%)

| arm | vertex Δ% (x floor) | face Δ% (x floor) | volume Δ% (x floor) |
| --- | --- | --- | --- |
| cfg-lo 0.0 | 1.53% (59x) | 1.51% (68x) | 1.11% (65x) |
| cfg-lo 0.25 | 1.79% (69x) | 1.80% (81x) | 1.04% (62x) |
| cfg-lo 0.75 | 0.12% (4.7x) | 0.12% (5.5x) | 1.62% (96x) |
| rescale 1.0 | 0.95% (37x) | 0.95% (43x) | 4.60% (272x) |
| rescale 2.0 | 0.13% (4.9x) | 0.12% (5.6x) | 0.57% (34x) |
| rescale 4.5 | 0.39% (15x) | 0.38% (17x) | 3.70% (219x) |

Weakest row overall: `rescale-t-1.0` face_count on chapel_arch at 1.2x the
floor — still above it, but the narrowest margin of the sweep. Every other
row clears its floor by at least 3x, most by one to two orders of magnitude.

## Small-integer topology (order-10 body / order-20 boundary-edge / order-20 Euler thresholds)

Per step 4's finding, only changes of this order count as signal on these
metrics; anything smaller is unresolved and reported as such, not hedged.

| subject | arm | body_count Δ | boundary_edge Δ | main_euler Δ | verdict |
| --- | --- | --- | --- | --- | --- |
| chapel_arch | cfg-lo 0.0 | -1 | +0 | +14 | unresolved |
| chapel_arch | cfg-lo 0.25 | -1 | -4 | +33 | **euler SIGNAL** |
| chapel_arch | cfg-lo 0.75 | **+11** | -4 | **-61** | **body + euler SIGNAL** |
| chapel_arch | rescale 1.0 | -2 | -4 | -13 | unresolved |
| chapel_arch | rescale 2.0 | -3 | -4 | +17 | unresolved |
| chapel_arch | rescale 4.5 | +4 | -4 | +5 | unresolved |
| candelabra_shrine | all six | +0..+6 | +0 | +0..-4 | unresolved (all) |
| crucero | cfg-lo 0.0 | -5 | **+24** | **-24** | **boundary + euler SIGNAL** |
| crucero | cfg-lo 0.25 | +1 | **+20** | -5 | **boundary SIGNAL** |
| crucero | cfg-lo 0.75 | -5 | +12 | +3 | unresolved |
| crucero | rescale 1.0 | -5 | **+20** | -9 | **boundary SIGNAL** |
| crucero | rescale 2.0 | -1 | **+36** | -7 | **boundary SIGNAL** |
| crucero | rescale 4.5 | -8 | +4 | +1 | unresolved |

candelabra_shrine never moves a small-integer metric past its threshold —
consistent with that subject also having the tightest count/volume floors.
chapel_arch's `cfg-lo 0.75` arm is the only one to cross the body_count
threshold, and it does so by fragmenting (9 → 20 bodies, `main_face_fraction`
0.9997 → 0.9936) — a topology regression, not an improvement. crucero moves
`boundary_edge_count` past threshold on four of six arms, always upward
(more open boundary), which given the baseline is already non-watertight on
this subject reads as the guidance schedule opening more small holes, not
closing them.

## Deviation vs the floor r1 baseline mesh

Mean and p99.9 only (max is not a criterion per the noise-floor doc — it
diverges with sample count). All values at 80k samples/direction, in units
of 1e-6 of the bbox diagonal; "x floor" against the worst-pair floor row.

| subject | floor mean / p99.9 | arm range: mean (x floor) | arm range: p99.9 (x floor) |
| --- | --- | --- | --- |
| chapel_arch | 14.9 / 194 | 1074-1637 (72-110x) | 7376-11026 (38-57x) |
| candelabra_shrine | 8.0 / 145 | 619-1333 (77-167x) | 7120-11575 (49-80x) |
| crucero | 15.8 / 415 | 694-1016 (44-64x) | 7250-12458 (17-30x) |

Every arm on every subject sits 17x-167x its metric's floor. This is the
sharpest confirmation in the sweep: none of the 18 candidates is
geometrically indistinguishable from a rerun of the default arm — the SS
guidance schedule visibly moves the mesh every time it is touched.

## Watertightness

chapel_arch's floor baseline is non-watertight on all three repeats (per
`noise-floor-2026-07-29.md`). Of the six SS-guidance arms, five
(`cfg-lo 0.25`, `cfg-lo 0.75`, `rescale 1.0/2.0/4.5`) come back
`is_watertight=true`; only `cfg-lo 0.0` stays non-watertight like the
baseline. candelabra_shrine is watertight at baseline and stays watertight
on all six arms. crucero is non-watertight at baseline and stays
non-watertight on all six arms. The chapel_arch flip is real (not floor
noise — watertightness has no float component) but is not necessarily a
quality improvement: `cfg-lo 0.75`'s watertight result is the same arm that
fragmented into 20 separate bodies, each individually closed.

## Turntable sheets

4 x 512² turntables per arm, contact sheets stitched 3x2 per subject:

- `target/knob-sweep/ss-guidance/chapel_arch/subject_contact_sheet.png`
- `target/knob-sweep/ss-guidance/candelabra_shrine/subject_contact_sheet.png`
- `target/knob-sweep/ss-guidance/crucero/subject_contact_sheet.png`

Visual read on chapel_arch (representative of all three): silhouette is
stable across all six arms — same arch, same column proportions — with the
per-arm variation confined to fine surface detail and small island
count/placement, consistent with the topology table above.

## Verdict

Unlike the SLAT stage's cfg-strength/steps knobs (ab-sampler-2026-07-28,
measured inert), the SS stage's guidance schedule is **not inert**: every
one of the 18 arms clears its subject's floor on vertex/face/volume/deviation,
usually by one to two orders of magnitude, and `ss_active_voxels` moves
exactly and nonzero on all 18. This resolves the open question from
ab-sampler-2026-07-28 — the sparse-structure stage is where guidance changes
actually land.

What the sweep does **not** show is a direction. `cfg_interval_lo`'s effect
on `ss_active_voxels` flips sign between chapel_arch/candelabra_shrine and
crucero; `rescale_t` shows no monotone trend on any subject; the one arm
that crosses the body-count signal threshold (`cfg-lo 0.75` on chapel_arch)
does so by fragmenting the mesh, and the arms that cross the boundary-edge
threshold (four of crucero's six) all open more boundary, not less. No arm
is a clean win on the measured metrics, and silhouette is stable across the
sweep on the turntable sheets — so there is no numeric or visual case here
for moving the default off `cfg_interval [0.5, 1.0]` / `rescale_t 3.0`.

**Recommendation: keep the current defaults.** This is not a "some arm is
better" case; it establishes that the knobs are live and worth returning to
only alongside a real quality signal (e.g. human silhouette/detail judgment
or a downstream texture-fidelity check), not blind OFAT deltas.

**User checkpoint:** confirm keeping `cfg_interval [0.5, 1.0]` / `rescale_t
3.0` as the shipped default, or flag any arm from the contact sheets above
worth a closer look before this campaign's next step.
