# G2 kit-cohesion gate — RE-GATE verdict

Date: 2026-07-31. Campaign Phase 2, P2.3. Opus visual gate over the rebuilt
`casa_corner` pilot after kit fix round `241f23a`, judged from
`target/town-kit/g2-regate/` (inspect matrix + street set, all AO-on/GTAO,
binaries at `897c40c`) against the original G2 FAIL.

The original G2 verdict (Q1 3/10, Q2 2/10, Q3 5/10, Q4 2/10) was never written
to `docs/reviews/town/` — it survives only in the P2.3 summary at
`tasks/todo.md:287-307` and in the gate agent's transcript. Its Q1–Q4
definitions (Cohesion / Roof arbitration / Palette risks on real geometry /
Defect sweep) and its D1–D10 defect ids are carried forward here verbatim so
the two records compare directly.

## Verdict

# G2 FAIL — one blocker

> Superseded the same day by "Blocker re-check" at the end of this file:
> the blocker was fixed (`6d665d6`), evidence re-rendered, and the gate is now
> **G2 PASS with watch items**. Everything below is the record of the FAIL round
> and is kept unchanged so the two rounds compare.

**Blocker: two of the four roof planes carry encalado instead of terracotta.**
Every other named defect class is fixed or improved, several decisively. The
kit does not proceed to P2.4 until the roof material/UV assignment is repaired
and re-shown.

| Question | G2 | re-gate | movement |
|---|---|---|---|
| Q1 Cohesion | 3 | **3** | palette cohesion won; asset-internal cohesion broke worse |
| Q2 Roof arbitration | 2 | **3** | risk now discharged *affirmatively* on the planes mapped right |
| Q3 Palette risks | 5 | **6** | R2 resolved, R3/R4 re-discharged, R1 confirmed worse |
| Q4 Defect sweep | 2 | **4** | 6 of 10 defects verified fixed; the new one outweighs them |

## The blocker

`casa_corner` builds two gable roofs (main block, wing) = four slopes. **Two of
them render as white lime plaster with no tile and no tile relief.**

- Geometry proof, not a shading read: `inspect/ship_normal/full_03.png` — the
  large plane at y 390–600 is up-facing (green) with a smooth normal field,
  while the band above it (y 340–390) carries tile ribs. `ship_clay/full_02.png`
  shows the same split with materials stripped: right slope ribbed, left slope
  perfectly smooth. `raking_beauty/full_02.png` lights the untiled slope as a
  brilliant white roof plane beside its tiled twin.
- Material proof: `ship_albedo/full_03.png` — the untiled slope's base colour is
  encalado, carrying the same mottle and the same ghost-plaque feature as the
  walls below it.
- In-zone consequence: `street/row/mid_00.png` — the untiled slope measures
  lum 0.665 / S 0.031 against the encalado wall's 0.542, i.e. **the missing
  roof is the brightest mass in the street frame.** `street/row/wide.png` at
  ~30 m reads as three white boxes with a thin red stripe on top; the terracotta
  that is supposed to carry the town silhouette is absent from the establishing
  shot.
- Attribution: this is a **regression of this round**, not a carried defect.
  The previous round's frame `target/town-kit/g2-pilot/inspect/ship_beauty/full_00.png`
  shows all four slopes tiled (badly mapped, but tiled). Two things changed
  together — the fix commit's per-plane roof UVs, and the first-ever BC7/BC5
  DDS sidecar bake the manifest documents. **The evidence set cannot separate
  them**, and the discriminating frame is cheap (see "Evidence a fix must show").

## Defect-class rulings

**(a) Roof slope UVs — PARTIALLY FIXED, then regressed.**
- One canonical scale: **fixed where mapped.** The main block's tiled slope runs
  a single pan scale end to end, ~28 pans across the 6 m block ≈ 21 cm/pan —
  correct teja gauge (`ship_beauty/full_03.png` ridge band, crop x 430–830
  y 320–400). The old 46.0-vs-16.4 px break inside one plane is gone; no
  field-vs-ridge-strip scale cut is present in any frame.
- Orientation: **fixed on the main block, still wrong on the wing.** The main
  slope's pans run eave-to-ridge and read as unambiguous barrel courses. The
  wing's tiled slope still runs its courses **across the fall line**
  (`ship_beauty/full_03.png` x 320–470 y 370–600; `street/row/mid_00.png`
  x 330–700) — at gameplay distance it reads as louvre slats, not teja.
- Valley junction: **fixed.** `ship_beauty/full_00.png` region (400,440)–(720,580)
  shows the wing tile field terminating cleanly against the main block's wall
  with continuous course spacing and **no black gaps** anywhere along the
  junction. This was the case the pilot was chosen to test; it now passes.
- Net: the sub-defect that failed hardest is fixed, and a worse one replaced it.

**(b) Wing long face sealed — FIXED.**
The wing's long face is a solid wall carrying a real window opening with reveal
depth (`ship_beauty/full_00.png` lower block; `ship_beauty/full_03.png`;
`street/window/close_town-kit.png`). No see-through void, no orphaned quoin
ladder — every quoin now terminates into wall. Compare the previous round's
`g2-pilot/.../full_00.png`, where the void and the free-standing quoin steps are
plainly visible.

**(c) Quoins — FIXED.**
- Size variation is real and reads: `ship_beauty/gameplay_03.png` shows three
  adjacent blocks of visibly different height and width, each with its own
  fracture, staining and edge damage.
- Per-block UV offsets killed the repeat: detrended vertical autocorrelation on
  the quoin column of `street/door/close_town-kit.png` peaks at **corr 0.205**
  (lag 62 px), down from the FAIL's **0.308** one-block peak. No identical face
  recurs; the raking frames no longer resolve an identical diamond boss per
  block (`raking_beauty/full_00.png`, `full_02.png`).
- The brick read is gone. In-zone the quoin ladder measures **S 0.049** at
  V 0.517 (`street/row/mid_00.png`) — a near-achromatic dressed grey, against
  the FAIL's warm tan hue 33° / S 0.217.

**(d) Eave sawtooth fringe + debris specks — FIXED, with one residual artifact.**
No sawtooth teeth on any eave in any frame; the eave line reads as a clean
course of rounded tile ends (`ship_beauty/full_03.png` ridge band;
`street/row/mid_00.png`). No stray specks trailing the silhouette in the normal
pass (`ship_normal/full_03.png`). **Residual, different class:** a smooth
elongated blob of tile geometry protrudes past the wing's eave and droops below
the eave line — visible in both `ship_beauty/full_03.png` (x ~430–470, y ~560–600)
and `street/row/mid_00.png` (x ~600–670, y ~270–320), so it is on the asset, not
the frame. Watch-level on its own; it sits on the same wing plane as the
orientation error, so a wing-plane fix should clear both.

**(e) Ashlar cool-grey grade — PASS, marginal on b\*.**
Albedo-channel Lab over three quoin blocks in `ship_albedo/gameplay_03.png`:

| block | L\* | a\* | b\* |
|---|---|---|---|
| A | 66.3 | 1.01 | 5.02 |
| B | 80.5 | 1.05 | 4.38 |
| C | 59.2 | 1.69 | 4.62 |

Mean L\* 68.7 lands inside the 68–74 target; a\* ≤ 1.5 holds on two of three
(1.69 on the darkest block); b\* sits **at** the ≤ 5 ceiling rather than under
it. The per-block L\* spread (59–81) is wider than the target band, but that
spread *is* the mechanism that killed defect (c)'s repeat, so it is accepted,
not charged. What matters for the vocabulary call is the rendered read, and it
is unambiguous: dressed cool limestone at S 0.049 in-zone, no warm tan anywhere.
§3's "pale grey dressed limestone ashlar" is now honoured.

## Q1 — Cohesion — 3 / 10

**What holds, and it is more than last round.** The quoins now sit in the same
material family as the photoscan props instead of fighting them; the wall values
still sit in the props' band (encalado wall lum 0.542, quoin 0.502, ground 0.440
in `street/row/mid_00.png`); the sealed wing removes the single worst
believability break; the party-wall run welds without a visible junction seam
(`street/row/mid_00.png` — max coplanar column-to-column tone step measured at
3.1/255 across the 700 px wall panel, i.e. no step at all).

**What fails.** Asset-internal consistency broke in a larger way than the texel
break it replaced: on one shell, two roof planes are terracotta and two are lime
plaster. At the only distance the player occupies, the roof is not merely wrong
— it is the brightest thing in frame (lum 0.665 vs the wall's 0.542). At ~30 m
the row carries no terracotta silhouette at all. A Castilian town whose roofs
read white cannot be scored higher than its predecessor on cohesion however much
the palette improved.

## Q2 — Roof arbitration — 3 / 10

**The accepted risk is now discharged affirmatively, for the first time.** The
main block's tiled slope proves photoscan normal relief carries barrel-tile
structure at true slope scale and survives distance: convincing 3D tubes with
self-shadow at full framing (`ship_beauty/full_03.png` ridge band), still
reading as courses under raking (`raking_beauty/full_00.png`), still reading as
tile in-zone at gameplay pitch (`street/row/mid_00.png` wing slope band). No
scalloped eave strip or geometric tile rows are needed — the arbitration
question the plan reserved for G2 is answered: **relief suffices, geometry is
not required.**

**Why the score is still low.** That answer is delivered on one of four planes.
One plane is rotated 90°, two carry no tile. The arbitration is discharged; the
asset is not.

## Q3 — Palette risks on real geometry — 6 / 10

- **R1 oak vs iron — CONFIRMED ADVERSE, now street-tested and worse.** At 2.3 m
  face-on (`street/window/close_town-kit.png`) the reja bar measures lum 0.090
  against the oak shutter's 0.084 — **Δlum 0.006**, below the FAIL's 0.010.
  Separation is silhouette only. The requested street test now runs and fails
  differently than predicted: at `street/row/mid_00.png` the door is a flat dark
  void (lum 0.169) with no plank read, and the reja is not resolvable. m4-vs-m6
  is confirmed as a real palette weakness, not a rendering accident.
- **R2 quoin vs roof — RESOLVED.** The ashlar grade collapsed quoin saturation
  to 0.049 against the roof's 0.246; they no longer occupy the same corner of
  the palette. The predicted 30°-vs-16° hue fight is moot — one participant is
  now achromatic.
- **R3 terracotta saturation — DISCHARGED again.** In-zone roof S mean 0.246
  (p95 0.371), V 0.482, hue 14.6°. Inside the reserved hue window, but the
  threat gate also demands S ≥ 0.7 and V ≥ 0.8; nothing here can telegraph.
- **R4 whitewash brightness — DISCHARGED.** In-zone encalado V mean 0.553,
  p95 0.678 — under §2's V ≤ 0.6 ambient ceiling on the mean (the FAIL measured
  0.597, at the ceiling), no clipping anywhere.
- **New, arising from the blocker:** the whitewash's share of visible surface is
  now far above what the palette ladder was balanced for, because it has taken
  the roofs. The R4 discharge holds per-pixel but should be re-read once the
  roofs are terracotta again.

## Q4 — Defect sweep — 4 / 10

| # | G2 defect | status |
|---|---|---|
| D1 | Roof UV scale discontinuity (46.0 vs 16.4 px, ≥3 scales) | **FIXED** — one canonical ~21 cm pan scale on the tiled main slope |
| D2 | Roof UV rotation, front slope | **PARTIAL** — main slope correct; wing slope still rotated 90° |
| D3 | Sawtooth eave fringe | **FIXED** — no teeth on any eave, any frame |
| D4 | Stray eave debris specks | **FIXED** — silhouette clean in `ship_normal/full_03.png` |
| D5 | Valley-junction seam + black gaps | **FIXED** — continuous, no gaps (`ship_beauty/full_00.png` (400,440)–(720,580)) |
| D6 | Open wing face + orphaned quoin | **FIXED** — sealed wall with a real window |
| D7 | Quoin repetition (corr 0.308) | **FIXED** — corr 0.205, visible size variation |
| D8 | Quoin reads as brick (warm tan) | **FIXED** — S 0.049 dressed grey, grade in band |
| D9 | Encalado mip-blur at macro | **PERSISTS** — macro frames carry edge energy 113–162 (var) vs 292 at gameplay and 384 at the 4 m door close, i.e. the closest frames are the softest |
| D10 | Ghost blocks under the whitewash | **PERSISTS, more prominent** — rectilinear block outlines and inscription-like marks read across the whole wall at gameplay and at 2–4 m (`ship_beauty/gameplay_00.png`, `street/door/close_town-kit.png`, `street/window/close_town-kit.png`) |
| **N1** | **Two of four roof planes carry encalado, no tile** | **NEW — blocker** |
| N2 | Blob of tile geometry protruding past the wing eave | **NEW — watch** |

## Street gaps the original judge named

- **(a) roof slopes / eave line — COVERED and adequate.** `row/wide.png` frames
  all three units' full roof runs; `row/mid_00.png` frames the eave line, the
  party junction and ~12 m of facade at gameplay pitch. The evidence is good
  enough that it is what convicts the roof.
- **(b) wall opening — COVERED and adequate.** `window/close_town-kit.png` shows
  the opening with reveal depth, sill and lintel face-on at 2.3 m.
- **(c) door + iron reja — COVERED, caveat accepted.** `door/close_town-kit.png`
  (oak leaf, limestone jamb, quoin run, player for scale) and
  `window/close_town-kit.png` (reja over shutter). The manifest's note that casa
  doors carry no reja by construction is accepted — the R1 pair is judged on the
  window, and R1 is ruled above on that basis.
- **(d) party-wall encalado repeat at ~30 m — TESTED, NOT DETECTED.** Detrended
  horizontal autocorrelation of the facade band: **0.226** at `row/mid_00.png`
  (~12 m of facade), **0.21–0.26** on the coplanar wall panels of the two full-res
  closes, **0.167** on `ship_beauty/full_03.png`'s wall band. The one strong
  periodicity in the set — corr 0.576 at lag 187 px in `row/wide.png` — is the
  **building module** (three placements of one glb, ~197 px/unit across the run),
  not the texture, and must not be attributed to the encalado. The
  Plaster003 diagonal-band and plaque features are present but do not recur at
  shipping facade length. **Risk discharged.**

## Carried watch items

| item | status |
|---|---|
| D9 encalado macro mip-blur | **SEEN, persists** — measured above; still the detail-layer candidate |
| D10 ghost blocks under whitewash | **SEEN, persists and reads worse** — now the dominant read of the whitewash at 2–4 m |
| m5 mineral-stain read | **NOT SEEN — no carrier.** `plaster_smoked` is chapel-only and is baked-but-unbound on `casa_corner`; this evidence set cannot judge it. Defer to G4. |
| m4/m6 door-vs-reja merge at street distance | **SEEN, CONFIRMED WORSE** — Δlum 0.006 at 2.3 m; unresolvable at street distance |
| m2/m3 hue proximity | **RESOLVED** — the ashlar grade removed the quoin from the chromatic field entirely (S 0.049) |
| m3 saturation at ceiling | **SEEN, unchanged and non-telegraphing** — in-zone mean S 0.246 under the 0.35 ceiling, p95 0.371 just over |

## Evidence a fix must show

1. **All four roof planes tiled**, proven in a channel that cannot be argued
   with: every up-facing plane showing tile ribs in `ship_clay` or `ship_normal`
   at all four inspect angles.
2. **The wing slope's pans running eave-to-ridge**, shown at the
   `street/row/mid_00.png` framing where the current rotation is legible.
3. **One canonical pan scale across main and wing**, stated as pans-per-metre
   measured on both, not as a screen-pixel period.
4. **The wing-eave blob gone** (N2), same two framings.
5. **One discriminating frame for the cause** — a single inspect angle of the
   *same* glb rendered on the embedded-PNG path with the DDS sidecars moved
   aside. If the slopes are tiled there, the defect is in the sidecar bake or
   its binding, not in the kit script, and the fix belongs in the texture
   pipeline. This costs one render and decides where the whole fix goes; it
   should be step one.

## Caveats acknowledged

- `row/` frames use 1024² DDS sidecars (GPU VRAM ceiling). Judged there:
  geometry, roof/eave read, encalado tiling repeat, in-zone value relationships.
  **Not** judged there: texel sharpness — that comes from the inspect matrix and
  the two full-res closes, both of which are full-resolution.
- `ao` debug-channel frames are uniform white by design (that channel samples the
  material occlusion texture, which `casa_corner` does not author; GTAO enters
  through shading). No inference was drawn from them.
- Casa doors carry no reja by construction; the R1 pair was judged on the window.
- Metal-channel frames that are all black at angles occluding the rejas are
  correct channel semantics, not empty frames.
- The `studio` and `furnace` lighting groups were not re-rendered this round;
  nothing in this verdict rests on them.

---

# Blocker re-check (same day)

Second Opus visual pass over the same evidence tree after the roof-regression
fix `6d665d6`. All frames re-rendered from the rebuilt `casa_corner.glb`,
binaries still `897c40c`, AO on, same lighting groups, same framings. Judged
from the PNGs only; nothing was re-rendered for this pass.

## Verdict

# G2 PASS — blocker cleared, three watch items

| Question | G2 | re-gate | re-check | movement |
|---|---|---|---|---|
| Q1 Cohesion | 3 | 3 | **7** | roof reads terracotta at every distance; value hierarchy now correct |
| Q2 Roof arbitration | 2 | 3 | **8** | relief-only tile discharged on **all four** planes, both lighting groups |
| Q3 Palette risks | 5 | 6 | **7** | R4's "re-read once the roofs are terracotta" now done, and it holds |
| Q4 Defect sweep | 2 | 4 | **7** | N1 fixed, D2 fully fixed; D9/D10/N2 persist, N3 new |

## Check 1 — all four roof slopes terracotta + tile relief — CLEARED

Checked at every framing the FAIL round convicted on, in the two channels that
cannot be argued with, plus beauty:

- `inspect/ship_normal/full_03.png` — the plane that was a smooth up-facing
  green field in the FAIL round now carries tile ribs edge to edge. No smooth
  up-facing plane anywhere in the frame.
- `inspect/ship_clay/full_03.png`, `ship_clay/full_00.png`, and the
  `sheet_ship_clay.png` / `sheet_ship_normal.png` index rows — tile relief on
  every roof plane at **all four** inspect angles, materials stripped.
- `inspect/ship_albedo/full_03.png` — every slope is terracotta base colour. No
  encalado mottle, no ghost-plaque feature on any roof plane.
- `inspect/ship_beauty/full_02.png` is the decisive angle: both main-block
  slopes are tiled and mirror each other about the ridge. Pan period measured on
  the two slopes at three heights: 27.22/27.13, 31.75/31.51, 37.07/36.57 px —
  identical gauge, and the growth down the frame is perspective, not a scale cut.
- `raking_beauty/full_00..03` (`sheet_raking_beauty.png`) — all four slopes
  carry relief self-shadow under raking light; none reads as a plaster plane.
- Street silhouette: `street/row/wide.png` at ~30 m is now three terracotta
  roofs over a whitewash run. The "three white boxes with a thin red stripe"
  read is gone; terracotta occupies 19.4 % of the roof-band pixels.
- In-zone value hierarchy inverted back to correct at `street/row/mid_00.png`:
  main roof lum **0.344**, wing roof **0.366**, encalado wall **0.532**, ground
  0.440. In the FAIL round the untiled slope measured 0.665 and was the
  brightest mass in frame; the roof is now the darkest large mass, as a
  Castilian street should read.
- Main and wing sit in one material: roof hue 13.6 deg (main) / 17.4 deg (wing),
  S 0.287 / 0.251, V 0.421 / 0.428. No two-material split remains.

Corroboration (not frame evidence): `target/town-kit/build_report.json` lists
four roof slopes, each with `relief_area > 0` and no `roof_faults`; relief/deck
ratio is 0.264 on the main slopes and 0.262 on the large wing slope, i.e. one
relief density across main and wing.

## Check 2 — wing slope UVs (D2 residual) — CLEARED

At both named framings the wing pans now run eave-to-ridge:

- `street/row/mid_00.png`, wing roof x 180–660 — on each wing slope the barrel
  axes run up the fall line into the ridge crease. The louvre-slat read is gone
  at gameplay pitch, which is where the FAIL round found it legible.
- `inspect/ship_beauty/full_03.png` x 160–460 — same at full framing: two slopes
  of barrel courses meeting at the ridge, mirrored.
- One canonical scale holds along the ridge: autocorrelation of the main slope's
  along-ridge profile in `ship_albedo/full_03.png` has a single clean peak at
  16 px (corr 0.936) with no second period. Across the 6 m block that is ~24
  pans, ~25 cm/pan — the same band the FAIL round recorded from an eye count
  (~21 cm), coarse end of teja gauge but one gauge, and the wing matches it by
  the relief-density figure above.

## Check 3 — regression sweep on the reworked shell union — NOTHING NEW

The fix replaced chained pairwise booleans with a single multi-operand EXACT
union, so the seams, welds and whitewash were re-swept against the FAIL round's
record.

- **Coplanar wall seams — none.** Column-to-column luminance step across the
  party-wall facade in `street/row/mid_00.png`: p99 **2.96/255** right of the
  door, **4.25/255** left of it, max 6.68. Same order as the FAIL round's 3.1,
  and invisible in the frame.
- **Main/wing corner weld — clean.** `ship_beauty/full_03.png` x 400–560
  y 540–780: the quoin ladder runs the full corner, every block terminates into
  wall, no gap, no z-fight, no double surface.
- **Valley junction (D5) still fixed.** `ship_beauty/full_00.png` region
  (380,420)–(740,600): the wing tile field terminates against the main block's
  gable wall with continuous course spacing and no black gaps.
- **Shell still sealed.** `street/door/wide.png` and `street/window/wide.png`
  show no new void, no orphaned quoin, no missing face at any angle.
- **Whitewash unchanged in kind.** The pre-fix albedo in
  `probe/ship_albedo/full_03.png` and the post-fix
  `inspect/ship_albedo/full_03.png` carry the same ghost-plaque and
  rectilinear-plate structure on the wall. The union rework introduced no new UV
  island boundary and no new tone plate — D10 is the same defect at the same
  strength, not a regression of this round.

## Check 4 — carried defects and the one new finding

| # | defect | status at re-check |
|---|---|---|
| N1 | Two of four roof planes carry encalado, no tile | **FIXED** — check 1 |
| D2 | Roof UV rotation, wing slope | **FIXED** — check 2 |
| D1, D3–D8 | scale, sawtooth, specks, valley, open face, quoin repeat, quoin hue | **still fixed** — re-confirmed in this evidence set |
| D9 | Encalado mip-blur at macro | **PERSISTS** — gradient-energy variance 1–3 at macro vs 16–45 at gameplay and 74 at the 4 m door close; the closest framings remain the softest |
| D10 | Ghost blocks under the whitewash | **PERSISTS, dominant** — `street/door/close_town-kit.png`, `street/window/close_town-kit.png`, `street/row/close_town-kit.png`: rectilinear plates and inscription-like marks are the whitewash's main read at 2–4 m |
| N2 | Tile blob protruding past the wing eave | **PERSISTS** — same lobe, same two framings: `ship_beauty/full_03.png` x 440–450 y 556–588, `street/row/mid_00.png` x 600–630 y 275–320 |
| **N3** | **Roof albedo mirror-wraps down the fall line** | **NEW — watch.** The main slope's eave-to-ridge luminance profile in `ship_albedo/full_03.png` is mirror-symmetric about y 473 at **corr 0.763** over 123 rows. It renders as a dark band across mid-slope that reads as a painted stripe, and it is legible at ~30 m in `street/row/wide.png`. Texture-layer, same family as D9/D10; it does not touch the terracotta silhouette. |

Also observed, art-level: the slopes meet at a bare arris with a thin plaster
verge — there is no ridge cap (caballete) course. Visible in
`ship_beauty/full_02.png` and `raking_beauty/full_02.png`. Not charged.

## Palette risks re-read with the roofs restored

- **R1 oak vs iron — CONFIRMED ADVERSE, unchanged.** Re-sampled across the reja
  at `street/window/close_town-kit.png` y 300: bars and oak both sit in the
  0.06–0.13 lum band; the bars are separated only by specular glints (0.13–0.21
  spikes) and by silhouette. The FAIL round's Δlum 0.006 stands.
- **R2 quoin vs roof — RESOLVED.** In-zone quoin S 0.042 against roof S 0.287.
- **R3 terracotta saturation — DISCHARGED.** In-zone roof S mean 0.287
  (p95 0.347), V 0.421, hue 13.6–17.4 deg. Under the 0.35 ceiling on the mean
  and nowhere near the S >= 0.7 / V >= 0.8 threat gate.
- **R4 whitewash brightness — DISCHARGED, and the FAIL round's caveat with it.**
  Encalado V mean **0.543**, p95 0.675, under §2's 0.6 ambient ceiling. With the
  roofs terracotta again the whitewash is no longer over-represented, so the
  ladder is balanced as designed.

## Carried watch items

| item | status carried forward |
|---|---|
| D9 encalado macro mip-blur | **OPEN** — measured above; still the detail-layer candidate |
| D10 ghost blocks dominate the whitewash at 2–4 m | **OPEN** — the dominant read of the encalado at close range |
| m4/m6 door-vs-reja merge, Δlum 0.006 | **OPEN** — re-confirmed; a real palette weakness, not a rendering accident |
| m5 mineral-stain read, unbound on `casa_corner` | **DEFERRED TO G4** — `build_report.json` binds five materials; `plaster_smoked` is baked but unbound, so this evidence set still cannot judge it |
| N2 tile blob past the wing eave | **OPEN** — unchanged by this fix |
| N3 roof albedo mirror-wrap down the fall line | **OPEN — new this round** |

## Caveats acknowledged

- `street/row/` frames still use 1024² DDS sidecars; texel sharpness was judged
  only from the full-res inspect matrix and the two full-res closes.
- `probe/ship_albedo/` is the **pre-fix** glb on the embedded-PNG path. It was
  used here only as the A/B for the whitewash and to confirm the FAIL round's
  defect; no verdict rests on it.
- `ao` debug-channel frames are uniform white by design; no inference drawn.
- `studio` and `furnace` groups were not re-rendered; nothing here rests on them.
