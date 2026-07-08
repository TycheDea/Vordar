# Asset provenance

Every third-party asset that enters `content/` is recorded here with its source,
license, and where it is used. Add a row *when the asset lands*, not after.

| Asset | Source | License | Used for / location |
|---|---|---|---|
| KayKit Adventurers characters (Knight, Mage, Barbarian, Rogue) | Kay Lousberg — https://kaylousberg.itch.io/kaykit-adventurers | CC0 1.0 | Current placeholder player races — `content/source/characters/*.glb` → `content/models/{human,dwarf,elf,valkyrie}.glb`. Slated for replacement by Mixamo characters (AA visual upgrade, Phase 5); sources kept until the user signs off on the new look. |
| DamagedHelmet glTF sample | Khronos glTF-Sample-Assets (model by theblueturtle_) — https://github.com/KhronosGroup/glTF-Sample-Assets | CC BY 4.0 | Renderer test fixture only (PBR/IBL verification) — `content/source/test/DamagedHelmet.glb`. Never shipped in game content. |
| MetalRoughSpheres glTF sample | Khronos glTF-Sample-Assets (Analytical Graphics) — https://github.com/KhronosGroup/glTF-Sample-Assets | CC BY 4.0 | Renderer test fixture only (metallic-roughness verification) — `content/source/test/MetalRoughSpheres.glb`. Never shipped in game content. |

## Incoming (planned, AA visual upgrade)

| Asset | Source | License | Planned use |
|---|---|---|---|
| Mixamo characters + animation clips | Adobe Mixamo — https://www.mixamo.com | Royalty-free for use in games (Adobe terms; no redistribution of raw assets) | Player race characters + clips (Phase 5) — sources in `content/source/characters/mixamo/`, built to `content/models/` |
| PBR texture sets, HDRIs, prop models | Poly Haven — https://polyhaven.com | CC0 | Environment ground/props/sky (Phases 2, 6) |
| PBR texture sets (backup) | ambientCG — https://ambientcg.com | CC0 | Environment textures where Poly Haven lacks a set |
| Particle pack (glows, sparks, smoke) | Kenney — https://kenney.nl/assets/particle-pack | CC0 | VFX texture atlas (Phase 7), runtime-tinted |
