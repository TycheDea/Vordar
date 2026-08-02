# Albedo and beauty macro-band table — all seven generated props, all three photoscan controls

Date: 2026-08-01
Scope: measurement only. Offscreen GPU stills through the shipped `asset_inspect`
path plus CPU analysis. No generation, no inference, no shipped asset or pipeline
code touched.

This is the probe `docs/reviews/town/decimation-attribution-2026-08-01.md` §8(c)
asked for: `results.json` carried a macro-band figure for exactly two assets, so
the campaign's **60–124 mm residual** could not be regressed against anything.

---

## Verdict

**The 60–124 mm band deficit does not survive. It was an artifact of measuring
two assets under one wrong assumption.**

The prior figure labelled both assets' macro frames with a single
`0.4854 mm/px`, derived from `2·0.6·tan 22.5°/1024` — i.e. from the *assumption
that the visible surface sits at the macro camera's 0.6 m aim distance*.
Measured per pixel in those same frames, `broken_column`'s surface really is at
0.68–0.78 m (label 1.3× too fine), but `rock_face_01`'s is at **2.6–17.2 m**
(label **14–26× too fine**). The row that read

> broken_column 0.80 · rock_face_01 2.08 &nbsp;&nbsp;"at 62.1 mm"

is comparing **~81 mm of column** against **~0.9–1.6 m of cliff**. It is not a
band comparison at all.

Measured correctly, per pixel, over ten assets and eight octaves:

- **In the albedo channel there is no population-wide macro deficit.** In
  absolute L* terms five of seven props exceed all three controls at 62 mm.
  Normalised for mean lightness (the controls are darker), the props *straddle*
  the controls: three below the control range at 62 mm (`gravestone` 1.42,
  `chapel_arch` 2.25, `broken_column` 2.92 against controls 3.20/3.87/4.68,
  ×100), one inside, three above. The largest per-asset shortfall is **2.3×**,
  on `gravestone`, not the 2.6× the prior figure attributed to the whole
  population — and the rank correlation between albedo band energy and the blind
  ranking is **positive** (ρ = +0.3…+0.6): more albedo macro structure predicts a
  *worse* read, not a better one.
- **The props and the controls diverge in a different quantity: the
  beauty-over-albedo gain** `G(λ) = E_beauty(λ)/E_albedo(λ)` — how much of the
  colour map's structure the material actually converts into light response. At
  the `gameplay` framing every control is ≥ 1.23 in every band it covers, while
  three of seven props never exceed 1.06 and two of those fall below 1.00 in the
  macro bands (lighting *removes* structure). The separation is partial — two
  props (`chapel_arch`, `gravestone`) overlap the controls — but `G` is the best
  predictor of the blind ranking tested here (ρ = **−0.90** at 31 mm), and it is
  the only quantity in the study that orders the two populations at all.
- **The roughness candidate correlates, weakly and with n = 1 on the winning
  side.** `rock_face_01` is the only asset in the study with sustained roughness
  variation from 31 mm to 497 mm (band std 0.78–0.88/255, flat across the whole
  ladder). The other two controls have essentially none (0.04–0.32), and they
  were never in a blind test. Three generated props have **exactly zero**
  (0.01–0.08). The statistic is zero-capable and it comes out zero.
- **A large asymmetry the campaign has not been pricing: the controls ship no
  `occlusionTexture` at all.** Their AO lives in the ARM red channel, which the
  shipped glTF never wires up, so the engine never samples it. The `ao` debug
  channel band energy is **0.00 at every band for all three controls** and
  2.0–8.1 for every generated prop. The props have baked AO the controls lack,
  and still lose.

**What the campaign has actually been comparing:** `rock_face_01` at its shipped
scale is an **18 × 13 × 14 m cliff**, 370 m² of surface. `broken_column` is
**0.56 × 1.71 × 0.56 m**. Under the fixed-metric-distance `macro` framing that
both blind tests used, every other asset in the study is sampled at
0.30–0.78 mm/px and `rock_face_01` alone at **3.42 mm/px** — an 11× different
shot. The two size-matched controls, `rock_07` (0.9 × 0.8 × 1.8 m) and
`rock_09` (1.6 × 0.7 × 3.2 m), sample at 0.30 and 0.39 mm/px like the props,
and their band spectra sit *inside* the props' range, not above it.

---

## 1. The instrument

### 1.1 Renders

Shipped `asset_inspect` (`client/vordar-client/src/bin/asset_inspect.rs`),
offscreen, `--lighting studio`, channels `albedo,beauty,ao,normal,rough`,
`--distance full,gameplay,macro`, `--angles 4`, `--size 2048x2048`. Ten assets:
the seven generated props and the three photoscan controls, each read from the
GLB/glTF the game loads and each rendered at its **median shipped zone scale**
from `content/zones/zones.ron`.

| asset | kind | shipped scale | bbox (m) | surface (m²) | albedo mm/texel |
|---|---|---|---|---|---|
| chapel_arch | gen | 1.00 | 5.46 × 5.50 × 1.42 | 136.7 | 17.86 |
| crucero | gen | 1.25 | 1.09 × 2.25 × 0.26 | 5.4 | 3.23 |
| broken_column | gen | 0.95 | 0.56 × 1.71 × 0.56 | 5.8 | 3.40 |
| gravestone | gen | 0.72 | 0.64 × 1.30 × 0.16 | 2.2 | 2.23 |
| candelabra_shrine | gen | 1.00 | 1.18 × 1.80 × 1.18 | 3.6 | 2.73 |
| olive_stump | gen | 1.00 | 1.43 × 1.80 × 1.39 | 12.3 | 5.07 |
| cypress | gen | 4.75 | 4.63 × 8.54 × 3.38 | 140.6 | 22.93 |
| *rock_face_01* | ctl | 3.60 | **17.83 × 12.83 × 13.78** | **369.5** | 23.46 |
| *rock_07* | ctl | 5.45 | 0.92 × 0.78 × 1.75 | 4.7 | 2.96 |
| *rock_09* | ctl | 22.05 | 1.63 × 0.72 × 3.19 | 10.7 | 4.38 |

(mm/texel at shipped scale reproduces the prior study's native-scale figures
exactly: `broken_column` 3.40/0.95 = 3.58, `rock_face_01` 23.46/3.60 = 6.52.)

### 1.2 How pixels are converted to millimetres of real surface

A perspective render has no single mm/px: the scale varies with depth and with
how obliquely the surface faces the camera. So the camera is reproduced on the
CPU and the geometry rasterised again, giving per-pixel depth and per-pixel
geometric normal:

```
footprint(p) = depth(p) · 2·tan(fovy/2) / R          metres across one pixel
s(p)         = footprint(p) / sqrt(|n·v|) · 1000     mm of surface per pixel
```

The `sqrt(cos)` divisor makes pixel *area* map to surface *area*, so an
isotropic filter in pixels is an isotropic filter in surface millimetres.
Analysis then runs only where `s` is uniform: pixels with `|n·v| ≥ 0.5` and
`|log2(s/s_med)| ≤ 0.35`, after which a single σ in pixels is a single
wavelength in mm. `s_ref` is the median `s` of that mask, reported per frame.

Band energy, per octave λ, matching both prior instruments' convention
(`λ = 2σ · mm/px`, so the ladder is anchored on the campaign's own 62.1/124.3 mm):

```
band(L) = G_σ(L)/G_σ(m) − G_2σ(L)/G_2σ(m)      (mask-normalised DoG)
E(λ)    = std of band inside the mask eroded by σ,   σ = λ / (2·s_ref)
```

`L` is CIE L* for `albedo`/`beauty`; the material debug channels are read on
their own 0–255 map scale. A band is reported only when `3 ≤ σ ≤ 400 px` and the
eroded mask holds at least `16σ²` pixels — i.e. 16 independent band cells. Every
dash in every table below is a band the frame cannot support, not a band omitted.
Four azimuths are combined as RMS.

Ladder: **3.88, 7.77, 15.53, 31.06, 62.13, 124.25, 248.5, 497.0 mm.**

### 1.3 Where each framing lands on each asset — and the framing anomaly

Mean `s_ref` (mm of surface per pixel at R = 2048) and mean analysed mask
fraction, per asset per framing:

| asset | macro (0.6 m) | gameplay (2.3 m) | full (fit) |
|---|---|---|---|
| chapel_arch | 0.322 · 0.65 | 1.119 · 0.26 | 5.176 · 0.10 |
| crucero | 0.346 · 0.37 | 1.044 · 0.12 | 1.667 · 0.05 |
| broken_column | 0.309 · 0.49 | 1.047 · 0.10 | 1.360 · 0.08 |
| gravestone | 0.304 · 0.31 | 1.072 · 0.05 | 0.966 · 0.05 |
| candelabra_shrine | 0.496 · 0.18 | 1.205 · 0.05 | 1.915 · 0.04 |
| olive_stump | 0.462 · 0.53 | 1.245 · 0.15 | 1.923 · 0.09 |
| cypress | 0.780 · 0.75 | 1.430 · 0.70 | 7.271 · 0.07 |
| *rock_face_01* | **3.422** · 0.33 | **4.006** · 0.22 | 18.538 · 0.10 |
| *rock_07* | 0.302 · 0.70 | 1.156 · 0.13 | 1.454 · 0.12 |
| *rock_09* | 0.391 · 0.52 | 1.231 · 0.17 | 2.475 · 0.11 |

`aim_close` places the eye 0.6 m from the *near-extent* vertex along the view
azimuth. On a 2 m prop that puts essentially the whole visible surface at 0.6 m.
On an 18 m cliff it puts a protruding spur at 0.6 m and the rest of the surface
metres behind it, so the frame is dominated by distant rock. **This is the entire
mechanism behind the 60–124 mm residual**, and it fires on exactly one asset:
the one that wins.

---

## 2. Instrument self-validation

Five checks, four of which can fail.

**(a) Camera replication — IoU 0.9990–1.0000.** The whole mm calibration rests on
the CPU rasteriser reproducing the GPU camera. Rasterising prop + the 40 m
calibration ground quad and comparing coverage against the GPU frame's own
coverage gives IoU **min 0.9990, median 0.9995** over 12 frames (3 framings ×
4 azimuths) on `broken_column` and **min 0.9995, median 1.0000 over 36 frames**
on the three controls — including the four `rock_face_01` macro frames whose
depth distribution the verdict rests on (0.9995 / 1.0000 / 0.9997 / 1.0000).
Before near-plane clipping was added the same check read IoU 0.12 — the test
fails when the instrument is wrong. (`camval_smoke.json`, `camval_ship.json`)

**(b) Control against itself — exact identity.** Each of the three controls was
rendered a second time, independently, into a separate output tree. All
**180/180 PNGs are byte-identical** (md5), so every band value reproduces at
ratio exactly **1.0000**. The renderer contributes no variance; this is the
identity the decimation study's `r = 1.0000` self-test is the analogue of, and
it is the weakest of the four because a deterministic renderer makes it cheap.

**(c) Scale-doubling identity — the strong test of the mm conversion.** Render an
asset at scale S and at 2S. Under the `full` framing the camera refits the
bounding box, so the two renders are the same picture — but every feature is now
twice as large in real millimetres. A correct conversion must satisfy
`E_2S(2λ) = E_S(λ)` exactly. This is not a tautology: any error in fovy,
resolution, aim distance, depth or obliquity breaks it.

**Both controls return ratio exactly 1.0000 for albedo in every band they cover
under `full`** — `rock_07` at 15.5→31.1, 31.1→62.1, 62.1→124.3, 124.3→248.5 and
`rock_face_01` at 124.3→248.5, 248.5→497. This is the identity result, the exact
analogue of the decimation study's `r = 1.0000`.

| framing | channel | n | median ratio | p10 | p90 |
|---|---|---|---|---|---|
| **full** (exact identity) | **albedo** | 12 | **1.0000** | 0.9853 | 1.0000 |
| full | beauty | 12 | 1.0011 | 0.9914 | 1.0139 |
| macro | albedo | 18 | 1.0035 | 0.9673 | 1.0788 |
| macro | beauty | 18 | 1.0101 | 0.9730 | 1.0761 |
| gameplay | albedo | 17 | 0.9781 | 0.8512 | 1.1210 |
| gameplay | beauty | 17 | 0.9313 | 0.8231 | 1.0488 |

The two deviations are both explained and both expected. The four *props*
deviate under `full` by 0.984–1.005 rather than exactly 1.000, because the
calibration ground quad's extent is fixed at 40 m and does not scale with the
asset, so a small prop's `full` frame is not quite the same picture at 2S.
`beauty` deviates by ±1–5 % everywhere because SSAO's radius is in world units
and therefore halves, in asset terms, when the asset doubles. Under `macro` and
`gameplay` the framing samples a *sub-region* of the surface at 2S, so the
identity there is statistical rather than exact — and it still lands within
0.4–7 % at `macro`. (`scale_identity.json`)

**(d) Scale-adaptive decimation.** Coarse bands are evaluated on a block-mean
decimation holding ≥ 8 px per σ (`gaussian_filter` costs O(n·σ)). Validated
against the full-resolution path on four frames spanning both assets and all
three framings: **ratio 1.0000 where no decimation applies, 0.9977–1.0098
where it does**, across all five channels. (`decimation_validation.json`)

**(e) Cross-framing reproducibility — the instrument's honest error bar.** The
same band on the same asset, measured from two different framings, samples a
different part of the surface. Over 220 such pairs:

| pair | n | median ratio | p90 | max |
|---|---|---|---|---|
| full / gameplay | 68 | 1.140 | 1.584 | 2.753 |
| full / macro | 64 | 1.236 | 2.013 | 2.774 |
| gameplay / macro | 88 | 1.123 | 1.559 | 3.914 |
| **all** | **220** | **1.171** | **1.678** | — |

**Read every table below against this: a 1.2× difference between two assets is
inside the instrument. A 2× difference is not.** The prior study's claimed
2.6× deficit would have been outside it — had it been a band comparison.

---

## 3. The table

### 3.1 Albedo — L* band std, `gameplay` framing (2.3 m, the most comparable)

| asset | 3.9 | 7.8 | 15.5 | 31.1 | **62.1** | **124.3** | 248.5 | 497 |
|---|---|---|---|---|---|---|---|---|
| chapel_arch | — | 0.93 | 1.32 | 1.50 | **1.37** | **0.95** | 0.50 | — |
| crucero | — | 2.69 | 2.88 | 2.62 | **2.49** | — | — | — |
| broken_column | — | 2.40 | 2.88 | 2.56 | **1.74** | — | — | — |
| gravestone | — | 1.90 | 1.91 | 2.14 | **0.76** | — | — | — |
| candelabra_shrine | — | 1.34 | 1.58 | 2.16 | **4.56** | — | — | — |
| olive_stump | — | 3.16 | 3.65 | 4.00 | **4.16** | **4.27** | — | — |
| cypress | — | — | 1.51 | 2.08 | **2.58** | **2.82** | 2.74 | — |
| *rock_face_01* | — | — | — | 1.38 | **1.55** | **1.54** | 1.43 | 1.58 |
| *rock_07* | — | 1.29 | 1.42 | 1.50 | **1.61** | **1.63** | — | — |
| *rock_09* | — | 0.82 | 0.98 | 1.11 | **1.19** | **0.92** | — | — |

### 3.2 Beauty — L* band std, `gameplay` framing

| asset | 3.9 | 7.8 | 15.5 | 31.1 | **62.1** | **124.3** | 248.5 | 497 |
|---|---|---|---|---|---|---|---|---|
| chapel_arch | — | 1.18 | 1.69 | 2.13 | **2.22** | **1.59** | 0.89 | — |
| crucero | — | 2.99 | 3.23 | 3.05 | **2.94** | — | — | — |
| broken_column | — | 2.89 | 3.32 | 3.02 | **2.24** | — | — | — |
| gravestone | — | 2.55 | 2.39 | 2.44 | **1.18** | — | — | — |
| candelabra_shrine | — | 1.40 | 1.55 | 2.07 | **4.44** | — | — | — |
| olive_stump | — | 3.22 | 3.83 | 4.24 | **4.43** | **3.72** | — | — |
| cypress | — | — | 1.35 | 1.80 | **2.15** | **2.43** | 2.45 | — |
| *rock_face_01* | — | — | — | 1.76 | **2.17** | **2.43** | 2.75 | 3.19 |
| *rock_07* | — | 1.77 | 1.89 | 1.93 | **2.15** | **2.43** | — | — |
| *rock_09* | — | 1.20 | 1.38 | 1.49 | **1.54** | **1.13** | — | — |

### 3.2b The same tables normalised for mean lightness

The controls are darker than most props (mean L* 30–48 against 48–63), and a
darker surface produces less absolute L* variation at equal reflectance
contrast. `100 · E(λ)/L̄` is the relative-contrast reading; the qualitative
answer is the one that has to survive both.

| asset | albedo 31.1 | **albedo 62.1** | **albedo 124.3** | beauty 31.1 | **beauty 62.1** | **beauty 124.3** |
|---|---|---|---|---|---|---|
| chapel_arch | 2.46 | **2.25** | **1.56** | 3.55 | **3.71** | **2.66** |
| crucero | 4.55 | **4.32** | — | 5.48 | **5.29** | — |
| broken_column | 4.29 | **2.92** | — | 5.46 | **4.05** | — |
| gravestone | 4.02 | **1.42** | — | 4.80 | **2.32** | — |
| candelabra_shrine | 3.42 | **7.23** | — | 3.47 | **7.44** | — |
| olive_stump | 10.42 | **10.85** | **11.13** | 11.86 | **12.39** | **10.41** |
| cypress | 10.21 | **12.66** | **13.82** | 16.22 | **19.43** | **21.95** |
| *rock_face_01* | 2.85 | **3.20** | **3.17** | 2.80 | **3.46** | **3.88** |
| *rock_07* | 4.37 | **4.68** | **4.74** | 4.65 | **5.18** | **5.86** |
| *rock_09* | 3.60 | **3.87** | **2.98** | 3.93 | **4.05** | **2.96** |

Normalised, three props fall below the whole control range at 62 mm
(`gravestone`, `chapel_arch`, `broken_column`), one is inside it, three are
above. **The deficit is real for some assets and it is not the one that was
pre-registered**: it is not a band (it is present at 31 mm too for
`chapel_arch`, absent at 31 mm for `gravestone` and `broken_column`), it is not
population-wide, and its largest instance is 2.3× rather than 2.6× on everything.

Note also that `rock_face_01` — the control that wins the blind test "by a large
gap" — has the **lowest** normalised albedo macro contrast of the three
controls (3.20 against 3.87 and 4.68). Whatever makes it read as stone, it is
not having the most macro albedo structure.

### 3.2c Absolute reading

At 62.1 mm the controls span **1.19–1.61** in absolute L*. Five of seven props are
above that band (`crucero` 2.49, `olive_stump` 4.16, `candelabra_shrine` 4.56,
`cypress` 2.58, `broken_column` 1.74); two are below (`gravestone` 0.76,
`chapel_arch` 1.37). At 124.3 mm the controls span 0.92–1.63; of the three props
that can reach the band, two are above it (`olive_stump` 4.27, `cypress` 2.82)
and one sits inside it (`chapel_arch` 0.95). **There is no macro albedo deficit
to explain.**

### 3.3 Roughness — the candidate, priced

`rough` is the shipped roughness the renderer actually resolves, in map units
(0–255), band-analysed identically. `map std` is the shipped
`metallicRoughnessTexture`'s own green-channel std, read from the bytes.

| asset | MR texture | AO texture | `vordar_detail` | roughness factor | map std/255 | rough E(15.5) | rough E(31.1) | **rough E(62.1)** | rough E(124.3) | rough E(497) |
|---|---|---|---|---|---|---|---|---|---|---|
| chapel_arch | no | yes | **on** | 0.85 | — | 3.07 | 1.89 | **0.95** | 0.46 | — |
| crucero | no | yes | **on** | 0.85 | — | 3.25 | 1.89 | **0.88** | — | — |
| broken_column | no | yes | **on** | 0.85 | — | 3.15 | 1.92 | **0.96** | — | — |
| gravestone | no | yes | **on** | 0.85 | — | 3.41 | 1.95 | **0.92** | — | — |
| candelabra_shrine | no | yes | off | 0.75 | — | **0.02** | **0.01** | **0.03** | — | — |
| olive_stump | no | yes | off | 0.85 | — | **0.04** | **0.02** | **0.07** | 0.02 | — |
| cypress | no | yes | off | 0.70 | — | **0.02** | **0.02** | **0.03** | 0.05 | — |
| *rock_face_01* | **yes** | **no** | n/a | 1.0 | **7.97** | — | 0.83 | **0.81** | 0.78 | **0.88** |
| *rock_07* | **yes** | **no** | n/a | 1.0 | 0.92 | 0.13 | 0.06 | **0.05** | 0.06 | — |
| *rock_09* | **yes** | **no** | n/a | 1.0 | 0.94 | 0.25 | 0.12 | **0.06** | 0.05 | — |

Three separate facts, none of them the simple story:

1. **Four of seven generated props are not flat.** `chapel_arch`, `crucero`,
   `broken_column` and `gravestone` carry `{"vordar_detail": true}` and take the
   world-space triplanar overlay's `roughness_delta`
   (`smirk/engine-renderer/src/snippets/detail_triplanar.wgsl`,
   `DETAIL_PERIOD = 0.45 m`). Their rendered roughness varies **more than any
   control's below 31 mm** (3.1–3.4 at 15.5 mm against `rock_face_01`'s 0.83)
   and then **collapses above it** — 0.9 at 62 mm, 0.46 at 124 mm, 0.16 at
   248 mm. The overlay is a fine-grain layer; it does not reach the macro band.
2. **The other three props are flat to measurement precision** — 0.01–0.08 at
   every band, i.e. zero.
3. **`rock_face_01` is the only asset with roughness structure that does not
   decay with scale**: 0.83 / 0.81 / 0.78 / 0.78 / 0.88 from 31 mm to 497 mm.
   Its map std of 7.97/255 is small in absolute terms, exactly as the decimation
   study warned — but it is *broadband*, and nothing else in the study is.
   The other two controls are 0.04–0.32 and fall like the props do.

So the roughness column correlates with the blind winner — and the correlation
rests on **one asset**, because the two size-matched controls have almost no
roughness variation either and were never blind-tested.

### 3.4 The channel where the props and the controls actually diverge

`G(λ) = E_beauty(λ) / E_albedo(λ)`: the factor by which the material's light
response amplifies the colour map's structure at that scale. `G = 1` means
lighting adds nothing; `G < 1` means it flattens.

| asset | 7.8 | 15.5 | 31.1 | **62.1** | **124.3** | 248.5 |
|---|---|---|---|---|---|---|
| chapel_arch | 1.27 | 1.27 | 1.42 | **1.62** | **1.67** | 1.80 |
| crucero | 1.11 | 1.12 | 1.16 | **1.18** | — | — |
| broken_column | 1.20 | 1.15 | 1.18 | **1.29** | — | — |
| gravestone | 1.34 | 1.25 | 1.14 | **1.55** | — | — |
| candelabra_shrine | 1.05 | 0.99 | 0.96 | **0.97** | — | — |
| olive_stump | 1.02 | 1.05 | 1.06 | **1.06** | **0.87** | — |
| cypress | — | 0.89 | 0.86 | **0.83** | **0.86** | 0.89 |
| *rock_face_01* | — | — | 1.27 | **1.40** | **1.58** | 1.92 |
| *rock_07* | 1.38 | 1.32 | 1.28 | **1.34** | **1.49** | — |
| *rock_09* | 1.45 | 1.40 | 1.35 | **1.29** | **1.23** | — |

**All three controls are ≥ 1.23 in every band they cover here** (`rock_face_01`
and `rock_07` rise with wavelength; `rock_09` falls, 1.45 → 1.23). Three props
(`candelabra_shrine`, `olive_stump`,
`cypress`) never exceed 1.06, and two of those are below 1.00 at most bands —
and those three are exactly the three with no detail-overlay roughness, the ones
whose material has nothing to respond with. The separation is not clean:
`chapel_arch` (1.27–1.80) and `gravestone` (1.14–1.55) sit inside the controls'
range, and `crucero`/`broken_column` (1.11–1.29) just below it.

The framing caveat matters here. At `macro`, `rock_09`'s gain falls to 0.91–0.98
above 31 mm, so "every control ≥ 1.23" is a `gameplay`-framing statement, not a
universal one. At `full` the controls run 1.23–2.17 and the props 0.97–1.52,
which reproduces the same ordering with more margin.

This is the reviewer's second cited tell — *"one uniform sheen with no specular
breakup"* — stated as a number, and it is the only quantity in the study that
separates the two populations cleanly.

### 3.5 AO: a control deficit, not a control advantage

| asset | ao E(15.5) | ao E(31.1) | ao E(62.1) | ao E(124.3) |
|---|---|---|---|---|
| generated props (7) | 3.5–6.3 | 3.7–7.7 | 2.0–8.1 | 3.5–7.7 |
| *all three controls* | **0.00** | **0.00** | **0.00** | **0.00** |

The controls' glTF declares `metallicRoughnessTexture` pointing at
`*_arm_1k.jpg` but **no `occlusionTexture`** — the ARM red channel is never
sampled. Every generated prop ships a real baked `occlusionTexture`. The
decimation study recorded "both sides ship AO"; in the bytes the engine reads,
only one side does, and it is the side that loses.

---

## 4. Does the 60–124 mm figure survive?

**No, not as a band.** Three independent reasons:

1. **Its denominator was wrong by 14–26× on the control.** Measured in the prior
   probe's own frames (scale 1.0 / 4.0, 1024², macro):

   | asset | eye→aim | actual surface depth p10/med/p90 | measured mm/px | assumed | error |
   |---|---|---|---|---|---|
   | broken_column | 0.60 m | 0.68 / 0.71 / 0.77 m | 0.625–0.647 | 0.4854 | ×1.29–1.33 |
   | *rock_face_01* ×4 | 0.60 m | **2.61 / 6.19–10.02 / 17.17 m** | **6.76–12.80** | 0.4854 | **×13.9–26.4** |

   The control's "62.1 mm" cell measured 0.86–1.64 m of surface; its "124.3 mm"
   cell measured 1.7–3.3 m. (`priorcheck.json`)

2. **Measured correctly, it is not a population-wide deficit in either
   normalisation.** In absolute L* the controls are 1.19–1.61 at 62.1 mm and
   five of seven props are above them. Normalised for lightness, the props
   straddle the controls, three below and three above, with the largest
   shortfall 2.3× on `gravestone` — and `broken_column`, the asset both blind
   tests actually judged, comes to 2.92 against `rock_face_01`'s 3.20, a 1.10×
   difference that is inside the instrument's 1.17 median reproducibility.

3. **The deficit is not banded and it is not in albedo.** What separates the two
   populations, as far as anything does, is broadband and multiplicative:
   `G(λ)` is ≥ 1.23 for every control at every band and ≤ 1.06 for three of seven
   props at every band, with no wavelength dependence beyond a mild rise. There
   is no window in 4–500 mm where the props' albedo is short of the controls'.

The one thing that *is* concentrated near 60–124 mm is the **roughness
crossover**: the detail overlay carries the four opted-in props above every
control below 31 mm and drops them below `rock_face_01` above 62 mm. If the
pre-registered window has any real referent, it is that crossover — in the
roughness channel, not the albedo one, and confined to four assets.

---

## 5. Rank correlation against the only per-prop perceptual data on disk

Row-39 blind ranking, 1 = best: `rock_face_01` 1, `crucero` 2, `gravestone` 3,
`broken_column` 4, `candelabra_shrine` 5. ρ < 0 means "more of this predicts a
better read". n = 5 — nothing here is significant; the *signs* are the finding.

| predictor | ρ | values (best → worst) |
|---|---|---|
| **G(31.1 mm) @macro** | **−0.90** | 1.29 · 1.08 · 0.94 · 1.07 · 0.94 |
| G(31.1 mm) @gameplay | −0.70 | 1.27 · 1.16 · 1.14 · 1.18 · 0.96 |
| G(15.5 mm) @macro | −0.60 | 1.22 · 1.04 · 1.10 · 1.11 · 0.95 |
| G(62.1 mm) @gameplay | −0.50 | 1.40 · 1.18 · 1.55 · 1.29 · 0.97 |
| mm per texel | −0.50 | 23.46 · 3.23 · 2.23 · 3.40 · 2.73 |
| rough E(15.5 mm) @macro | −0.40 | 0.76 · 3.27 · 3.25 · 3.14 · 0.01 |
| rough E(62.1 mm) @gameplay | +0.00 | 0.81 · 0.88 · 0.92 · 0.96 · 0.03 |
| roughness map std/255 | +0.00 | 7.97 · 0 · 0 · 0 · 0 |
| **albedo E(15.5 mm) @macro** | **+0.30** | 1.17 · 2.62 · 2.32 · 2.67 · 1.75 |
| **albedo E(31.1 mm) @macro** | **+0.60** | 1.62 · 2.30 · 2.91 · 2.43 · 2.36 |
| albedo E(62.1 mm) @gameplay | +0.50 | 1.55 · 2.49 · 0.76 · 1.74 · 4.56 |

Eighteen predictors have data on all five ranked assets, and their signs are
completely consistent by family:

| family | n | ρ range | every sign |
|---|---|---|---|
| albedo band energy | 4 | +0.30 … +0.60 | positive |
| beauty band energy | 4 | +0.10 … +0.50 | positive |
| **beauty/albedo gain `G`** | 4 | **−0.90 … −0.50** | **negative** |
| rendered roughness band energy | 5 | −0.40 … +0.00 | non-positive |

The campaign's premise — *the generated props lack macro albedo structure* —
predicts a strongly negative ρ for albedo band energy. Every one of the four
albedo predictors measures **positive**, and so does every beauty one. The only
family that points the premise's way is `G`, and it points there at every band
and both framings. Note also that `roughness map std/255` scores **exactly 0.00**: it is
constant across the four generated props, so it can only ever separate the
control from the pack, never order the props. That is the statistic coming out
zero where it should.

---

## 6. Parameter sweeps

SWEEP_TABLE

---

## 7. What this rules out

- **A banded 60–124 mm albedo deficit shared by the population.** Ruled out. The
  prior figure was a mislabelled denominator on one asset; measured per pixel,
  the props straddle the controls in that band under both normalisations.
- **A broadband albedo deficit shared by the population.** Ruled out over
  3.9–497 mm. There is no octave in which the seven props are systematically
  below the three controls in albedo. Where props are low it is asset-specific
  (`chapel_arch` at every band, `gravestone` and `broken_column` only above
  31 mm) and never worse than 2.3×.
- **"The controls have more macro albedo structure, and that is why they read as
  stone."** Ruled out on its own terms: the control that wins the blind test has
  the *least* normalised macro albedo contrast of the three controls.
- **AO as a control advantage.** Ruled out, and inverted: the controls' AO is
  never sampled by the engine (band energy exactly 0.00 at every band, all three
  controls, all framings), while every prop ships one.
- **Texel density.** Confirmed refuted and now generalised beyond the original
  two assets: mm/texel ranks *negatively* against the blind order because the
  winner is the coarsest (23.5 mm/texel against the props' 2.2–5.1) — the same
  inversion the decimation study found for triangle size.
- **"Generated props ship flat roughness."** Ruled out as stated. Four of seven
  take a triplanar roughness overlay and are *rougher-varying than every control*
  below 31 mm. The true statement is narrower: **no generated prop carries
  roughness variation above 62 mm**, and one control does.
- **Under-tessellation.** Not re-tested here, and nothing in this pass disturbs
  the decimation study's refutation. Worth recording that this pass supplies the
  albedo-domain measurement §6 of that report said the "shading painted into
  albedo" tell required — and the albedo bands do not show the deficit either.

What this pass does **not** rule out, and cannot: that the deficit is in the
*character* of the macro structure rather than its energy. `E(λ)` is a variance.
Two surfaces can carry identical band energy with completely different
morphology — the controls' structure could be edge-like and occluding where the
props' is blotchy and flat, exactly as the reviewer's *"painted cracks that never
occlude"* describes. **Energy is the wrong statistic for that hypothesis and
this table cannot test it.**

---

## 8. The next probe, and its cost

Two candidates. Neither is a fix and neither should be built before it is
measured.

**(a) Cheapest, and it discriminates: phase/morphology, not energy.** The
question this table opens is whether the props' macro albedo structure — which
is *present in the right amount* — is the wrong *kind*. The measurement is a
per-band morphology statistic on the frames already on disk: gradient kurtosis,
structure-tensor coherence and the sign-skew of the band (do dark features form
thin connected lines, as cracks do, or isotropic blobs?), plus the cross-channel
correlation between the albedo band and the AO/normal bands at the same
wavelength — "does the painted crack coincide with a real crevice?" is exactly
`corr(albedo_band, ao_band)`, and it is zero-capable. **Cost: no new renders, no
GPU, ~1 h of analysis on the existing `target/band-table/ship` tree.** This is
the probe to run.

**(b) Only if (a) comes back null: an A/B on `G`.** `G(λ)` is the one quantity
that separates the populations, but it is a *composite* of roughness, normal map
and AO, and this pass cannot decompose it. Splitting it needs render variants
that do not exist as flags — swapping one control's ARM roughness onto one prop
and re-rendering, and conversely flattening one control's roughness. **Cost: a
texture-authoring step plus ~10 min of GPU stills, and it changes an asset on
disk, so it needs its own go-ahead.** Do not price a per-texel roughness map for
the whole set before that A/B: §3.3 shows the winning control's roughness signal
is 0.8/255 broadband, and the two controls that carry none were never tested.

A third thing is worth saying plainly even though it is not a probe: **the blind
tests have been comparing an 18 m cliff at 3.4 mm/px against 2 m props at
0.3 mm/px.** Any future blind test should include `rock_07` and `rock_09` — the
size-matched controls — or scale `rock_face_01` down to prop size. On this
table's evidence the size-matched controls are *not* obviously better than the
props, and no blind test has ever asked a reviewer to rank them.

---

## 9. Reproducing this

Renders (kept): `target/band-table/<tag>/<asset>/studio_<channel>/<framing>_NN.png`

| tag | contents |
|---|---|
| `ship` | primary: 10 assets × 5 channels × 3 framings × 4 azimuths, 2048², shipped scale |
| `native` | same at scale 1.0 (scale sweep) |
| `s1024`, `s4096` | resolution sweep |
| `ang8` | 8 azimuths |
| `x2` | shipped scale × 2 (scale-doubling identity) |
| `prior` | the prior probe's exact framing (1024², scale 1.0 / 4.0) |
| `selftest` | independent re-render of the three controls (byte-identity check) |

Scripts and results (session scratchpad
`…/c5e0823f-2866-44d5-9508-d0d4ab7579df/scratchpad/bandtab/`, nothing added to
the repo):

- `scene.py` — asset_inspect camera + perspective rasteriser (depth, face normal, surface-scale map)
- `analyze.py` — masked-DoG band energies per frame; `report.py`, `synth.py` — aggregation and tables
- `material.py` — material column from the shipped bytes → `material.json`
- `validate_cam.py` → `camval_smoke.json`; `validate_dec.py` → `decimation_validation.json`;
  `validate_scale.py` → `scale_identity.json`; `framing_agree.py` → `framing_agreement.json`
- `priorcheck.py` → `priorcheck.json` — the prior probe's frames re-measured
- `buildcache.py`, `runsweeps.py`, `sweepcmp.py` — cache and sweep drivers
- Band results: `bands_{ship,native,s1024,s4096,ang8,x2,cos04,cos07,tol020,tol050,ratio14,ratio28,erode05,erode20}.json`

Per `tasks/lessons/2026-07-21-keep-verification-artifacts.md` the scratchpad is
Temp and will be lost; the `bands_*.json`, `material.json`, `priorcheck.json`
and `geometry.json` files are the ones worth promoting if any of this is to be
re-checked.

Files read (shipped bytes):
`content/models/props/{chapel_arch,crucero,broken_column,gravestone,candelabra_shrine,olive_stump,cypress}/<prop>.glb`,
`content/models/props/{rock_07,rock_09,rock_face_01}/<prop>_1k.gltf` and their
`textures/*_arm_1k.jpg`, `content/zones/zones.ron`.
