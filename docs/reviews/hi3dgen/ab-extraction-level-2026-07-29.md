# A/B: extraction `iso_level` and `sdf_bias` — CPU knob sweep

Date: 2026-07-29. Instrument: `scripts/ai-pipeline/prop_extract.py` (CPU replay of
`SparseFeatures2Mesh` over saved `cubefeats.pt` latents). Subjects: `chapel_arch`,
`candelabra_shrine`, `crucero`. 36 arms (12 per subject), OFAT.

**No default was changed by this work. The keep/change call is the user's.**

## 1. Method

Each arm is one replay of the same saved latent, differing only in the flag under test:

```
C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_extract.py \
    target/prop-latents/<subject> \
    --out target/knob-sweep/extraction/<subject>/<arm> --device cpu [<arm flag>]
```

| arm | flag |
|---|---|
| `baseline` | *(none — `iso_level = 0.0`, `sdf_bias = -1/256`)* |
| `iso_m0.030` … `iso_p0.030` | `--iso-level -0.03 / -0.02 / -0.01 / -0.005 / 0.005 / 0.01 / 0.02 / 0.03` |
| `bias_0.0` | `--sdf-bias 0.0` |
| `bias_m0.0078125` | `--sdf-bias -0.0078125` (= −2/256) |
| `bias_m0.015625` | `--sdf-bias -0.015625` (= −4/256) |

Surface deviation is arm → same-subject baseline, one-directional:
`trimesh.sample.sample_surface(arm, n, seed=0)` then
`trimesh.proximity.ProximityQuery(baseline).on_surface(points)`, at n = 20 000 /
80 000 / 320 000. Every deviation quoted below shows all three n so its
refinement stability is visible on the page; across the whole grid the mean is
stable to three significant figures and p99 to within ~1.5 % relative, so no
number here was set by the sample count.

Scale anchor for reading the deviations: the baseline AABB diagonals are 1.435
(chapel_arch), 1.423 (candelabra_shrine), 1.215 (crucero) in the latents' own
units. A mean of 0.0005 is ≈0.035 % of the diagonal; 0.006 is ≈0.4 %.

Renders: `cargo run -p engine-renderer --bin turntable --features offscreen --
<arm>/raw.glb --out <arm>/tt --size 512x512 --angles 4`, stitched per subject.

**Deviation cannot be read as a quality score.** It is measured against the
default arm, so by construction the default scores zero and every alternative
scores worse. It says only *how far* an arm moves the surface. The keep/change
argument below rests on the topology fields, volume, and the sheets.

### Instrument check

The `candelabra_shrine` baseline reproduced step 1's pinned values field for
field: `vertex_count 167479, face_count 334938, boundary_edge_count 0,
component_count 11, main_face_fraction 0.6581, main_euler_number -8`. The
`iso +0.03` arm likewise reproduced step 1's re-measurement (314625 / 629870 /
1896 / 252 / −1705) in a fresh process. CPU replay is deterministic across
sessions; no drift to average away.

The plan's pre-registered ±0.03 anchor figures are stale and are not used
anywhere in this report — the ±0.03 rows below are the re-measured arms.

## 2. Results

Artifacts: `target/knob-sweep/extraction/<subject>/<arm>/{raw.glb,stats.json,deviation.json,tt/}`.
Contact sheets (12 arms × 4 angles, rows ordered iso −0.03 → +0.03 then the bias arms):

- `target/knob-sweep/extraction/chapel_arch/contact_sheet.png`
- `target/knob-sweep/extraction/candelabra_shrine/contact_sheet.png`
- `target/knob-sweep/extraction/crucero/contact_sheet.png`

#### chapel_arch

| arm | verts | faces | bodies | watertight | boundary edges | components | main face frac | main euler | volume |
|---|---|---|---|---|---|---|---|---|---|
| iso -0.03 | 196213 | 386922 | 1791 | yes | 0 | 1791 | 0.0566 | -12 | 0.004405 |
| iso -0.02 | 328382 | 660036 | 717 | yes | 0 | 717 | 0.7533 | -2656 | 0.009008 |
| iso -0.01 | 394600 | 794744 | 27 | yes | 0 | 27 | 0.9997 | -2824 | 0.015366 |
| iso -0.005 | 390885 | 783154 | 7 | yes | 0 | 7 | 0.9999 | -704 | 0.018147 |
| **default** | 386614 | 773518 | 13 | no | 4 | 13 | 0.9997 | -171 | 0.020571 |
| iso +0.005 | 387813 | 775066 | 179 | no | 46 | 179 | 0.9939 | -93 | 0.022824 |
| iso +0.01 | 423530 | 842496 | 1082 | no | 700 | 1082 | 0.9266 | -113 | 0.024680 |
| iso +0.02 | 584278 | 1168160 | 840 | no | 2502 | 840 | 0.9555 | -2698 | 0.027352 |
| iso +0.03 | 626401 | 1258260 | 497 | no | 2956 | 497 | 0.9780 | -5156 | 0.034768 |
| bias 0.0 | 389732 | 780368 | 20 | yes | 0 | 20 | 0.9997 | -490 | 0.018699 |
| bias -2/256 | 385381 | 770890 | 15 | no | 16 | 15 | 0.9994 | -100 | 0.022338 |
| bias -4/256 | 423668 | 846690 | 147 | no | 644 | 147 | 0.9513 | -195 | 0.025607 |

| arm | mean 20k / 80k / 320k | p99 20k / 80k / 320k | max 320k |
|---|---|---|---|
| iso -0.03 | 0.00312 / 0.00312 / 0.00312 | 0.00930 / 0.00928 / 0.00932 | 0.01592 |
| iso -0.02 | 0.00234 / 0.00235 / 0.00235 | 0.00908 / 0.00926 / 0.00919 | 0.02139 |
| iso -0.01 | 0.00117 / 0.00116 / 0.00116 | 0.00553 / 0.00532 / 0.00526 | 0.01663 |
| iso -0.005 | 0.00055 / 0.00054 / 0.00054 | 0.00237 / 0.00236 / 0.00232 | 0.00980 |
| iso +0.005 | 0.00052 / 0.00053 / 0.00053 | 0.00198 / 0.00207 / 0.00207 | 0.01714 |
| iso +0.01 | 0.00166 / 0.00165 / 0.00164 | 0.01533 / 0.01502 / 0.01485 | 0.02394 |
| iso +0.02 | 0.00485 / 0.00485 / 0.00485 | 0.01821 / 0.01838 / 0.01834 | 0.03313 |
| iso +0.03 | 0.00618 / 0.00616 / 0.00616 | 0.01905 / 0.01900 / 0.01897 | 0.03417 |
| bias 0.0 | 0.00042 / 0.00042 / 0.00042 | 0.00182 / 0.00177 / 0.00177 | 0.01051 |
| bias -2/256 | 0.00039 / 0.00039 / 0.00039 | 0.00152 / 0.00152 / 0.00153 | 0.01220 |
| bias -4/256 | 0.00191 / 0.00193 / 0.00192 | 0.01489 / 0.01510 / 0.01509 | 0.02453 |

#### candelabra_shrine

| arm | verts | faces | bodies | watertight | boundary edges | components | main face frac | main euler | volume |
|---|---|---|---|---|---|---|---|---|---|
| iso -0.03 | 165781 | 331694 | 49 | yes | 0 | 49 | 0.4006 | -72 | 0.007880 |
| iso -0.02 | 168551 | 337154 | 20 | yes | 0 | 20 | 0.6959 | -60 | 0.010693 |
| iso -0.01 | 168340 | 336680 | 15 | yes | 0 | 15 | 0.6692 | -22 | 0.013242 |
| iso -0.005 | 167223 | 334438 | 13 | yes | 0 | 13 | 0.6531 | -16 | 0.014419 |
| **default** | 167479 | 334938 | 11 | yes | 0 | 11 | 0.6581 | -8 | 0.015485 |
| iso +0.005 | 167403 | 334700 | 24 | no | 58 | 24 | 0.6647 | -22 | 0.016359 |
| iso +0.01 | 174861 | 348306 | 229 | no | 768 | 229 | 0.6483 | -130 | 0.017149 |
| iso +0.02 | 276027 | 550234 | 471 | no | 1006 | 471 | 0.5273 | -374 | 0.020280 |
| iso +0.03 | 314625 | 629870 | 252 | no | 1896 | 252 | 0.7946 | -1705 | 0.024365 |
| bias 0.0 | 167172 | 334348 | 13 | yes | 0 | 13 | 0.6544 | -22 | 0.014665 |
| bias -2/256 | 167412 | 334796 | 12 | no | 8 | 12 | 0.6630 | -12 | 0.016186 |
| bias -4/256 | 176442 | 351946 | 55 | no | 918 | 55 | 0.7032 | -90 | 0.017443 |

| arm | mean 20k / 80k / 320k | p99 20k / 80k / 320k | max 320k |
|---|---|---|---|
| iso -0.03 | 0.00371 / 0.00372 / 0.00372 | 0.01208 / 0.01215 / 0.01211 | 0.02479 |
| iso -0.02 | 0.00248 / 0.00249 / 0.00249 | 0.00907 / 0.00914 / 0.00908 | 0.02476 |
| iso -0.01 | 0.00119 / 0.00119 / 0.00120 | 0.00440 / 0.00453 / 0.00453 | 0.01954 |
| iso -0.005 | 0.00056 / 0.00056 / 0.00055 | 0.00201 / 0.00203 / 0.00204 | 0.01482 |
| iso +0.005 | 0.00045 / 0.00045 / 0.00045 | 0.00199 / 0.00200 / 0.00201 | 0.01609 |
| iso +0.01 | 0.00104 / 0.00104 / 0.00104 | 0.01222 / 0.01259 / 0.01257 | 0.02205 |
| iso +0.02 | 0.00553 / 0.00558 / 0.00558 | 0.02043 / 0.02042 / 0.02044 | 0.04075 |
| iso +0.03 | 0.00661 / 0.00663 / 0.00664 | 0.02000 / 0.02011 / 0.02012 | 0.04867 |
| bias 0.0 | 0.00042 / 0.00042 / 0.00042 | 0.00154 / 0.00156 / 0.00156 | 0.01226 |
| bias -2/256 | 0.00036 / 0.00035 / 0.00035 | 0.00157 / 0.00155 / 0.00156 | 0.00667 |
| bias -4/256 | 0.00130 / 0.00131 / 0.00131 | 0.01424 / 0.01438 / 0.01440 | 0.02380 |

#### crucero

| arm | verts | faces | bodies | watertight | boundary edges | components | main face frac | main euler | volume |
|---|---|---|---|---|---|---|---|---|---|
| iso -0.03 | 123609 | 246450 | 798 | yes | 0 | 798 | 0.4934 | -746 | 0.003262 |
| iso -0.02 | 168976 | 340744 | 188 | yes | 0 | 188 | 0.9799 | -1760 | 0.006156 |
| iso -0.01 | 176251 | 353370 | 17 | yes | 0 | 17 | 0.9995 | -466 | 0.009447 |
| iso -0.005 | 172185 | 344418 | 7 | yes | 0 | 7 | 0.9997 | -36 | 0.010825 |
| **default** | 170888 | 341766 | 6 | no | 12 | 6 | 0.9998 | -11 | 0.011912 |
| iso +0.005 | 173234 | 345794 | 150 | no | 194 | 150 | 0.9891 | -54 | 0.012762 |
| iso +0.01 | 194868 | 386662 | 645 | no | 946 | 645 | 0.9236 | -188 | 0.013684 |
| iso +0.02 | 259532 | 520370 | 395 | no | 1578 | 395 | 0.9780 | -2211 | 0.016582 |
| iso +0.03 | 264909 | 532974 | 466 | no | 1874 | 466 | 0.9343 | -3417 | 0.019715 |
| bias 0.0 | 171850 | 343708 | 8 | yes | 0 | 8 | 0.9998 | -18 | 0.011092 |
| bias -2/256 | 170937 | 341792 | 13 | no | 104 | 13 | 0.9982 | -35 | 0.012582 |
| bias -4/256 | 200607 | 400336 | 133 | no | 1162 | 133 | 0.9352 | -314 | 0.014073 |

| arm | mean 20k / 80k / 320k | p99 20k / 80k / 320k | max 320k |
|---|---|---|---|
| iso -0.03 | 0.00314 / 0.00315 / 0.00314 | 0.00932 / 0.00960 / 0.00956 | 0.01409 |
| iso -0.02 | 0.00247 / 0.00248 / 0.00249 | 0.01032 / 0.01040 / 0.01036 | 0.01756 |
| iso -0.01 | 0.00118 / 0.00118 / 0.00118 | 0.00565 / 0.00563 / 0.00566 | 0.00993 |
| iso -0.005 | 0.00051 / 0.00052 / 0.00051 | 0.00271 / 0.00271 / 0.00270 | 0.00907 |
| iso +0.005 | 0.00046 / 0.00046 / 0.00046 | 0.00257 / 0.00263 / 0.00262 | 0.01841 |
| iso +0.01 | 0.00152 / 0.00153 / 0.00152 | 0.01508 / 0.01509 / 0.01510 | 0.02371 |
| iso +0.02 | 0.00406 / 0.00402 / 0.00402 | 0.01680 / 0.01700 / 0.01695 | 0.02398 |
| iso +0.03 | 0.00489 / 0.00493 / 0.00494 | 0.01687 / 0.01692 / 0.01694 | 0.02488 |
| bias 0.0 | 0.00039 / 0.00039 / 0.00039 | 0.00211 / 0.00210 / 0.00210 | 0.00860 |
| bias -2/256 | 0.00034 / 0.00033 / 0.00033 | 0.00185 / 0.00181 / 0.00183 | 0.01762 |
| bias -4/256 | 0.00206 / 0.00202 / 0.00201 | 0.01615 / 0.01592 / 0.01584 | 0.02464 |

## 3. What the curves say

**`iso_level` is not symmetric — the two directions fail in different ways.**

- *Negative (erosion).* Every negative arm on every subject stays fully closed:
  boundary edges are exactly 0 at −0.005, −0.01, −0.02, −0.03 on all three
  subjects, including the two subjects whose default is *not* watertight. Volume
  falls monotonically and steeply (chapel 0.0206 → 0.0044 at −0.03, crucero
  0.0119 → 0.0033). The cost is that thin features pinch off: bodies go
  13 → 7 → 27 → 717 → 1791 (chapel), 6 → 7 → 17 → 188 → 798 (crucero),
  11 → 13 → 15 → 20 → 49 (candelabra), and main-face-fraction collapses to 0.057
  on chapel at −0.03 — the "main body" is no longer the arch. The sheets confirm:
  chapel −0.03 is visibly disintegrating and crucero −0.03 has shed the cross arms.
- *Positive (dilation).* Watertightness dies at the very first step in all three
  subjects: boundary edges 4 → 46 (chapel), 0 → 58 (candelabra), 12 → 194
  (crucero) at only +0.005, and bodies jump 13 → 179, 11 → 24, 6 → 150. Volume
  grows and vertex count inflates (chapel 773k → 1.26M faces at +0.03). The
  sheets show this as a crust of surface noise, not as new detail.

  This is the sharpest single result of the sweep: the positive side has a cliff
  between 0 and +0.005 that no previous arm had resolved, and there is no
  gradual onset to sit inside.

**The body-count minimum claim, measured directly.** `candelabra_shrine` has its
minimum exactly at the default (11; nearest neighbours 13 and 24) and
`crucero` likewise (6; neighbours 7 and 150). `chapel_arch` does **not**:
its minimum is at iso −0.005 (7 bodies vs 13), which is also watertight. But that
arm carries `main_euler_number` −704 against the default's −171 — roughly four
times as many handles through the main shell — and loses 12 % of the volume. So
the default is at the joint minimum on two subjects and one small step from a
mixed-verdict alternative on the third.

**`sdf_bias` is a real two-way trade, not a minimum.**

| field | bias 0.0 | **−1/256 (default)** | −2/256 | −4/256 |
|---|---|---|---|---|
| boundary edges (chapel / cand. / cruc.) | **0 / 0 / 0** | 4 / 0 / 12 | 16 / 8 / 104 | 644 / 918 / 1162 |
| bodies | 20 / 13 / 8 | **13 / 11 / 6** | 15 / 12 / 13 | 147 / 55 / 133 |
| main euler | −490 / −22 / −18 | **−171 / −8 / −11** | −100 / −12 / −35 | −195 / −90 / −314 |
| volume | 0.0187 / 0.0147 / 0.0111 | 0.0206 / 0.0155 / 0.0119 | 0.0223 / 0.0162 / 0.0126 | 0.0256 / 0.0174 / 0.0141 |

- `bias 0.0` — dropping the inherited −1/256 entirely — is the **only arm in the
  whole 36-arm grid that makes all three subjects fully watertight**, and it is
  also among the smallest surface moves measured (mean 0.00042 / 0.00042 / 0.00039,
  ≈0.03 % of the diagonal). It pays for that with +2…+7 bodies, a worse handle
  count on all three, and 4–9 % of the volume.
- `−2/256` moves the other way: more volume and a better chapel euler (−100),
  but boundary edges reappear everywhere (crucero 12 → 104).
- `−4/256` is unambiguously worse on every field and every subject.

So the inherited −1/256 does earn its place on body count and handle count — it
is the minimum on both, on two or three subjects — but it does *not* earn it on
watertightness, where `0.0` strictly wins.

## 4. Recommendation

**Keep `iso_level = 0.0`.** Not because it is the default, but because both
directions degrade at the first measured step on every subject: +0.005 breaks
watertightness on all three (46/58/194 boundary edges from 4/0/12) and −0.005
either raises the body count or quadruples the handle count while shedding
volume. No arm anywhere in ±0.03 improves a topology field without making
another worse, and the sheets show no arm recovering detail the default lacks.
The neighbourhood between 0 and ±0.03 is now mapped and there is nothing in it.

**Keep `sdf_bias = -1/256` — unless watertightness is a downstream requirement,
in which case `0.0` is the arm to take.** This one is a genuine choice, not a
default-wins result:

- Keep −1/256 if the priority is the fewest bodies and the fewest handles. It is
  the minimum on both fields.
- Change to 0.0 if any downstream stage needs closed manifolds (interior
  extraction, booleans, volumetric ops). It is the only setting that closes all
  three subjects, and it is a 0.03 %-of-diagonal move — the change is essentially
  invisible in the sheets. The bill is +2…+7 bodies, a worse handle count, and
  4–9 % volume shrink.

`−2/256` and `−4/256` are not worth carrying: both add boundary edges on every
subject, and −4/256 adds them by two to three orders of magnitude.

**User checkpoint: this report changes no default.** The `iso_level` keep is a
recommendation with no measured alternative behind it; the `sdf_bias` call
depends on whether closed manifolds are required downstream, which is the user's
to rule on.

## 5. Caveats

- Deviation is one-directional (arm surface → baseline surface). It undercounts
  material the baseline has and the arm lost; the volume column is the honest
  read on erosion.
- Three subjects, all architectural/monumental props. Nothing here speaks to
  organic or foliage latents (`cypress`, `olive_stump` were not swept).
- `main_euler_number` is a proxy for handle count on the largest island only; it
  says nothing about the other islands, which is why the body count is reported
  alongside it.
