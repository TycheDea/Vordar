# SLAT guidance mini-sweep: `cfg_interval` lower bound and `rescale_t`

2026-07-29. Rework 4 step 7. ab-sampler measured the SLAT stage's
`cfg_strength` (3 vs 5) and `steps` (6/12/25) inert to <1%, but `cfg_interval`
and `rescale_t` were in neither grid. This step closes that gap: two knobs,
two values each (the far pair from step 6's SS-guidance sweep, since the near
pair there had already cleared the floor by double digits), two subjects.

## What ran

8 runs, one candidate each, `--out` batch dir per arm:

```
C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_hi3dgen.py \
    target/prop-batch/b3/arch/cand_0/concept.png --seed 0 \
    --out target/knob-sweep/slat-guidance/chapel_arch/<tag> \
    [--slat-cfg-interval-lo {0.0,0.75} | --slat-rescale-t {1.0,4.5}]

    target/prop-batch/candelabra-z/cand_4/concept.png --seed 4
    --out target/knob-sweep/slat-guidance/candelabra_shrine/<tag>
```

All other knobs at checkpoint default (ss 50/5.0/[0.5,1.0]/3.0, slat
6/5.0/[0.5,1.0]/3.0, `occupancy_threshold` 0.0). Artifacts under
`target/knob-sweep/slat-guidance/<subject>/<tag>/cand_<seed>/`. Baseline is
step 4's floor `r1` mesh for each subject (measured three times already; not
re-run). 8 runs, ~46 s wall each including shared per-process model load
(~27-30 s) — well inside the ≤20 min budget.

## Predicate: manifest carries the arm's value; SS stage untouched

For every arm: `sampler_params.slat` carries the arm's nominal
`cfg_interval[0]` or `rescale_t` with the other SLAT knob held at its
default ([0.5, 1.0] / 3.0), **and** `sampler_params.sparse_structure` is
bit-identical to the default (50 / 5.0 / [0.5, 1.0] / 3.0), **and**
`ss_active_voxels` is IDENTICAL to the subject's step-4 floor value (chapel
14588, candelabra 8417) — the SS stage runs before SLAT and is
bit-reproducible at fixed seed, so a SLAT-side knob cannot move it.

| subject | arm | slat cfg_interval | slat rescale_t | sparse_structure untouched | ss_active_voxels | == floor | predicate_ok |
| --- | --- | --- | --- | --- | --- | --- | --- |
| chapel_arch | cfg-interval-lo-0.0 | [0.0, 1.0] | 3.0 | yes | 14588 | yes | **true** |
| chapel_arch | cfg-interval-lo-0.75 | [0.75, 1.0] | 3.0 | yes | 14588 | yes | **true** |
| chapel_arch | rescale-t-1.0 | [0.5, 1.0] | 1.0 | yes | 14588 | yes | **true** |
| chapel_arch | rescale-t-4.5 | [0.5, 1.0] | 4.5 | yes | 14588 | yes | **true** |
| candelabra_shrine | cfg-interval-lo-0.0 | [0.0, 1.0] | 3.0 | yes | 8417 | yes | **true** |
| candelabra_shrine | cfg-interval-lo-0.75 | [0.75, 1.0] | 3.0 | yes | 8417 | yes | **true** |
| candelabra_shrine | rescale-t-1.0 | [0.5, 1.0] | 1.0 | yes | 8417 | yes | **true** |
| candelabra_shrine | rescale-t-4.5 | [0.5, 1.0] | 4.5 | yes | 8417 | yes | **true** |

All 8 arms valid: no cross-stage leak, every predicate clean.

## Results vs floor (noise-floor-2026-07-29.md)

Deltas are arm value minus the subject's floor/r1 baseline. Status uses the
pre-registered thresholds: counts/volume need >floor-pct (~0.03-0.05%);
small-integer topology (`body_count`, `boundary_edge_count`,
`main_euler_number`) needs order **10 / 20 / 20** to count as signal; `max`
deviation is never a criterion, only mean and p99.9 at 80k samples.

### chapel_arch (floor: vertex 384284±112 [0.0291%], face 768858±232
[0.0302%], volume 0.0208617422±8.458e-06 [0.0405%], body_count 9,
boundary_edge_count 4, main_euler_number -163, dev80k mean 14.9e-6 / p99.9
194e-6)

| arm | vertex Δ% | face Δ% | volume Δ% | body_count | boundary_edge | euler | dev80k mean | dev80k p99.9 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| cfg-interval-lo-0.0 | 0.560% (x19) SIGNAL | 0.567% (x19) SIGNAL | 9.98% (x247) SIGNAL | 10 (Δ+1) unresolved | 0 (Δ-4) unresolved | -188 (Δ-25) **SIGNAL** | 698e-6 (x47) | 5005e-6 (x26) |
| cfg-interval-lo-0.75 | 0.403% (x14) SIGNAL | 0.411% (x14) SIGNAL | 6.36% (x157) SIGNAL | 8 (Δ-1) unresolved | 0 (Δ-4) unresolved | -130 (Δ+33) **SIGNAL** | 442e-6 (x30) | 4185e-6 (x22) |
| rescale-t-1.0 | 2.840% (x98) SIGNAL | 2.875% (x95) SIGNAL | 7.48% (x185) SIGNAL | 24 (Δ+15) **SIGNAL** | 0 (Δ-4) unresolved | -332 (Δ-169) **SIGNAL** | 950e-6 (x64) | 6436e-6 (x33) |
| rescale-t-4.5 | 1.110% (x38) SIGNAL | 1.113% (x37) SIGNAL | 7.98% (x197) SIGNAL | 8 (Δ-1) unresolved | 0 (Δ-4) unresolved | -148 (Δ+15) unresolved | 471e-6 (x32) | 5258e-6 (x27) |

### candelabra_shrine (floor: vertex 167870±15 [0.0089%], face 335724±30
[0.0089%], volume 0.0154566715±3.615e-06 [0.0234%], body_count 10,
boundary_edge_count 0, main_euler_number -6, dev80k mean 8.0e-6 / p99.9
145e-6)

| arm | vertex Δ% | face Δ% | volume Δ% | body_count | boundary_edge | euler | dev80k mean | dev80k p99.9 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| cfg-interval-lo-0.0 | 0.951% (x107) SIGNAL | 0.957% (x108) SIGNAL | 0.562% (x24) SIGNAL | 9 (Δ-1) unresolved | 0 (Δ+0) unresolved | -14 (Δ-8) unresolved | 352e-6 (x44) | 3661e-6 (x25) |
| cfg-interval-lo-0.75 | 0.144% (x16) SIGNAL | 0.144% (x16) SIGNAL | 0.158% (x7) SIGNAL | 10 (Δ+0) unresolved | 0 (Δ+0) unresolved | -6 (Δ+0) unresolved | 200e-6 (x25) | 2396e-6 (x17) |
| rescale-t-1.0 | 0.857% (x96) SIGNAL | 0.852% (x96) SIGNAL | 2.40% (x102) SIGNAL | 14 (Δ+4) unresolved | 0 (Δ+0) unresolved | -6 (Δ+0) unresolved | 405e-6 (x51) | 5087e-6 (x35) |
| rescale-t-4.5 | 0.083% (x9.4) SIGNAL | 0.083% (x9.4) SIGNAL | 0.384% (x16) SIGNAL | 10 (Δ+0) unresolved | 0 (Δ+0) unresolved | -6 (Δ+0) unresolved | 156e-6 (x19) | 1480e-6 (x10) |

`degenerate_face_count` is 0 and `is_watertight` is true on all 8 arms
(baselines: chapel_arch not watertight, candelabra_shrine watertight — arms
do not preserve chapel_arch's original openness, itself a signal but not a
predicate this step scores).

## Adjudication

**Both SLAT knobs are live, not inert — the opposite of `cfg_strength` and
`steps`.** Every arm on both subjects clears the vertex/face/volume floor by
7x to 250x, and mean/p99.9 surface deviation from baseline sits 9x to 64x
above the subject's repeat-noise floor. This is not noise: cfg_interval and
rescale_t visibly redraw fine surface detail (see render sheets — brick
coursing on chapel_arch, candle/plate fluting on candelabra_shrine differ
per arm) while holding gross topology recognizable. `main_euler_number` also
clears the order-20 threshold on 3 of 8 arms (chapel_arch's two most extreme
settings, cfg-interval-lo-0.0 and rescale-t-1.0/-4.5's neighbor), consistent
with the knobs perturbing enough vertices to occasionally open or close a
small hole. `body_count`/`boundary_edge_count` stay under their order-10/20
thresholds on every arm — the small-integer topology metrics remain
unresolved at this measurement's noise, same as steps 4-6 found.

Render sheets: `target/knob-sweep/slat-guidance/chapel_arch/subject_contact_sheet.png`,
`target/knob-sweep/slat-guidance/candelabra_shrine/subject_contact_sheet.png`
(2x2, one tile per arm, 4-angle turntable each).

## Recommendation

`slat_cfg_interval_lo` and `slat_rescale_t` are **not** candidates for
"inert, leave at default" the way `slat_cfg` and `slat_steps` were. They
measurably change output geometry at every tested value, in both directions
(vertex count moves +0.56% to +2.9% at cfg-interval-lo/rescale-t lowered,
-0.08% to -1.1% at cfg-interval-lo/rescale-t raised). Picking a production
value for these two knobs is a quality decision (which fine-detail character
looks best), not a correctness one — this step's job was only to establish
that they are live, which it did unambiguously. No default-value change is
proposed here; that call needs a human looking at the render sheets against
the concept art, not another metric.

**User checkpoint on the recommendation** — no source changed, no Rust
touched, per the finding's gate.

## Verification predicate

- Every arm's `sampler_params.slat` carries the arm's nominal value; the
  other SLAT knob and the entire `sparse_structure` block hold checkpoint
  default, verified programmatically (predicate table above, all 8 true).
- `ss_active_voxels` is bit-identical to the subject's step-4 floor value on
  all 8 arms (14588 chapel_arch, 8417 candelabra_shrine) — no cross-stage
  leak.
- Deltas reported against the pre-registered floor table; sub-floor moves
  (`body_count`, `boundary_edge_count`, and `main_euler_number` where under
  20) are labelled unresolved, not adjudicated as trend.

## Adopted values

Rework 4 closed with all five knobs confirmed kept at their current defaults.
All four sweeps (extraction-level, occupancy, ss-guidance, slat-guidance)
recommended no changes; no confirmation runs were spent because no default moved.

| knob | value | reason |
| --- | --- | --- |
| `iso_level` | 0.0 | ab-extraction-level: ±0.03 range yields no improvement to any topology metric; `strip_interior_faces` closes holes one stage later, so open manifolds are accepted. |
| `sdf_bias` | -1/256 | ab-extraction-level: only arm closing all three subjects (`sdf_bias = 0.0`) was declined for the same reason (watertight output not needed downstream). |
| `occupancy_threshold` | 0.0 | ab-occupancy: live knob (±60 facets clears floor by 5×) but no arm dominates; both directions trade off topology vs surface thickness. |
| `ss_cfg_interval` | [0.5, 1.0] | ab-ss-guidance: live knob but every arm crossing a signal threshold is a regression; incumbent value kept. |
| `ss_rescale_t` | 3.0 | ab-ss-guidance: live knob but every arm crossing a signal threshold is a regression; incumbent value kept. |
| `slat_cfg_interval` | [0.5, 1.0] | ab-slat-guidance (this report): live knob measured to redraw fine surface detail; production value is a visual quality call, not a correctness one. Incumbent kept pending render-sheet review. |
| `slat_rescale_t` | 3.0 | ab-slat-guidance (this report): live knob measured to redraw fine surface detail; production value is a visual quality call, not a correctness one. Incumbent kept pending render-sheet review. |

One decision remains open: occupancy and SLAT guidance are measurably live
knobs, and which arm looks best on each is a human judgment against the render
sheets under `target/knob-sweep/occupancy/<subject>/` and
`target/knob-sweep/slat-guidance/<subject>/`. Adopting a new arm later requires
only changing the default and running its confirmation set — a mechanical
defaults-only change that does not revisit rework 4's sweep results.
