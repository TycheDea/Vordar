# Hi3DGen repeat noise floor, and the occupancy curves that place the threshold arms

2026-07-29. Rework 4 step 4. Three byte-identical repeats of one seed per
subject, so every later A/B row in this campaign can be read against a measured
floor instead of against zero.

## What ran

Nine runs, one candidate each, distinct `--out` per repeat so the resume-skip
cannot serve a cached candidate:

```
C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_hi3dgen.py \
    target/prop-batch/b3/arch/cand_0/concept.png \
    --out target/knob-sweep/floor/chapel_arch/r<k> --seed 0 --dump-ss-logits
                target/prop-batch/candelabra-z/cand_4/concept.png  --seed 4
                target/prop-batch/b3/crucero/cand_21/concept.png   --seed 21
```

All defaults otherwise: 50/6 sampler steps, cfg 5.0/5.0, cfg interval [0.5, 1.0],
rescale_t 3.0/3.0, `occupancy_threshold` 0.0, extraction `res` 256,
`iso_level` 0.0, `sdf_bias` -0.00390625, `min_component_fraction` 1e-4,
turbo normal predictor. Each run 44-53 s wall (of which ~27 s model load),
9 runs in ~7 min GPU against the ≤25 min budgeted in §8.

Artifacts under `target/knob-sweep/floor/<subject>/r<k>/cand_<seed>/`:
`raw.glb`, `hi3dgen_manifest.json`, `ss_logits.npy`, `normal.png`,
`concept_rgba.png`. Per-subject rollups: `floor_stats.json`,
`floor_dev80000.json`, `floor_occ.json`.

## Floor table: three raw repeat values, then max − min

Spread is `max − min` across the three repeats; the percentage is that spread
over the repeat mean. `topo_stats` fields are `prop_extract.topo_stats`
(`scripts/ai-pipeline/prop_extract.py:31`), computed on the exported `raw.glb`.

### chapel_arch (concept `b3/arch/cand_0`, seed 0)

| metric | r1 | r2 | r3 | floor | rel. |
| --- | --- | --- | --- | --- | --- |
| vertex_count | 384284 | 384172 | 384274 | 112 | 0.0291% |
| face_count | 768858 | 768626 | 768822 | 232 | 0.0302% |
| volume | 0.0208617422 | 0.0208623371 | 0.0208702005 | 8.458e-06 | 0.0405% |
| body_count | 9 | 9 | 11 | 2 | 20.7% |
| component_count | 9 | 9 | 11 | 2 | 20.7% |
| boundary_edge_count | 4 | 4 | 4 | 0 | 0% |
| main_boundary_edge_count | 4 | 4 | 4 | 0 | 0% |
| main_face_fraction | 0.9997 | 0.9997 | 0.9996 | 0.0001 | 0.0100% |
| main_euler_number | -163 | -159 | -159 | 4 | 2.49% |
| degenerate_face_count | 0 | 0 | 0 | 0 | - |
| ss_active_voxels | 14588 | 14588 | 14588 | 0 | 0% |
| is_watertight | false | false | false | 0 (identical) | - |

### candelabra_shrine (concept `candelabra-z/cand_4`, seed 4)

| metric | r1 | r2 | r3 | floor | rel. |
| --- | --- | --- | --- | --- | --- |
| vertex_count | 167870 | 167855 | 167866 | 15 | 0.0089% |
| face_count | 335724 | 335694 | 335716 | 30 | 0.0089% |
| volume | 0.0154566715 | 0.0154545740 | 0.0154581889 | 3.615e-06 | 0.0234% |
| body_count | 10 | 10 | 10 | 0 | 0% |
| component_count | 10 | 10 | 10 | 0 | 0% |
| boundary_edge_count | 0 | 0 | 0 | 0 | - |
| main_boundary_edge_count | 0 | 0 | 0 | 0 | - |
| main_face_fraction | 0.6621 | 0.6620 | 0.6621 | 0.0001 | 0.0151% |
| main_euler_number | -6 | -6 | -6 | 0 | 0% |
| degenerate_face_count | 0 | 0 | 0 | 0 | - |
| ss_active_voxels | 8417 | 8417 | 8417 | 0 | 0% |
| is_watertight | true | true | true | 0 (identical) | - |

### crucero (concept `b3/crucero/cand_21`, seed 21)

| metric | r1 | r2 | r3 | floor | rel. |
| --- | --- | --- | --- | --- | --- |
| vertex_count | 180776 | 180729 | 180762 | 47 | 0.0260% |
| face_count | 361534 | 361454 | 361524 | 80 | 0.0221% |
| volume | 0.0131029145 | 0.0131042769 | 0.0131051282 | 2.214e-06 | 0.0169% |
| body_count | 9 | 7 | 7 | 2 | 26.1% |
| component_count | 9 | 7 | 7 | 2 | 26.1% |
| boundary_edge_count | 12 | 16 | 8 | 8 | 66.7% |
| main_boundary_edge_count | 12 | 16 | 8 | 8 | 66.7% |
| main_face_fraction | 0.9998 | 0.9999 | 0.9999 | 0.0001 | 0.0100% |
| main_euler_number | -13 | -18 | -16 | 5 | 31.9% |
| degenerate_face_count | 0 | 2 | 0 | 2 | - |
| ss_active_voxels | 6715 | 6715 | 6715 | 0 | 0% |
| is_watertight | false | false | false | 0 (identical) | - |

### What the table says about each metric

- **Counts and volume are usable.** vertex/face floors are 0.009-0.030%,
  volume 0.017-0.041%. A sweep arm has to move these by well over 0.05% before
  the move is the arm's and not the GPU's.
- **The small-integer topology metrics are not usable at this noise.**
  `body_count`/`component_count` moved 9 -> 11 (chapel) and 9 -> 7 (crucero)
  between byte-identical runs; `boundary_edge_count` moved 12 -> 16 -> 8 and
  `main_euler_number` by 5 on crucero. These are counts of features at the
  scale of the float noise itself: a stray 20-face island appearing or a
  4-edge hole closing costs one body or four boundary edges. For steps 5-7,
  only a change of order **10 bodies / 20 boundary edges / 20 Euler** on a
  subject counts as signal; anything smaller is unresolved and must be
  reported as such, not as a delta.
- **`ss_active_voxels` has an exactly zero floor** - see the occupancy section.

## Deviation floor

Symmetric sampled surface deviation between two repeat meshes, as a fraction of
the bbox diagonal (the repo's deviation convention, per
`scripts/ai-pipeline/proptex/export.py:33-38`). The instrument, driven over the
three repeat pairs:

```python
def deviation(a, b, n, seed):
    pa, _ = trimesh.sample.sample_surface(a, n, seed=seed)
    pb, _ = trimesh.sample.sample_surface(b, n, seed=seed + 1)
    da = trimesh.proximity.closest_point(b, pa)[1]
    db = trimesh.proximity.closest_point(a, pb)[1]
    d = np.concatenate([da, db])
    diag = float(np.linalg.norm(a.bounding_box.extents))
    return d.mean() / diag, np.percentile(d, 99.9) / diag, d.max() / diag
```

All three pairs at 80k samples per direction (units: 1e-6 of the bbox diagonal):

| subject | pair | mean | p99.9 | max |
| --- | --- | --- | --- | --- |
| chapel_arch | r1-r2 | 14.36 | 193.7 | 1031 |
| chapel_arch | r1-r3 | 13.78 | 188.7 | 1074 |
| chapel_arch | r2-r3 | 14.90 | 187.1 | 731 |
| candelabra_shrine | r1-r2 | 7.02 | 118.7 | 761 |
| candelabra_shrine | r1-r3 | 7.17 | 123.9 | 1035 |
| candelabra_shrine | r2-r3 | 7.97 | 145.4 | 828 |
| crucero | r1-r2 | 15.76 | 382.8 | 1227 |
| crucero | r1-r3 | 15.45 | 415.2 | 1535 |
| crucero | r2-r3 | 15.75 | 405.7 | 1540 |

**Deviation floor** (worst pair per subject): chapel_arch mean 14.9e-6 /
p99.9 194e-6; candelabra_shrine 8.0e-6 / 145e-6; crucero 15.8e-6 / 415e-6 of
the diagonal. A step 5-7 arm whose deviation from baseline sits at or under
these is geometrically indistinguishable from a rerun.

### Stability triple

Pair r1-r2 recomputed at 20k / 80k / 320k samples per direction, to show the
floor is a property of the meshes and not of the sample count:

| subject | stat | 20k | 80k | 320k | spread |
| --- | --- | --- | --- | --- | --- |
| chapel_arch | mean | 14.45 | 14.36 | 14.34 | 0.8% |
| chapel_arch | p99.9 | 195.8 | 193.7 | 192.5 | 1.7% |
| chapel_arch | max | 690.6 | 1031 | 1486 | 115% |
| candelabra_shrine | mean | 7.10 | 7.02 | 7.00 | 1.5% |
| candelabra_shrine | p99.9 | 131.0 | 118.7 | 117.1 | 11.9% |
| candelabra_shrine | max | 728.5 | 760.6 | 1053 | 44% |
| crucero | mean | 15.92 | 15.76 | 15.69 | 1.5% |
| crucero | p99.9 | 391.1 | 382.8 | 399.2 | 4.2% |
| crucero | max | 1133 | 1227 | 1487 | 31% |

**mean** is converged: 16x more samples moves it by ≤1.5%, so 80k is already in
its limit. **p99.9** is converged on chapel_arch and crucero and converging on
candelabra_shrine (20k -> 80k costs 9.4%, 80k -> 320k 1.3%), so 80k is
acceptable for it and 20k is not. **max** grows monotonically with sample count
on all three subjects - it is an extreme-value statistic with no limit at this
sample budget, and must not be used as a criterion in steps 5-7. Report mean
and p99.9 only.

## Occupancy curves

Active-cell count against the `--occupancy-threshold` value `t`, i.e.
`(ss_logits > t).sum()` over the 64^3 = 262144-cell decoded grid. The logits are
a finite set, so this step function is exact - it is evaluated directly on the
sorted logits, with no interpolation and no bin width to choose. All three
repeats are shown side by side; `jit` is their max − min at that threshold.

| t | chapel r1/r2/r3 | jit | candelabra r1/r2/r3 | jit | crucero r1/r2/r3 | jit |
| --- | --- | --- | --- | --- | --- | --- |
| -60 | 17454 / 17454 / 17454 | 0 | 9351 / 9351 / 9351 | 0 | 7476 / 7476 / 7476 | 0 |
| -40 | 15829 / 15829 / 15829 | 0 | 8722 / 8722 / 8722 | 0 | 7009 / 7009 / 7009 | 0 |
| -30 | 15318 / 15318 / 15318 | 0 | 8602 / 8602 / 8602 | 0 | 6880 / 6880 / 6880 | 0 |
| -20 | 14992 / 14992 / 14992 | 0 | 8523 / 8523 / 8523 | 0 | 6816 / 6816 / 6816 | 0 |
| -12 | 14816 / 14816 / 14816 | 0 | 8470 / 8470 / 8470 | 0 | 6769 / 6769 / 6769 | 0 |
| -8 | 14735 / 14735 / 14735 | 0 | 8454 / 8454 / 8454 | 0 | 6747 / 6747 / 6747 | 0 |
| -4 | 14672 / 14672 / 14672 | 0 | 8433 / 8433 / 8433 | 0 | 6728 / 6728 / 6728 | 0 |
| -1 | 14610 / 14610 / 14610 | 0 | 8419 / 8419 / 8419 | 0 | 6715 / 6715 / 6715 | 0 |
| **0** | **14588** / 14588 / 14588 | 0 | **8417** / 8417 / 8417 | 0 | **6715** / 6715 / 6715 | 0 |
| +1 | 14571 / 14571 / 14571 | 0 | 8413 / 8413 / 8413 | 0 | 6710 / 6710 / 6710 | 0 |
| +4 | 14511 / 14511 / 14511 | 0 | 8404 / 8404 / 8404 | 0 | 6695 / 6695 / 6695 | 0 |
| +8 | 14446 / 14446 / 14446 | 0 | 8383 / 8383 / 8383 | 0 | 6680 / 6680 / 6680 | 0 |
| +12 | 14374 / 14374 / 14374 | 0 | 8365 / 8365 / 8365 | 0 | 6664 / 6664 / 6664 | 0 |
| +20 | 14205 / 14205 / 14205 | 0 | 8323 / 8323 / 8323 | 0 | 6626 / 6626 / 6626 | 0 |
| +30 | 13909 / 13909 / 13909 | 0 | 8242 / 8242 / 8242 | 0 | 6567 / 6567 / 6567 | 0 |
| +40 | 13520 / 13520 / 13520 | 0 | 8131 / 8131 / 8131 | 0 | 6468 / 6468 / 6468 | 0 |
| +60 | 11748 / 11748 / 11748 | 0 | 7639 / 7639 / 7639 | 0 | 5864 / 5864 / 5864 | 0 |

Logit ranges: chapel_arch [-220.82, 178.67], candelabra_shrine
[-209.01, 181.51], crucero [-207.83, 193.18]. The full 0.25-spaced curve is in
each subject's `floor_occ.json`.

**The curve has no run-to-run jitter at all**, and the reason is stronger than
"small": the three `ss_logits.npy` files are byte-identical per subject
(sha256 `eac39421...` chapel, `9ac18c60...` candelabra, `054da5d8...` crucero).
The sparse-structure stage is bit-reproducible at a fixed seed on this machine;
every metric spread in the floor table above is produced downstream of it, in
the SLat sampler and the extractor. That also explains the exactly-zero
`ss_active_voxels` floor - it is the same number computed from the same bits,
not a coincidence of thresholding.

## Pre-registered arms for step 5

The Path asks for arm values "where the active count moves by clearly more than
its measured repeat jitter". That jitter is exactly zero, so it cannot be the
yardstick - any threshold at all would pass it. The yardstick used instead is
the floor that actually limits step 5's readout: the **vertex-count floor**,
worst case 0.029% (chapel_arch). Arms are placed so the implied change in
active voxels clears that by at least ~35x on the least responsive subject
(candelabra_shrine), with the far pair an order of magnitude beyond the near
pair.

**Chosen: `--occupancy-threshold` ∈ {-60, -20, +20, +60}**, against the 0.0
baseline. Implied active-voxel counts, read off the exact curves above:

| arm | chapel_arch | candelabra_shrine | crucero |
| --- | --- | --- | --- |
| -60 | 17454 (+19.65%) | 9351 (+11.10%) | 7476 (+11.33%) |
| -20 | 14992 (+2.77%) | 8523 (+1.26%) | 6816 (+1.50%) |
| 0.0 (baseline) | 14588 | 8417 | 6715 |
| +20 | 14205 (-2.63%) | 8323 (-1.12%) | 6626 (-1.33%) |
| +60 | 11748 (-19.47%) | 7639 (-9.24%) | 5864 (-12.67%) |

Two below 0.0 and two above, as the Path requires. The near pair is the
smallest move that is unambiguously above the mesh floor on every subject
(≥1.1%); the far pair is chosen at ±60 because it is where chapel_arch's
response is near-symmetric (+19.65% / -19.47%), and it stays inside the logit
range on all three subjects. **These values are fixed as of this document and
may not be revised once step 5's results are seen.**

## Verification predicate

- Every metric in the floor table is stated as its three raw repeat values, not
  only as a spread.
- Vertex-count floor against rework 6's measured ~0.012% (541220/541286/541242,
  66 / 541249): this run measures 0.0291% (chapel_arch), 0.0089%
  (candelabra_shrine), 0.0260% (crucero). Same order of magnitude, worst case
  2.4x rework 6's value and best case 0.7x of it, with rework 6's number inside
  the range spanned by the three subjects. No 10x excursion, so the harness is
  the one rework 6 measured and the floor is the same phenomenon.
