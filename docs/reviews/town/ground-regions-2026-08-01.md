# Rocalba ground regions — visual gate

Frames: `target/zone_review/rocalba_ground_regions/` (2026-08-01).
Baseline: `target/zone-review/p30-chapel/` (same named shots, pre-change).
Premise: `docs/town-premise.md` §2, §3, §4.

## Verdict — PASS WITH FIXES

The ruled design survived contact with pixels. The **region mechanism is
correct and the placement ruling is sound**: the grid-snap claim is true to
sub-pixel, UV continuity holds across the seam, and nothing outside the two
rectangles moved. What fails is the **material itself** — the cobble renders at
the wrong stone scale, with inverted joint values, in the wrong hue family —
and one **placement leak** where the plaza rectangle punches through the casa
rows into open field.

| Axis | Score | Note |
|---|---|---|
| Boundary quality | **4 / 5** | Geometrically perfect; loses a point for a zero-transition edge that saws stones in half |
| Extent believability | **2 / 5** | Reads as a rug at eye level: a hard rectangle with a re-entrant notch in open ground |
| Material read | **2 / 5** | Fails §3 on stone scale, joint colour, and hue family; passes §2's numeric ceiling |
| Zone improvement | **3 / 5** | Structurally a clear win (the street finally has a floor); chromatically a step back |

---

## 1. Boundary quality — the load-bearing claim holds

**No staircase.** Measured on the plaza's west edge in `mid_plaza.png`
(cobble/earth segmented by B−R > 4, first sustained run per row, rows 300–870):

| Edge | n rows | RMS residual to a fitted line | max residual |
|---|---|---|---|
| `mid_plaza.png` plaza west edge | 81 | **0.488 px** | 2.43 px |
| `mid_plaza.png` plaza east edge (upper segment, rows 360–629) | 195 | 2.14 px | 12.07 px |

0.49 px RMS over 570 rows of image height is a straight line with antialiasing
noise and nothing else. The ruling — quantization only bites diagonal and
curved bounds — is **confirmed**.

Every apparent "step" in the frames is a **union corner**, not quantization.
The 28 px jump at row 629 of the plaza east edge, and the L-notch at
`mid_gate.png` (~x 1330, y 500), are both the single re-entrant corner where
the plaza (|z| ≤ 12.5) hands off to the narrower street (|z| ≤ 9.375) at
x = 12.5. At 3× zoom that corner is two clean straight runs meeting at 90°.

`mid_gate.png` whole-edge fits show RMS 15–17 px only because the fit spans
that corner; split at the corner, each run is straight.

**What costs the point.** The boundary is a razor decal cut: cobble at
V = 0.48 abuts earth at V = 0.49 with a one-pixel transition and no
intermediate state — no dirt drift over the stones, no gravel scatter, no
partial course. Worse, the rectangle **saws individual paving stones in half**
rather than running along a joint — plainly visible at the top-left of the
`mid_gate.png` notch and along the whole plaza west edge in `mid_plaza.png`.
Real paving ends at a stone, not through one.

**The 0.275 m under-facade claim is true.** Along the whole north row in
`mid_north_row.png` (wall/ground junction, zoomed 2×), the cobble meets the
wall base with no earth gap and no cobble emerging above it. At the row's east
end (~x 1290) the paving edge steps ~0.2 m further north as it leaves the
building's occlusion — it reads as the paving turning the corner, not as a
defect. **That claim is not the problem; the plaza rectangle is (§2 below).**

---

## 2. Extent — the rug read is real, and there is a leak

**Leak (blocking).** `mid_street.png`, lower right (crop x 880–1400,
y 700–900): a rectangular apron of cobble sits **outside** the casa row, past
the blank back wall, in open field. This is the plaza rectangle's |z| ≤ 12.5
extent running 3.3 m past the facade line at z = ±9.2 for all |x| ≤ 12.5 —
through the buildings and out the back. Confirmed by masking the frame on
B−R > 4: the red block clears the building silhouette entirely. Paving behind
the street wall, visible from the orbit camera, justified by nothing.

**Rug read.** `close_crucero.png` and `close_well_basin.png` are the honest
test, because they are at player eye height. From spawn the player sees the
paving stop at a hard straight line ~12 m out in every direction, with bare
meseta beyond and no wall, kerb, or prop line at that radius to explain it.
`mid_gate.png` is the worst frame: the notch corner sits in open ground beside
the gate with nothing aligned to it — an unmistakably synthetic geometric
event in a field.

**Where it works.** Inside the street corridor (`mid_north_row.png`,
`mid_street.png` centre) the boundary is fully occluded by the casa rows and
the read is excellent — this is where the change earns its keep.

So: the rectangles are the right *shape* for the street and the wrong
*termination* for the plaza. The plaza needs either an edge the world explains
(the casa rows, a kerb course, a scatter of loose stones and drift) or a
smaller radius that keeps the seam inside the built envelope.

---

## 3. Material read — passes §2's ceiling, fails §3's description

All figures are the rendered mean of the whole cobble surface (mean RGB →
HSV), not a per-pixel clamp, per §2's measurement clause.

| Frame | Range | Subject | H | S | V |
|---|---|---|---|---|---|
| `close_crucero.png` | **~2–3 m (walk-up)** | cobble, whole surface | 225.5° | **0.082** | **0.462** |
| `close_crucero.png` | ~2–3 m | stones (darkest 12 %) | 223.1° | 0.118 | 0.411 |
| `close_crucero.png` | ~2–3 m | joints (brightest 12 %) | 320.3° | 0.016 | **0.537** |
| `close_well_basin.png` | ~2–4 m | cobble, near-left | 246.3° | 0.038 | 0.491 |
| `mid_north_row.png` | ~8–12 m | cobble foreground | 223.5° | 0.101 | 0.476 |
| `mid_plaza.png` | ~15–25 m | cobble | 223.7° | 0.097 | 0.483 |
| `mid_gate.png` | ~15–25 m | cobble | 224.1° | 0.096 | 0.469 |

Reference surfaces in the same frames: cracked earth **12–14°, S 0.10,
V 0.48–0.49**; dressed limestone **18.3°, S 0.094, V 0.444**; encalado
**S 0.028, V 0.467**; terracotta **14.1°, S 0.281, V 0.360**.

**§2 ambient ceiling (S ≤ 0.35, V ≤ 0.6): PASS**, comfortably, at the binding
2.3 m walk-up range (S 0.082, V 0.462) and at every mid range. **Threat
reservation: PASS** — 223° is 200° away from the reserved window.

**§3 description: FAIL on three counts.**

1. **Stone scale.** Measured from `mid_north_row.png`: the texture repeat
   autocorrelates at lag 345 px in the near band, and `tile: 7.0` means that
   lag is 7.0 m. Joint-to-joint spacing in the same band (peaks > 50 px)
   medians **107 px → 2.17 m per stone**. The generation prompt in
   `content/textures/ground/worn_cobble/generation_manifest.json` asked for
   "fist-sized weathered stones"; the delivered albedo holds ~3 × 3 stones per
   2048² tile, and at tile 7.0 they render as **~2.2 m flagstones**. At
   walk-up range (`close_crucero.png`, mannequin for scale) the cobblestone
   identity is simply gone — those are cathedral floor slabs. This is not a
   tile-value bug alone: at a tile small enough to give 0.3 m setts (~1.0 m)
   the texture would repeat every metre. **The asset is wrong, not the number.**

2. **Joints inverted.** §3: "dark earth-brown packed joints". Measured joints
   are the **brightest and most neutral** thing in the surface — V 0.537 vs
   V 0.411 for the stones, S 0.016, no brown at all. Confirmed at source: the
   albedo's joints are pale cream. The material's own generation prompt asked
   for "soot-darkened deep joints" and got the opposite. Rendered, the paving
   reads as pale grout under dark slabs — the exact inverse of the premise.

3. **Hue family.** §3 says "mixed cool greys from pale to slate"; §2 says
   "warm stone bias 20°–50° **only when chromatic**". At S 0.10 the cobble is
   exactly as chromatic as the dressed limestone the doc itself calls warm
   (S 0.094 @ 18°) — so it is chromatic by the doc's own standard, and its hue
   is 223°. Root cause is measurable: the albedo is near-neutral
   (S 0.013 @ 240°), so under a cool overcast dome it takes the sky's colour
   straight, while cracked earth's warm albedo resists it. Against the earth at
   12° and the terracotta at 14° the result is a near-complementary contrast —
   the plaza reads blue, and that is why it looks applied rather than
   belonging. "Cool grey" was over-delivered into blue.

---

## 4. Tiling — a visible 7 m repeat, worse than what it replaced

Horizontal-band autocorrelation on the ground (row bands, per-row and
per-column mean removed, normalized to lag 0):

| Band (`mid_north_row.png`) | Baseline (cracked earth) | New (cobble) |
|---|---|---|
| y 560–600 | lag 280 px, **r = 0.400** | lag 280 px, **r = 0.585** |
| y 700–740 | lag 312 px, r = 0.373 | lag 311 px, r = 0.457 |
| y 850–890 | lag 347 px, r = 0.444 | lag 345 px, r = 0.485 |

Same lags in both, so the period is unchanged (`tile: 7.0` for both materials);
the **correlation strength rises by up to +0.19**. A 7 m repeat at r ≈ 0.5 is
visible: at 2.6× contrast the same dark-stone cluster recurs four times across
`mid_north_row.png`'s near band, and at native contrast it is detectable once
looked for. It is worse on cobble than on earth for the reason in §3.1 — only
~3 stones fit in a repeat, so the eye has a countable motif instead of noise.

`mid_plaza.png` measures only r ≈ 0.07 because that camera's image-x is not
aligned to a world axis and the period smears; it is not evidence of absence.

**UV continuity: PASS.** `client/vordar-client/src/ground.rs:112` writes
`[x / tile_g, z / tile_g]` from the same world x,z for base and region quads,
and both use tile 7.0 — phase is identical across the seam by construction.
Confirmed in pixels: no scale change or phase jump at any boundary in
`mid_plaza.png`, `mid_gate.png`, or `close_crucero.png`.

---

## 5. Did it improve the zone? Yes, structurally — with a chromatic cost

Baseline `target/zone-review/p30-chapel/mid_north_row.png` against the new
frame is the clearest comparison. Before: a row of houses standing on bare
cracked earth, reading as a film-set facade dropped in a desert with no ground
plane that belongs to them. After: the same row standing on a street. The
paving does the job §3 asked of it — it makes the space between the buildings
into a *place* rather than a gap.

The cost is real and should not be waved through: the new floor is the coldest
and most chromatically opposed surface in a frame whose roofs (14°), earth
(12°) and limestone (18°) were previously in one warm family. The before frame
was harmonized; the after frame has a blue floor under terracotta roofs.

Net: a win, held back entirely by the material, not by the region mechanism.

---

## 6. Regressions — none

Per-frame mean absolute RGB difference against `target/zone-review/p30-chapel/`
across all 28 shared named shots. Every frame whose ground lies outside the two
rectangles is **byte-identical** (0.00): `mid_chapel`, `mid_chapel_skyline`,
`mid_graveyard`, `close_chapel`, `close_chapel_arch`, `close_gravestone`,
`close_cypress`, `close_broken_column`, `close_rock_07/09/face_01`,
`close_candelabra_shrine`, `interior_apse`, `interior_door`.

Changed frames are exactly the ones standing on the new regions:
`mid_gate` 5.08, `close_crucero` 4.54, `mid_north_row` 4.54, `close_well_basin`
2.58, `close_gate_arch` 1.53, `close_wall_segment` 1.29, casa closes 0.20–0.43
(shadow catch on the new floor), `contact_sheet` 30.18 (composite).

Two observations, neither a regression from this change:

- **`wide.png` changed by 0.02.** The height fog buries the whole town at that
  framing — the paving contributes nothing at zone scale, and equally the "rug"
  problem is invisible from there. Pre-existing fog condition.
- **Shadow-map aliasing** on the casa shadows is now legible because it falls on
  a flat paved surface instead of noisy earth (`mid_north_row.png`, y 545–690,
  serrated shadow edges). Pre-existing, newly exposed.

The chapel precinct and the graveyard are unpaved, correctly — §3 scopes cobble
to "plaza and streets" and §4's "chapel path" is an approach, not a street.
Noting it only so a later pass does not read the omission as an oversight.

---

## 7. Fixes

**Blocking**

1. **Regenerate `worn_cobble` at the stone scale §3 actually describes.**
   Rendered stones measure 2.17 m; §3 says cobblestones and the asset's own
   prompt says fist-sized. Target ~0.25–0.4 m setts with enough stones per tile
   that the 7 m repeat stops being countable (§3.1, §4). This subsumes the tile
   value — do not fix it by lowering `tile`, which would only shrink the repeat
   period and make §4 worse.
2. **Invert the joints to §3's spec** — "dark earth-brown packed joints". They
   currently render as the brightest, most neutral part of the surface
   (V 0.537 vs stones V 0.411, S 0.016). Same regeneration as fix 1 (§3.2).
3. **Warm the albedo so it lands neutral-grey under the overcast dome.** The
   albedo is near-neutral (S 0.013 @ 240°) and therefore renders at 223°,
   S 0.10 — chromatic by §2's own standard and outside the 20°–50° warm window.
   Same regeneration as fixes 1 and 2 (§3.3).
4. **Stop the plaza rectangle punching through the casa rows.** |z| ≤ 12.5 runs
   3.3 m past the z = ±9.2 facade line and shows as an apron of paving in open
   field behind the row (`mid_street.png` lower right, §2). Clamp the plaza's z
   extent to the street's ±9.375 where the rows stand, or otherwise keep the
   union inside the built envelope.

**Non-blocking**

5. **Give the exposed plaza edge something the world explains.** The seam at
   r ≈ 12.5 is a hard rectangle at player eye height with no kerb, drift, or
   scatter, and it saws paving stones in half where it lands (§1, §2). Either a
   dressing pass (loose stones, dirt drift over the outer course) or a smaller
   plaza that keeps the seam behind the casa rows.
6. **The re-entrant notch at (12.5, 9.375) is legible in open ground beside the
   gate** (`mid_gate.png`). Geometrically correct, visually synthetic. Resolved
   for free if fix 4 aligns the two rectangles' z extent.
