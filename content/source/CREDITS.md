# Asset provenance

Every third-party asset that enters `content/` is recorded here with its source,
license, and where it is used. Add a row *when the asset lands*, not after.

| Asset | Source | License | Used for / location |
|---|---|---|---|
| KayKit Adventurers characters (Knight, Mage, Barbarian, Rogue) | Kay Lousberg — https://kaylousberg.itch.io/kaykit-adventurers | CC0 1.0 | Placeholder races dwarf/elf/valkyrie — `content/source/characters/*.glb` → `content/models/{dwarf,elf,valkyrie}.glb`. The human was replaced by the VRoid body (below); remaining races follow once the direction is locked. |
| Mixamo auto-rig + 11 animation clips | Adobe Mixamo — https://www.mixamo.com | Royalty-free for use in games (Adobe terms; no redistribution of raw assets) | Rigging service for the VRoid body + shared clip library — `content/source/characters/mixamo/{Character.fbx,clips/}`, merged by `scripts/asset-pipeline/mixamo_to_glb.py` into `content/models/human.glb` |
| DamagedHelmet glTF sample | Khronos glTF-Sample-Assets (model by theblueturtle_) — https://github.com/KhronosGroup/glTF-Sample-Assets | CC BY 4.0 | Renderer test fixture only (PBR/IBL verification) — `content/source/test/DamagedHelmet.glb`. Never shipped in game content. |
| MetalRoughSpheres glTF sample | Khronos glTF-Sample-Assets (Analytical Graphics) — https://github.com/KhronosGroup/glTF-Sample-Assets | CC BY 4.0 | Renderer test fixture only (metallic-roughness verification) — `content/source/test/MetalRoughSpheres.glb`. Never shipped in game content. |
| Evening Road 01 (Pure Sky) HDRI, 2k | Poly Haven (Jarod Guest / Sergej Majboroda) — https://polyhaven.com/a/evening_road_01_puresky | CC0 | Zone sky + IBL environment — `content/textures/env/evening_road_01_puresky_2k.hdr` |
| Brown Mud Leaves 01 texture set, 2k | Poly Haven (Rob Tuytel) — https://polyhaven.com/a/brown_mud_leaves_01 | CC0 | Zone ground PBR set — `content/textures/ground/mud_leaves/` |
| Rock 07, Rock 09, Rock Face 01, Dead Quiver Trunk models, 1k | Poly Haven (Rico Cilliers / Dimitrios Savva) — https://polyhaven.com/models | CC0 | Zone props — `content/models/props/` (fetched via `scripts/asset-pipeline/fetch_polyhaven.mjs`) |
| "Human - Male_" VRoid character (user-authored) | Created in VRoid Studio by the project owner | Owner's asset (VRoid Studio output belongs to its creator, commercial use allowed) | Character-direction look-test — `content/source/characters/vroid/Human - Male_.vrm` → `content/models/statue_vroid.glb` (via `scripts/asset-pipeline/vrm_to_glb.mjs`), placed as a start-zone statue |

## Incoming (planned, AA visual upgrade)

| Asset | Source | License | Planned use |
|---|---|---|---|
| PBR texture sets, HDRIs, prop models | Poly Haven — https://polyhaven.com | CC0 | Environment ground/props/sky (Phases 2, 6) |
| PBR texture sets (backup) | ambientCG — https://ambientcg.com | CC0 | Environment textures where Poly Haven lacks a set |
| Particle pack (glows, sparks, smoke) | Kenney — https://kenney.nl/assets/particle-pack | CC0 | VFX texture atlas (Phase 7), runtime-tinted |
