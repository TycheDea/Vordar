# Decimation attribution — is under-tessellation the campaign's stone-read deficit?

Date: 2026-08-01
Scope: attribution study on artifacts already on disk. No GPU, no Blender, no regeneration.
Hypothesis under test: *the campaign's long-standing "generated stone does not read as
stone" deficit is substantially geometric under-tessellation, and the 60–124 mm residual
band recorded in `tasks/prop-texture-redesign.md` is triangle-scale faceting.*

Prior evidence this pass builds on:
- chapel_arch decimation probe — `tasks/todo.md:1019-1046`
- blind tests, band energies, `blend_views` attribution — `tasks/prop-texture-redesign.md:2960-2971`, `:3138-3183`, `:3337-3345`
- post-mortem the guardrails come from — `tasks/lessons/2026-07-26-a-visible-mechanism-is-not-an-attributed-one.md`

---

## Verdict: **REFUTED as the campaign's principal stone-read deficit.**

Three independent measurements agree, and the third is decisive:

1. **Decimation destroys essentially nothing in the 60–124 mm band.** The
   attribution statistic `D(λ) = 1 − r(λ)²` — the fraction of the pre-decimation
   mesh's geometric relief variance in octave band λ that the shipped mesh fails
   to reproduce — is **≤ 0.004 at 68 mm for every generated prop**. It is
   0.0038 for chapel_arch, 0.0000 for broken_column. The statistic is zero-capable
   and it comes out at zero here.

2. **Per-prop triangle coarseness does not predict per-prop perceptual deficit.**
   Over the five assets in the row-39 blind test, Spearman ρ(perceptual rank,
   mean triangle edge) = **−0.10**. Against the *direct* decimation-damage
   measurement rather than a proxy, over the four generated props, ρ = **−0.80**:
   the prop that lost the most geometry to decimation (gravestone, D = 0.152 at
   4 mm) ranks second *best*, and the one that lost the least (candelabra_shrine,
   D = 0.005) ranks *worst*.

3. **The photoscan control that wins is the coarsest mesh in the study.**
   `rock_face_01` — picked correctly with high confidence, "large gap, not one of
   the four" — has a 60.8 mm mean triangle edge natively and is rendered at
   `REFERENCE_SCALE = 4.0` (`client/vordar-client/src/bin/asset_inspect.rs:49`),
   giving **243 mm in the frame the reviewer judged**. At that scale **100.0 % of
   its surface is geometrically incapable of carrying 62 mm relief** and 99.9 %
   incapable at 124 mm, against 37–62 % for the generated props it beat. The
   control's mesh is strictly less able to carry the residual band than every
   asset that loses to it.

**Confirmed, but as a different and narrower defect:** decimation *is* the cause
of chapel_arch's melted carving, and the damage is real and large — but it lives
at **4–17 mm**, not 60–124 mm, and it is confined to three props
(chapel_arch, cypress, olive_stump). It is not what the blind tests were measuring.

`blend_views` is not exonerated by this pass and was not tested by it. What this
pass establishes is that under-tessellation joins it in the "real, faithful,
irrelevant to the 60–124 mm residual" category — with the difference that
under-tessellation has now had a magnitude test applied and failed it.

---

## 1. Per-prop geometry, from the shipped bytes

Read directly from the GLB/glTF the game loads (`content/models/props/<prop>/…`),
node transforms applied, world-space triangle areas and edge lengths. Zone
instance scales from `content/zones/zones.ron:105-141` (`PropDef::scale`,
`game/vordar-game/src/world/zones.rs:127`).

| prop | tris | area m² (native) | tri/m² | mean edge mm | √(2A/n) mm | p90 edge mm | zone scale | world mean edge mm | world area m² |
|---|---|---|---|---|---|---|---|---|---|
| chapel_arch | 14 999 | 136.68 | 110 | **170.3** | 135.0 | 289 | 1.0 | **170** | 136.7 |
| crucero | 15 000 | 3.48 | 4 314 | 24.9 | 21.5 | 41 | 1.5 | 37 | 7.8 |
| broken_column | 14 998 | 6.38 | 2 349 | 39.7 | 29.2 | 78 | 0.85–1.0 | 40 | 6.4 |
| gravestone | 14 999 | 4.32 | 3 470 | 27.9 | 24.0 | 43 | 0.70–0.75 | 20 | 2.2 |
| olive_stump | 14 999 | 12.33 | 1 216 | 48.1 | 40.5 | 78 | 0.9–1.2 | 48 | 12.3 |
| cypress | 14 997 | 6.23 | 2 407 | 33.2 | 28.8 | 51 | 4.25–5.5 | **166** | 155.8 |
| candelabra_shrine | 14 998 | 3.55 | 4 225 | 28.3 | 21.8 | 44 | 1.0 | 28 | 3.6 |
| *rock_face_01* (photoscan) | 20 174 | 28.51 | 708 | **60.8** | 53.2 | 112 | 3.2–4.0 | **243** | 456.1 |
| *rock_07* (photoscan) | 14 844 | 0.16 | 93 335 | 5.3 | 4.6 | 8 | 4.9–5.9 | 31 | 5.5 |
| *rock_09* (photoscan) | 12 416 | 0.02 | 566 456 | 2.3 | 1.9 | 4 | 19.8–24.3 | 55 | 12.9 |

The premise's "14.5 cm mean triangle edge" for chapel_arch is confirmed within
convention: √(2A/n) = 135.0 mm, plain mean edge 170.3 mm, area-weighted √(2a) 170.0 mm.

### Correction to the premise: `assets.json` does not describe the shipped bytes

The premise cites per-asset budgets (chapel_arch 14 000, olive_stump 20 500,
broken_column 12 000). **None of those were applied to anything that ships.**
Every shipped generated prop carries 14 997–15 000 triangles — a flat budget.
`content/models/assets.json`'s per-asset `tri_budget` was introduced by `9e92cab`
("Give each prop the triangle budget its own geometry needs", 2026-07-29), which
is **not** an ancestor of `4c46519` (2026-07-28), the commit that rebuilt six of
the seven props, nor of `519c780` (2026-07-25) which last touched chapel_arch.
The values are config that has never been run.

This strengthens the premise's mechanism claim (a flat budget with no size
scaling) and weakens its arithmetic (the per-prop numbers are not the shipped
ones). It also means **applying the current `assets.json` as-is would make four
of seven props coarser than they ship today**: broken_column 15 k → 12 k,
crucero 15 k → 9.5 k, gravestone 15 k → 8 k, candelabra_shrine 15 k → 5 k.

---

## 2. The prior numbers, recovered

Source: `tasks/prop-texture-redesign.md`; per-measurement values in
`…/b4432896-…/scratchpad/results.json`; instrument `…/analysis.py`.

**Blind test #1** (`:2960-2971`) — five anonymized stone sheets, mapping withheld.
Picked sheet B = `rock_face_01`, the photoscan, correct, high confidence. Ranking
of the rest: **crucero > gravestone > broken_column > candelabra_shrine**. Gap:
"Large. Not one of the four." Cited tells: *shading painted into albedo rather
than earned by geometry; one uniform sheen with no specular breakup; painted
cracks that never occlude.*

**Blind test #2** (`:3337-3345`) — A = wta, B = shipped, C = `rock_face_01`,
D = hwta. Photoscan identified correctly, ~90 % confidence. "Reads as real stone
at 0.6 m": **C 8 · D 4 · A 3 · B 2.5**.

**The 60–124 mm residual** (`:3180-3183`), from `results.json`, macro renders at
0.485 mm/px (`2·0.6·tan 22.5°/1024`, a camera-frame calibration independent of
object scale — so these are world mm at the rendered scale):

| render | 62.1 mm | 124.3 mm |
|---|---|---|
| broken_column studio_albedo macro_00 / _02 | 0.80 / 0.65 | 0.31 / 0.11 |
| rock_face_01 studio_albedo macro_01 / _02 | 2.08 / 1.72 | 1.22 / 0.33 |
| broken_column studio_beauty macro_00 | 1.18 | 0.31 |
| rock_face_01 studio_beauty macro_01 | **4.68** | **2.69** |

**Texel density** (`:3167-3171`): broken_column 3.58 mm/texel vs the control's
6.52 mm/texel — the generated prop is 1.8× denser and still loses. There is **no
per-prop texel-density table for the other six props**; `mesh_density.py` ran only
on those two. `prop_audit.py`, the committed instrument that would produce one, is
currently inoperable on the rebuilt props (`target/prop-redesign-after/audit.txt`:
UV island containment error).

Two facts in that table decide a great deal on their own, before any new measurement:

- The 60–124 mm deficit is present in the **albedo** render (0.80 vs 2.08),
  which contains no shading and therefore no geometry. Under-tessellation cannot
  contribute to it at all.
- Blind test #2 holds geometry **constant** — A, B and D are the same
  `broken_column` mesh with three different albedo atlases — and moves the score
  2.5 → 4 on a 5.5-point gap. At least 27 % of the gap is texture-domain with the
  mesh untouched.

---

## 3. The instrument

`dec_attr.py` (session scratchpad; see §7). For each prop, the **shipped GLB** and
its **pre-decimation `clean_hires.glb`** are rasterized to orthographic depth maps
in world units from *n* azimuths at 15° elevation, resolution *R*. An orthographic
depth map of a triangle mesh is exactly affine inside each triangle, so this
measures geometry and nothing else — no shading, no textures, no normal map.

Per octave band σ (specified in mm, converted to px per prop), on the mask covered
by *both* meshes eroded by 2σ via exact Euclidean distance transform:

```
band(z) = G_σ(z)/G_σ(mask) − G_2σ(z)/G_2σ(mask)      (mask-normalised DoG)
r(σ)    = Pearson(band(shipped), band(hires))
D(σ)    = 1 − r(σ)²
```

`D` is the fraction of the pre-decimation relief *variance* in that octave that
the shipped mesh fails to reproduce **in the correct spatial position**. It is 0
when decimation preserved the band and 1 when the band is replaced by
uncorrelated content of any amplitude — which is precisely the failure mode the
chapel_arch probe identified ("not attenuated, replaced by equal-amplitude
faceting noise"). Amplitude-only metrics cannot see that failure; this one can.

**Instrument validation.** The three photoscan controls have no pre-decimation
reference, so they are run against themselves. The pipeline returns
**r = 1.0000 and D = 0.0000 at every band for all three** — the statistic's zero
is real, not an artifact of the estimator.

**Prior-probe reproduction.** Re-running the original probe's own script
(`ctrl.py`: single front view, 2048² box-downsampled to 1024², cumulative
high-pass, 11×11 erosion) reproduces `tasks/todo.md:1019-1046` exactly:

| σ | ~mm | hires RMS | decimated RMS | corr |
|---|---|---|---|---|
| 1.5 | 8 | 0.00313 | 0.00318 | **0.343** |
| 3.0 | 17 | 0.00509 | 0.00510 | **0.585** |
| 6.0 | 34 | 0.00781 | 0.00773 | **0.764** |
| 12.0 | 68 | 0.01153 | 0.01132 | 0.876 |
| 24.0 | 135 | 0.01695 | 0.01668 | **0.940** |

The prior probe is confirmed, not contradicted. The difference in this pass is a
*cumulative high-pass* replaced by an *octave band-pass*. A high-pass at σ carries
all content finer than σ, so its correlation stays depressed at coarse σ purely
because the destroyed fine content is still inside the filter. Decomposing into
octaves shows where the loss actually lives — and it is not where the campaign's
residual is.

### Parameter sweep

Every free parameter swept toward its limit; `D` is stable.

| prop | setting | D(4 mm) | D(8 mm) | D(17 mm) | D(34 mm) | **D(68 mm)** |
|---|---|---|---|---|---|---|
| chapel_arch | R=1024 v=4 | — | 0.3152 | 0.0905 | 0.0211 | **0.0040** |
| chapel_arch | R=2048 v=4 | 0.5904 | 0.2883 | 0.0832 | 0.0189 | **0.0038** |
| chapel_arch | R=4096 v=4 | 0.5829 | 0.2845 | 0.0824 | 0.0189 | **0.0038** |
| chapel_arch | R=2048 v=8 | 0.6722 | 0.3411 | 0.0979 | 0.0215 | **0.0038** |
| chapel_arch | R=2048 v=4, DoG ratio 1.4 | 0.6910 | 0.3681 | 0.1209 | 0.0296 | **0.0056** |
| broken_column | R=1024 v=4 | 0.0498 | 0.0118 | 0.0012 | 0.0002 | **0.0000** |
| broken_column | R=2048 v=4 | 0.0412 | 0.0104 | 0.0012 | 0.0002 | **0.0000** |
| broken_column | R=4096 v=4 | 0.0402 | 0.0100 | 0.0010 | 0.0002 | **0.0000** |
| broken_column | R=2048 v=8 | 0.0502 | 0.0132 | 0.0014 | 0.0002 | **0.0000** |
| gravestone | R=1024 / 2048 / 4096 | 0.1575 / 0.1518 / 0.1510 | 0.0321 / 0.0317 / 0.0319 | 0.0068 / 0.0070 / 0.0068 | 0.0018 | — |
| candelabra_shrine | R=1024 / 2048 / 4096 | 0.0110 / 0.0054 / 0.0036 | 0.0020 / 0.0010 / 0.0006 | 0.0002 | 0.0002 | — |

R has converged by 2048 (4096 moves D(68 mm) by 0.0000). View count 4 → 8 moves
the finest band by ≤ 0.07 and the 68 mm band by 0.0000. Narrowing the DoG from
2.0 to 1.4 raises D uniformly by ~20 % — a narrower band admits less correlated
neighbouring content — and leaves D(68 mm) at 0.0056. **No setting puts the
residual band above 0.6 %.**

A scale-adaptive decimation is used for large σ (work at ≥ 8 px per σ). Validated
against the full-resolution path on gravestone at R=2048: r agrees to four
decimals (0.9967 both at 17 mm, 0.9992 both at 34 mm).

---

## 4. Attribution: `D(λ)` per prop, R=2048, 4 views

Bands in native mesh mm — the scale at which the blind-test sheets were rendered
for the generated props (`asset_inspect --scale` defaults to 1.0, and the study's
own `6.38 m²` / `28.5 m²` citations are native areas).

| prop | hires → shipped | D(4 mm) | D(8 mm) | D(17 mm) | D(34 mm) | **D(68 mm)** |
|---|---|---|---|---|---|---|
| chapel_arch | 773 704 → 14 999 | **0.590** | **0.288** | 0.083 | 0.019 | **0.004** |
| cypress | 417 452 → 14 997 | **0.598** | **0.253** | 0.056 | 0.010 | **0.003** |
| olive_stump | 742 440 → 14 999 | **0.273** | 0.074 | 0.015 | 0.003 | **0.001** |
| gravestone | 253 732 → 14 999 | 0.152 | 0.032 | 0.007 | 0.002 | — |
| crucero | 191 354 → 15 000 | 0.044 | 0.010 | 0.002 | 0.000 | — |
| broken_column | 326 406 → 14 998 | 0.041 | 0.010 | 0.001 | 0.000 | **0.000** |
| candelabra_shrine | 187 022 → 14 998 | 0.005 | 0.001 | 0.000 | 0.000 | — |
| *rock_face_01* (self-test) | 20 174 → 20 174 | 0.000 | 0.000 | 0.000 | 0.000 | **0.000** |
| *rock_07* (self-test) | 14 844 → 14 844 | 0.000 | 0.000 | 0.000 | — | — |
| *rock_09* (self-test) | 12 416 → 12 416 | 0.000 | — | — | — | — |

(Dashes: the band exceeds the prop's own extent or its mask after erosion falls
below 2 000 px. The trend is monotone decreasing, so every dashed cell is bounded
above by the cell to its left.)

**The requested statistic, stated plainly: the fraction of the measured 60–124 mm
residual band accounted for by triangle-scale faceting is ≤ 0.4 %, per prop, for
every generated prop that ships.** It is 0.0 % for broken_column, the asset both
blind tests actually judged in detail.

### The exact geometric-Nyquist statistic

An independent, parameter-free cross-check that needs no reference mesh. An
orthographic depth map being exactly affine per triangle, relief of wavelength λ
cannot exist on a triangle whose in-plane footprint exceeds λ/2. Define
`G(λ)` = area fraction of the surface on such triangles — the fraction that is
*structurally blind* at λ, regardless of what the generator produced.

At the scale each asset was rendered for the blind test (generated ×1,
`rock_face_01` ×4 per `REFERENCE_SCALE`):

| perceptual rank | prop | area-wt footprint mm | **G(62 mm)** | **G(124 mm)** |
|---|---|---|---|---|
| 1 (winner) | *rock_face_01* @×4 | 319.4 | **1.000** | **0.999** |
| 2 | crucero | 33.0 | 0.368 | 0.114 |
| 3 | gravestone | 33.7 | 0.372 | 0.093 |
| 4 | broken_column | 50.2 | 0.616 | 0.135 |
| 5 (worst) | candelabra_shrine | 42.6 | 0.432 | 0.143 |

Stable across all four footprint conventions tried (√2a, longest edge,
equilateral-equivalent, mean edge): the winner is blind on 100 % of its surface
under every one of them, and even at its *native* scale (footprint 79.9 mm) it is
blind on 93.7 % at 62 mm — still worse than every prop it beat.

The winner cannot carry the band in question at all. The losers can carry
40–90 % of it. **The hypothesis predicts the opposite of what is observed.**

---

## 5. The ranking test

Does per-prop triangle coarseness predict per-prop perceptual deficit? Ground
truth is the row-39 blind ranking, the only per-prop perceptual data on disk.

| predictor | set | ρ | stability |
|---|---|---|---|
| mean triangle edge | all 5 (incl. control) | **−0.10** | −0.15…−0.10 across 4 footprint conventions |
| geometric blindness G(62 mm) | all 5 (incl. control) | **−0.10** | −0.10 across all 4 conventions |
| mean triangle edge | 4 generated only | +0.80 | +0.74…+0.80 |
| **decimation damage D(4 mm)** | 4 generated only | **−0.80** | −0.80 at R = 1024, 2048 and 4096 |
| **decimation damage D(17 mm)** | 4 generated only | **−0.80** | −0.80 at R = 1024, 2048 and 4096 |

Read honestly:

- The **+0.80** within the four generated props is the hypothesis's best showing,
  and it does not survive inspection. It rests on crucero (33.0 mm) beating
  gravestone (33.7 mm) — a 2 % separation, well inside the spread between
  conventions — and on one inverted pair. With n = 4 nothing here is significant.
- The moment the control enters — the asset that actually reads as stone — the
  correlation collapses to **−0.10**. Adding the one data point that carries the
  effect being explained destroys the effect.
- Against the **direct** measurement of decimation damage rather than a
  triangle-count proxy, the sign is **negative and stable at −0.80 across a 16×
  range of instrument resolution**. gravestone lost 15.2 % of its 4 mm relief
  variance and ranks 2nd; candelabra_shrine lost 0.5 % and ranks last.

There is no reading of this data in which under-tessellation orders the props the
way the reviewer did.

---

## 6. What the residual band is not, and one measured candidate

Not in scope to attribute here, but three facts from the shipped bytes bound the
search and are worth recording because they cost nothing:

- **Every generated prop ships a flat scalar roughness and no
  `metallicRoughnessTexture`** (`roughnessFactor` 0.85, or 0.75/0.70 for
  candelabra_shrine/cypress). **All three photoscan controls ship a spatially
  varying ARM map.** The reviewer's second cited tell — *"one uniform sheen with
  no specular breakup"* — is a literal description of a constant roughness
  scalar, and it is in the bytes, not in an inference.
- Magnitude, measured, so this is a candidate and not a conclusion:
  `rock_face_01_arm_1k.jpg` roughness has an overall std of only **7.97/255**, and
  band std **1.18 at 52 mm / 1.26 at 104 mm** (native px scale). `rock_07` and
  `rock_09` roughness are essentially flat (std 0.92 and 0.94). That is a small
  absolute signal — it is not obviously large enough to carry a 4-point gap on
  its own, and it should be priced before it is bought.
- Both sides ship AO (generated props carry a baked `occlusionTexture`;
  the controls' ARM red channel), so AO is not a present/absent difference.

The reviewer's *first* tell — "shading painted into albedo rather than earned by
geometry" — is the one that sounds most like this hypothesis. It is not:
`D(60–124 mm) ≈ 0` says the shipped mesh reproduces the generated relief in that
band faithfully. What the tell describes is albedo carrying luminance structure
that does not move with the light, which is a texture-domain property and is
exactly what the albedo-band table above measures.

---

## 7. Reproducing this

All scripts in the session scratchpad
(`…/c5e0823f-2866-44d5-9508-d0d4ab7579df/scratchpad/`), none added to the repo:

- `dec_raster.py` — orthographic depth rasterizer, world units, GLB + glTF.
- `dec_attr.py` — band-pass residual correlation, shipped vs `clean_hires`.
- `nyquist.py` — exact `G(λ)` geometric-blindness statistic.
- `geom.py` — per-prop triangle/area/edge table from shipped bytes.
- `ctrl.py`, `depth.py`, `glbprobe.py` — the prior probe's own instrument, reused.
- Results: `main_2048_v4.json`, `sw_{1024,4096}_v4.json`, `sw_2048_v8.json`,
  `sw_ratio14.json`, `nyquist.json`.

Per `tasks/lessons/2026-07-21-keep-verification-artifacts.md`, these live only in
a Temp scratchpad and will be lost. The `.json` results are the ones worth
promoting if any of this is to be re-checked.

Files read (shipped bytes): `content/models/props/{chapel_arch,crucero,broken_column,gravestone,olive_stump,cypress,candelabra_shrine}/<prop>.glb`,
`content/models/props/{rock_07,rock_09,rock_face_01}/<prop>_1k.gltf`,
`content/models/props/rock_*/textures/*_arm_1k.jpg`, `content/models/assets.json`,
`content/zones/zones.ron`. Pre-decimation references:
`target/prop-batch/b3/arch/cand_0/clean_hires.glb`,
`target/prop-batch/timed/cand_0/clean_hires.glb`,
`target/prop-batch/rebuild/{crucero/cand_21,gravestone/cand_1,olive_stump/cand_0,cypress/cand_21,candelabra_shrine/cand_4}/clean_hires.glb`
(each matched to its shipped prop by exact `clean_height` from `cleanup_stats.json`).

---

## 8. What should be rebuilt, and at what cost

**Do not launch a whole-set retessellation to close the stone-read gap.** It
would be the same failure as the triplanar detail layer: a real mechanism, faithfully
implemented, aimed at a band it does not control. The measured return in the
60–124 mm band is bounded above by 0.4 % of that band's variance.

Three things are worth doing, in this order.

**(a) Free — stop `assets.json` from making things worse.** The per-asset
`tri_budget` values from `9e92cab` have never been applied and, if applied,
would coarsen four of seven props. Whatever is decided about budgets, the current
file should not be run as-is. (No edit made in this pass; flagged only.)

**(b) Cheap and justified on its own merits — retessellate chapel_arch, and only
chapel_arch.** Its defect is measured, large and specific: D = 0.590 at 4 mm and
0.288 at 8 mm, 100 % blind at 62 mm, 170 mm triangles against 1–5 cm carving.
That is the melted-carving defect and it is a legibility problem for a 5.5 m
hero prop the player walks under — an argument that stands without any reference
to the stone-read campaign. Cost: to reach a 40 mm footprint it needs
**~171 000 triangles** (12× its current budget); 20 mm needs ~683 000, i.e.
essentially shipping the hires mesh. cypress (D = 0.598 at 4 mm) and olive_stump
(D = 0.273) are the same defect one tier down and would cost ~7 800 and ~15 400
triangles respectively for a 40 mm footprint — both *cheaper than what they ship
today* relative to their surface area, because the flat 15 k budget over-spends on
small props and starves large ones. A budget derived from surface area rather than
hand-set per asset is the correct shape, and it is a small change.

For reference, the full set at a uniform footprint target:

| target footprint | total tris, 7 props | vs shipped (104 990) |
|---|---|---|
| 40 mm | 216 215 | 2.1× |
| 20 mm | 864 868 | 8.2× |
| 10 mm | 3 459 480 | 33× |

**(c) The next probe on the actual gap should not be geometric.** The cheapest
discriminating measurement still unmade is a per-prop version of the row-41 band
table: `results.json` has albedo/beauty macro bands for exactly two assets
(broken_column and the control), which is why the 60–124 mm figure has no per-prop
resolution and cannot be regressed against anything. Rendering the existing
`asset_inspect` albedo + beauty macro pass over all seven props and re-running
`analysis.py` would produce a per-prop residual-band table at no generation cost —
GPU time for 7 × 2 × 3 stills, no model inference. That table is what any further
attribution needs, and its absence, not a missing mechanism, is why this campaign
keeps re-litigating causes. `prop_audit.py` should be repaired in the same pass
(its UV-island containment check currently errors on the rebuilt props).

**Not recommended without that table:** buying a per-texel roughness map. §6 shows
the control's own roughness variation is small in absolute terms; it may well be
another faithful-but-irrelevant mechanism, and it should be priced against a
per-prop measurement before it is built.
