# RUN-C1 concept screen — 64 images, Opus visual gate

Date 2026-08-01. Grid `target/concept-c1/<slot>/seed_<n>/concept.png`, 8 slots ×
8 seeds. Judged against `tasks/town/p31-c1-concepts.md` §3 and
`docs/town-premise.md` §2–§3. **All 64 images were opened and viewed.** Colour
claims are measured (numpy HSV over the background-masked object), never
impressions.

Generator context that shapes every failure below: `prop_concept.json` zeroes
the negative branch and samples at cfg 1.0, so every exclusion was carried by
affirmative wording alone. A forbidden thing on screen is expected generator
behaviour, not proof the subject is dead. Each verdict therefore names its
failure mode — **prompt-fixable** or **structural**.

---

## 1. Verdict table

| slot | verdict | passes /8 | winning seed(s) | score against its own §3.2 criterion |
|---|---|---|---|---|
| **C1** ruined arch | **FAIL — prompt-fixable** | 0 / 8 | none | Round arch ✓, voussoir joints ✓, undecorated ✗, broken stumps ✗ (0/8) — **2 of 4** |
| **C2** retablo, flat reredos | **FAIL — structural** | 0 / 8 | none | Not the subject. Panels/frame/gilt clauses unassessable — **0 of 3** |
| **C3** retablo, three-bay | **PASS** | 5 / 8 | **1** (alt 4) | Bays ✓, niche depth ✓, niche empty ✓ (8/8), plain pediment ✓ 5/8 — **4 of 4** |
| **C4** shrine pillar | **PASS-WITH-PROMPT-FIX** | 0 / 8 clean, 8/8 on subject | 6 (after re-roll) | Recess self-shadowed ✓, hood line ✓, candle gold ✓, plain square post ✓ — **4 of 4**, killed by U1 framing |
| **C5** ox cart | **FAIL — prompt-fixable, low confidence** | 0 / 8 | none | Two wheels ✓ but **solid discs ✗ 0/8**; pole continuous ✓ 7/8; sacks ✓; no figure ✓ — **3 of 5** |
| **C6** tall pricket stand | **PASS** | 3 / 8 | **4** (alt 7, 8) | Continuous post ✓ 8/8, three legs ✓ 8/8, ≥4 candles ✓ 3/8, flames gold ✓ — **4 of 4** on 3 seeds |
| **C7** low tiered rack | **FAIL — structural** | 0 / 8 | none | Two tiers ✓ 3/8, both tiers counted ✓ 2/8, **members thicker than C6 ✗ 0/8** — **1 of 4** |
| **C8** gate brazier | **PASS** | 6 / 8 | **8** (alt 4) | Vessel with a wall ✓ 8/8, three legs ✓ 8/8, **fire gold, zero red ✓ 8/8**; openwork only 1/8 — **3 of 3** |

---

## 2. Per-slot evidence from the pixels

### C1 — ruined freestanding arch — FAIL, prompt-fixable

Eight seeds, one mode. Every image is a complete, upright, **veined polished
white-grey stone** portal with a moulded impost cornice at each springing and a
moulded base plinth — a triumphal-arch/Renaissance doorcase, not a poor
Castilian ruin. The veining in seeds 2, 6, 7, 8 is unmistakably marble figuring
(seed 7's piers are swirled Carrara-style clouding); seeds 1, 3, 5 read as
terrazzo/breccia. That is **U3 instant fail** on `marble`, in all 8.

The slot's own criterion also fails on the feet: **not one seed terminates in a
broken stump.** All eight stand on a clean two-step moulded plinth, and seed 6
inverts the brief entirely by leaving the *spandrels* ragged while the feet stay
pristine. Seeds 3, 4, 5 additionally carry a ground plane with a cast shadow
(**U1**).

The one thing this slot was created to catch **did not happen**: at 100 % zoom
the voussoir joints, the impost arrises and the plinth mouldings are all clean
edges, not smears. **U6 passes in all 8.** The shipped `chapel_arch`'s melted
carving does not reproduce here. Colour is legal and cool — object mean
S 0.045–0.090, V 0.582–0.653, no warm drift (the R−B +31 failure did not recur).

Diagnosis: `dressed`, `ashlar` and `voussoirs` are the marble attractor, and
`archway fragment` reads to this model as "architectural portal, intact". Both
are wording, not subject. Prompt-fixable.

### C2 — retablo, flat panelled reredos — FAIL, structural

Eight seeds, one mode, and it is not the subject. Every image is a **modern
picture frame** — deep ogee-moulded dark oak, mitred corners, standing on a
table with a cast shadow — containing a naive-art **landscape triptych** of a
hill village. Seed 1 shows red pantile roofs and a spire; seed 4 contains a
20th-century **orange motor car** at lower left; seed 6 has a small blue human
figure on the road; seeds 2, 5, 7, 8 are all the same church-in-a-valley
composition. There is no altarpiece anywhere in the grid.

The phrase "plain rectangular frame around three flat painted panels" is read
whole as *framed picture*, and the qualifier "altarpiece" loses. Deleting the
loser costs nothing here — C3 answers the same question and answers it well.

### C3 — retablo, three-bay with pediment — PASS

The strongest slot in the grid on subject fidelity: 8/8 return a **three-bay
dark-oak retablo** with two flanking recessed panels and a **central arched
niche that is genuinely empty and genuinely deep** — the niche interior is
self-shadowed with a visible back plane and visible reveal in every seed
(clearest in 1, 4, 7, 8). Gilt is confined to a thin bead: seed 1's gilt is a
single line following the niche arch, seeds 2/4/5/8 run one thin gilt fillet on
the panel surrounds and the plinth. Panels are distinguished from the frame by
**depth**, not colour — the gilt bead sits proud, the panel face sits back.
Oak reads correctly dark near-black brown with warm grain (object mean S 0.099–
0.224, V 0.372–0.463).

Pediment clause splits the field. **Seed 1 is exactly the criterion**: a plain
triangular pediment on a plain cornice. Seeds 2, 4, 6 give a plain cornice with
no crest — acceptable. Seeds 5, 7, 8 add a swan-neck/scrolled crest and fluted
pilasters, and **seed 3 carries carved foliate capitals — U7 fail**. So 5/8
clean, and the winner is unambiguous.

Two caveats, neither fatal: the object reads at cabinet scale (~0.6 m) rather
than 3 m, and it sits on a floor with a soft cast shadow in all 8 (**U1**, see
§5). The panel imagery is grisaille townscape rather than "muted painting", which
is a texture-stage concern, not a geometry one.

### C4 — wayside shrine pillar — PASS-WITH-PROMPT-FIX

Every criterion clause is met, 8/8. The recess is deep enough to shadow its own
interior with a real reveal and a real back wall (seeds 1, 5, 6, 7 show the
side-wall gradient explicitly); the slab hood casts a hard distinct line in all
8; the candle is unambiguously a candle with a small gold flame; the post is
plainly square with no plinth mouldings except seed 8, which adds a cornice
under the recess. No melted detail anywhere.

**And all 8 are cropped by the bottom frame edge.** The post runs out of the
image in every seed — this is **U1** ("one object, fully inside the frame"), and
it is not cosmetic: `stage_geometry` consumes the concept, so a cropped concept
extracts a cropped prop. That alone blocks carry-forward.

Second defect: the stone reads as **speckled granite/terrazzo**, not dressed
limestone — no chisel tool-marks, no coursing, and in seeds 2, 6 the speckle is
coarse enough to read as aggregate. Colour is legal (object mean S 0.080–0.140,
V 0.557–0.657) but the material identity is wrong.

### C5 — ox cart abandoned mid-load — FAIL on its own criterion

The cart itself is excellent: plank bed legible in all 8, iron strapping crisp,
sack load reading as cloth (though plumper than "slumped" — seeds 1, 3, 6, 8 are
closer to cushions than grain sacks), no draught animal, no figure.

**But every one of the eight wheels pairs is spoked.** Ten to twelve turned
spokes, a hub boss and an iron tyre, rendered cleanly, in 8/8 — against a prompt
that says "two solid disc wooden wheels" in the second clause. The single
attribute the whole slot exists to protect is the single attribute the sampler
overrode unanimously. Criterion fails 0/8.

Pole: continuous, ground-touching and un-fused in seeds 1, 3, 4, 5, 6, 7, 8.
**Seed 2's pole is fully detached** and floats as a separate object beside the
cart. Ground plane with cast shadow in all 8 (**U1**).

Confidence that a prompt fix lands here is **low** — an 8/8 override of an
explicit adjective is a strong prior, not a seed accident.

### C6 — votive stand, tall pricket — PASS, and the thin-iron answer is YES

**Thin iron survives generation cleanly.** The post is a single continuous
member from foot to tray in 8/8, with no waisting, no break, no blob and no
fuse into the tray; at 100 % zoom the post's silhouette edges are hard, its
mid-collar is a crisp torus, and the tripod legs read as three separate bars
that meet at a collar (seeds 2, 5, 6 are the cleanest). Nothing melted. Nothing
smeared. This is an emphatic feasibility pass at the concept stage.

Candle count splits the field: **seeds 4, 7 and 8 carry four individually
countable candles in separate iron drip cups**; seeds 1, 2, 3, 5, 6 carry a
single pillar on a bare tray and fail the "at least four" clause. Flames measure
hue 27.6°–30.2°, S 0.235–0.244, V 0.949–0.967 — gold, not white-hot, though 5°–7°
below the premise's 35° candle-gold floor.

Poverty caveat: the feet terminate in smith scrolls in seeds 1, 3, 5, 7, 8 and
the object reads as a genteel drinks table rather than village work. Seed 4 has
the plainest feet (ball toes) and four cups — it is the winner.

### C7 — votive stand, low tiered rack — FAIL, structural

The slot's hypothesis was mass. The grid returned the opposite: a **thin
wire-rod cage**, with frame members visibly *thinner* than C6's post in all 8.
At the stated 0.9 m the rods scale to roughly 8–12 mm, under **U9**'s ~3 cm
floor for unsupported elements — the very failure mode C7 was supposed to avoid.

Two stepped tiers separated by visible air appear only in seeds 2, 4, 5; seeds
1, 3, 6, 7, 8 are a single deck with one or two raised cups. Candles countable
on *both* tiers only in seeds 2 and 4. Period read is wrong throughout —
square-section welded wire and machined tealight cups read as contemporary
homeware, not 1490s smith work.

C7 does not merely lose the A/B; it inverts its own premise, so no prompt fix is
proposed.

### C8 — gate brazier, lit — PASS, and the colour hazard did not fire

**The reserved-band failure the spec predicted did not happen.** Flame pixels
(V > 0.75, S > 0.2) measure median hue **33.5°–38.7°** across the eight seeds,
p95 42°–54° — candle-gold, with a cream-white core. The fraction of flame pixels
below hue 15° (true red) is **0.03 %–0.41 %**, and full threat-band pixels
(hue 350°–25° ∧ S ≥ 0.7 ∧ V ≥ 0.8) number **0, 2, 4, 6, 18 and 49** out of
~650 000 object pixels — hairlines at the flame's hottest edge, not clusters.
Seeds 4 and 8 are the cleanest (0.05 % and 0.03 % red). No red embers, no orange
glow; the coal beds read charcoal-black with a faint blue base flame in seeds 3
and 8. **The brazier ships lit. The unlit fallback is not needed.**

The architecture around the fire also stays legal: the iron body measures object
mean S 0.045–0.067 at V 0.367–0.513 — matte black, no warm cast.

The **basket** is the real defect. Seven of eight return a **solid hammered
bowl** with no openwork — a vessel with a wall, so the stated criterion passes,
but not the lattice the concept described. **Seed 8 alone returns true
openwork**: an upper band of upright iron bars riveted to two hoops with the
coal bed visible through the gaps. Three splayed legs are distinct and meet the
bowl in all 8. Two seeds fail **U7**: seed 7 carries a **foliate leaf relief**
band around the bowl and seed 5 carries engraved scrollwork.

---

## 3. Every colour-law violation, measured

Method: numpy HSV per pixel, background masked by corner-colour match ∧ S < 0.06.
Two readings are reported because §2 defines its ceilings on *a whole bound
surface averaged under ship lighting*, which a concept PNG cannot supply —
whole-object means are the closest analogue, sub-region means are the worst case.

**No image in the grid contains a threat-band cluster.** Maximum is 85 px
(0.012 % of the object) on C4 seed 6.

| where | measured | law | reading |
|---|---|---|---|
| C4 s6 candle-flame core | hue **24.2°**, S **0.720**, V **0.830**, 85 px | threat band = hue 350–25 ∧ S ≥ 0.7 ∧ V ≥ 0.8 | **inside the band**, 0.8° under the boundary. Also on s1 (5 px), s2 (2 px), s4 (5 px), s7 (8 px) |
| C8 s5 / s8 flame core | hue **24.1°** S 0.750 V 0.811 (49 px) / hue **23.0°** S 0.759 V 0.836 (18 px) | as above | **inside the band**, hairline. s1 6 px, s2 2 px, s6 4 px, s3/s4/s7 zero |
| C7 s4 rivet rust speck | hue 22.0°, S 0.715, V 0.864, 7 px | as above | **inside the band**, negligible area |
| C4 recess interior (s1/s6/s7) | hue **23.0 / 25.3 / 23.6°**, S **0.627 / 0.651 / 0.596**, V 0.405–0.421 | hue inside reserved window ⇒ S ≤ 0.35 | **over cap by ~0.25.** V too low for the threat conjunction, but this is candle-lit *stone*, exactly the "warm glow on surrounding architecture" hazard — and it is on C4, not C8 |
| C8 rust patches, all 8 seeds | hue **20.4–23.4°**, S **0.543–0.640**, V 0.199–0.258 | as above | **over cap by ~0.20–0.29.** Prompt says "sparse desaturated brown rust, never bright orange"; the render is neither bright nor orange but it is over the saturation cap for its hue. Whole-object iron mean S 0.045–0.067 passes comfortably |
| C6 rust at rivets (s4/7/8) | hue 20.4–24.1°, S 0.465–0.511, V 0.160–0.191 | as above | over cap, ~1.5 % of object area |
| C5 cart timber (s1/4/8) | hue 24.2–25.7°, S 0.504–0.580, V 0.227–0.249 | as above | over cap in the warm-grain sub-region; whole-object mean S 0.076–0.201 passes |
| C1 stone, all 8 | mean S **0.045–0.090**, V **0.582–0.653** | ambient S ≤ 0.35 | **legal, and cool.** The R−B warm drift that produced the arch atlas did not recur |
| C2 / C3 panels | mean S 0.099–0.276, V 0.313–0.463; zero threat px | ambient band | **legal** — even C2's painted pantile roofs stay out of the reserved band |
| C6 / C8 flames | C6 hue 27.6–30.2 S 0.24; C8 hue 36.3–42.9 S 0.34–0.37, V 0.97 | candle-gold 35–50°, S 0.55–0.85 | C8 **on-hue**, under the S floor (paler than spec). C6 **5–7° below** the 35° floor — legal (outside the reserved window) but not on-spec |

Ruling on the flame hairlines: a candle or brazier flame is an HDR emissive and
its hottest core crossing 25° for tens of pixels is not what §2 reserves the band
*against*. I do not treat them as U5 failures. The **C4 recess interior** is a
different matter — that is non-emissive stone reading at S 0.60+ inside the
window, and it should be cooled by prompt.

---

## 4. Revised prompts (no negative channel, ~60 tokens, affirmative only)

### C1 — `subject`

```
ruined round stone arch standing alone, rough grey limestone blocks with open weathered joints, plain unmoulded ring, both legs ending in jagged rubble stumps of collapsed masonry, chalky matte pale grey, cool grey with sandy grit, dark soot in the joints, poor village churchyard ruin
```

Changes and why: `dressed`/`ashlar`/`voussoirs` deleted — those three words are
the marble attractor (8/8). `rough … blocks with open weathered joints` supplies
the countable-voussoir read affirmatively. `both legs ending in jagged rubble
stumps of collapsed masonry` states the break as a positive property, replacing
the plinth. `chalky matte` fights the polish; `poor village churchyard ruin`
fights the grandeur. `fragment` is dropped — it produced intact portals.

### C4 — `subject`

```
whole freestanding stone shrine post seen entire from cap to foot, plain square rough limestone pillar, flat slab hood on top, deep dark hooded recess near the top, one small white wax candle inside with a small pale gold flame, cool pale grey limestone, chisel tool marks, matte, dim shadowed recess
```

Changes: `whole … seen entire from cap to foot` is the framing fix — the
workflow's `single object centered` suffix is not enough to stop the crop.
`rough … chisel tool marks` fights the granite speckle. `dim shadowed recess`
and `small pale gold flame` (replacing "warm candle-gold flame") cool the recess
interior off its measured S 0.60+.

### C5 — `subject`

```
abandoned Castilian carreta, its two wheels are flat solid rounds of joined oak plank banded with an iron tyre, unbroken timber discs, plain plank bed, long straight draught pole resting on the ground, load of slumped grain sacks, dark oak, deep near-black brown, silvered light-grey weathering, matte black iron
```

Changes: the word `wheel` is moved out of the head noun and the discs are
described as what they are made of — `flat solid rounds of joined oak plank`,
`unbroken timber discs`. `carreta` leads, since `ox cart` co-activates the
spoked-wagon prior. **Confidence is low** — see §5 and the C5 ruling.

### C6 — `subject`

```
tall wrought iron votive stand, one slender post on three plain splayed legs with flat feet, wide round top tray crowded with a ring of eight short white wax candles each in its own iron drip cup, small warm gold flames, matte black wrought iron, charcoal-grey worn edges, plain smith work
```

Changes: `crowded with a ring of eight … each in its own iron drip cup` fixes
the 5/8 single-candle miss. `three plain splayed legs with flat feet` fixes the
scrolled toes (U7). Rust clause dropped — it contributed the only over-cap
saturation on this slot and adds nothing to a `painted_metal` surface class.

### C8 — `subject`

```
wrought iron gate brazier, deep round basket built of upright iron bars riveted between two hoops, wide gaps between the bars showing the coal bed through the wall, three splayed legs, pale gold flame, yellow-gold firelight, glowing pale gold embers, matte black wrought iron, smooth undecorated sides, plain smith work
```

Changes: only the basket. `built of upright iron bars riveted between two hoops`
+ `wide gaps … showing the coal bed through the wall` describes the lattice as
construction rather than as the adjective `openwork`, which produced a solid
bowl 7/8. `smooth undecorated sides` is the affirmative kill for seed 7's
foliate relief and seed 5's scrollwork. **The colour clauses are unchanged and
must stay unchanged — they worked.**

C3 and C7 get no revised prompt: C3 needs none (carry seed 1 forward), C7 is
cancelled.

---

## 5. Chain rulings — does each chain proceed to RUN-H?

| chain | slot | ruling | candidates | wall |
|---|---|---|---|---|
| **H1** `chapel_arch` | C1 | **HOLD.** Do not enter H1 on this grid — no candidate may be carried forward (marble, U3). Re-roll C1's concept only, 8 seeds ≈ 4 GPU-min, with the §4 wording. Proceed only if the re-roll returns rough coursed limestone with stump feet; if it returns marble a second time, the arch goes to the kit and 34 min is released. | 4, gated | 34 min |
| **H3** retablo | **C3** | **PROCEED.** Retablo A/B settled: **C3 wins, C2 is deleted.** Carry `C3/seed_1/concept.png` via `gen_prop.py --skip-concept`. No re-roll needed. | 6 | 51 min |
| **H4** shrine pillar | C4 | **HOLD, one cheap re-roll.** The subject is the strongest read in the grid and I would keep it against OPEN-3 — the lit recess is a beat the `crucero` cannot carry, and it is visually distinct. But all 8 are frame-cropped and cannot be fed to `stage_geometry`. Re-roll 8 seeds ≈ 4 GPU-min, then proceed. | 3, gated | 26 min |
| **H6** cart | C5 | **HOLD.** Re-roll 8 seeds ≈ 4 GPU-min with the §4 wheel wording. If discs still do not come, take the plan's own contingency — generated bed + procedural disc wheels from `build_town_kit.py` — rather than spending 26 chain-minutes on a prop whose defining feature the sampler will not draw. | 3, gated | 26 min |
| **H7** votive stand | **C6** | **PROCEED. Thin iron is feasible — this is a clear YES.** The slender post rendered as a single crisp continuous member in 8/8 with no melting, blobbing or fusing; the tripod legs stayed three distinct bars in 8/8. H7 runs on **silhouette A (C6)**, seed 4. **C7 is cancelled and deleted** — it returned members thinner than C6's post in 8/8, inverting its own mass hypothesis, and falls under U9. Optional 4-min re-roll to get eight candles instead of four; seed 4 is shippable without it. | 3 | 26 min |
| **H5→** gate brazier | **C8** | **PROCEED, and the brazier ships lit.** No candidate showed red or orange fire; the colour verdict is a pass on all 8. Carry `C8/seed_8/concept.png` — the one true openwork basket and the second-cleanest flame. Optional 4-min re-roll would raise openwork from 1/8 to a choice, but seed 8 already answers the slot. | 3 | 26 min |

**Thin-iron feasibility answer, stated plainly:** *yes*. Slender wrought-iron
members generate legibly at this operating point. C6 proves it at ~10 mm
apparent section; C7's failure is a period/mass drift, not an iron-thinness
failure. H7 runs.

Chain budget is unchanged at 22 candidates ≈ 3.1 h, of which 12 candidates
(H3, H7, H5→) are released now and 10 (H1, H4, H6) are gated behind ~12
GPU-minutes of concept re-rolls. Two concepts are deleted: **C2** and **C7**.

---

## 6. What made me doubt the screen itself

1. **U1 disqualifies most of the grid on a technicality.** C2, C3, C5, C6 and C7
   all sit on a floor plane with a soft cast shadow, and C1 seeds 3–5 do too —
   that is studio product-photo lighting, not attached floor geometry. As
   written ("no cast-shadow floor"), U1 fails ~40 of 64 images and the gate goes
   vacuous. It should read *no floor geometry continuous with the object*.
   C4's failure is a different thing and a real one: the object leaves the frame.

2. **§2's ceilings are not measurable on a concept PNG.** The premise defines
   S ≤ 0.35 as a whole bound surface averaged under ship lighting at 2.3 m. A
   concept has neither ship lighting nor a range. Whole-object means pass
   everywhere; sub-region means fail on four slots. The rubric does not say which
   is binding, so U4/U5 are underspecified at this stage — I reported both rather
   than pick silently.

3. **Three per-slot criteria turn on a single adjective the sampler overrides.**
   C5's "solid disc" (0/8), C6's "at least four candles" (3/8) and C7's "thicker
   than C6's post" (0/8). When a criterion rides one word at cfg 1.0 with no
   negative channel, a FAIL measures the sampler's prior, not the subject's
   viability — which is exactly the confusion this screen was built to avoid. C5
   is the case where I am least confident the FAIL is informative.

4. **Nothing conveys scale to the model.** `height_m` never reaches the prompt,
   and it shows: C3 reads as a 0.6 m tabletop cabinet, C6 as a drinks table, C7
   as contemporary homeware. Criteria phrased as "reads as a landmark at 8 m" or
   "legible at 256 px" are therefore not really screened by these images.

5. **C1's stated reason for existing did not reproduce.** The slot was created
   because the shipped `chapel_arch`'s melted carving *was already visible in its
   own concept*. Across 8 seeds here, U6 passes cleanly — every arris and joint
   is an edge. Either the operating point has moved since that concept, or the
   melt entered at the Hi3DGen/bake stage rather than the concept stage. That is
   worth resolving before 34 minutes of H1: if the defect is downstream, a clean
   C1 concept will not prevent it.

6. **Filing note.** The four existing town reviews live flat in `docs/reviews/`
   (`g1-…`, `g2-…`, `p24-…`, `p30-…`). This report was written to the requested
   `docs/reviews/town/` and so creates a new directory that splits the
   convention; move it up one level if the flat layout is intended.

---

# RUN-C1b re-roll screen — C4, C5, C8, Opus visual gate

Date 2026-08-01. Grid `target/concept-c1b/<slot>/seed_<n>/concept.png`, prompts
from §4 above, operating point unchanged (`prop_concept.json`, z-image-turbo,
8 steps, cfg 1.0, negative branch inert). **All 24 images of C4, C5 and C8 were
opened and viewed**, alongside the corresponding `target/concept-c1/` images for
a direct A/B. Colour claims are measured.

**C1 was not screened.** Its chain (H1) was cancelled after a probe measured the
shipped arch's melted carving as entering at **decimation**, not concept: the
concept is crisp, and 14,999 tri on a 5.5 m object leaves a ~14.5 cm mean
triangle edge that replaces 1–5 cm carving with equal-amplitude faceting noise.
The 8 C1b images under `target/concept-c1b/C1/` are moot and were left unopened.
Everything §1–§6 above says about C1 stands as written.

**U1 is applied in its corrected reading** — *no floor geometry continuous with
the object*, not "no cast-shadow floor". Every image in both grids is a studio
plate: the object stands on a smooth backdrop with a soft cast shadow and no
attached floor. U1 is not a discriminator anywhere in this screen.

---

## 7. Re-roll verdict table

| slot | verdict | A/B vs RUN-C1 | winning seed | carry-forward |
|---|---|---|---|---|
| **C4** shrine pillar | **PASS** | **improved** — the crop is fixed 8/8 and the recess left the reserved hue window | **203** (frontal alt **201**) | `concept-c1b/C4/seed_203/concept.png` |
| **C5** ox cart | **FAIL — regression** | **worse** — period and material identity collapsed; the wheel clause did not move | **none in C1b**; revert to `concept-c1/C5/seed_6` | `concept-c1/C5/seed_6/concept.png` |
| **C8** gate brazier | **PASS** | **improved** — genuine openwork 7/8 (was 1/8), U7 ornament gone, colour result reconfirmed | **407** (alt **406**) | `concept-c1b/C8/seed_407/concept.png` |

---

## 8. C4 — wayside shrine pillar — PASS

### The framing defect is fixed, 8/8

Measured object bounding boxes (largest connected non-backdrop component):

| grid | bottom margin, px, per seed | object share of frame |
|---|---|---|
| RUN-C1 | **2, 2, 2, 2, 2, 2, 2, 2** — bbox runs to y=1021 of 1024 in all 8 | 31.4 – 37.4 % |
| RUN-C1b | **28, 39, 29, 18, 36, 28, 41, 38** | 22.1 – 26.3 % |

`whole … seen entire from cap to foot` landed cleanly. In all eight re-roll
images the plinth, the shaft, the shelf, the recess and the hood are inside the
frame with visible margin. `stage_geometry` can now consume the concept without
extracting a truncated post. This was the sole blocker on the slot and it is
gone.

### The recess colour finding — measured, and it is fixed

The recess interior is the largest dark connected component in the object's
upper 45 %. Reported at three value thresholds to show the reading is not an
artefact of the threshold:

| grid / seed | hue (circ. mean) | S median | % of recess pixels inside the reserved 350–25° window |
|---|---|---|---|
| C1 s2 | 24.9° | **0.727** | **50.2 %** |
| C1 s4 | 24.2° | **0.716** | **64.4 %** |
| C1 s5 | 23.0° | **0.524** | **90.1 %** |
| C1b s201 | 33.5° | 0.511 | 1.2 % |
| C1b s202 | 32.1° | 0.524 | 8.7 % |
| C1b s203 | 32.8° | **0.283** | 10.1 % |
| C1b s204 | 32.0° | 0.514 | 13.6 % |
| C1b s205 | 35.3° | 0.310 | 6.4 % |
| C1b s206 | 30.5° | 0.634 | 12.5 % |
| C1b s207 | 27.9° | 0.551 | 30.5 % |
| C1b s208 | 38.0° | 0.333 | 19.7 % |

Values shift by under 1.5° of hue and under 0.04 of S across V<0.45 / 0.50 /
0.55 on every seed except 207 (see §11).

**The fix is a hue move, not a saturation move.** The first screen's finding was
non-emissive stone sitting *inside* the reserved window at S 0.60+; the re-roll
moves the recess to 30–38°, out of the window and into candle-gold, where the
S ≤ 0.35 in-window cap does not bind. The fraction of recess pixels actually
inside the window falls from 50–90 % on the worst originals to 1–20 %. Seed 203
is best on both axes (S 0.283, hue 32.8°).

**Threat-band pixels: zero in all 8 re-roll images** (RUN-C1 had 5, 2, 0, 5, 0,
**85**, 8, 0). Flames measure hue median 26.0–37.2°, S 0.232–0.382, V 0.776–0.949
on 55–501 px — candle-gold, small, no red.

### Does the recess read at 2 m?

Weber contrast of the recess against the surrounding stone, computed on the
image downsampled to 256 / 128 / 64 px (a 64 px whole object is roughly a 28 m
view; at the 2.3 m walk-up a 2 m post spans ~800 px of a 1080p frame, so 2 m is
the easy end of this range):

seeds 201 **0.43/0.41/0.38**, 202 0.33/0.30/0.26, 203 0.21/0.19/0.17,
205 0.32/0.32/0.28, 206 0.34/0.34/0.29, 207 0.48/0.43/0.40,
208 0.46/0.44/0.41, 204 0.12/0.12/0.13.

Contrast is flat across a 4× resolution sweep on every seed — the read is not a
resolution artefact. Seven of eight hold 0.17–0.44 down to 64 px; only 204 is
weak. **The recess reads at 2 m in 8/8 and at ~28 m in 7/8.**

### Residual defect: the stone is still not dressed limestone

`rough … chisel tool marks` did **not** land. Six of eight read as a coarse
speckled conglomerate/terrazzo — the same defect the first screen recorded, not
improved. Worse, **204 and 208 read as a crystalline white sugar-stone that is
marble-adjacent**, and marble is a §3 explicit exclusion; they are excluded from
selection on that basis alone. Two more carry a period drift the first grid did
not have: **202 and 205 have upswept pagoda eaves on the hood** and read as
Japanese stone lanterns, not Castilian humilladeros. **207 is off-subject** — its
recess contains a brown dome, and there is no slab hood.

**Seed 203 is the only image in either C4 grid whose stone reads as chalky
worked limestone**, with pick-and-chisel texture over a matte pale grey. It is
also the lowest-saturation recess and carries a plain gabled slab hood, a real
reveal and back plane, a plain square post and no ornament. It is a
three-quarter view; **201 is the frontal alternative** (flat slab hood, deepest
reveal, contrast 0.41, recess S 0.511) if the geometry stage turns out to want a
frontal input — see §11.

---

## 9. C5 — ox cart — FAIL, and the re-roll made it worse

**Wheel type, as required: spoked. 8/8, again.** Ten-to-twelve turned spokes, a
hub boss and an iron tyre, exactly as in RUN-C1. Moving `wheel` out of the head
noun, leading with `carreta`, and describing the discs as `flat solid rounds of
joined oak plank` / `unbroken timber discs` changed nothing at all. Under the
standing ruling this is **not disqualifying** — spoked wheels are period-plausible
for 1490s Castile and C6 measured away the thin-geometry risk that motivated the
disc mandate. The slot is judged on cart quality, and that is where it fails.

**The revised prompt regressed the material and the period.** A/B, both grids
opened:

| | RUN-C1 | RUN-C1b |
|---|---|---|
| timber | silvered grey-brown weathered oak, matte — §3's "dark oak … silvered light-grey weathering" almost verbatim | **charred shou-sugi-ban pine** with fiery orange flame-grain; visibly **glossy/lacquered** in 302, 305, 307, 308 |
| draught pole | wooden, grain visible, attached, in 7/8 (s2 detached) | **machined steel tube with a turned ferrule** in 301, 304, 305, 306, 308; wooden in 303, 307 |
| wheels | wooden spokes, wooden felloes, iron tyre, 8/8 | wooden spokes but **pneumatic tread tyres** in 308; hollow steel axle stubs throughout |
| load | grain sacks reading as cloth (plump) | plumper still — reads as **upholstered pillows** in 301, 302, 305, 308 |
| net read | a poor rural cart | a **20th-century agricultural trailer** with a decorative burnt-timber body |

Colour is not what failed, and measurement says so: warm-timber hue 24.4–28.5°
(C1: 25.0–27.5°), S median 0.400–0.448 (C1: 0.373–0.500), in-window-over-S 0.35
pixels 5.5–11.7 % of object (C1: 9.1–18.0 %), **zero threat-band pixels in both
grids**. The two grids are indistinguishable on every number in the rubric. The
regression is entirely in material identity and period, which this rubric has no
metric for.

`silvered light-grey weathering` was kept verbatim and lost to `dark oak, deep
near-black brown`, which the sampler resolved as *charring*; `long straight
draught pole` lost its wood, because the material words in the prompt now all
attach to the wheels and the bed.

**Ruling: revert to the original grid.** Under the standing ruling, RUN-C1's C5
grid passes on cart quality — the first screen already recorded plank bed
legible 8/8, iron strapping crisp, sacks as cloth, no draught animal, no figure —
and its only recorded failure was the wheel clause the ruling now waives.
**Carry `concept-c1/C5/seed_6/concept.png`**: complete cart, wooden pole attached
and ground-reaching, twelve-spoke iron-tyred wheels, three sacks, open plank bed,
silvered oak, the largest frame margins in the grid (t250 b167 l93 r58).
Alternate `concept-c1/C5/seed_3`. Seed 2 stays excluded (detached pole); seed 4
grazes the right frame edge (2 px).

The C1b grid should not be carried forward in any seed.

---

## 10. C8 — gate brazier — PASS, openwork landed, colour reconfirmed

### Openwork: 1/8 → 7/8

| seed | basket |
|---|---|
| 401 | upright bars riveted between two hoops over the upper half, coal visible through the gaps; solid bowl below — **genuine**, same class as C1 seed 8 |
| 402 | **full-height bar cage**, coal bed visible rim to base, blue base flame |
| 403 | top rail with short stub bars over a **solid bowl** — the only weak one |
| 404 | openwork upper band, wide gaps, embers glowing through; no flame above the rim |
| 405 | **full-height bar cage**, coal visible throughout |
| 406 | **full-height bar cage**, plainest legs in the grid, no ornament |
| 407 | **full-height bar cage**, coal bed most legible through the wall, matte black iron |
| 408 | **full-height bar cage**, coal visible, clean gold flame |

Describing the lattice as construction (`built of upright iron bars riveted
between two hoops`, `wide gaps … showing the coal bed through the wall`) rather
than as the adjective `openwork` worked exactly as §4 predicted. Genuine
openwork **7/8**, full-height cage **5/8**, against **1/8** in RUN-C1.

`smooth undecorated sides` also worked: **no seed carries relief or engraving**,
where RUN-C1 had foliate relief on s7 and scrollwork on s5 (**U7 cleared**). Five
seeds add two ring carrying-handles and four have scroll toes on the legs —
forged features, not carving; not a U7 hit, a mild drift off "plain smith work".
Three splayed legs are distinct and meet the bowl in 8/8.

### The colour result is confirmed, not refuted

| | RUN-C1 | RUN-C1b |
|---|---|---|
| flame hue, median | 33.6 – 38.7° | **34.1 – 41.6°** |
| flame hue, p95 | 42.1 – 53.7° | 42.7 – 61.1° |
| flame S / V, median | 0.355 – 0.389 / 0.973–0.976 | 0.306 – 0.436 / 0.969–0.980 |
| flame px below 15° (true red) | 0.01 – 0.40 % | 0.10 – 2.65 % |
| threat-band px (350–25° ∧ S≥0.7 ∧ V≥0.8) | 0, 0, 0, 2, 4, 6, 18, 49 | **0, 1, 14, 20, 52, 55, 182, 485** |
| threat px as % of object | ≤ 0.011 % | ≤ **0.104 %** |
| iron body, V<0.45 subset, mean S | 0.251 – 0.363 | **0.184 – 0.260** (cooler, less rust) |

**Confirmed: the brazier ships lit and the fire is candle-gold.** Flame medians
sit on-hue in the 35–50° band in six of eight and within 1° of it in the rest;
no seed shows red or orange fire, and the iron body is *less* rusty than in
RUN-C1 (the dropped rust clause carried over from C6's edit).

The threat count is the one number that moved the wrong way. The maximum rose
from 49 px to **485 px on seed 404** — 0.10 % of a 468k-pixel object, hue median
23.0°, at the edges of glowing embers in the one seed whose fire has no flame
above the rim to cover the bed. Seeds 406 (0 px), 403 (1 px), 407 (14 px) and
408 (20 px) are at or below the RUN-C1 floor. Consistent with §3's ruling, an
HDR ember core crossing 25° for tens of pixels is not what §2 reserves the band
against; I do not treat it as a U5 failure, but **404 is excluded from selection**
on it.

**Winner: seed 407** — full-height openwork cage with the coal bed most legible
through the wall, matte black iron (S 0.202 at V 0.226), flame hue 36.1° at
S 0.436 (the closest in either grid to §2's candle-gold S 0.55 floor), 14 threat
px. **Alternate 406** — zero threat pixels, plainest legs, no handles, full cage.

### One thing the openwork buys that must be priced

The bars measure **16–25 mm median** at C8's stated 1.0 m height (401 25 mm,
402 24 mm, 405 25 mm, 406 16 mm, 407 24 mm, 408 20 mm; p25 10–14 mm). C8's
budget is **7,000 tri**. Scaling the C1 decimation probe's own figure (14.5 cm
mean triangle edge at 14,999 tri on a 5.5 m object; edge ∝ L/√N) gives a mean
edge of **~3.9 cm at 1.0 m and 7,000 tri** — *wider than the bars themselves*.
The lattice this re-roll successfully produced is, on that estimate, below the
scale decimation preserves, and would come through as the same faceting noise
the arch probe identified. Two cheap responses, either of which is decided
outside this screen: raise C8's tri budget, or carry **401** (openwork as a bold
upper band over a solid bowl, a coarser feature) instead of a full cage. The
estimate rests on one scaling law from one probe and should be confirmed by
measuring the post-decimation mesh, not assumed.

---

## 11. Chain rulings after the re-roll

| chain | slot | ruling | carry |
|---|---|---|---|
| **H4** shrine pillar | C4 | **PROCEED.** The crop blocker is cleared 8/8 and the recess colour finding is resolved by hue. No further re-roll. | `concept-c1b/C4/seed_203/concept.png` (frontal alt `seed_201`) |
| **H6** cart | C5 | **PROCEED on the original grid.** The re-roll is a regression and is discarded. The standing ruling on wheel type releases RUN-C1's grid, which passes on cart quality; the plan's procedural-disc-wheel contingency is **not needed** and no third concept run is warranted. | `concept-c1/C5/seed_6/concept.png` (alt `seed_3`) |
| **H5→** gate brazier | C8 | **PROCEED, lit.** Openwork is now a choice rather than a single seed, U7 is clear, and the colour verdict is reconfirmed on fresh pixels. Price the bar-width/decimation note above before committing the tri budget. | `concept-c1b/C8/seed_407/concept.png` (alt `seed_406`, or `seed_401` if the coarser lattice is preferred) |

All three chains are released. Twelve GPU-minutes of re-roll bought two fixes
and one discard.

---

## 12. What made me doubt this screen

1. **C5's failure is invisible to every number in the rubric.** Hue, saturation,
   threat count and in-window area are statistically identical across the two
   grids, yet one reads as a Castilian cart and the other as a farm trailer with
   a charred body. The rubric has no metric for "modern industrial", and it has
   now missed that failure twice. If C5 mattered more, this would need a
   criterion, not a judgement.

2. **Two re-rolls, one method, opposite outcomes.** C4 and C8 got exactly what
   §4 asked for; C5 did not move on its target adjective and dragged period and
   material down with it. That is the first screen's doubt #3 confirmed, not
   resolved: at cfg 1.0 with no negative channel, an affirmative rewrite either
   hits the model's prior or bounces off it, and nothing in the screen predicts
   which in advance.

3. **My recess and flame masks are pixel thresholds, not geometry.** I showed
   both are stable under a 4× resolution sweep and a threshold sweep, which is
   the honest limit of the claim. **Seed 207 is the exception** — its "recess"
   reading jumps from hue 27.9°/S 0.551 at V<0.45 to hue 42.7°/S 0.074 at
   V<0.50, because the component being measured changes. 207 is off-subject and
   excluded anyway, but the instability is real and is the reason the sweep is
   reported rather than a single number.

4. **I could not settle whether a three-quarter concept costs anything
   downstream.** `prop_hi3dgen.py` feeds `concept.png` to a single-image-to-3D
   model that reconstructs in the image's own camera frame, and the multiview
   retexture calls view 0 "front". If nothing re-yaws the mesh between those two
   stages, a 3/4 concept sets the mesh's front 30–40° off axis. Every carried
   concept in this campaign is three-quarter or near it, so this is not specific
   to C4 seed 203 — but it is unverified, and `seed_201` is named as the frontal
   fallback for exactly that reason.

5. **The C4 material defect is unfixed and I passed the slot anyway.** Six of
   eight are speckled conglomerate and two are marble-adjacent crystalline
   white, against a §3 vocabulary that mandates dressed limestone and names
   marble as excluded. I pass on the strength of one seed. If 203 fails the
   geometry stage there is no second limestone candidate in either grid, and the
   slot would need a third concept run aimed only at material.

6. **The C8 decimation estimate is the screen's weakest number.** It transports a
   scaling law from a 5.5 m arch to a 1.0 m brazier off a single probe, and a
   cage has far more surface per unit bounding volume than a solid arch, which
   pushes the true mean edge in the pessimistic direction. It is a flag, not a
   finding.
