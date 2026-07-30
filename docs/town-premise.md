# Rocalba — Place Premise (start zone)

**Rocalba** ("white rock"): a small Castilian hill town, late 15th century,
rendered as semi-realistic religious dark fantasy (VQ-A1). This document is the
binding place premise for the start-town campaign: **every generated asset cites
this doc**, and **every generation prompt takes its material colors verbatim
from §3**. Concept, texture, and judging stages all read against it.

Register: dry meseta hilltop, whitewash and pale stone, terracotta roofs,
wrought iron. Nothing here is grand — Rocalba was poor, devout, and small.

---

## 1. The premise — emptied at vespers

The bell rang for vespers and the whole town went in to pray — the laborers
from the terraces, the baker with flour still on his hands, the widow with her
mourning candle. Somewhere between the first psalm and the last, everyone in
Rocalba stopped being there. Not killed: *absent*. Doors found barred from the
inside with no one behind them. Supper laid and never eaten. The chapel's
candles lit for the office and never guttering since, though no hand has
trimmed a wick. The town is not ruined; it is **interrupted**, and the
interruption has never ended.

No one lives here now. Travelers use the east road and do not stop after dark.

### What the player sees (the story told without words)

- **Candles burning everywhere** — chapel, window sills, wayside shrines, the
  gate brazier. Dense candle-gold emissives (§2) are the town's signature and
  its wrongness: light that should have died long ago, still faithfully lit.
- **Doors barred from the inside.** Facades are shut: oak doors closed, rejas
  locked, shutters latched. The player never wonders why buildings can't be
  entered — the town itself refuses.
- **Zero NPCs, zero bodies.** Dread without gore: no blood, no corpses, no
  violence. Only evidence of interruption — a bucket rope hanging slack down
  the well, loaves never collected, a cart abandoned mid-load.
- **Mid-ritual stillness.** The one enterable building, the chapel (§6), holds
  the moment itself: votive stands ablaze, the retablo gilt catching the
  candlelight, the office begun and never finished.
- **Time has leaned on the edges.** The perimeter — wall fragments, the
  graveyard, the old ruin cluster — is weathered and broken, while the plaza
  core is eerily kept. Decay increases with distance from the chapel.

---

## 2. Color law (binding — quotes VQ-A4)

All town surfaces are **ambient world**: saturation ≤ 0.35, value ≤ 0.6,
warm stone bias 20°–50° only when chromatic. Emissives are the exceptions:

| Role | Hue | S | V |
|---|---|---|---|
| Candle-gold (flame, gilt glint, shrine glow) | 35°–50° | 0.55–0.85 | 0.75–1.0, flames HDR emissive |
| Votive cool (player VFX only) | 190°–230° | 0.15–0.45 | 0.85–1.0 |
| Threat / telegraph (reserved — never on architecture) | 350°–25° | 0.7–1.0 | 0.8–1.0 |

Consequences for asset work:

- Candle-gold sits ≥ 10° above the threat band; no architectural material may
  glow, and rust/terracotta must stay desaturated enough (S ≤ 0.35) never to
  read as telegraph.
- The texture stage drifts warm when colors are only implied — prompts must
  name the explicit colors given in §3, especially "pale grey", "off-white",
  and "cool grey".

---

## 3. Materials — closed vocabulary (exhaustive)

Assets may use **only** these six materials. The quoted description lines are
written for verbatim use in generation prompts.

**Encalado** — whitewashed lime render over rubble limestone (default wall):
> "off-white matte lime render, chalky bone-white with a cool pale-grey
> undertone, never cream or yellow; hairline cracks; render flaked at bases
> and corners exposing grey-brown rubble limestone; grey rain streaks under
> sills; faint soot shadow above lintels"

**Dressed limestone** — quoins, portals, the chapel, the well, the gate:
> "pale grey dressed limestone ashlar, cool light grey with faint sandy
> flecks, matte; crisp chisel tool-marks softened by wear; edges rounded;
> thin dark-grey soot settled in joints and carving recesses"

**Terracotta barrel tile** — every roof (the one warm material; roofs only):
> "weathered terracotta barrel tiles, muted dusty red-brown, low saturation;
> patchy grey-green lichen; whole courses slipped or missing near eaves"

**Dark oak** — doors, shutters, beams, cart and barrel wood:
> "dark oak, deep near-black brown, silvered light-grey weathering on raised
> grain; deep gouges and checks; dark iron staining around nail heads"

**Wrought iron** — rejas, hinges, candle stands, gate fittings, bell:
> "matte black wrought iron, charcoal-grey worn highlights on edges; sparse
> desaturated brown rust at joints and rivets, never bright orange"

**Worn cobble** — plaza, streets, chapel floor:
> "worn grey cobblestones, mixed cool greys from pale to slate, crowns
> polished smooth; dark earth-brown packed joints; no moss"

**Explicitly excluded** — never generated, never installed: **brick, thatch,
half-timber, marble.** (Brick and half-timber read wrong for a Castilian stone
town; thatch reads northern; marble reads too rich for Rocalba.)

---

## 4. Layout

World axes: **+X east, +Z north**. Everything sits inside **r ≈ 55** of spawn
origin; the play radius hard-clamps at 65 and the ground is flat to r = 70.
Spawn is the plaza center (origin) and must stay clear, as must the portal
corridor east to the portal at (22, 0, 0).

```
                                +Z (north)
          cypress field                 casas — north street wall
       (-48,42) (-42,48)          (inward facades, backs never seen)
                        \      _______ _______ ______
                         \    |casa   |casa   |casa  |
                          \    ¯¯¯¯|       |¯¯¯|  ¯¯¯
                               ____|       |___
   -X (west)                  |        PLAZA MAYOR       east road   gate   portal
                              |     [spawn (0,0,0)]    ============ arch == (22,0,0)
                              |      [well ~(0,-6)]      crucero    ~(15,0)   → east
                               ¯¯¯¯|       |¯¯¯          (15,6)      wall     zone
                          ____ ____|       |____          shrine   fragments
                         |casa|casa |casa |casa |          on the   N and S
                          ¯¯¯¯ ¯¯¯¯  ¯¯¯¯  ¯¯¯¯           approach  of gate
              graveyard        casas — south street wall
           (-31,-27) area
          ┌────────────┐
          │ CHAPEL     │  old ruin cluster: arch (-26,-34),
          │ ~(-30,-30) │  broken column, gravestones, cypress
          └────────────┘
                                -Z (south)
```

Zones (approximate centers):

| Zone | Where | Contents |
|---|---|---|
| Plaza mayor | origin, r ≈ 12 | well basin offset south of spawn; cart and barrel dressing at the edges; widest open space |
| Street walls | N and S of plaza | 8–10 casas as continuous facades — party walls touching, no gaps that expose building backs; streets wide enough for the orbit camera |
| Chapel precinct | SW, ~(-30, -30) | the chapel (§6), graveyard, existing ruin cluster and gravestones as the older, more decayed quarter |
| East gate | ~(15, 0) astride the road | gate arch + wall fragments running N and S; the existing crucero at (15, 6) becomes the gate shrine |
| Approaches | road edges | wayside shrines (crucero, shrine niche) on the east road and the chapel path |
| Perimeter | r ≈ 40–55 | graveyard spill, cypress lines (existing cypress pairs), olive stumps, half-quarried wall fragments fading into open field |

The existing start-zone dressing (ruin cluster, gravestones, cypresses, olive
stumps, crucero) is retained and re-read as Rocalba's fabric: the SW ruins are
the town's older, longer-dead quarter beside the chapel. Beyond the east gate,
the road leads to the east zone, whose existing dressing — worn cobble, ruined
gate fragment, toppled colonnade — reads as Rocalba's abandoned edge.

Buildings are visuals-only props with mirrored collision prefabs; facades face
**inward** to the street so the orbit camera never clips through open backs.

---

## 5. Building register (~10 kit types)

| Type | Premise — who, and what vespers left behind |
|---|---|
| casa_small_A | Day-laborer's one-room house; supper laid, hearth cold, door barred from inside |
| casa_small_B | Widow's house; a single mourning candle still lit in the window reja |
| casa_two_story | Wool merchant's house; upper shutters latched, a bale abandoned on the balcony hoist |
| casa_corner | Baker's house with street oven; loaves set out to cool and never collected |
| wall segment | Town wall, never finished — courses half-quarried, gaps filled with rubble |
| east gate arch | The only formal entrance; gates open (never shut that night), porter's brazier still lit |
| chapel | The parish chapel, mid-vespers (§6) — the one enterable building |
| well basin | Plaza well; bucket rope hanging slack down the shaft, no bucket recovered |
| reja set | Window and gate grilles, wrought iron; every one intact and locked — nothing broke in, nothing broke out |

Casa doors, shutters, and rejas are always closed. State of repair follows §1:
plaza-facing facades kept, perimeter pieces weathered and broken.

---

## 6. The chapel — the one enterable landmark

Anchors the SW precinct. Nave **~7 × 16 m**, stone barrel vault springing to
**~10–12 m**. Dressed pale-grey limestone throughout (§3), worn cobble floor,
dark oak west doors standing open — the only open door in Rocalba.

- **Partially broken vault**: a collapse over the western nave bays leaves the
  vault open to the sky, admitting light shafts; rubble from the fall lies
  swept aside, not cleared. The east end and its vaulting are intact.
- **Retablo** at the east end: dark oak frame, painted panels in ambient-world
  values, gilt details that glint candle-gold (35°–50°) — the richest surface
  in town, and still modest.
- **Votive stands** flanking the retablo and along the nave piers: wrought
  iron, dense with lit candles — the strongest candle-gold emissive cluster in
  the zone, HDR flames per VQ-A4/VQ-C3.
- **Mid-ritual dressing**: the office interrupted — benches in place, nothing
  overturned, no damage newer than the old vault fall. The dread is stillness,
  not wreckage.
- Interior wall surfaces above the candle line carry smoke-darkened plaster:
  encalado (§3) with a soot-grey gradient rising to the vault.

Lighting intent: cool sky shafts through the breach against warm candle-gold
below — the votive/candle opposition of VQ-A4 staged in one room.

---

## 7. Prompt contract

Every generation prompt for this campaign must:

1. Cite the premise: `place: Rocalba (docs/town-premise.md)`.
2. Take material color text verbatim from §3 — never paraphrase colors.
3. Use only §3 materials; never brick, thatch, half-timber, or marble.
4. Keep surfaces in the ambient-world band (§2); emissives only as candle-gold.
5. State the vespers condition where it shows: doors barred, candles lit,
   no figures, no gore.
