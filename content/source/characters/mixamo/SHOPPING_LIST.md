# Mixamo shopping list — AA player characters (Phase 5 inputs)

Mixamo has no API, so these are manual downloads from https://www.mixamo.com
(free Adobe account). Everything below is searchable by the exact name given.
Total: 4 characters + 11 animation clips shared across all races
(~15 minutes of clicking). The build pipeline (Blender CLI) does the rest.

## Download settings

**Characters** (one per race, downloaded from the character page with no
animation selected — T-Pose):
- Format: **FBX Binary (.fbx)**
- Pose: **T-Pose**

**Animations** (select the character first so retargeting previews correctly —
any character works, clips are downloaded *without skin*):
- Format: **FBX Binary (.fbx)**
- Skin: **Without Skin**
- Frames per Second: **30**
- Keyframe Reduction: **none**
- **In Place: checked** whenever the option exists (walk/run/attacks)

## Folder layout (create as you download)

```
content/source/characters/mixamo/
  human/     Character.fbx + the 11 clips
  elf/       Character.fbx        (clips shared — see note)
  dwarf/     Character.fbx
  valkyrie/  Character.fbx
  clips/     the 11 animation FBX files (downloaded once, without skin)
```

**Note:** animation FBXes are skeleton-only and Mixamo rigs share the same
skeleton, so download each clip **once** into `clips/` — the pipeline retargets
them onto all four characters. Only the 4 character FBXes are per-race.

## Characters (search name → save as)

| Race | Mixamo search | Save as | Why |
|---|---|---|---|
| human | **Paladin J Nordstrom** | `human/Character.fbx` | Full plate, grounded medieval read — the baseline knight |
| elf | **Erika Archer Without Bow Arrow** | `elf/Character.fbx` | Slender ranger silhouette; the "without bow" variant avoids a welded prop |
| dwarf | **Castle Guard 01** | `dwarf/Character.fbx` | Stocky heavy-armor build; pipeline height-normalizes him short |
| valkyrie | **Nightshade J Friedman** | `valkyrie/Character.fbx` | Armored female warrior silhouette |

If a listed character is missing/renamed, pick the closest armored fantasy
character — realistic proportions, no welded weapons, no modern clothing.

## Animation clips (download once, into `clips/`)

| Slot | Mixamo search | Save as | Settings notes |
|---|---|---|---|
| idle | **Sword And Shield Idle** | `clips/idle.fbx` | subtle combat-ready sway |
| walk | **Sword And Shield Walk** | `clips/walk.fbx` | In Place ✓ |
| run | **Sword And Shield Run** | `clips/run.fbx` | In Place ✓ |
| attack 1 (melee) | **Sword And Shield Slash** | `clips/attack_slash.fbx` | In Place ✓ if offered |
| attack 2 (heavy) | **Standing Melee Attack Downward** | `clips/attack_heavy.fbx` | In Place ✓ if offered |
| attack 3 (cast) | **Standing 2H Magic Attack 01** | `clips/attack_cast.fbx` | projectile/bolt cast |
| hit | **Standing React Small From Front** | `clips/hit.fbx` | flinch react |
| death | **Standing Death Forward 01** | `clips/death.fbx` | |
| jump/leap | **Standing Jump** | `clips/leap.fbx` | for the leap ability; In Place ✓ |
| dodge | **Standing Dodge Backward** | `clips/dodge.fbx` | spare slot, cheap to grab now |
| t-pose ref | **T-Pose** | `clips/tpose.fbx` | retarget sanity reference |

Search results vary slightly by account/catalog version; if an exact name is
missing, any clip with the same verb works — keep the save-as filename, the
pipeline keys on filenames, not Mixamo names.

When done, say the word and Phase 5 proceeds: Blender CLI converts and merges
these into per-race `.glb` files; nothing else is manual.
