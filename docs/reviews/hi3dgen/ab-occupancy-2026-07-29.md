# Occupancy-threshold sweep

2026-07-29. Rework 4 step 5. Four pre-registered `--occupancy-threshold`
arms per subject (`-60, -20, +20, +60`, vs. the `0.0` baseline), scored
against the noise floor measured in
`docs/reviews/hi3dgen/noise-floor-2026-07-29.md`.

## What ran

12 runs, one candidate each:

```
C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_hi3dgen.py \
    target/prop-batch/b3/arch/cand_0/concept.png \
    --out target/knob-sweep/occupancy/chapel_arch/t<value> --seed 0 \
    --occupancy-threshold <value>
                target/prop-batch/candelabra-z/cand_4/concept.png  --seed 4
                target/prop-batch/b3/crucero/cand_21/concept.png   --seed 21
```

`<value>` in `{-60, -20, 20, 60}`, chosen and pre-registered in step 4's
report (`docs/reviews/hi3dgen/noise-floor-2026-07-29.md` § "Pre-registered
arms for step 5"). All other flags default, matching step 4's floor runs.
Artifacts under `target/knob-sweep/occupancy/<subject>/t<value>/cand_<seed>/`.
Baseline (`t=0.0`) is step 4's `r1` repeat under
`target/knob-sweep/floor/<subject>/r1/cand_<seed>/`.

Scoring reused step 4's driver
(`scripts/ai-pipeline/prop_extract.py:31`'s `topo_stats`, and the
`deviation()` function quoted in the noise-floor report) rather than a new
instrument, so this table is directly comparable to step 4's floor table.

## Verification predicate

Every arm's manifest `extraction.occupancy_threshold` equals its directory's
nominal value, and every arm's `ss_active_voxels` equals step 4's
pre-registered implied count exactly (the curve's repeat jitter is zero, so
this is integer equality, not a tolerance):

| subject | arm | manifest threshold | nominal | manifest voxels | implied voxels | match |
| --- | --- | --- | --- | --- | --- | --- |
| chapel_arch | -60 | -60.0 | -60.0 | 17454 | 17454 | yes |
| chapel_arch | -20 | -20.0 | -20.0 | 14992 | 14992 | yes |
| chapel_arch | +20 | 20.0 | 20.0 | 14205 | 14205 | yes |
| chapel_arch | +60 | 60.0 | 60.0 | 11748 | 11748 | yes |
| candelabra_shrine | -60 | -60.0 | -60.0 | 9351 | 9351 | yes |
| candelabra_shrine | -20 | -20.0 | -20.0 | 8523 | 8523 | yes |
| candelabra_shrine | +20 | 20.0 | 20.0 | 8323 | 8323 | yes |
| candelabra_shrine | +60 | 60.0 | 60.0 | 7639 | 7639 | yes |
| crucero | -60 | -60.0 | -60.0 | 7476 | 7476 | yes |
| crucero | -20 | -20.0 | -20.0 | 6816 | 6816 | yes |
| crucero | +20 | 20.0 | 20.0 | 6626 | 6626 | yes |
| crucero | +60 | 60.0 | 60.0 | 5864 | 5864 | yes |

All 12 arms pass: the dump-time curve and the run-time chain agree exactly on
every subject. No arm is invalidated.

## Per-subject tables

Baseline is step 4's `r1` repeat (same seed, `occupancy_threshold=0.0`). The
`floor` column is that metric's step-4 repeat-noise spread for the subject;
`abs` is the arm's absolute value and `Δ` its signed change from baseline.
`ratio` on the deviation rows is the arm's mean/p99.9 divided by the floor's
worst-pair mean/p99.9 — over 1x means the move clears the floor.

### chapel_arch (concept `b3/arch/cand_0`, seed 0; baseline r1 above)

| metric | floor | baseline | t=-60 | t=-20 | t=+20 | t=+60 |
| --- | --- | --- | --- | --- | --- | --- |
| vertex_count | 112 (0.0291%) | 384284 | 374938 (Δ-9346, -2.43%) | 385834 (Δ+1550, +0.40%) | 381862 (Δ-2422, -0.63%) | 358128 (Δ-26156, -6.81%) |
| face_count | 232 (0.0302%) | 768858 | 749830 (Δ-19028, -2.47%) | 771812 (Δ+2954, +0.38%) | 764296 (Δ-4562, -0.59%) | 718234 (Δ-50624, -6.59%) |
| volume | 8.458e-06 (0.0405%) | 0.0208617422 | 0.0328431872 (Δ+0.01198, +57.4%) | 0.0239089120 (Δ+0.00305, +14.6%) | 0.0198126240 (Δ-0.00105, -5.03%) | 0.0082758646 (Δ-0.01259, -60.3%) |
| body_count | 2 (unresolved) | 9 | 37 (Δ+28) | 12 (Δ+3, unresolved) | 10 (Δ+1, unresolved) | 27 (Δ+18) |
| boundary_edge_count | 0 | 4 | 4 (Δ0, unresolved) | 0 (Δ-4, unresolved) | 0 (Δ-4, unresolved) | 4 (Δ0, unresolved) |
| main_euler_number | 4 (unresolved) | -163 | -51 (Δ+112) | -94 (Δ+69) | -304 (Δ-141) | -1043 (Δ-880) |
| ss_active_voxels | 0 | 14588 | 17454 (+19.65%) | 14992 (+2.77%) | 14205 (-2.63%) | 11748 (-19.47%) |
| is_watertight | n/a | false | false | true | true | false |
| deviation mean (80k, ×1e-6 diag) | 14.9 | - | 2241.7 (150x) | 1146.0 (76.9x) | 1081.3 (72.6x) | 2340.7 (157.1x) |
| deviation p99.9 (80k, ×1e-6 diag) | 194 | - | 13873.8 (71.5x) | 7128.9 (36.7x) | 7091.2 (36.6x) | 16384.9 (84.5x) |

Body count / boundary-edge / Euler deltas below the "order 10 bodies / 20
boundary edges / 20 Euler" bar from step 4 are labelled unresolved above;
`t-20`/`t+20`'s body-count moves (+3/+1) are unresolved, but their Euler
moves (+69/-141) clear the bar and are real. Every vertex/face/volume/
deviation delta on every arm clears its floor by 1-2 orders of magnitude —
this subject's thin arch pillars are highly sensitive to the occupancy cut in
both directions, confirming the Evidence's hypothesis.

### candelabra_shrine (concept `candelabra-z/cand_4`, seed 4; baseline r1 above)

| metric | floor | baseline | t=-60 | t=-20 | t=+20 | t=+60 |
| --- | --- | --- | --- | --- | --- | --- |
| vertex_count | 15 (0.0089%) | 167870 | 177342 (Δ+9472, +5.64%) | 169750 (Δ+1880, +1.12%) | 167942 (Δ+72, +0.043%) | 171173 (Δ+3303, +1.97%) |
| face_count | 30 (0.0089%) | 335724 | 354664 (Δ+18940, +5.64%) | 339488 (Δ+3764, +1.12%) | 335860 (Δ+136, +0.041%) | 342486 (Δ+6762, +2.01%) |
| volume | 3.615e-06 (0.0234%) | 0.0154566715 | 0.0196961856 (Δ+0.00424, +27.43%) | 0.0163426995 (Δ+0.00089, +5.73%) | 0.0151038997 (Δ-0.00035, -2.28%) | 0.0116983057 (Δ-0.00376, -24.31%) |
| body_count | 0 | 10 | 14 (Δ+4, unresolved) | 11 (Δ+1, unresolved) | 15 (Δ+5, unresolved) | 7 (Δ-3, unresolved) |
| boundary_edge_count | 0 | 0 | 0 (Δ0) | 0 (Δ0) | 0 (Δ0) | 0 (Δ0) |
| main_euler_number | 0 | -6 | -16 (Δ-10, unresolved) | -14 (Δ-8, unresolved) | -6 (Δ0) | -68 (Δ-62) |
| ss_active_voxels | 0 | 8417 | 9351 (+11.10%) | 8523 (+1.26%) | 8323 (-1.12%) | 7639 (-9.24%) |
| is_watertight | n/a | true | true | true | true | true |
| deviation mean (80k, ×1e-6 diag) | 8.0 | - | 1770.2 (221.3x) | 588.3 (73.5x) | 706.4 (88.3x) | 1514.1 (189.3x) |
| deviation p99.9 (80k, ×1e-6 diag) | 145 | - | 14600.1 (100.7x) | 6100.7 (42.1x) | 10225.4 (70.5x) | 12362.4 (85.3x) |

Every vertex/face/volume/deviation delta clears its floor on this subject
too, including the near arm's smallest move (`t+20` vertex_count, +0.043%,
still ~4.8x the 0.0089% floor). Body-count and Euler moves all stay under
the unresolved bar except `t+60`'s Euler (-62, clears 20).

### crucero (concept `b3/crucero/cand_21`, seed 21; baseline r1 above)

| metric | floor | baseline | t=-60 | t=-20 | t=+20 | t=+60 |
| --- | --- | --- | --- | --- | --- | --- |
| vertex_count | 47 (0.0260%) | 180776 | 175831 (Δ-4945, -2.74%) | 180774 (Δ-2, unresolved) | 179899 (Δ-877, -0.49%) | 167820 (Δ-12956, -7.17%) |
| face_count | 80 (0.0221%) | 361534 | 351620 (Δ-9914, -2.74%) | 361530 (Δ-4, unresolved) | 359788 (Δ-1746, -0.48%) | 336186 (Δ-25348, -7.01%) |
| volume | 2.214e-06 (0.0169%) | 0.0131029145 | 0.0165966447 (Δ+0.00349, +26.66%) | 0.0132560229 (Δ+0.00015, +1.17%) | 0.0123832567 (Δ-0.00072, -5.49%) | 0.0062790894 (Δ-0.00682, -52.08%) |
| body_count | 2 (unresolved) | 9 | 10 (Δ+1, unresolved) | 6 (Δ-3, unresolved) | 7 (Δ-2, unresolved) | 8 (Δ-1, unresolved) |
| boundary_edge_count | 8 (unresolved) | 12 | 28 (Δ+16, unresolved) | 20 (Δ+8, unresolved) | 26 (Δ+14, unresolved) | 36 (Δ+24) |
| main_euler_number | 5 (unresolved) | -13 | -5 (Δ+8, unresolved) | -11 (Δ+2, unresolved) | -20 (Δ-7, unresolved) | -305 (Δ-292) |
| ss_active_voxels | 0 | 6715 | 7476 (+11.33%) | 6816 (+1.50%) | 6626 (-1.33%) | 5864 (-12.67%) |
| is_watertight | n/a | false | false | false | false | false |
| deviation mean (80k, ×1e-6 diag) | 15.8 | - | 1563.2 (98.9x) | 606.5 (38.4x) | 734.7 (46.5x) | 2643.3 (167.3x) |
| deviation p99.9 (80k, ×1e-6 diag) | 415 | - | 12340.2 (29.7x) | 6399.8 (15.4x) | 9129.4 (22.0x) | 18077.5 (43.6x) |

`t-20` is the one place in the whole sweep where counts alone look flat
(vertex/face deltas -2/-4, inside the 47/80 floor) - but volume still moves
+1.17% (69x its 0.0169% floor) and deviation mean/p99.9 sit at 38x/15x their
floors, so the mesh is not a rerun of baseline even though the vertex/face
counters read that way: same count, different shape. Every other arm on
this subject clears counts, volume and deviation cleanly. Body-count,
boundary-edge and Euler deltas stay unresolved on every arm except `t+60`'s
boundary-edge (+24) and Euler (-292) moves.

### Deviation sample-count stability check

20k -> 80k -> 320k mean values move by <1% on every arm/subject (matching
step 4's convergence finding), confirming these numbers are not an artifact
of the sample budget:

| subject | arm | mean 20k | mean 80k | mean 320k |
| --- | --- | --- | --- | --- |
| chapel_arch | t-60 | 2224.9 | 2241.7 | 2240.6 |
| chapel_arch | t+60 | 2346.5 | 2340.7 | 2340.1 |
| candelabra_shrine | t-60 | 1759.6 | 1770.2 | 1772.9 |
| candelabra_shrine | t+60 | 1515.1 | 1514.1 | 1512.3 |
| crucero | t-60 | 1567.8 | 1563.2 | 1563.1 |
| crucero | t+60 | 2643.7 | 2643.3 | 2652.1 |

(units ×1e-6 of bbox diagonal; `max` omitted per step 4's finding that it has
no limit at this sample budget and must not be used as a criterion.)

## Renders

Turntable, 4 angles x 512x512, per arm, under
`target/knob-sweep/occupancy/<subject>/t<value>/render/` (`contact_sheet.png`
+ 4 `frame_NN.png`). A per-subject comparison sheet stitching the four arms
side by side is at
`target/knob-sweep/occupancy/<subject>/subject_contact_sheet.png`. Visual
read on chapel_arch (thinnest-featured subject): the arch silhouette is
intact at every threshold from -60 to +60, but the pillar and arch-rib
surfaces visibly thicken at -60 (more retained cells) and show more
faceting/voxel-stepping at +60 (fewer retained cells); the flatter faces are
not visibly different by eye at any arm. Renders are gitignored artifacts
under `target/`, not committed.

## Recommendation

Occupancy threshold is not a free parameter in practice at this resolution:
every arm on every subject moves vertex/face/volume and deviation well past
step 4's measured floor (best case ~5x, typical case 30-200x), so the
0.0-vs-other-threshold question is resolved, not "unresolved at measured
noise" like the small-integer topology counters. The response is also
strongly asymmetric in volume (chapel_arch: +57% at -60 vs -60% at +60;
similar magnitude on the other two subjects) and non-monotonic in vertex/face
count relative to volume (voxel count always moves monotonically with `t`,
but mesh vertex/face count does not, because extraction topology - number of
disconnected islands - changes independently of cell count).

No arm in {-60, -20, +20, +60} strictly dominates the 0.0 baseline on visual
or topological grounds from this data alone: `+60` recovers the fewest
active voxels and produces the most faceted thin surfaces (worse for the
thin-feature-survival goal this step was testing); `-60` recovers the most
voxels and thickens surfaces, which is the opposite failure mode. The
in-between arms (±20) move counts/volume by single-digit percentages without
the visibly worse faceting seen at ±60.

**Recommendation: keep the current default (`occupancy_threshold = 0.0`).**
It sits at the visual and volumetric midpoint between the two failure modes
observed at the ±60 extremes, and nothing in this sweep's data shows a
clear improvement over it for any subject. **User checkpoint required**
before this is treated as final - the visual call on which faceting level is
acceptable is a judgment step 4's floor data cannot make.
