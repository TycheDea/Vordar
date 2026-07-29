# Hi3DGen multi-view conditioning A/B — verdict

Date: 2026-07-29. Rework 2 (`plan-rework2-multiview-conditioning-2026-07-28.md`
step 8). 18 GPU candidates: 3 arms (`sv` = front only, `mv-stoch` =
`--mv-mode stochastic`, `mv-multi` = `--mv-mode multidiffusion`) x seeds
{0,1,2} x 2 subjects (`olive_stump` prop, `pilgrim_monk` character). All
without `--deterministic` (rework 6 measured it as only partially effective;
see below).

## Verdict

**Do NOT adopt multi-view conditioning.** The mechanism (`--view` /
`--mv-mode`, landed in step 2) stays plumbed, opt-in, and dormant — it is not
wired into `gen_prop.py` / `gen_character.py` defaults.

Two evidence lines carry this verdict — items 1 and 2 below, which agree with
each other and neither of which depends on orientation. Item 3 is null rather
than supporting.

1. **Visual review of the 18 contact sheets** (`target/mv-ab/<subject>/sheets/<arm>_<seed>/contact_sheet.png`) —
   detail retention on both subjects orders **sv > mv-stoch > mv-multi**.
   - `olive_stump` seed 1 (`target/mv-ab/olive_stump/sheets/{sv,mv-stoch,mv-multi}_1/contact_sheet.png`):
     `sv` holds crisp bark striation, distinct trunk burls, an articulated
     root flare. `mv-stoch` is intermediate. `mv-multi` is visibly smoothed —
     burls washed out, striations softened, branch stubs blunted.
   - `pilgrim_monk` seed 0 (`target/mv-ab/pilgrim_monk/sheets/{sv,mv-stoch,mv-multi}_0/contact_sheet.png`):
     the concept's satchel (strap across the chest, pouch at the hip) is
     reproduced by `sv`, faint in `mv-stoch`, and absent in `mv-multi`, which
     keeps only the cord belt. This seed was chosen in step 5 precisely
     because the satchel was consistent across its three concept panels, so
     its loss is a fidelity failure against the provided conditioning, not a
     wash.
2. **`vertex_count`** (no orientation dependence, no free parameter) — prop:
   both MV arms give fewer vertices than `sv` at all 3 seeds, min|delta|
   21,999 (stoch) / 16,009 (multi) against a noise floor of 221 — 3-8% fewer
   vertices. Character: indistinguishable at N=3.
3. **Connectivity** (`main_face_fraction`, `component_count`,
   `boundary_edge_count`): indistinguishable at N=3 on both subjects under
   the pre-registered thresholds.

Multi-view conditioning as implemented reduces detail rather than adding
back/side fidelity, and multidiffusion is the more destructive mode.

## WITHDRAWN: every `iou_front` / `iou_back` / `iou_side` claim in both `ab.json` files

**A reader must not carry any IoU number in either `ab.json` forward.**

`fit_yaw`'s argmax is degenerate. Measured across all 18 candidates (from
`target/mv-ab/olive_stump/yaw_convergence.json`,
`target/mv-ab/olive_stump/yaw_convergence2.json`,
`target/mv-ab/pilgrim_monk/yaw_curves.json`), the gap between the IoU at the
fitted argmax and the IoU at (argmax + 180 deg) runs **0.0014–0.1053**, and
**7 of 18 candidates fall below 0.01** — while the cross-arm effects being
claimed on `iou_front` were 0.0037–0.0081, the same order as the ambiguity
itself.

Confirmed visually: all three `pilgrim_monk` `sv` candidates fit ~180 deg off
(the fitted "front" render shows the monk's back — hood from behind, no face,
no satchel) while the MV arms fit correctly. Spot-check on seed 0's full curve
(`yaw_curves.json` `sv/0`): argmax 150 deg -> IoU 0.88176; argmax+180 (330
deg) -> IoU 0.87311; gap 0.00865, below the 0.01 ambiguity floor and an order
below the visual difference between "front" and "back". So step 7's reported
"MV beats sv on iou_front" compared MV's front silhouette against sv's back
silhouette — not a conditioning effect.

**Step 6's prop `iou_front` deltas did survive both free-parameter checks**
that were run against them — scan step refined 5deg -> 1deg -> 0.5deg (sign
intact, min|delta| 0.00358-0.00434) and raster resolution 512/256 ->
1024/512 (sign intact, min|delta| 0.00250-0.00304, every value shifting down
0.002-0.005) — **but 2 of its 9 candidates (both seed 0) also sit below the
0.01 ambiguity gap**, so that claim is not clean at N=3 either. It is
recorded as withdrawn along with the rest; its sign happened to agree with
the visual read, which is not the same as being licensed by the metric.

## Correction to step 7's `ab.json`

The `"systematic MV-vs-SV yaw difference"` step 7's `ab.json` reports for
`pilgrim_monk` (`sv` ~150-160 deg, MV arms ~5-15 deg) is **NOT a conditioning
effect**. It is which of two near-tied silhouette peaks (front vs. back)
happened to win the argmax on each arm. Step 7's Path asked for a systematic
yaw difference to be reported as a conditioning effect; this one is not, and
the `ab.json` text making that claim is superseded by this report.

## Instrument fixed at `865db1d` (already landed, not redone here)

`fit_yaw` is now a two-stage fit (5deg coarse sweep, then 1deg refine over
+/-4deg of the coarse argmax — converged, since 1deg -> 0.5deg moves IoU by
at most +0.0003), and `main()` now emits `iou_at_yaw_plus_180` and
`front_back_peak_gap` so the degeneracy is visible in every future run. No
threshold was added: emitting the raw gap is the fix, and a reader compares
it against whatever effect size they intend to claim.

## Floors — three limitations, stated plainly

- `target/mv-ab/noise_floor.json` was measured on **chapel_arch**
  (`source_glb: target/prop-batch/b3/arch/cand_0/concept.png`,
  `front_matte: target/prop-solid-validation/chapel_arch_e2e/cand_0/concept_rgba.png`),
  a different subject from either A/B subject. Every floor applied in both
  `ab.json` files (`iou_front` 0.000146, `vertex_count` 221,
  `main_face_fraction` 1.06e-05) is therefore a **cross-subject** floor.
- `iou_back` / `iou_side` have **no floor at all** — the noise-floor probe
  only ran `--front`. So back/side fidelity, the rework's actual question,
  was never adjudicable by metric and was answered visually instead (see
  Verdict, item 1).
- `component_count` (10) / `boundary_edge_count` (20) used the
  pre-registered order-10 / order-20 thresholds from
  `docs/reviews/hi3dgen/noise-floor-2026-07-29.md`, not `noise_floor.json`'s
  own N=3 values of 1 and 0, which would license noise as signal.

## Rework 6 (GPU determinism) evidence

`--deterministic` is only partially effective. `target/mv-ab/noise_floor.json`'s
`determinism_probe`: three runs of `prop_hi3dgen.py ... --deterministic
--seed 0`, `raw.glb` sha256 — `det-r1` and `det-r2` byte-identical
(`fa35dc9748...`), `det-r3` differs (`a5d846e91e...`). 2 of 3 runs
byte-identical; spconv sparse convolutions run outside
`torch.use_deterministic_algorithms`. All 18 A/B candidates ran without
`--deterministic`, so both arms and the noise floor share one (non-
deterministic) regime.

*(Appended to finding 6's entry in `reworks-hi3dgen-2026-07-28.md` — see
that file.)*

## Finding 20 resolution

The cross-call-pattern gap (duplicated-view multidiffusion vs. its
single-view twin) measured **1025 vertices (0.267%)** against the
same-call-pattern floor of 221 (0.0575%), a factor of 4.6, with
`ss_active_voxels` 14588 (sv) vs 14591 (dup). The mechanism is batch shape —
2 conditioning rows vs. 1 selects different kernels and different float
rounding over 8 steps — not the averaging itself. Resolution: this belongs
in the fork's algebraic contract test (already landed, fork `3488bbf`); the
answer is **no band, not a wider one**.

## No user decision on default mode

Neither mode is adopted, so the plan's "MV-S vs MV-M default" question is
moot. Recorded here rather than left open.

## What would reopen the mechanism

(a) an orientation-robust fidelity metric — the promising route is
normal-map correlation, since Hi3DGen already writes its predicted front
normal map as `normal.png` in every candidate directory, and normals differ
strongly between front and back exactly where silhouettes do not (queued as
finding 24 below);
(b) a same-subject noise floor that includes `iou_back` / `iou_side`
(queued as finding 25 below);
(c) evidence that the detail loss is a mode or weighting issue rather than
intrinsic to averaging conditioning rows.

## Per-seed tables — `olive_stump` (prop)

Source: `target/mv-ab/olive_stump/ab.json`.

### Claimed metrics

| metric | arm | seed 0 | seed 1 | seed 2 | floor | claim |
|---|---|---|---|---|---|---|
| vertex_count | sv | 556526 | 421970 | 533988 | 221 | — |
| vertex_count | mv-stoch | 534527 | 398251 | 497598 | 221 | **lower than sv** (min\|delta\| 21999) |
| vertex_count | mv-multi | 540517 | 394181 | 491367 | 221 | **lower than sv** (min\|delta\| 16009) |
| main_face_fraction | sv | 0.99365 | 0.99650 | 0.99881 | 1.06e-05 | — |
| main_face_fraction | mv-stoch | 0.99424 | 0.98775 | 0.99761 | 1.06e-05 | indistinguishable at N=3 |
| main_face_fraction | mv-multi | 0.99371 | 0.98732 | 0.99740 | 1.06e-05 | indistinguishable at N=3 |
| component_count | sv | 17 | 20 | 13 | 10 | — |
| component_count | mv-stoch | 31 | 38 | 21 | 10 | indistinguishable at N=3 (min\|delta\| 8) |
| component_count | mv-multi | 33 | 36 | 14 | 10 | indistinguishable at N=3 (min\|delta\| 1) |
| boundary_edge_count | sv | 8 | 24 | 110 | 20 | — |
| boundary_edge_count | mv-stoch | 16 | 30 | 22 | 20 | indistinguishable at N=3 (sign not shared) |
| boundary_edge_count | mv-multi | 4 | 12 | 34 | 20 | indistinguishable at N=3 (min\|delta\| 4) |

### WITHDRAWN — silhouette IoU (see withdrawal section above)

| metric | arm | seed 0 | seed 1 | seed 2 |
|---|---|---|---|---|
| iou_front | sv | 0.80682 | 0.81432 | 0.85374 |
| iou_front | mv-stoch | 0.79405 | 0.81064 | 0.84352 |
| iou_front | mv-multi | 0.79699 | 0.80857 | 0.84072 |
| iou_back (no floor) | sv | 0.75235 | 0.75543 | 0.75960 |
| iou_back (no floor) | mv-stoch | 0.77238 | 0.79001 | 0.75681 |
| iou_back (no floor) | mv-multi | 0.78920 | 0.79138 | 0.75653 |
| iou_side (no floor) | sv | 0.73502 | 0.72304 | 0.75504 |
| iou_side (no floor) | mv-stoch | 0.75583 | 0.79831 | 0.73223 |
| iou_side (no floor) | mv-multi | 0.75627 | 0.78369 | 0.72602 |

`fitted_yaw_deg`: sv {65, 180, 165}, mv-stoch {330, 170, 195}, mv-multi
{150, 175, 195} — the fitted-front azimuth moving around at this ambiguity
level is expected given the argmax degeneracy above, not a conditioning
signal.

## Per-seed tables — `pilgrim_monk` (character)

Source: `target/mv-ab/pilgrim_monk/ab.json`. `cross_subject_floor_caveat`
applies (floors measured on chapel_arch, not this subject).

### Claimed metrics

| metric | arm | seed 0 | seed 1 | seed 2 | floor | claim |
|---|---|---|---|---|---|---|
| vertex_count | sv | 185822 | 188692 | 183974 | 221 | — |
| vertex_count | mv-stoch | 184884 | 179284 | 196589 | 221 | indistinguishable at N=3 (sign not shared) |
| vertex_count | mv-multi | 185383 | 178121 | 194035 | 221 | indistinguishable at N=3 (sign not shared) |
| main_face_fraction | sv | 0.96362 | 0.93729 | 1.0 | 1.06e-05 | — |
| main_face_fraction | mv-stoch | 0.96720 | 0.93027 | 0.99871 | 1.06e-05 | indistinguishable at N=3 (sign not shared) |
| main_face_fraction | mv-multi | 0.99972 | 0.93316 | 0.99775 | 1.06e-05 | indistinguishable at N=3 (sign not shared) |
| component_count | sv | 4 | 12 | 1 | 10 | — |
| component_count | mv-stoch | 6 | 16 | 12 | 10 | indistinguishable at N=3 (min\|delta\| 2) |
| component_count | mv-multi | 2 | 15 | 8 | 10 | indistinguishable at N=3 (sign not shared) |
| boundary_edge_count | sv | 0 | 0 | 0 | 20 | — |
| boundary_edge_count | mv-stoch | 0 | 0 | 0 | 20 | indistinguishable at N=3 (delta 0) |
| boundary_edge_count | mv-multi | 0 | 0 | 0 | 20 | indistinguishable at N=3 (delta 0) |

### WITHDRAWN — silhouette IoU and yaw-difference claim (see corrections above)

| metric | arm | seed 0 | seed 1 | seed 2 |
|---|---|---|---|---|
| iou_front | sv | 0.88176 | 0.91416 | 0.90094 |
| iou_front | mv-stoch | 0.91686 | 0.93323 | 0.90774 |
| iou_front | mv-multi | 0.91579 | 0.93384 | 0.90902 |
| iou_back (no floor) | sv | 0.88014 | 0.90319 | 0.88358 |
| iou_back (no floor) | mv-stoch | 0.92357 | 0.95057 | 0.92054 |
| iou_back (no floor) | mv-multi | 0.91774 | 0.92621 | 0.91982 |
| iou_side (no floor) | sv | 0.73396 | 0.76376 | 0.77806 |
| iou_side (no floor) | mv-stoch | 0.85583 | 0.88441 | 0.82387 |
| iou_side (no floor) | mv-multi | 0.82717 | 0.88716 | 0.82019 |
| fitted_yaw_deg | sv | 150 | 160 | 160 |
| fitted_yaw_deg | mv-stoch | 10 | 5 | 15 |
| fitted_yaw_deg | mv-multi | 15 | 185 | 15 |

The `ab.json`'s own `summary.fitted_yaw` text calling this a conditioning
effect ("the view set is steering which way the candidate ends up facing")
is the claim corrected above — it is the front/back argmax tie, confirmed by
the seed-0 curve gap (0.00865, `target/mv-ab/pilgrim_monk/yaw_curves.json`
`sv/0`).

## Artifacts

- Contact sheets: `target/mv-ab/olive_stump/sheets/<arm>_<seed>/contact_sheet.png`,
  `target/mv-ab/pilgrim_monk/sheets/<arm>_<seed>/contact_sheet.png` (18 total).
- Raw data: `target/mv-ab/olive_stump/ab.json`, `target/mv-ab/pilgrim_monk/ab.json`.
- Noise floor / determinism probe: `target/mv-ab/noise_floor.json`.
- Yaw-degeneracy support: `target/mv-ab/olive_stump/yaw_convergence.json`,
  `target/mv-ab/olive_stump/yaw_convergence2.json`,
  `target/mv-ab/pilgrim_monk/yaw_curves.json`.
