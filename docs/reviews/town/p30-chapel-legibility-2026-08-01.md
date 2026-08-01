# P3.0 gate — chapel church legibility (Rocalba start town)

Date: 2026-08-01
Frames: `target/zone-review/p30-chapel/` (27 PNG, offscreen, in-engine, shipped start-zone lighting)
Spec under review: `tasks/town/p30-chapel-legibility.md`
Prior gate this pass answers: G2 kit re-gate (`docs/reviews/town/g2-kit-regate-2026-07-31.md`)

---

## Verdict: **FAIL**

**Blocker — the ragged crown is a battlement.** Ten `chapel_crown_*` blocks per
side all share a bottom face at local y = **6.58** (measured, every accessor
`min[1]` identical) across a 6.9 m run from x 1.05 to 7.97, with tops between
6.68 and 7.25. That is a dead-level 6.9 m line with a 0.57 m nibbled frieze on
top of it: the pass moved the level coped wall top down 0.92 m instead of
destroying it. In `mid_chapel.png` (wall-top run, right of the fracture) and in
`mid_graveyard.png` the run reads as evenly-stepped square merlons in bright
dressed limestone — a crenellated parapet. It pushes the building's read from
*ruined church* toward *fortified enclosure* at every angle except the head-on
facade, which is the one angle where the espadaña was already carrying the read
on its own. The conviction the pass was built to kill is still in the pixels.

This is not a scoring judgement. The espadaña, cross and vault-lip fracture are
real gains and are worth keeping; the crown run cancels them.

---

## Scores

### 1. Church legibility — **5/10**

**Carries it.** `mid_chapel_facade.png`: the east elevation now has a bellcote
gable with a round-headed tronera, a cross seated on the apex, and a
round-headed portal below. `chapel_espadana_body` occupies x [8.00, 8.60],
exactly the east wall's own x span — coplanar as specced, no applied-tower look
— and is 3.6 m wide (z ±1.80) on a 7.6 m wall, correctly slender. The cross
foot is at y 12.40, exactly `chapel_espadana_gable`'s max y: seated, not
floating (the apparent float in `mid_graveyard.png` is foreshortening, the ridge
passes behind it). From this frame the building reads as a village church.

**Does not carry it.** Everything else in the frame set:

- **The oculus is a blob.** `mid_chapel_facade.png` / `mid_graveyard.png`: a
  flat ellipse *brighter* than the wall, no reveal shadow, no surround, bisected
  by a masonry joint that runs straight through it. It is a 1.0 m bore in the
  0.6 m `chapel_east_head1` panel with no dressing of any kind, so at every
  framing you see lit nave floor through it and read a paint smudge. The portal
  got an 11-wedge ring; the oculus got nothing. That asymmetry is the defect.
- **The portal ring does not survive distance.** `chapel_portal_wedge0..10` all
  span x [8.00, 8.60] — the wall's exact thickness, perfectly flush. A 0.35 m
  radial band (intrados crown 4.40, extrados 4.73 measured) with no silhouette
  and no shadow line resolves only in `mid_chapel_facade.png`; in
  `mid_graveyard.png` it has already collapsed into a single flat lighter arc.
- **The saetera is invisible.** `chapel_side_1_A_head1/sill1` give a 0.5 × 1.6 m
  straight (un-splayed) slot at x [−4.25, −3.75], y [4.20, 5.80]. In
  `mid_chapel.png` the corresponding stretch of south wall shows only a slightly
  darker vertical joint. From inside (`interior_apse.png`) no opening is legible
  at all.
- **Range.** `wide.png` is a gray smudge — the fog erases the entire town, and
  with it the espadaña silhouette, which is the one cue this pass exists to
  deliver at range.

**Expected and not found:** any frame that silhouettes the espadaña against sky.
Every provided frame looks down on it with the vault, the interior, or the
ground behind. The pass's headline element has never been reviewed in the
condition it is designed for.

### 2. Collapse legibility — **4/10**

**Works, and should be kept.** `mid_chapel.png` south-wall crop: the vault lip
genuinely reads as a break. `chapel_lip_wedge0..17` terminate at different x —
haunch ribs cantilever visibly into the void, crown ribs die within a few
centimetres — and there is a real 0.9 m stepped fracture scar in the side wall
at the span A/B junction. That silhouette is the strongest single thing in the
pass. F5(b) landed.

**Defeats it.** The crown run (see blocker). And exterior rubble is still zero.
§F5(c) declined to add any and deferred the cause to R2; the two `rock_07` props
at world (−25.5, −24.1) and (−21.6, −34.3) are natural brown boulders and read
as boulders, not as fallen masonry — visible in `mid_graveyard.png`, where the
ground around the collapsed east half is otherwise clean earth. **This gate does
not accept the deferral as satisfying the conviction.** The interior rubble is
also weak: `chapel_rubble0..6` are seven axis-aligned boxes, 0.18–0.28 m tall,
0.22–0.50 m on plan, all with `min[1] = 0.00` (sitting exactly on the floor,
none bedded, none rotated), all clustered in x [0.28, 2.41] while the collapse
runs to x 7.60. In `interior_door.png` they read as sugar cubes scattered on a
swept floor. The vault's 18 wedges on r_out 3.5417 give voussoirs with roughly
1.24 m arc faces; nothing on the floor is in that size family.

Net: a ruin with a battlement and a swept floor. The break is convincing; the
aftermath is not.

### 3. Cohesion — **8/10**

The strongest axis, and the G2 failure did not recur. Every new chapel element
binds `limestone_dressed` (checked across `chapel_espadana_body`,
`chapel_espadana_gable`, `chapel_portal_wedge*`, `chapel_crown_*`,
`chapel_east_head1`) and inherits the same `matlib.project_uv` at
TEXEL_SCALE_M = 13.1072 — no hero atlas anywhere on the building. Comparing
`close_chapel.png` (wall at ~2 m, with mannequin) against `mid_chapel.png`, the
block scale and texel density are continuous between old nave wall and new
espadaña. The warm/cool banding on the east facade in `mid_chapel_facade.png` is
the triplanar projection sampling different regions of the *same* map, not a
second material.

Marked down two points for the bell: `chapel_bell` + `chapel_bell_yoke` are the
only `iron_wrought` on the building and they render as flat pure black with no
shading gradient at any distance (`mid_graveyard.png`, `mid_chapel_facade.png`).
A 0.52 m cube-bounded black shape hanging under a black slab does not read as a
bell — no flare, no crown, no mouth — and it is the one element that visibly
does not belong to the same material system as the stone.

### 4. The two riders — **2/10**

**Quoins: not fixed, and the mechanism is in the spec's own measurements.**
`close_casa_small_a.png` and `close_casa_two_story.png` show quoin blocks
projecting from the render with **black air gaps above and below** successive
courses — the stack does not bond, each block reads as glued on. §0 of the spec
measured why: `_quoins` draws per-course half-widths of 0.21–0.25 m on the short
face against a wall half-thickness of **0.225 m**. Every course whose half-width
falls below 0.225 is swallowed by the render and vanishes, leaving a gap between
the courses that survive. The spec named the jag ("1.5 cm recessed to 14.6 cm
proud") and shipped it unchanged.

**Ridge caps: geometry present, read absent.** 26 `casa_corner_main_roof_ridge_*`
meshes exist. All 26 carry the **identical** UV rectangle U [0.4832, 0.5168]
V [0.4918, 0.5082] — a single 0.44 × 0.21 m patch of albedo stamped 26 times.
In `mid_north_row.png` the ridge is a uniform dark line, tonally identical to
the mirror band below it, and does not read as a course of cover tiles.

**Roof mirror band: not fixed, and §3.2 diagnosed the wrong asset.** The shipped
decks are no longer `_roof_deck_panel` output. All 25 `casa_corner_main_roof_tile_a_*`
share U [0.3721, 0.6268] V [0.3721, 0.6237]; all 25 `tile_b_*` share
U [0.3732, 0.6279] V [0.3763, 0.6279]. The whole slope is **one texture patch
stamped 25 times**, so every dark region inside that patch recurs as a band down
the fall line — `mid_north_row.png` at 3× shows four such bands per slope, with
the tile-cap shapes flipping direction at each boundary. The patch is also
symmetric about UV 0.5, which is the mirror signature the original 0.763
correlation measured. §3.2's origin-corner + `vbias` fix was written against a
per-panel builder the shipped tiles never pass through, so it cannot have
applied. The defect is not merely unfixed; it now repeats, which is worse than
the single mid-slope band that was convicted.

### 5. New defects — **3/10**

1. Crown-block battlement (the blocker), `mid_chapel.png`, `mid_graveyard.png`.
2. Oculus as an undressed bore reading as a bright decal, `mid_chapel_facade.png`.
3. Bell as an unlit black cube, `mid_graveyard.png`.
4. Coplanar hatch/moire on the span-B wall top — a regular diagonal cross-hatch
   over the deck surface right of the crown run in `mid_chapel.png`, the
   signature of two coplanar faces fighting. Visible at gameplay distance.
5. `chapel_bell_yoke` spans z ±0.86 against a tronera bore of r 0.80, so 0.06 m
   of yoke buries into each jamb. Defensible as bearing, but at the rendered
   angle it reads as a black bar laid across and hiding the arch head — the one
   cue that makes a tronera a tronera.
6. Rubble boxes unrotated and floor-flush (see §2).

Boolean cut edges elsewhere are clean: the tronera arch head, the portal bore
and the saetera reveals show no cracks, no stray verts, no interpenetration in
any close frame. The espadaña/wall junction is sound — coplanar x spans, no
seam, no z-fight. That is why this is a 3 and not a 1.

---

## Required fixes

Numbers below are derived from shipped accessor extents in
`content/models/props/{chapel,casa_corner}/*.gltf`. Where a number is a *read*
decision rather than a clearance, it is named as such and left to be computed —
per `tasks/lessons/2026-07-31-nominal-dimensions-are-not-placed-extents.md`.

**1. Destroy the level crown line (blocker).**
Frame: `mid_chapel.png`, `mid_graveyard.png`. Evidence: all ten
`chapel_crown_{±1}_*` have `min[1] = 6.58`.
The blocks are not the bug — the *datum* is. The ragged edge has to belong to
`chapel_side_±1_B_wall0` itself, whose top is currently a single level box face
at 6.6, with the blocks as its residue rather than a frieze applied to it.
The profile must cross the old datum in both directions so no horizontal line
survives anywhere in the run. Hard ceiling, derived: the intact crown at
y = 7.50 (`chapel_side_±1_A_wall0` max[1]); the collapsed crown must stay below
it or the contrast inverts. There is **no** measured floor constraint — span B
carries no openings (the saeteras are in span A, `chapel_side_±1_A_head1`), so
how deep the low points go is a read decision and must be judged from a render,
not chosen from a number. Also drop `bevel=0.0` on whatever blocks survive: the
square arris is half of why they read as merlons.

**2. Re-derive the roof deck UVs against the per-tile builder.**
Frame: `mid_north_row.png`. Evidence: 25 identical UV rects per slope, given
above. §3.2's fix targets a code path the shipped tiles do not take; find the
builder that emits `*_roof_tile_*` and fix it there. Acceptance, both required:
no two tiles on one slope share a UV rectangle, and the deck's total V span
stays strictly inside a single tile (< 1.0) so no wrap seam is reintroduced.
Do not re-run the correlation metric on the same 30 m frame that scored 0.763 —
the metric and the fix must be independent.

**3. Same treatment for the ridge caps.**
Evidence: 26 identical rects U [0.4832, 0.5168] V [0.4918, 0.5082]. Each cap
must sample its own advance along the ridge.

**4. Raise the quoin short-face minimum above the wall half-thickness.**
Frames: `close_casa_small_a.png`, `close_casa_two_story.png`. Evidence: spec §0,
short-face per-course half-width 0.21–0.25 m against wall half-thickness
0.225 m. Every course below 0.225 disappears into the render and opens a gap.
The hard floor is **strictly greater than 0.225 m** plus whatever surface relief
the `encalado` render carries, so no course is ever swallowed. How much greater
— i.e. how proud the chain sits — is the read decision and must be judged from a
close frame, not picked. The long face (half-widths 0.32–0.37 vs 0.225) already
clears and needs no change.

**5. Dress the oculus, and do it with a reveal, not just a ring.**
Frame: `mid_chapel_facade.png`. A flush ring would repeat the portal's failure
(fix 6). The bore needs a splay or chamfer that catches shadow on the outer
edge. If a dressed ring is also wanted, the envelope is available and checks
out: matching the portal's 0.35 m radial band on a r = 0.50 bore centred at
z = 0, y = 5.90 gives outer r = 0.85 → z ±0.85 inside `chapel_east_head1`'s
z ±1.20 (0.35 m clear each side) and y [5.05, 6.75] inside its y [4.13, 7.50]
(0.75 m clear above and below), with 0.32 m to the portal extrados crown at
y = 4.73. Ring width itself remains a read decision.

**6. Give the portal ring a shadow line without growing the AABB.**
Evidence: `chapel_portal_wedge*` x [8.00, 8.60] equals the east wall's x span
exactly. Pushing the ring proud is **not** free — the chapel's glTF x span is
[−11.629, 8.600] = 20.229, which `footprints.ron` records verbatim as
`size: (20.23, 8.2)`, so any eastward growth breaks the footprint match and the
D5 lint. The clean move is the inverse: set the surrounding wall face
(`chapel_east_wall0`, `_wall2`, `_head1`) back within the existing 0.60 m
thickness so the ring stands proud into recovered depth. Collision is
unaffected — `chapel_door_jamb` occupies the full local 8.0–8.6 and is authored
separately. Recess depth is a read decision.

**7. Rubble that reads as fallen vault.**
Frames: `interior_door.png`, `mid_graveyard.png`. Three separate faults, all
measured: the seven boxes are axis-aligned (no rotation), all sit at
`min[1] = 0.00` (unbedded), and all fall in x [0.28, 2.41] while the collapse
runs x 1.0 → 7.60. Size family is also wrong — the vault's 18 wedges on
r_out 3.5417 give voussoir arc faces near 1.24 m; nothing on the floor is within
an order of magnitude. Rotate, bed, spread across the full collapsed span, and
include pieces in the voussoir size family.

**8. Exterior rubble.**
Frame: `mid_graveyard.png` — clean earth around the collapsed half. §F5(c)
deferred this to R2 on the strength of two `rock_07` boulder props, which read
as boulders. Either R2 lands before G4 or the kit supplies it; the deferral
alone does not clear the original conviction and this gate will re-check it.

**9. Bell profile and material.**
Frame: `mid_graveyard.png`, `mid_chapel_facade.png`. A 0.52 m cube-bounded
`iron_wrought` shape rendering as flat pure black. Needs a bell profile (flare,
crown) and a response that catches sky light, and the yoke must stop hiding the
tronera's arch head.

**10. Resolve the coplanar hatch on the span-B wall top.**
Frame: `mid_chapel.png`, right of the crown run — regular diagonal cross-hatch
over the deck. Find the coplanar pair and separate them.

---

## Watch items for the G4 ship gate

- **Missing framing.** No frame in this set silhouettes the espadaña against
  sky. G4 must include an eye-height frame from the east approach, at the
  distance the player first sees Rocalba. The pass's headline element is
  currently ungated in its design condition.
- **Fog vs. range legibility.** `wide.png` reduces the whole town to a gray
  smudge. Whatever that camera's distance is, church legibility there is zero.
  Check the aerial-perspective curve against the actual first-sight framing —
  this may be a lighting finding, not a kit one.
- **Hero/kit stone mismatch, pre-existing.** `close_chapel_arch.png` and
  `close_crucero.png`: the ruined gate arch and the crucero are a visibly
  softer, waxier, lower-frequency stone than the kit ashlar in
  `close_chapel.png`. Out of P3.0 scope — these are not this pass's assets — but
  it is the G2 failure class and will be judged at G4.
- **Chapel interior is a bare box.** `interior_apse.png` shows no apse feature,
  no altar, no niche; two candelabra stand on open floor. Out of scope for a
  legibility pass, in scope for a ship gate on the town's one interior.
- **Saetera splay.** Straight 0.5 × 1.6 m slots through 0.6 m of wall read as
  joints from outside and as nothing from inside. If they are meant to carry
  any of the church read, they need a splayed reveal; if they are not, say so
  and stop paying for them.

---

# RE-GATE — round two (fresh judge)

Date: 2026-08-01
Frames: `target/zone-review/p30-chapel/` (28 PNG, re-rendered offscreen,
in-engine, shipped start-zone lighting)
Fix round under review: `tasks/town/p30-fix-round.md`
The round-one verdict above is untouched — it is the record of what was found.

## Verdict: **PASS with fixes**

The blocker is dead. Across `mid_chapel.png`, `mid_graveyard.png` and the close
crops of both wall runs, no horizontal line survives on the collapsed crown: the
blocks descend as broken courses with cantilevered slabs and daylight under
their soffits, and their bottom faces sit at different heights along the run. At
every angle in the set the building now reads as a ruined village church, not a
fortified enclosure. All four graded axes improved, three of them sharply. What
remains is polish plus one item that was not attempted.

## Scores

### 1. Church legibility — **8/10** (was 5, +3)

`mid_chapel_skyline.png` closes round one's missing-framing item and carries the
verdict on its own: at standing height on the east approach, the espadaña is
silhouetted against sky with a legible tronera, a bell hanging in it, and the
cross seated on the gable apex. Nothing else in the frame competes; the read is
unambiguous at first sight.

- **Oculus fixed.** `mid_chapel_facade.png` at 4× (facade crop): the bore now
  carries a proud dressed ring that throws a shadow on its upper-left arc and
  reads as a round window at 30 m. The round-one "bright blob bisected by a
  joint" is gone.
- **Portal ring survives distance.** Same crop: the ring stands proud of the
  recessed field and casts a continuous shadow line down both haunches. Fix 6's
  recess — setting the wall back rather than pushing the ring out — was the
  right call and it works at gameplay framing, not only in close-up.
- **Saetera** now reads as a genuine recessed slot at 2 m (`close_chapel.png`,
  upper left), but is still near-invisible at mid range. Unchanged as a G4
  question: splay it or stop paying for it.
- **Range** is unchanged: `wide.png` is still a gray smudge. Carried forward as
  a lighting watch item, not a kit one.

### 2. Collapse legibility — **7/10** (was 4, +3)

**The battlement is gone.** `mid_chapel.png` crop of the north-wall run at 6×:
five to six blocks step down toward the fracture, each a thin slab cantilevering
over shadow, bottoms staggered. `mid_chapel.png` crop of the south-wall top at
4×: the same run seen from the other side shows blocks of unequal height with
some rising above their neighbours, no shared datum, no square merlon rhythm.
`mid_graveyard.png` crop at 5× confirms it from the third angle, alongside the
vault-lip voussoirs, which still read as the strongest single element in the
pass.

**Interior rubble landed.** `interior_door.png`: pieces are rotated, tilted,
bedded into the paving, and vary from chips to wedge-shaped slabs in the
voussoir family. It reads as fallen masonry, not sugar cubes on a swept floor.
It is also visible through the portal in `mid_chapel_skyline.png`, which is
where a player first meets it.

**Held back by two things.** Seen from above (`mid_graveyard.png` floor crop at
3×) the *collapsed* bay's floor is the sparsest part of the spread — the pieces
cluster where the roof survives, which is backwards. And exterior rubble is
still zero (fix 8): the ground outside the collapsed east half is clean earth
except two boulder props and the arch's own debris. That no longer cancels the
read the way it did at round one — the broken crown and the interior scatter now
carry the aftermath — but it is the reason this is a 7 and not a 9.

### 3. Cohesion — **8/10** (unchanged)

The world-anchored projection is a real gain here, not only on the riders: every
congruent element now carries its own patch, so block-to-block texture no longer
repeats anywhere on the building, and texel density is continuous from the old
nave wall to the espadaña, the rings and the crown (`close_chapel.png` against
`mid_chapel_facade.png`).

Two deductions, both the same ones as round one and both improved rather than
cleared:

- **Bell.** `iron_wrought` still renders near-black, but the new flared profile
  makes the silhouette do the work (`mid_chapel_skyline.png`, and the bellcote
  crop of `mid_chapel_facade.png`), and the yoke no longer hides the tronera
  arch head. It reads as a bell now. It does not block.
- **Interior plaster liner.** `interior_apse.png` and, from outside through the
  collapse, `mid_chapel.png` / `mid_graveyard.png`: `plaster_smoked` above the
  ashlar dado is a near-featureless green-grey panel at a detail frequency an
  order below the stone beside it, and it occupies most of what you see through
  the breach at mid range. Verified pre-existing (`plaster_smoked` is already in
  `HEAD:scripts/asset-pipeline/townkit/buildings.py`), so it is not charged to
  this round — but it is now the largest low-information surface in the frame.

### 4. The riders — **8/10** (was 2, +6)

- **Roof band: fixed, and measured independently of round one's metric.**
  `mid_north_row.png`, right slope, region x[860,1300] y[250,430]: normalized
  autocorrelation of the column-mean luminance peaks only at ~24.4 px and its
  harmonics (0.52, 0.43, 0.45, 0.41) — the tile pitch itself. The row-mean
  autocorrelation down the fall line has **no** peak above 0.35 at any lag. The
  four mirrored bands per slope are gone; at 3× the slope reads as individually
  weathered pantiles.
- **Ridge caps: fixed.** Ridge crop of `mid_north_row.png` at 3×: caps vary
  cap-to-cap in tone and wear and read as a course of cover tiles, no longer one
  uniform dark line.
- **Quoins: fixed.** `close_casa_small_a.png`, `close_casa_two_story.png`: the
  chain bonds, every course reaches the render, and the black air gaps above and
  below courses are gone. Held off 10 by the close-up material read — the quoin
  stone is waxier and browner than the ashlar it should match, and thin dark
  slivers sit at some short-face returns (`close_casa_small_a.png`, right-hand
  return of the second and fourth courses).

### 5. New defects — **7/10** (was 3, +4)

Cleared from round one: the crown battlement, the undressed oculus, the black
cube bell, the yoke across the arch head, the unbedded rubble, and the coplanar
hatch (the span-B wall top in `mid_chapel.png` at 4× is clean paving with no
moire — fix 10's diagnosis of the floor sitting exactly on `GROUND_TOP_Y` was
correct).

Introduced or still open:

1. The crown's descent is close to a uniform stair — `mid_chapel.png` at 6×
   shows five successive steps of near-equal rise and run. Far weaker than the
   battlement it replaced and invisible at gameplay framing, but it is a
   regularity.
2. The oculus bore reads as filled, not open (`mid_chapel_skyline.png` at 3×):
   inside the ring sits a light disc crossed by two bright straight bars. The
   ring works; what shows through it does not.
3. Vertical striping on limestone — see the adjudication below.

## Adjudications requested

**Divergence on fix 4 (quoins) — accepted on merit.** The record prescribed a
half-width floor above the wall half-thickness; the fix instead bonded an
inter-course air gap. The pixels back the fix: `close_casa_small_a.png` and
`close_casa_two_story.png` show a continuous bonded chain with no swallowed
course and no gap, which is the outcome the record actually demanded. Round one
reasoned from the spec's pre-fix code — exactly the failure named in
`tasks/lessons/2026-07-31-nominal-dimensions-are-not-placed-extents.md`.

**Divergence on fix 6 (cylindrical UV on `barrel_shell` wedges) — accepted on
merit.** `mid_graveyard.png` at 5× shows the vault-lip voussoirs reading as
individual radial blocks whose joints follow the ring; the portal and oculus
rings in `mid_chapel_skyline.png` read as rings rather than as wall courses cut
to an arch shape. A world box projection is genuinely blind to the depth
direction here and the divergence is justified.

**Fix 8 (exterior rubble) not done — does not block, but is not closed.**
`mid_graveyard.png`: the ground outside the collapsed half is clean. With the
broken crown and the interior scatter now landed, the aftermath reads well
enough for this gate. It carries to G4 as an R2 (layout) requirement. Note for
that owner: "a collision box is wrong and axis-aligned yaw kills the scatter" is
a description of a content-lint rule that does not fit ankle-height debris — the
rule is the thing to fix, not the placement to contort around it.

**Bell / `iron_wrought` — does not block; the material revisit is warranted.**
The silhouette carries the read at both framings that matter
(`mid_chapel_skyline.png`, `mid_chapel_facade.png`). But a metallic-1.0
near-black albedo giving f0 ≈ 0.03 with IBL specular as its only response is a
material that can never catch a highlight, and it is shared with every reja and
the crucero. Raise it at G4 as a campaign-level materials finding, out of P3.0.

**Striping on limestone — confirmed in the pixels; the attribution is not.**
`close_chapel.png` at 3×: these are hard 1–2 px dark seams, perfectly straight,
running the full wall height, crossing block joints and relief without following
them. "Faint" undersells it — at ~2 m they are the most legible thing on the
wall. The same artifact appears on `close_wall_segment.png`, and the affected
set matches exactly the materials carrying `vordar_detail` (`limestone_dressed`
in `chapel.gltf`, `wall_segment.gltf`, and the casas' quoins), which is
consistent with the overlay attribution — and the shipped tile is measurably not
seamless (`content/textures/detail/limestone/diff_2048.png`: mean |Δ| across the
wrap edge 4.50 vs 3.47 between interior neighbours; the normal map 9.34 vs
6.92). What does *not* follow from the report is the conclusion that this is
independent of the round. A tile-wrap artifact should seam in both axes at the
same period, and on a flat wall patch of `close_chapel.png` I measure 2.16× more
line energy vertically than horizontally — which the grazing camera can explain,
but so can a UV change made this round. Attribution is unsettled and must not be
recorded as settled. Fix 1 below names the probe.

## Required fixes

Numbers are not prescribed where I cannot derive them from the shipped geometry
or from a render; the intent is stated instead.

**1. Settle the striping's owner before G4.** Frames: `close_chapel.png`,
`close_wall_segment.png`. Cheapest decisive probe: re-render the same wall at
the same distance **head-on**, and separately with `DETAIL_ALBEDO_STRENGTH` and
`DETAIL_NORMAL_STRENGTH` at 0. If the lines survive the second render they are
the kit's UVs and belong to this campaign; if they vanish, the tile wrap owns it
and the fix is a seamless detail tile, not a chapel change. Do not close this
from reasoning.

**2. Open the oculus.** Frame: `mid_chapel_skyline.png`. The ring is right; the
bore should read as a void — dark, with the reveal catching light — rather than
as a filled disc with bright bars across it. Whether that means occluding what
shows through, deepening the reveal, or both is a read decision to be judged
from a re-render at the same framing.

**3. Move the interior rubble's weight into the collapsed bay.** Frame:
`mid_graveyard.png` floor crop. The scatter is densest where the vault still
stands and thinnest where it fell. The mass belongs under the breach; how far to
shift it must be judged from the same overhead framing, not chosen.

**4. Break the crown's stair rhythm.** Frame: `mid_chapel.png` at 6×. Five
near-equal steps in a row. One or two blocks rising against the descent, or one
course that shears rather than steps, is enough — judge from a re-render, do not
pick a height.

## Watch items for the G4 ship gate

- **Exterior rubble (R2).** Carried from round one's fix 8, still open. Fix the
  content-lint rule that makes ankle-height debris impossible to place.
- **`iron_wrought` campaign revisit.** Bell, rejas, crucero — metallic 1.0 on a
  near-black albedo cannot catch light.
- **Interior plaster liner.** Pre-existing, but it is the largest low-detail
  surface visible through the breach at mid range. Round one's "bare box" watch
  item should widen from furniture to the wall finish itself.
- **Fog vs range legibility.** `wide.png` unchanged — the town is still erased.
  Lighting owns this, not the kit.
- **Hero/kit stone mismatch.** `close_chapel_arch.png`, `close_crucero.png`:
  still visibly softer, waxier and lower-frequency than the kit ashlar in
  `close_chapel.png`. Unchanged from round one, and the quoin stone in
  `close_casa_two_story.png` now sits in the same suspect family.
- **Saetera splay.** Reads as a slot at 2 m, as nothing at mid range. Decide
  whether it carries any of the church read.

---

# FOLLOW-UP ADJUDICATION — the four re-gate fixes

Date: 2026-08-01
Frames: `target/zone-review/p30-chapel/` (28 PNG, re-rendered), `target/stripe-probe/`.
Scope: the four non-blocking fixes only. The re-gate PASS is not reopened.
Every number below is re-derived from the shipped `content/models/props/chapel/chapel.gltf`
accessors and from `client/vordar-client/src/bin/zone_review.rs`'s own camera
constants, not from the fix round's report.

## Verdict: **ACCEPTED WITH RESIDUALS** — PASS stands

All four fixes land in the pixels. One residual, non-blocking, carried to G4.

**1. Striping — accepted, and the overturned attribution is correct.**
`shipped_grazing45_detail_off.png` carries ~10 hard, perfectly straight,
full-height lines across the wall; `final_grazing45_detail_off.png` and
`final_grazing45_detail_on.png` carry none, and the on/off pair is
indistinguishable, so the detail overlay never owned it. `parent_grazing45_*`
has none either. Head-on: `shipped_headonB_detail_on.png` shows six such lines
over two courses, `final_headonB_detail_on.png` shows zero — better than the
claimed 8 → 4. `close_chapel.png` is clean at 2 m. `final_headon90_*` being
byte-identical to `shipped_headon90_*` is a valid negative control (that camera
is on span A, which did not move), not a dead probe. Round one's code-read
attribution to the triplanar overlay is refuted by these frames; the fix-round
record is the one to believe.

**2. Oculus — divergence accepted; it reads as a round window.**
At 8× on `mid_chapel_skyline.png` and `mid_chapel_facade.png` the bore is a dark
disc well below the wall's value, the dressed ring stands proud, and the reveal
throws a lit crescent on the lower-left inner face. That is a void with a
surround, not a filled disc: round two's "light disc crossed by two bright bars"
is gone, and round one's "brighter than the wall" blob doubly so. The record
left the means open ("occluding what shows through, deepening the reveal, or
both"), so the `oak_dark` shutter is inside its latitude, not a divergence
needing merit. The reverted 1.15 m tube is correctly abandoned — the skyline
camera's ~8° off-bore angle is a real constraint, not an excuse. Only residue:
at 8× the shutter is a uniform plane with one faint plank seam. Invisible past
~15 m; not carried.

**3. Rubble — divergence accepted, and the original diagnosis WAS a camera
artifact.** Stated explicitly, as asked, and confirmed independently:
- The vault occupies local x ∈ [−8.000, 0.000] (`chapel_vault_wedge0..17`,
  every wedge). All 23 pieces lie in x ∈ [1.176, 6.981]. Not one is under the
  roof, before or after. "Clusters where the roof survives" cannot be true of
  this model.
- The `graveyard` shot is `radius 30, pitch 0.8 rad, aim_y 1.6` — eye 23.1 m up,
  20.9 m out. The east wall's inner face (local x 7.60, top y 7.50) shadows the
  bay floor from local x 2.2 to 7.6 at that geometry, so the frame shows only a
  2 m sliver of bay; the rest of the floor in it is the nave and apse seen down
  the vault tunnel. The two candelabra in that floor crop are the same pair
  `interior_apse.png` places at the apse — decisive that the surface round two
  graded was the roofed half.
- The real defect reproduces. Bedding now measures the rotated low point: pieces
  sink 0.057–0.173 m below the paving top (y = 0.050) and stand 0.075–0.617 m
  proud, nine of them ≥ 0.40 m. Mean piece centre x = 4.19 against a bay midpoint
  of 4.18 — the triangular weighting is on the bay, not on an edge.
- `interior_door.png` is the frame that judges it, and it carries: tilted slabs
  in the voussoir size family bedded into the paving, chips between them, mass
  where the vault came down. `crown_A.png` and `mid_chapel.png`'s bay-floor
  sliver agree from above.

**4. Crown rhythm — accepted; no stair survives the read.**
Measured tops, west→east. Side +1: 7.101, 7.169, 5.148, 4.630, 4.585, 5.024,
5.525, 6.158, 6.226 — a −2.021 m shear, then a five-block rise of 0.439 / 0.501
/ 0.633 / 0.068 over runs 0.49–1.17 m. Side −1: 5.979, 5.630, 6.606, 6.503,
6.219, 4.714, 5.956, 5.958, 6.179 — a +0.976 counter-rise and a −1.505 m notch.
The record's figures reproduce exactly. In `mid_chapel.png` at gameplay framing
and `crown_A.png` close, the run reads as a wall torn open: one dominant collapse
notch, cantilevered leaves over shadow, unequal block heights. The residual
monotone run is not perceptible as a stair at any framing in the set, because
its rises span 9× and the shear next to it is 3× the largest of them.

## Residuals — non-blocking, carried to G4

**1. The same striping signature still ships on `wall_segment`.**
`close_wall_segment.png`, corner pier: a 3-px band at image x 260–262 that is
+30 luma over the wall face right of it (101) and +34 over the 9-px band left of
it (96), while matching the return face's own value (130) — present on 92 % of
the wall's height, crossing every course and the coping. Four bands where a
chamfered arris has three; this is the coplanar-sliver read fix 1 convicted on
the chapel, not a bevel highlight. It is **not** this campaign's regression:
`wall_segment`'s POSITION accessors are byte-identical across `3feb4a7`
(measured), so the geometry predates the commit and only its UVs moved. But the
re-gate's fix 1 named `close_wall_segment.png` as one of its two frames, and the
artifact is still in that frame, so the striping item is not fully closed. Owner
is the kit's box builder, not the chapel.

## The two confirmations

**Nothing regressed elsewhere.** `git status` shows only `chapel.bin`,
`chapel.gltf`, `chapel.textures/manifest.json` and `townkit/buildings.py`
modified — no other model's bytes moved. The chapel's shipped global AABB is
x [−11.6287, 8.6000], z [±4.1000] → spans 20.2287 × 8.2000, matching
`content/chapters/chapter03/footprints.ron`'s `size: (20.23, 8.2)`. Measured
from the accessors, not carried from the record.

**The skipped close-ups.** The reasoning is sound *for regression*: with the
renderer, lighting and every other model unchanged, a close-up that does not
contain the chapel cannot differ. Spot-checked `close_casa_two_story.png`
(quoin chain still bonded, no course voids) and `interior_apse.png` — both as
the re-gate left them. The caveat is that byte-identity licenses skipping a
regression check, not a fresh look: the one non-chapel frame fix 1's own brief
named, `close_wall_segment.png`, still carries the artifact it was named for
(residual 1), and that was reachable only by opening it.
