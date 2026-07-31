# P2.4 Rocalba layout review — Phase 2 gate judgment

Date: 2026-07-31. Campaign Phase 2, P2.4. Opus visual judgment of the **layout**
(placement, composition, readability, premise fidelity) — the kit's materials and
geometry were gated separately at G2 (`docs/reviews/town/g2-kit-regate-2026-07-31.md`)
and are not re-litigated here.

Evidence: `target/town-layout/p24/` — `wide.png`, `mid_00`–`mid_02`,
`interior_apse.png`, `interior_door.png`, 18 `close_*.png`, `contact_sheet.png`.
Layout of record: `content/zones/zones.ron` (start zone) against
`docs/town-premise.md` and `tasks/town/p24-layout.md` §4.
Geometric claims below were re-derived from the installed glTF AABBs under each
placement's pos/scale/yaw (scratch script, not a re-render).

## Verdict

# PASS with fixes — 3 required, 4 minor

The skeleton is right and the row work is genuinely good: facade lines land exactly,
party walls read as a terrace, both corner houses anchor their row ends with the wing
turning outward, and the gate/wall/breach frames the east approach as designed. Three
things must change before this is the shipping town: a ruin prop stands **inside** the
chapel nave, the cypresses are 1.8 m shrubs so the premise's silhouette motif does not
exist, and the plaza's only candle prop is still a leftover feel-check placement.

None of it is structural rework — every required fix is a coordinate or a scale in
`zones.ron`, plus one stale clearance claim in the subplan.

| # | Criterion | Score |
|---|---|---|
| 1 | Premise fidelity | **7** / 10 |
| 2 | Composition / massing | **8** / 10 |
| 3 | Readability at gameplay framing | **6** / 10 |
| 4 | Interiors | **5** / 10 |
| 5 | Defect sweep | **6** / 10 |

---

## 1. Premise fidelity — 7/10

Every structural beat of `docs/town-premise.md` §4 is on the ground and in the right
place:

- **Plaza with well** — `mid_02.png`: the well stands in open ground in front of a
  continuous casa frontage; `close_well_basin.png` shows the drum, posts and headbeam
  reading as a town well at human scale.
- **Two facing casa rows** — `wide.png` (fog-corrected crop): two terraces of four
  attached houses each, facades turned inward. Facade lines are exact: every north-row
  AABB starts at z = +9.1 and every south-row AABB ends at z = −9.1, an 18.2 m street.
- **Chapel precinct SW with graveyard** — chapel at (−30, −29) with the nave running
  E–W, three gravestones 2.4–3.1 m off its north wall, `chapel_arch` + `broken_column`
  as the older ruin quarter to the south. Decay does increase outward as §1 asks.
- **East gate in the wall** — `wide.png` crop at the gate: the arch (5.75 m) astride
  the road with two fragment runs (2.6 m) N and S, and the deliberate 3.7 m breach at
  z ∈ [3.25, 6.95] with the crucero standing in it.

What keeps this off 9–10 is that the premise's *atmosphere* beats are absent, and two
of them are placement decisions this task owned:

- **No candle-gold anywhere in the zone.** §1 calls dense candle emissive "the town's
  signature and its wrongness". Not one frame shows a lit anything. Mostly a
  content/material item for a later phase — but the single candle prop that exists
  (`candelabra_shrine`) is parked at (−4, −5) in the middle of the plaza, and
  `zones.ron:78` still labels it "First Z-Image prop, near spawn for the feel-check".
  In `mid_02.png` it reads as an object dropped on bare dirt, not a shrine (fix F3).
- **The cypress silhouettes do not exist.** The model is 1.8 m tall at scale 1.0, so
  the four field cypresses and the graveyard cypress are shrubs — `mid_00.png` and
  `mid_01.png` are entire frames of two dark bushes on an empty plain (fix F2).
- Carried, not layout's to fix: plaza and streets are `cracked_earth` ground where §3
  specifies worn cobble; the well has no rope down the shaft (§5); the chapel interior
  carries none of §6's retablo / votive stands / benches.

## 2. Composition / massing — 8/10

- **Row rhythm is the strongest thing in the layout.** `wide.png` crops of both rows:
  four volumes per row with three ridge heights (5.57 / 5.76 / 8.47 m tops), depths
  staggered 8.2–10.75 m so the backs break the line while the fronts stay flush.
- **Party walls read as party walls.** `mid_02.png` (right third, 2× crop): two houses
  meet on a shared quoin chain, the lower roof dies into the taller flank, no gap and
  no z-fight. Measured overlap is 0.72–0.74 m per junction — the intended 0.6 m wall
  plus eaves.
- **Corner houses anchor both row ends and mirror each other**: north corner at the
  east end, wing outward to x = 14.06; south corner at the west end, wing outward to
  x = −19.46. The rows stagger ~4 m at each end, which reads as accretion rather than
  a stamped street.
- **The gate frames the approach.** Arch + two fragments per side, the north side left
  open for the shrine breach. The gate is visibly the tallest thing on the wall line.

Deductions:

- **The plaza is a street, not a plaza mayor.** Nothing closes its west end and nothing
  terminates the east end but 5 m of open ground before the gate line; premise §4 draws
  a square of r ≈ 12. Worth a second-street or a perpendicular casa in a later pass —
  flagged, not required now.
- **The chapel cannot carry distance dominance from the plaza.** It is 43 m from the
  plaza centre on an axis the south row completely occludes, under fog density 0.0055.
  `wide.png` at the ~130 m turntable fit shows the whole town near-erased — authored
  zone fog, not a defect, but it means the SW precinct is a destination you walk to,
  never a landmark you see.
- The north row's back is ~30 m of unbroken blank whitewash on ground the player can
  walk (play radius 65). Sealed shells are correct per G2; the blankness is a cost.
- A 0.54 m impassable slot survives between the north corner house's wing (x = 14.06)
  and the wall fragment at x = 14.6. Reads as the wall abutting the house; noted only.

## 3. Readability at gameplay framing — 6/10

What the evidence shows reads correctly:

- **Doors face where the player expects.** `mid_02.png`: the south row's plaza frontage
  carries dark-oak doors in dressed reveals and locked rejas at window height, all
  turned to the plaza — §1's "doors barred from the inside" reads without a word. Both
  rows' orientations are confirmed independently by `wide.png`, where the north row
  presents blank sealed backs to the NE camera.
- **Well and crucero are legible landmarks** (`mid_02.png`, `close_crucero.png` — the
  crucero's carved head clears the player's head, ~2.3 m).

What holds this at 6 is half observation, half missing evidence:

- **Three of the four premise beats have no gameplay-framing frame at all.** No mid
  shot exists of the east gate, the crucero breach, the chapel exterior, the graveyard,
  or the north row's facade. `zone_review`'s single-linkage clustering
  (`CLUSTER_RADIUS = 20`) merged all 30 town props into one "cluster" and framed it at
  `MID_RADIUS = 14` on the computed AABB centre (−5.16, −6.73) — i.e. it spent the
  town's only mid shot on the plaza, and spent the other two on cypress pairs. The
  clustering shot model cannot cover a 100 m town (fix F7).
- The road and the plaza carry no surface cue distinguishing them from open field, so
  "streets read as streets" rests entirely on the facing rows. Between the plaza and
  the gate (x 10→15) there is neither a surface nor a prop.
- The candelabra reads as debris rather than dressing (see F3).

## 4. Interiors — 5/10

The relocation itself is correct, and the frames prove it:

- `interior_apse.png` — looking west down the relocated nave: dressed limestone dado,
  smoke-darkened plaster above it rising out of frame (§6's soot gradient), heavy floor
  slabs, the flat apse end closing the view. This is the kit chapel at (−30, −29), not
  the old graybox.
- `interior_door.png` — the eastward sightline works: both oak leaves stand swung open
  against the inner wall (the only open door in Rocalba, §6), and the view carries
  through the opening to open ground with the olive stump at (−8, −30) sitting on the
  axis. Collapse rubble lies on the floor of the entry bays.
- The collapse side is right *in intent and wrong on the page*: the built vault covers
  the west/apse half and the entry half is open (`wide.png` chapel crop shows the
  barrel extrados over the west half and an open roofless east half). §6's literal text
  says the collapse is over the *western* bays — written when the door was also west.
  With the door flipped east (subplan §7 checkpoint 2), preserving the altar end means
  collapsing the entry end, which is what was built. **The premise amendment must cover
  both sentences, not just the door.**

Faults:

- **A carved pillar of the `chapel_arch` ruin stands inside the nave**
  (`interior_door.png`, left of frame, full height of the shot — see F1). It is the
  most prominent object in the chapel's interior and it is a prop that is supposed to
  be outside the south wall.
- **Neither interior frame shows the vault, the collapse rim, or the sky shaft.** Both
  are pitched 17° down (`NAVE_PITCH = 0.3`) from an eye 3.1 m up, so the frames are
  floor-and-dado; §6's headline effect — cool sky shafts against candle-gold — is
  unverified by this evidence set and its warm half does not exist yet.
- Rubble reads as seven uniform ~0.3 m boxes (kit-side, `buildings.py:548-554`); no
  retablo, votive stands or benches are placed.

## 5. Defect sweep — 6/10

Clean on the classes that would have been expensive:

- **No floating or sunken buildings.** Every casa AABB bases at exactly y = −0.50; the
  chapel's −0.65 is its 0.15 m floor slab; the well's −2.50 is the modelled shaft.
- **No bad yaws.** Both rows' facades verified against the frames, not just the table.
- **No z-fighting at party walls** despite the deliberate 0.6 m overlaps (`mid_02.png`
  junction crop), and no visual/collision divergence at the corner wings.
- Spawn ring and portal corridor hold against the *visual* AABBs too: nearest prop face
  to origin is the well at 4.25 m; only the gate arch crosses the corridor and it
  crosses as its 3.2 m opening.

Found:

| id | defect | evidence |
|---|---|---|
| F1 | `chapel_arch` pillar inside the chapel nave | `interior_door.png`; rotated AABB |
| F2 | cypress props are 1.8 m tall — no silhouette | `mid_00.png`, `mid_01.png` |
| F3 | `candelabra_shrine` is a feel-check placement mid-plaza | `mid_02.png`, `zones.ron:78` |
| F4 | `olive_stump` (−16, 18) clips casa_small_b's NW corner | AABB overlap 0.26 × 0.25 m |
| F5 | `rock_07`/`rock_09` placements are 0.1–0.3 m tall — inert | `close_rock_07/09.png` |
| F6 | the three gravestones sit on one straight line | zones.ron z = −22.5/−22.0/−22.5 |

---

## Fix list

**Required**

- **F1 — move `chapel_arch` clear of the chapel.** `zones.ron`
  `(−26.0, −0.5, −34.0) yaw 40` → **`(−26.0, −0.5, −36.5)`** (yaw unchanged).
  The model is 5.46 × 1.42 m; at yaw 40 its footprint spans z ∈ [−36.29, −31.71], while
  the chapel's south wall runs z = −33.1 (outer) / −32.5 (inner) — so its west pillar
  crosses the wall and stands on the nave floor at x ≈ −28, which is the tall carved
  column filling the left of `interior_door.png`. At z = −36.5 the north extent is
  −34.2, clearing the wall by 1.1 m.
  Also correct `tasks/town/p24-layout.md` §4: "chapel_arch ruin (−26,−34) clears the
  chapel south wall (z=−33.1) by 0.9 m — stays" is false; the check used the model's
  nominal depth and ignored that yaw 40 swings a 5.46 m span into z.
- **F2 — scale the cypresses.** All five placements (`−48,42`; `−42,48`; `30,−52`;
  `36,−46`; `−34,−40`) are 1.8 m tall at scale 1.0. Multiply the authored scales by
  ~5 (→ 8–10 m) so premise §4's cypress lines and the graveyard cypress read as
  silhouettes. Cheap and it fixes two of the six non-close frames.
- **F3 — give `candelabra_shrine` a premise slot.** Remove the (−4, −0.5, −5) plaza
  placement. Best use: two copies inside the nave flanking the apse, e.g.
  `(−38.0, −0.5, −27.6) yaw 0` and `(−38.0, −0.5, −30.4) yaw 180` — §6's votive stands,
  and the one enterable building currently has no dressing at all. Second choice: the
  porter's brazier beside the gate jamb at `(13.6, −0.5, 2.3)`.

**Minor**

- **F4** — `olive_stump (−16.0, −0.5, 18.0)` → `(−17.0, −0.5, 19.0)`; its AABB corner
  overlaps casa_small_b's back corner by ~0.25 m in both axes.
- **F5** — the four `rock_07`/`rock_09` placements measure 0.4 × 0.5 m × 0.10 m and
  0.6 × 0.8 m × 0.31 m at their authored scales; they contribute nothing. Delete them
  or raise the scales by ~10×. (Pre-existing dressing, inherited by this layout.)
- **F6** — jitter the gravestone z values (e.g. −21.2 / −23.4 / −22.6) so the patch
  reads as a graveyard rather than a row of three.
- **F8 (optional)** — the well at z = −5.5 leaves only 2.45 m between its south face
  and the south facade line, which is why it hugs the houses in `mid_02.png`. z = −4.6
  keeps the spawn ring clear (nearest face 3.35 m > 3) and doubles the frontage gap.

**Evidence (required for the gate record, not for the layout)**

- **F7** — re-render targeted mid frames before the Phase 2 gate closes: east gate +
  crucero breach, chapel exterior on the plaza-side approach, chapel precinct /
  graveyard, north row facade. `zone_review`'s cluster shot cannot cover this town —
  either give it explicit shot targets or drop `CLUSTER_RADIUS` well below 20 m.
  Criteria 1–3 are scored on a partly unmeasured town until this exists.

## Watch items carried

From G2, confirmed present, none dominant — recorded only:

- **D9 mip-blur** and **D10 ghost blocks in the whitewash** — both visible in
  `close_casa_corner.png` at 2.3 m (faint rectangular plaques in the render, soft
  surface); neither reads at street or plaza distance.
- **N3 roof mirror-band** — visible as a hard brightness step across every roof slope
  in `wide.png`; at establishing framing it is the most noticeable roof artifact in the
  silhouette.
- **No ridge cap** — reads where ridges meet in the `wide.png` row crops.

Non-layout, for later phases:

- No candle-gold emissive exists anywhere in the zone (premise §1/§2 signature).
- Plaza/street ground is `cracked_earth`; premise §3 specifies worn cobble for plaza,
  streets and chapel floor. The east zone already ships a `worn_cobble` ground.
- The chapel's exterior vault carries bare limestone extrados with no tile covering.
- Fog density 0.0055 erases the chapel from any plaza-side view; if the SW precinct is
  meant to read as a landmark, that is a lighting decision, not a placement one.
- Premise §6 needs a two-line amendment, not one: doors **east**, collapse over the
  **entry (east) bays**, apse end intact.

---

# Re-judgment on F7 evidence

Date: 2026-07-31, same day. Everything above is the round-one record and stands as
written; this section appends the re-score on evidence that did not exist then.

Evidence: `target/town-layout/p24-f7/` — `mid_gate.png`, `mid_chapel.png`,
`mid_graveyard.png`, `mid_north_row.png` (F7's four hand-targeted gameplay-framing
shots), plus a full re-render of `wide.png`, both interiors, 18 `close_*`.
Layout of record: `content/zones/zones.ron` start zone with F1–F6/F8 applied.
Geometry cross-checks re-derived from the installed glTF node graphs under each
placement's pos/scale/yaw.

## Revised verdict

# PASS with fixes — 1 required, 2 recommended

Round one's three required fixes are all applied, and two of the three landed
cleanly. The third (F3) landed as a *placement idea* and broke as *geometry*: the
votive stands are centred exactly on the apse wall plane, so half of each
candelabrum is inside the masonry. That is one coordinate on two lines.

The four new frames also settle what round one could not see, and the news is
mixed. The north row is the best thing in the town and now proven at 28 m. The
cypresses are the only silhouettes that survive the fog and they now frame the
zone. Against that: **the chapel does not read as a chapel from outside**, and its
collapse does not read as a collapse. Neither is a layout coordinate — but neither
was measured before, and both belong on the record.

| # | Criterion | Round 1 | Now | Δ |
|---|---|---|---|---|
| 1 | Premise fidelity | 7 | **7** | 0 (re-composed) |
| 2 | Composition / massing | 8 | **8** | 0 |
| 3 | Readability at gameplay framing | 6 | **7** | +1 |
| 4 | Interiors | 5 | **6** | +1 |
| 5 | Defect sweep | 6 | **7** | +1 |

## Did the fixes land?

**F1 — chapel_arch clear of the nave: LANDED.** `interior_door.png` has no pillar.
The tall carved column that filled the left of the round-one frame is gone and the
nave is a clean room. `mid_graveyard.png` (right of frame) shows the arch standing
free south-east of the chapel as a half-surviving carved portal; its silhouette
grazes the chapel's corner from that azimuth, which reads as an old portal fragment
beside the church rather than an error. Rotated AABB now Z ∈ [−38.8, −34.2] against
the south wall at z = −33.1: 1.1 m clear, as specified.

**F2 — cypress scale: LANDED at distance, with a named cost up close.**
`wide.png` is the proof: under the same fog that erases the town, the two NW and two
SE cypresses are the only dark verticals left, and they now bracket the settlement.
`mid_graveyard.png` shows the graveyard cypress as a proper spire, planted, shadowed,
roughly 3:1 in the frame. §4's cypress lines exist for the first time.
The cost is real and should not be talked away: `close_cypress.png`. At ×5 the
foliage plates are ~0.6 m across and there is no trunk — a player standing at the
tree sees a 9 m wall of half-metre dark shards. All five cypresses sit at r 52–64,
inside the play radius, so this read is reachable. **Verdict: keep the scale.** The
silhouette is a premise beat and the close read is a model problem; a scale number
cannot buy both, and the answer if both are wanted is a cypress model with a trunk
and finer foliage, not a different multiplier.

**F3 — votive stands: PLACEMENT RIGHT, GEOMETRY BROKEN. Required fix below.**
The idea works — `interior_apse.png` shows two symmetric iron pieces flanking the
apse, and they read as deliberate chapel dressing rather than parked objects. But
`close_candelabra_shrine.png` shows the prop is a **five-branch floor candelabrum
with five white tapers**, and the apse frame shows **one taper and two orphan cups
per unit**, with the foot rendering as a dark smudge disconnected from everything
above it. The chapel's apse end wall is a full-height panel at exactly **x = −38.00**
(307 verts, y −0.65 → 10.00, z −32.80 → −25.20). Both stands are placed at
x = −38.0 with a ±0.59 m footprint. Half of each candelabrum is inside the wall, and
the render shows precisely the half that is not.

**F4 — olive_stump: LANDED.** A full pairwise AABB sweep over all 38 start-zone
placements now returns only the six intended party-wall overlaps and the chapel ×
candelabra containment. No frame covers (−17, 19), so this is a geometric
confirmation, not a visual one.

**F5 — rocks: LANDED.** `close_rock_07.png` shows a knee-high angular boulder with
real relief and a cast shadow. See the rock_09 ruling below.

**F6 — gravestone jitter: LANDED.** `mid_graveyard.png` (left of frame) shows three
cross-headed markers on a loose diagonal, not a row. They read as grave markers at
30 m.

**F8 — well: UNVERIFIED.** No frame in this set covers the plaza. The four F7 shots
are isolated-group renders and their select radii exclude the well; `wide.png` shows
it as a few pixels. The plaza frontage gap claim rests on the round-one geometry
only. The same applies to the *removal* half of F3, which cannot create a defect.

## rock_09 ruling — keep 24.3, no change

`close_rock_09.png` settles it. At 24.3 the prop is a 3.6 × 3.8 × 0.79 m dark
brown-ochre outcrop that reads unambiguously as a **field boulder**: clear angular
relief, a cast shadow with a grounded contact edge, and no texture stretching at any
point of the visible face. It is thigh-high on the human reference and reads as
something you walk around. It does not read as pavement and it does not read as a
stretched-texture artifact. The 0.79 m height that triggered the watch is simply what
this model is — a flat-aspect slab — and the footprint carries the read instead.

**Concrete number: unchanged, `scale: 24.3`.** Same ruling for `rock_09` at
(−11, 27) scale 19.8 and both `rock_07` placements.

One carried note, not a scale question: both rock models are noticeably warmer and
more saturated than the town's grey register (§2 caps S ≤ 0.35 with a 20°–50° warm
bias). Neither appears in any mid frame, so this is a colour-law item for a later
pass on inherited Poly Haven dressing, not a placement fix.

## Do the four premise beats read?

**North row facade — yes, emphatically.** `mid_north_row.png` is the best frame the
town has produced. A continuous four-house terrace, three ridge heights staggered
across it, quoin chains, iron rejas at window height, and closed plank doors — at
native resolution the doors are visibly horizontal oak boarding, so §1's "doors
barred from the inside" reads without a word. The corner house's step at the east end
reads as accretion.

**East gate — yes, with a weak shrine.** `mid_gate.png` shows the arch astride the
road as the tallest thing on the wall line, fragment runs to the south, and the
3.7 m breach with the crucero standing in it. What does not read is the *shrine*: at
16 m the crucero reduces to a 1.8 m cross silhouette that is indistinguishable from
the graveyard markers 45 m away. `close_crucero.png` shows it is a finely carved
interlaced wayside cross — that carving is invisible at gameplay distance, and the
gate beat lands as "a grave beside the wall". Recommendation below.

**Graveyard precinct — yes, thinly.** `mid_graveyard.png` reads as a churchyard: the
chapel's flank, three markers off its north wall, the cypress and the carved arch
ruin behind. §1's decay-increasing-outward gradient is legible in one frame. What
holds it thin is that three markers, one tree and one arch is the whole precinct —
there is no enclosure, no path, no ground change, and nothing between the graveyard
and the town.

**Chapel exterior — no.** This is the finding this whole re-render bought.
`mid_chapel.png` shows the chapel from the plaza side as a rectangular ashlar box
with half a barrel vault: **no bell gable, no cross, no window, no portal surround,
no vertical accent of any kind**. The AABB top of 10.03 m is the vault itself,
nothing above it. It reads as a cistern or a vaulted crypt. Premise §4/§6 make this
the town's one enterable landmark anchoring the SW precinct, and at gameplay framing
it carries no ecclesiastical signature at all.

Second miss in the same frame: **the collapse does not read as a collapse.** The
open half's wall tops are perfectly level under a continuous coping course, the
vault's cut edge is a clean arc of regular voussoirs, and there is no rubble outside
the building anywhere. It reads as *never roofed* — a hall under construction — not
as §6's vault that fell. The seven uniform cubes inside are the only fall evidence
and they are sealed in the interior.

Neither is a coordinate, and the kit was gated separately at G2 — but G2 judged
materials and geometry, not whether the building reads as a church, so this went
unmeasured. It is recorded here as measured, and one cheap layout mitigation exists
(recommended fix R2).

## New findings on ground nothing had judged

| id | finding | evidence | owner |
|---|---|---|---|
| F9 | both `candelabra_shrine` stands are half inside the apse end wall | `interior_apse.png`; wall plane x = −38.00 vs placement x = −38.0 ±0.59 | **layout, required** |
| F10 | quoin chains strand on flush facades — stone blocks pasted on flat whitewash with no corner behind them, at every party junction | `mid_north_row.png` 2× crop, left half | kit |
| F11 | chapel reads as a vaulted box, not a chapel; collapse reads as never-roofed | `mid_chapel.png` | kit + layout mitigation R2 |
| F12 | crucero's gameplay-distance silhouette equals the gravestones' | `mid_gate.png` vs `mid_graveyard.png` | layout, recommended R1 |
| F13 | 0.20 m see-through slot between the gate's south pier (z = −3.25) and the wall fragment at z = −5.5 (z = −3.45) | `mid_gate.png` 3× crop, bottom left | noted only |

On F10: the cause is a placement decision — fronts flush at z = ±9.1 with a 0.73 m
party overlap means neither house has a real corner in the facade plane, so both
sets of kit quoins render as loose blocks on a continuous wall, drifting off-vertical
as they descend. But no overlap value fixes it, because a flush terrace facade has no
corner to quoin; the fix is kit-side (drop quoins from the abutting edge). Layout can
only mitigate by stepping alternate houses 0.3–0.4 m so a corner exists, which trades
away the exact facade line the row work is built on. **Not a layout-gate blocker;
carried to the kit with the frame cited.**

On F13: adjacent wall fragments already sit 0.40 m apart by design, so a 0.20 m slot
at the pier reads as one more joint in an unfinished wall. Optional.

Clean on re-sweep: no floating or sunken props across all 38 placements (every base
at y = −0.50 except the chapel's 0.15 m floor slab, the well's modelled shaft, and
the rocks' 0.01 m bed); no bad yaws in any of the four new framings; no new
intersections beyond F9.

Confirmed but out of scope (G2, `docs/reviews/town/g2-kit-regate-2026-07-31.md`):
the roof mirror-band is **more** severe at 28 m gameplay framing than at the wide
establishing shot — `mid_north_row.png` shows every slope split into hard light/dark
bands with a visible horizontal mirror seam. Round one recorded it as a wide-shot
watch item; it should be re-priced as a gameplay-framing artifact. Also confirmed:
no ridge cap, whitewash tiling blotches, and the chapel interior's plaster reading
green-damp rather than §6's soot-grey.

## Fix list (this round)

**Required — 1**

- **F9 — clear the votive stands of the apse wall.** `zones.ron`, both lines:
  `(−38.0, −0.5, −27.6)` → **`(−37.0, −0.5, −27.6)`** and
  `(−38.0, −0.5, −30.4)` → **`(−37.0, −0.5, −30.4)`** (yaws and z unchanged).
  The apse end wall is the full-height panel at x = −38.00; the prop's half-depth is
  0.59 m, so x = −37.0 stands it 0.41 m clear and puts all five tapers in the room.
  Re-render `interior_apse.png` to confirm five candles per stand.

**Recommended — 2**

- **R1 — give the gate shrine its own silhouette.** `crucero` at (15, −0.5, 6.0)
  `scale: 1.0` → **`1.5`** (1.80 m → 2.70 m, i.e. 87 % of the 3.09 m wall fragments).
  A crucero is a monument on a stepped plinth; at 1.8 m it is a grave marker, and the
  town already has three of those. Verify against `mid_gate.png` on re-render — if
  the carving reads oversized at 1.5, the honest answer is a plinth on the model
  rather than a smaller multiplier.
- **R2 — put the chapel's fall on the ground outside it.** Two or three rock props
  within 2–3 m of the collapsed east half's walls — `rock_07` at scale 5–7 and
  `rock_09` at scale 15–22, north side around z ≈ −23.5…−24.5 and south side around
  z ≈ −34…−35, both with x ∈ [−29, −23]. Keep x > −21 clear along z ∈ [−30.5, −27.5]:
  that is the east door axis, and the `olive_stump` at (−8, −30) currently terminates
  it in `interior_door.png`. Three lines in `zones.ron` buy the one thing §6 says
  about this building and `mid_chapel.png` currently denies.

**Escalated, not layout's**

- The chapel needs an exterior ecclesiastical signature — espadaña, bell, or a cross
  on the apse gable (F11). Kit work, and it is what decides whether the SW precinct
  is a landmark or a shed.
- F10 quoins, and the roof mirror-band re-pricing.

**Carried unchanged from round one** — no candle-gold emissive, `cracked_earth`
instead of §3 worn cobble, fog density erasing the chapel from plaza views, the
missing retablo/benches, the uniform rubble cubes, and the §6 two-line amendment.
None of these were inflated into required fixes and none moved this round.
