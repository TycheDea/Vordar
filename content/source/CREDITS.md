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

## AI pipeline models (governance ledger)

Every model the AI content pipeline (Phase A) touches, cleared before it's allowed to
generate anything that ships. Nothing generates from a `Blocked` or `Pending` row.

| Model | Version / repo | License | Verdict | Source | Basis |
|---|---|---|---|---|---|
| SDXL base | `stabilityai/stable-diffusion-xl-base-1.0` | CreativeML Open RAIL++-M | Cleared | https://huggingface.co/stabilityai/stable-diffusion-xl-base-1.0 | HF `license` field = `openrail++`; commercial use permitted, subject to Attachment A content-based use restrictions. |
| FLUX.1-schnell | `black-forest-labs/FLUX.1-schnell` | Apache 2.0 | Cleared | https://huggingface.co/black-forest-labs/FLUX.1-schnell | HF `license` field = `apache-2.0`, no use restrictions. |
| ControlNet pose (xinsir, SDXL) | `xinsir/controlnet-openpose-sdxl-1.0` | Apache 2.0 | Cleared | https://huggingface.co/xinsir/controlnet-openpose-sdxl-1.0 | HF `license` field = `apache-2.0` — more permissive than the OpenRAIL++ base it targets; not the OpenRAIL-family tag the subplan expected. |
| ControlNet depth (xinsir, SDXL) | `xinsir/controlnet-depth-sdxl-1.0` | Apache 2.0 | Cleared | https://huggingface.co/xinsir/controlnet-depth-sdxl-1.0 | Same finding as pose — HF `license` field = `apache-2.0`. |
| Pano checkpoint | `ProGamerGov/sdxl-360-diffusion` | None stated (SDXL fine-tune, inherits base obligations at minimum) | Pending | https://huggingface.co/ProGamerGov/sdxl-360-diffusion | HF API returns no `license` field. Confirmed empty, not a fetch error. It's an SDXL-base fine-tune checkpoint (inherits OpenRAIL++ at minimum) but the fine-tuner added no terms of their own. Download for infra/eval only; do not generate shippable assets with it until this is resolved with the author or a community consensus citation. |
| TRELLIS core | `microsoft/TRELLIS` | MIT | Cleared | https://github.com/microsoft/TRELLIS/blob/main/LICENSE | Root `LICENSE` = MIT (Microsoft Corporation). The `microsoft/TRELLIS-image-large` checkpoint is separately tagged `license:mit` on HF. |
| TRELLIS submodule — diffoctreerast | `JeffreyXiang/diffoctreerast` (installed by TRELLIS's `setup.sh`, not a `.gitmodules` entry) | Custom non-commercial research license (derivative of the Inria/MPII gaussian-splatting license) | Blocked | https://github.com/JeffreyXiang/diffoctreerast/blob/master/LICENSE | LICENSE text bars commercial use/distribution without prior written consent. Confirmed research-only. Scope check: only `trellis/renderers/octree_renderer.py` (radiance-field preview rendering) imports it; the glTF mesh-extraction path (`postprocessing_utils.to_glb`, which uses Flexicubes + `GaussianRenderer` + `nvdiffrast`) does not import diffoctreerast, so game-shipped mesh output is unaffected. |
| TRELLIS submodule — Modified Flexicubes | `MaxtirError/FlexiCubes` (`.gitmodules` entry at `trellis/representations/mesh/flexicubes`) | Apache 2.0 | Cleared | https://github.com/MaxtirError/FlexiCubes/blob/main/LICENSE.txt | Root `LICENSE.txt` = Apache 2.0 (NVIDIA Corporation copyright notice). This is the submodule the mesh-extraction path (glTF output) actually depends on. |
| UniRig | `VAST-AI/UniRig` | MIT | Cleared | https://huggingface.co/VAST-AI/UniRig | HF `license` field = `mit`. |
| MatForger | `gvecchio/MatForger` → repo no longer exists (HF returns 404); superseded by `gvecchio/StableMaterials` (same architecture, per author's model card) | CreativeML Open RAIL-M (via HF `openrail` tag; no separate LICENSE file in the repo) | Cleared, on the successor repo | https://huggingface.co/gvecchio/StableMaterials | **Divergence, flagged per project policy:** `gvecchio/MatForger` 404s — confirmed via direct fetch (HTML `og:title` = "404 – Hugging Face"), not a transient error. The named successor `StableMaterials` is tagged `license:openrail`, which HF resolves to the CreativeML Open RAIL-M text (same family as SDXL's RAIL++-M): commercial use permitted subject to Attachment A use restrictions — not "research purposes" as the stale/generic model-card phrasing implied. Confirm with whoever scoped A1 that `StableMaterials` is the intended replacement before using it. |
| Ubisoft CHORD | `ubisoft/ubisoft-laforge-chord` | Ubisoft Machine Learning License (Research-Only, Copyleft) | Blocked | https://github.com/ubisoft/ubisoft-laforge-chord/blob/main/LICENSE | Re-verified: LICENSE text states "Commercial use is strictly prohibited" and requires derivatives to be redistributed "under the same exact terms as this License" (copyleft). |
| DiT360 | `Insta360-Research-Team/DiT360` (LoRA adapter for `black-forest-labs/FLUX.1-dev`) | Adapter: MIT. Base model: FLUX.1-dev Non-Commercial License | Blocked | https://github.com/Insta360-Research-Team/DiT360/blob/main/LICENSE ; https://github.com/black-forest-labs/flux/blob/main/model_licenses/LICENSE-FLUX1-dev | Re-verified: adapter's own LICENSE is MIT, but FLUX.1-dev's license restricts use to "Non-Commercial Purposes" (§1c/§2b) and bars "any commercial or production purposes" (§4a). A clean adapter license doesn't clear a non-commercial base-model dependency. |
| Hunyuan3D | `Tencent-Hunyuan/Hunyuan3D-2` (also 2.1) | Tencent Hunyuan 3D Community License Agreement | Blocked | https://github.com/Tencent-Hunyuan/Hunyuan3D-2/blob/main/LICENSE | Re-verified: license text excludes "the European Union, United Kingdom and South Korea" from its licensed Territory; confirmed present in both the 2.0 and 2.1 LICENSE files. |
| HY-Motion | `Tencent-Hunyuan/HY-Motion-1.0` | Tencent HY-MOTION 1.0 Community License Agreement | Blocked | https://huggingface.co/tencent/HY-Motion-1.0/blob/main/LICENSE.txt | Re-verified: same EU/UK/South Korea territory exclusion as Hunyuan3D, confirmed via the repo's `LICENSE.txt`. |
