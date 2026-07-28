# AI content pipeline (Phase A)

Runbook for the local tools used to generate 2D/3D source assets. Governance
rules (what's cleared to ship) live in `content/source/CREDITS.md`, not here.

## ComfyUI

Install location: `C:\tools\ComfyUI\` (portable build, v0.28.0). Models live
under `C:\tools\ComfyUI\ComfyUI\models\`.

**Headless start** (never `run_nvidia_gpu.bat` — it passes
`--windows-standalone-build`, which auto-launches a browser):

```
C:\tools\ComfyUI\python_embeded\python.exe -s C:\tools\ComfyUI\ComfyUI\main.py --listen 127.0.0.1 --port 8188
```

Server is up when `GET http://127.0.0.1:8188/system_stats` returns 200.

**Stop:** kill the `python.exe` process whose command line contains
`ComfyUI\main.py` (e.g. `Get-CimInstance Win32_Process -Filter "name='python.exe'" | Where-Object { $_.CommandLine -like '*ComfyUI\main.py*' } | Stop-Process`).

### Model inventory

`C:\tools\ComfyUI\ComfyUI\models\`:

| Folder | Files |
|---|---|
| `checkpoints\` | `sd_xl_base_1.0.safetensors`, `flux1-schnell-fp8.safetensors`, `sdxl_360_diffusion.safetensors` |
| `controlnet\` | `controlnet-openpose-sdxl-1.0.safetensors`, `controlnet-depth-sdxl-1.0.safetensors`, `Qwen-Image-InstantX-ControlNet-Union.safetensors` |
| `text_encoders\` | `clip_l.safetensors`, `t5xxl_fp8_e4m3fn.safetensors`, `qwen_3_4b_fp8_mixed.safetensors`, `qwen_2.5_vl_7b_fp8_scaled.safetensors` |
| `vae\` | `ae.safetensors`, `qwen_image_vae.safetensors` |
| `diffusion_models\` | `z_image_turbo_bf16.safetensors`, `qwen_image_fp8_e4m3fn.safetensors` |
| `model_patches\` | `Z-Image-Turbo-Fun-Controlnet-Union.safetensors` |

`flux1-schnell-fp8.safetensors` is the Comfy-Org all-in-one checkpoint
(UNet+CLIPs+VAE bundled) — load it with `CheckpointLoaderSimple`, not the
modular UNet/CLIP/VAE loader chain. The standalone `clip_l`/`t5xxl` text
encoders are kept for a future modular path. `ae.safetensors` is on disk —
fetched as the Z-Image concept/multiview VAE (`workflows/prop_concept.json`,
`workflows/prop_multiview.json`), not for that modular path. The four
`qwen_*`/`Qwen-Image-*` files are the Qwen-Image bake-off arm, kept as the
documented fallback base (2026-07-20 ruling,
`tasks/ai-pipeline/research/a5b-bakeoff-results.md`).

SHA256 for every downloaded file: `scripts/ai-pipeline/models.sha256`.

License verdicts: `content/source/CREDITS.md` → "AI pipeline models"
table. The pano checkpoint (`sdxl_360_diffusion.safetensors`) is eval-only —
license Pending, do not generate shippable assets with it.

### `check_models.py` — confirm ComfyUI sees every downloaded model

With the server running headless:

```
python scripts/ai-pipeline/check_models.py
```

Queries `GET /models/<folder>` for `checkpoints`, `controlnet`,
`text_encoders`, `vae`, `diffusion_models` and diffs against the expected
filename set. Exit 0 = all present, no extras; non-zero prints what's
missing/extra.

### `comfy_run.py` — submit a workflow, collect outputs + provenance

```
python scripts/ai-pipeline/comfy_run.py <workflow.json> --out <dir> [--wait-timeout SEC]
```

With the server running headless, POSTs the workflow JSON (API format) to
`/prompt`, polls `/history/<prompt_id>` until complete, downloads every
output file to `<dir>/`, and writes `<dir>/manifest.json`.

Workflow JSONs live in `scripts/ai-pipeline/workflows/`. `smoke.json` is a
`EmptyImage → SaveImage` pair that needs no model — use it to verify the
server/CLI wiring without GPU-heavy inference:

```
python scripts/ai-pipeline/comfy_run.py scripts/ai-pipeline/workflows/smoke.json --out <tmp-dir>
```

**Provenance manifest contract** (`manifest.json`): every run's asset is
reproducible from this file alone.

| Key | Content |
|---|---|
| `workflow` | The submitted workflow JSON, with negative seed sentinels resolved to their concrete value |
| `prompt_id` | ComfyUI's prompt id for this run |
| `prompts` | Text of every `CLIPTextEncode` node, keyed by node id |
| `seed` | Resolved `seed`/`noise_seed` value per node id |
| `models` | Every `*Loader` node's filename input, cross-referenced against `models.sha256` for its sha256. A model missing from the manifest is a refusal, not a `null`: the texture pipeline's stage keys hash these, so an unhashed model would leave the cache blind to a model swap |
| `outputs` | Each saved output file: node id, output kind, filename, subfolder, type, local save path |

## TRELLIS (image → 3D, eval-only)

Install location: `C:\tools\TRELLIS\` — native Windows, via the
`IgorAherne/trellis-stable-projectorz` fork (commit `90c829d`) of
`microsoft/TRELLIS`, chosen over `nitinmukesh/TRELLIS-for-windows` and the
WSL2 fallback because it ships prebuilt cp311/cu128 wheels (no MSVC builds
required). Venv: `C:\tools\TRELLIS\venv` (Python 3.11.9, torch
2.7.1+cu128).

Runtime env vars (required for every run):

```
ATTN_BACKEND=xformers
SPCONV_ALGO=native
```

`TRELLIS-image-large` checkpoint is pre-fetched into the standard
HuggingFace cache (`~/.cache/huggingface/hub/models--microsoft--TRELLIS-image-large`,
3.3 GB) — `from_pretrained("microsoft/TRELLIS-image-large")` resolves it
from there, no explicit path needed.

The installed xformers wheel now has compiled CUDA kernels:
`xformers==0.0.31.post1` (same release line as torch 2.7.1, `cu128` index).
Fresh-machine reinstall (`--no-deps --force-reinstall` swaps only this one
package, torch untouched):

```
C:\tools\TRELLIS\venv\Scripts\python.exe -m pip install --force-reinstall --no-deps xformers==0.0.31.post1 --index-url https://download.pytorch.org/whl/cu128
```

**TRELLIS is eval-only** under the strict NC-tooling ruling (2026-07-19,
`content/source/CREDITS.md` → "AI pipeline models" → TRELLIS core row): its
only glb export path (`trellis/utils/postprocessing_utils.py`) hard-imports
`nvdiffrast.torch` (NC-licensed) to bake the texture, so its outputs never
enter `content/`. Hi3DGen (below) is the production image→3D backbone
precisely because its geometry-only chain has no NC dependency anywhere.
TRELLIS stays installed as a baseline: a one-off textured run under
`target/trellis-eval/` sanity-checks Hi3DGen's geometry and the A3 texture
stage's material register against TRELLIS's native (NC) bake — comparison
in `tasks/ai-pipeline/a3.md`'s decision log. Fork quirk:
`postprocessing_utils.py` imports `api_spz` from the repo root, so eval
runners execute with `cwd=C:\tools\TRELLIS`.

`diffoctreerast` (one of TRELLIS's vendored submodules) has a broken DLL on
this install — irrelevant here, since its license is Blocked anyway and it
sits outside the glTF mesh-extraction path TRELLIS actually uses.

## StableMaterials (tiling PBR materials)

Install location: `C:\tools\StableMaterials\venv` (Python 3.11.9). Versions
actually installed (no pins needed): torch `2.11.0+cu128`, torchvision
`0.26.0+cu128`, diffusers `0.39.0`, transformers `5.14.1`, accelerate
`1.14.0`, `opencv-python-headless` `5.0.0.93` (added for `hdr_post.py`'s
Radiance `.hdr` writer — see below).

Weights: `C:\tools\StableMaterials\weights` (`gvecchio/StableMaterials`,
fetched with `--local-dir`, 5.1 GB on disk; `unet_lcm/` excluded — the
optional fast-inference distilled UNet, unused by the default pipeline).
SHA256 for every downloaded file: `scripts/ai-pipeline/models.sha256`.

### texconv.exe (DDS-bake prerequisite)

`bake_textures.mjs` (used by the bake step below) locates it via the
`TEXCONV` env var, else `smirk\texconv.exe` — gitignored, machine-local, not
a repo asset. Restore on a fresh machine:

```
winget install Microsoft.DirectXTex.Texconv
```

then copy the installed exe (`Get-Command texconv.exe` once winget's shim is
on PATH, or check `$env:LOCALAPPDATA\Microsoft\WinGet\Links\`) to
`smirk\texconv.exe`. If winget is unavailable, get it from
https://github.com/microsoft/DirectXTex/releases instead.

### `gen_material.py` — generate a tileable ground PBR set

```
C:\tools\StableMaterials\venv\Scripts\python.exe scripts/ai-pipeline/gen_material.py "<prompt>" --out <dir> [--size N] [--seed N]
```

`--size` default 2048; `--seed` default random (the resolved value is always
recorded). Generation is always native at 512×512 — StableMaterials' output
is crispest there (1024-native generation washes out its crack-network
structure) and `tileable=True`'s circular padding can't run below it. Sizes
above 512 are reached by chained ×2 whole-canvas SDXL img2img hops
(`sd_xl_base_1.0.safetensors`) applied to the albedo (`diff`) map only — a
normal map is a tangent-space unit vector and roughness a physical scalar,
so diffusion "detail" on either would be wrong rather than just noisier;
both get a plain Lanczos resize to the target size instead.
`enable_model_cpu_offload()` on the SDXL pipe is mandatory on a 12 GB card:
without it the 2048 hop spills into WDDM shared memory (~28 min); with it,
~4 min (~4.5 min wall time for a full 2048 set end to end).

Tileability seam gate: an 8px edge strip per map (left/right, top/bottom),
mean-abs-pixel-diff threshold 20, PASS/FAIL printed per map/edge pair; any
FAIL exits 1 — retry with a different `--seed` rather than treat one
failure as broken. Writes `<out>/generation_manifest.json` (model, prompt,
seed, size, native_generation_size, upscale_hops, upscale_model,
guidance_scale, num_inference_steps, tiling_check) — provenance for the
generation step only, distinct from `bake_textures.mjs`'s own
`manifest.json` written into the same directory by the next step.

**Preview recipe:** only `--size 512` previews (~25 s) predict final
structure at any target size — 1024-native previews wash out. Preview at
512, then finalize at the same `--seed` and the real `--size` once the seam
gate passes.

### Bake, lint, render loop

```
node scripts/asset-pipeline/bake_textures.mjs ground <texture-dir>
```

Needs `texconv.exe` (above). Produces `diff_<N>.dds` (BC7_UNORM_SRGB),
`nor_gl_<N>.dds` (BC5_UNORM), `rough_<N>_mr.dds` (BC7_UNORM, composed from
the roughness map via a `0r01` texconv swizzle), and `manifest.json`
(`{source, images}`) alongside the source PNGs.

```
cargo test -p vordar-game --test content_lint
```

`material_textures_have_fresh_sidecars` (VQ-C5) asserts the manifest exists,
its recorded sha256 matches each source file's current hash, and every
listed `.dds` sidecar exists on disk.

```
cargo run -p vordar-client --release --features offscreen --bin render_material -- <texture-dir> --out <dir> [--angles N] [--size WxH]
```

`--angles` default 4, `--size` default `512x512`. Requires `--features
offscreen` (`vordar-client`'s own feature, forwarding to
`engine-renderer/offscreen`; a plain `cargo build -p vordar-client` skips
the bin entirely). Writes `frame_NN.png` per angle plus a stitched
`contact_sheet.png` for vision review.

`content/textures/ground/cracked_earth/` is the committed A1 fixture built
with this pipeline; its provenance row lives in
`content/source/CREDITS.md`'s asset-provenance table. Both models' license
verdicts (StableMaterials — Cleared, `openrail`, listed under the
superseded `gvecchio/MatForger` row; SDXL base — Cleared, OpenRAIL++)
already exist in that file's "AI pipeline models" ledger — no new ledger
row for either model, only the fixture's own asset-provenance row.

## HDRI / skybox generation (Phase A2)

Three pano-generation paths were bake-off'd against a shared post stage
(`hdr_post.py`) and reviewed in-engine via `turntable --hdri`. Full
generation-budget accounting, per-path stats, and the reference-HDRI
calibration table live in `tasks/ai-pipeline/a2.md`.

### Bake-off outcome

**Winner: Path 3** (`gen_pano_sdxl.py`, circular-x SDXL) — the only path
passing `hdr_post.py`'s seam gate on both bake-off prompts (0.0197 / 0.0126
vs. the 0.02 gate); best in-engine light (neutral ambient, crisp
directional sun) and unconditionally licensed (SDXL OpenRAIL++). This is
the production path.

**Runner-up: Path 2** (`gen_pano_d360.py`, Diffusion360) — the best single
seam and fastest (~33 s/run), but failed the seam gate on the overcast
prompt (0.075) and floods the IBL with oversaturated ambient on the dusk
prompt. Kept as an alternate for dramatic-sky one-offs.

**Path 1** (`sdxl_360_diffusion.safetensors` via ComfyUI,
`workflows/pano_sdxl360.json` + `comfy_run.py`) — blocked at the machine
gate: seam MAD 0.175 / 0.090, since vanilla ComfyUI sampling has no
circular-padding machinery and raw equirects don't wrap. Its license
(Cleared, conditional — `content/source/CREDITS.md`) is moot as long as it
stays blocked. Option kept open: run the sdxl_360 checkpoint through
`gen_pano_sdxl.py`'s circular-x path via `from_single_file` once the
license is formalized (see `tasks/ai-pipeline/sdxl360-license-request.md`);
the checkpoint stays on disk meanwhile.

Full per-candidate notes: `tasks/ai-pipeline/a2.md` → "Bake-off decision
log".

### `gen_pano_sdxl.py` — Path 3, production (StableMaterials venv)

```
C:\tools\StableMaterials\venv\Scripts\python.exe scripts/ai-pipeline/gen_pano_sdxl.py "<prompt>" --out <dir> [--seed N]
```

Vanilla SDXL (`sd_xl_base_1.0.safetensors`, already on disk) has no native
tiling/circular nodes, so wrap-tileability comes from a context manager
that monkeypatches `torch.nn.Conv2d.forward` globally for the duration of
each pipeline call: random even x-roll, `F.pad` circular in x only (y
keeps the conv's native zero padding), run the original forward, crop back
by `round()` of the padding scaled to the conv's width change, then
unroll. Because the patch is global while active it covers the UNet *and*
the VAE encode/decode, so both latent and pixel space wrap in x.

Generation is native **1536×768** (SDXL's native area budget — going
straight to 2048×1024 degrades the output, A1's lesson), 40 steps, cfg 7,
then one whole-canvas img2img hop to **2048×1024** (strength 0.35, 40
steps, cfg 7.0). Both passes run under `enable_model_cpu_offload()`
(mandatory on a 12 GB card at this size) with `vae.enable_tiling()` off
(tiling would cut the x-wrap the conv patch builds). An inline wrap-seam
check (8 px left/right strips, MAD threshold 20) gates the output — FAIL
exits 1. Writes `<out>/pano_2048x1024.png` + `<out>/generation_manifest.json`.

### `gen_pano_d360.py` — Path 2, alternate (Diffusion360 venv)

```
C:\tools\Diffusion360\venv\Scripts\python.exe scripts/ai-pipeline/gen_pano_d360.py "<prompt>" --out <dir> [--seed N]
```

Diffusion360 (`archerfmy0831/sd-t2i-360panoimage`, code
`ArcherFMY/SD-T2I-360PanoImage` @ `3e980d2`) needs its own venv — its
pinned `diffusers==0.26.0` is two majors behind the StableMaterials venv's
`0.39.0` and the two can't share an environment (`pipeline_sr.py` imports
`LoraLoaderMixin`, removed from modern diffusers):

```
C:\Users\egm_8\AppData\Local\Programs\Python\Python311\python.exe -m venv C:\tools\Diffusion360\venv
C:\tools\Diffusion360\venv\Scripts\python.exe -m pip install torch==2.7.1 --index-url https://download.pytorch.org/whl/cu128
C:\tools\Diffusion360\venv\Scripts\python.exe -m pip install diffusers==0.26.0 transformers==4.44.2 huggingface_hub==0.25.2 accelerate safetensors
```

`huggingface_hub==0.25.2` is a required pin, not incidental: `diffusers
0.26.0` calls `cached_download`, which is removed in hub 0.26. Weights live
in `C:\tools\Diffusion360\weights\{sd-base,sr-base,sr-control}` (~12.4 GB;
`sd-i2p/` and the RealESRGAN checkpoint excluded, see divergence 3 in
`tasks/ai-pipeline/a2.md`); SHA256 per file in `models.sha256`.

Two runner-local caveats, both handled inside `gen_pano_d360.py` itself
(no vendored file edited):
- The RealESRGAN upscale stage is dropped — the diffusion SR stage alone
  already reaches 3072×1536, above the 2048×1024 target — so the runner
  loads `pipeline_base.py`/`pipeline_sr.py` directly via
  `importlib.util.spec_from_file_location` instead of importing
  `txt2panoimg`, whose package `__init__` transitively imports
  `realesrgan` (deliberately not installed in this venv).
- `enable_model_cpu_offload()` reports `pipe.device` as `cpu` to the
  vendored fork's `get_weighted_text_embeddings`, which then resolves the
  wrong device. The script monkeypatches `DiffusionPipeline.device` to
  scan for an active accelerate hook's `execution_device` first, falling
  back to the original property.

Pipeline: base txt2img at 1024×512 (20 steps, cfg 7.5, `<360panorama>, `
trigger prefix) → diffusion SR img2img+ControlNet at 3072×1536 (7 steps,
strength 0.8, cfg 15) → Lanczos downsample to 2048×1024. Writes
`<out>/pano_2048x1024.png` + `<out>/generation_manifest.json`.

### `hdr_post.py` — LDR equirect PNG → game-ready Radiance `.hdr`

```
C:\tools\StableMaterials\venv\Scripts\python.exe scripts/ai-pipeline/hdr_post.py <ldr.png> --out <file.hdr> [--sun auto|AZ,EL|none] [--sun-intensity N] [--seed-manifest <generation_manifest.json>]
```

The shared post stage all three paths feed: sRGB→linear, a monotonic
highlight-expansion curve into HDR range, a cosine wrap-seam cross-blend,
optional sun-disc injection, self-checks, then a Radiance write via
`cv2.imwrite` (`#?RADIANCE` / `32-bit_rle_rgbe`).

`--sun auto` (default) places the sun at the circular-mean centroid of the
brightest above-horizon region; `--sun AZ,EL` (degrees) places it
explicitly; `--sun none` skips injection entirely (for overcast/no-sun
skies). Either way the output is **hard-clamped at 30000**:
`EquirectImage::decode_hdr` converts f32→f16 on upload, and f16 tops out at
65504 — 30000 keeps margin under that ceiling while already exceeding
anything the engine visibly uses (kloppenheim's measured peak was 36416,
above the clamp; evening_road's measured soft-sun peak was ~20).

The highlight-expansion curve was calibrated against `cv2`-probed stats of
two CC0 references, `evening_road_01_puresky_2k.hdr` (measured peak 20.1,
median 0.554) and `kloppenheim_02_puresky_1k.hdr` (measured peak 36415.9,
median 0.097) — full table in `tasks/ai-pipeline/a2.md`'s reference
calibration section — so a lit dusk sky lands near evening_road's measured
register and a hard-sun sky lands near kloppenheim's.

Self-checks (any FAIL exits 1): exact 2048×1024, all values finite and
≥ 0, peak ≤ 30000, median in [0.02, 2.0], wrap-seam MAD ≤ 0.02. Writes
`<out>.hdr` + `<out stem>.manifest.json` (source hash, every resolved
parameter including sun az/el, output stats, chained
`generation_manifest.json` provenance if `--seed-manifest` was given).

### `turntable --hdri` — review render under a generated environment

```
cargo run -p engine-renderer --release --features offscreen --bin turntable -- content/source/test/DamagedHelmet.glb --out <dir> --angles N --size WxH --hdri <file.hdr>
```

`--hdri` is optional; omitting it renders under the hardcoded default
(`castilian_plateau_dusk_2k.hdr`). This is the engine-side gate for any
generated HDRI — the full `load_environment_hdr` → `decode_hdr` → f16
upload → IBL bake → sky + lit render path, exercised end to end.

### Fixture

`content/textures/env/castilian_plateau_dusk_2k.hdr` (+ its
`.manifest.json`) is the Phase A2 fixture: Path 3, seed 7, sun az
263.1°/el 8°. Provenance and shippability note: `content/source/CREDITS.md`
→ "Castilian Plateau Dusk HDRI, 2k" row.

## Prop generation (Phase A3)

Image → 3D prop pipeline: a Z-Image concept image feeds Hi3DGen for untextured
geometry, then a Blender-only stage textures it. Full task-by-task record,
the texture-strategy ruling, and the three-pass candidate review live in
`tasks/ai-pipeline/a3.md`.

### Hi3DGen (image → untextured geometry)

Install location: `C:\tools\Hi3DGen\Hi3DGen` — `Stable-X/Hi3DGen` @
`c29f668`, MIT (nvdiffrast/kaolin/flexicubes/diffoctreerast/flash-attn
stripped by its authors "for commercial use" — why it replaced TRELLIS 1 as
the production backbone under the strict NC ruling; see the TRELLIS section
above). Venv: `C:\tools\Hi3DGen\venv` (Python 3.11.9).

Install (fresh machine):

```
git clone https://github.com/Stable-X/Hi3DGen C:\tools\Hi3DGen\Hi3DGen
C:\Users\egm_8\AppData\Local\Programs\Python\Python311\python.exe -m venv C:\tools\Hi3DGen\venv
C:\tools\Hi3DGen\venv\Scripts\python.exe -m pip install torch==2.7.1 torchvision --index-url https://download.pytorch.org/whl/cu128
C:\tools\Hi3DGen\venv\Scripts\python.exe -m pip install --no-deps xformers==0.0.31.post1 --index-url https://download.pytorch.org/whl/cu128
C:\tools\Hi3DGen\venv\Scripts\python.exe -m pip install C:\tools\TRELLIS\whl\cumm_cu128-0.7.13-cp311-cp311-win_amd64.whl
C:\tools\Hi3DGen\venv\Scripts\python.exe -m pip install C:\tools\TRELLIS\whl\spconv_cu128-2.3.8-cp311-cp311-win_amd64.whl
C:\tools\Hi3DGen\venv\Scripts\python.exe -m pip install diffusers==0.28.0 accelerate kornia==0.8.0 timm==1.0.28 transformers==4.46.3 huggingface_hub==0.24.6 pillow tqdm scipy trimesh numpy==1.26.4 scikit-image opencv-python-headless einops
```

Runtime env vars (same convention as TRELLIS, required for every run):

```
ATTN_BACKEND=xformers
SPCONV_ALGO=native
```

Actually-installed versions, and why they diverge from the README's own pin
set (torch 2.4.0/xformers 0.0.27.post2/spconv 2.3.6): this box's proven
TRELLIS-architecture stack was used instead, and held — no torch-pin
fallback was needed. torch `2.7.1+cu128`; xformers `0.0.31.post1` (CUDA
kernels compiled, verified with the probe below); spconv `2.3.8` — its
`cumm-cu128 <0.8.0,>=0.7.11` pin isn't on PyPI (PyPI only serves 0.8.2), so
the local `cumm_cu128-0.7.13-cp311-cp311-win_amd64.whl` wheel from the
TRELLIS fork must install **before** the spconv wheel above, or spconv pulls
the incompatible PyPI cumm and breaks. `triton` (listed in
`requirements.txt`) is omitted entirely — no official Windows wheel, and
StableNormal-turbo never demanded it at runtime; `triton-windows` is the
evidenced fallback if a future weight update does. Three more pins were
forced once BiRefNet/YOSO's actual remote code ran (discovered running the
first real smoke, not from `requirements.txt`): `timm` → `1.0.28` (BiRefNet
needs `timm.layers`, absent from the older `0.6.7`), `diffusers` →
`0.28.0` (pinned to yoso's `model_index.json` version; newer diffusers
removes an import path yoso needs), `huggingface_hub` → `0.24.6` (diffusers
0.28 calls `cached_download`, removed from newer hub releases).

Verify with the cheap CUDA-kernel probe (no generation):

```
C:\tools\Hi3DGen\venv\Scripts\python.exe -m xformers.info
C:\tools\Hi3DGen\venv\Scripts\python.exe -c "import torch, xformers.ops as xops; assert torch.cuda.is_available(); q = torch.randn(1, 256, 8, 64, device='cuda', dtype=torch.float16); o = xops.memory_efficient_attention(q, q, q); torch.cuda.synchronize(); print('xformers CUDA OK', tuple(o.shape))"
```

`xformers.info` must report no "Need to compile C++ extensions" warning.
Importing `Hi3DGenPipeline` (from `C:\tools\Hi3DGen\Hi3DGen`, with
`ATTN_BACKEND=xformers SPCONV_ALGO=native` set) prints `[SPARSE] Backend:
spconv, Attention: xformers`.

**Weights** (~6.9 GB total; all MIT or Apache-2.0 — verdicts in
`content/source/CREDITS.md`):

| Repo | Role | Location |
|---|---|---|
| `Stable-X/trellis-normal-v0-1` | normal-conditioned geometry pipeline (2.65 GB) | `C:\tools\Hi3DGen\Hi3DGen\weights\trellis-normal-v0-1` -- `prop_hi3dgen.py`'s actual load path (`GEOMETRY_WEIGHTS`, offline via `HF_HUB_OFFLINE=1`), not just a cache copy |
| `Stable-X/yoso-normal-v1-8-1` | StableNormal-turbo predictor (2.63 GB) | `C:\tools\Hi3DGen\Hi3DGen\weights\yoso-normal-v1-8-1` |
| `ZhengPeng7/BiRefNet` | background removal (~0.44 GB) | standard HF cache (snapshot `e2bf8e4`) |
| `hugoycj/StableNormal` (Apache-2.0 code, fork of `Stable-X/StableNormal`) | normal-predictor code | torch.hub snapshot `hugoycj_StableNormal_main` |
| `facebookresearch/dinov2` | Hi3DGen's own `image_cond_model` (`dinov2_vitl14_reg`, 1.22 GB, `Hi3DGen/trellis-normal-v0-1/pipeline.json`), loaded every geometry run | torch.hub snapshot `facebookresearch_dinov2_main`, weights in the default torch hub checkpoint cache |

One dependency the original plan missed: DINOv2 is not listed in Hi3DGen's
`requirements.txt` — found running the first real smoke. It downloads via
StableNormal's YOSO predictor's own internal `torch.hub.load` on first run,
into the default torch hub cache, and is reused on every run after.

SHA256 for every downloaded weight file: `scripts/ai-pipeline/models.sha256`
(one `Hi3DGen/<relative-path>` line per file). Verify them against disk
(offline, no GPU, no server) before a generation batch:

```
python scripts/ai-pipeline/check_weights.py
```

Re-hashes every `Hi3DGen/...` manifest line against its real root from the
table above (Hi3DGen's own `weights/`, the BiRefNet HF hub snapshot, or the
DINOv2 torch hub checkpoint cache). Exit 0 = every file matches; non-zero
prints each offending path with its expected vs. actual hash, or flags a
missing file.

### Scripts

Five scripts chain a concept image into a shippable prop glb. Every stage is
resumable — re-running the same command skips any stage whose output already
exists.

The Blender-headless stages need two wheels inside Blender's own Python
(`xatlas` for the cleanup unwrap, `opencv-python-headless` for the texture
stage's dilate/inpaint). Blender does not scan the regular Python user
site, so install into its per-user modules dir:

```
& "C:\Program Files\Blender Foundation\Blender 5.2\5.2\python\bin\python.exe" -m pip install --target "$env:APPDATA\Blender Foundation\Blender\5.2\scripts\addons\modules" xatlas opencv-python-headless
```

**1. `prop_hi3dgen.py`** — image → untextured geometry (Hi3DGen venv,
cwd-independent):

```
C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_hi3dgen.py <image.png> --out <dir> [--seed N] [--steps N]
```

BiRefNet matte → opaque-fraction refusal gate → StableNormal-turbo normal
prediction → `Hi3DGenPipeline` geometry → `to_trimesh` export. A matte with
no usable alpha (opaque fraction ≥ 0.995 — BiRefNet did nothing, a raw RGB
image) or no opaque pixels at all exits non-zero before `concept_rgba.png`
is written, since a degenerate matte would otherwise reconstruct the
background as geometry in this script's own `preprocess_image` step, the
matte's only consumer. Writes `<out>/raw.glb` (bare geometry — texturing is
a later stage), `<out>/concept_rgba.png` (the BiRefNet-matted concept at the
input's own framing), `<out>/normal.png` (the predicted normal map — the
geometry stage's only input, so it separates a normal-stage failure from a
sampler one), and `<out>/hi3dgen_manifest.json`. `--steps`
overrides both sampler stages
uniformly; omitted, each stage keeps `app.py`'s own default (50
sparse-structure / 6 slat). Peak VRAM measured at 10.6–12.3 GiB of 12 across
the shipped props — see the VRAM sequencing rule under `gen_prop.py` below.
The manifest records the run end to end: fork revision + dirty flag, the
post-merge sampler params, the normal map's hash, the per-stage `elapsed_s`
split, resolved attention/spconv backends, the dep versions that decide mesh
geometry, and GiB VRAM peaks (allocated and reserved) with a warning past 90%
of the card.

**2. `prop_cleanup.py`** — Blender headless normalize + decimate:

```
& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" --background --python scripts/ai-pipeline/prop_cleanup.py -- <raw.glb> <clean.glb> [--height M] [--tri-budget N]
```

Arg convention matches `mixamo_to_glb.py`: everything after `--` is the
script's own argv. Strips loose floaters, scale/ground-normalizes to
`--height` (default 1.8 m), decimates to `--tri-budget` (default 15000),
then xatlas-unwraps the decimated mesh into a single 1024-target UV atlas
(`uv_charts`/`uv_utilization` in the stats line) — the layer every
`prop_texture.py` bake targets, so the atlas is identical across texture
re-runs. Exports `<clean stem>_hires.glb` before decimating — the
high-poly source `prop_texture.py`'s normal bake needs (it carries no UVs;
the bake is selected-to-active). Structural failures (no mesh, zero area)
exit non-zero rather than patch silently.

**3. `prop_texture.py`** — Blender headless texture bake:

```
& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" --background --python scripts/ai-pipeline/prop_texture.py -- <clean.glb> <hires.glb> <textured.glb> --asset NAME --seed N
```

Basecolor is a **multiview retexture** (the evidenced strategy for prop
classes needing true material register or strong backsides; full ruling and
rejected-option evidence: `tasks/ai-pipeline/a3.md` → "Texture strategy
log"). Requires `--asset` and `--seed`: ortho depth renders of the clean
mesh from four azimuths feed Z-Image Turbo + its Fun ControlNet-depth model patch
(`workflows/prop_multiview.json`). Opposite views are tiled side by side
into one 2048×1024 conditioning canvas per sampling pass (Paint3D's
front+back grid — the model attends to both views at once, so they come
out consistent) with silhouettes dilated 5 px so it paints material past
the true edge; the decoded canvas is split into per-view images and
reprojected into the atlas with facing weights, a depth-occlusion test,
and silhouette edge padding, and island texels no view covered are
Telea-inpainted from their surroundings. Because coverage is purely
geometric, the stage scores an azimuth/elevation candidate grid before
generating anything and greedily adds up to 2 extra views (extra canvas)
whenever one would newly cover ≥3% of the island — Text2Tex-style
next-best-view, aimed at up/down-facing texels the 15°-elevation base
views can't weight. The ComfyUI server lifecycle lives entirely inside
this stage (started headless, killed after).

The generated views are lit renders, so after generation each one is
decomposed by MaterialAnything's material estimator (`prop_pbr.py`,
subprocess in its own venv below) into a delit `albedo.png`, conditioned on
a camera-space normal render of the same view. The albedo blend is the
exported basecolor (the lit `gen.png` is conditioning intermediate only).
Roughness/metallic ship as the glTF scalar factors via the resolved asset's
contract (`content/models/surface_classes.json`, `proptex.registry`).
Every stage of the chain is a content-addressed cache unit under
`target/prop-cache/<stage>/<key>/`, keyed by the sha256 of its stage source
set, its resolved params and its input content hashes; each entry holds its
outputs beside the `key.json` that is both its key and its provenance
record. A killed run therefore resumes at stage granularity, and a stage
reruns exactly when its key changes. The normal map is a real hires→clean Cycles bake (the estimator's
bump head is discarded in its favor).

The asset's **`view_res`** field (`content/models/assets.json`) sets the
per-view depth/normal render and Z-Image + Fun ControlNet-depth generation
resolution independently of the MaterialAnything estimator's input, which
stays pinned at 768×768 either way: `prop_pbr.py` downscales each generated
view to 768 for the estimator call and upscales its albedo output back to
the view's own resolution (`full_res = gen.size`) — that split is
unconditional, not gated by this field. Raising `view_res` only sharpens
the source imagery blended into the atlas; it does not change
`texture_size` (the atlas's own baked resolution, a separate field).

The asset's **`texture_size`** field (`content/models/assets.json`) is the
pixel resolution every basecolor/normal image is baked at. Independent of
`prop_cleanup.py`'s xatlas packing: the mesh's UVs are resolution-independent
0–1 floats, so a `texture_size` above the resolution the atlas was packed
for is safe (island gutters only grow) and re-running cleanup is not needed
to raise it.

**MaterialAnything estimator venv** (`C:\tools\MaterialAnything`, clone of
`3DTopia/MaterialAnything` @ `be3d6b3`, MIT code; weights HF
`xanderhuang/material_estimator`, Apache-2.0, ~4.3 GB). Its
`diffusers==0.28.2` pin predates every other venv's, hence the isolation;
`huggingface_hub<0.26` + `accelerate==0.31.0` are required companions
(newer hub removed `cached_download`, which diffusers 0.28 imports):

```
py -3.11 -m venv C:\tools\MaterialAnything\venv
C:\tools\MaterialAnything\venv\Scripts\python.exe -m pip install torch==2.7.1 torchvision --index-url https://download.pytorch.org/whl/cu128
C:\tools\MaterialAnything\venv\Scripts\python.exe -m pip install diffusers==0.28.2 transformers==4.42.4 "huggingface_hub<0.26" accelerate==0.31.0 einops opencv-python safetensors Pillow numpy
C:\tools\MaterialAnything\venv\Scripts\huggingface-cli.exe download xanderhuang/material_estimator --local-dir C:\tools\MaterialAnything\pretrained_models\material_estimator
```

**Writing an asset's `subject` for this strategy — name every material's colour.**
Z-Image runs at cfg 1, and unlike SDXL at cfg 7 it does not infer a material's
colour from its name: `"melted wax candles"` produced black iron candles,
`"pale cream-white wax candles"` produced correct ones. Each view is an
independent generation, so an unnamed colour gets resolved differently per view
and the disagreement survives the facing-weighted blend into the atlas.
Name the colour of each material and nothing more — prompt verbosity trades
directly against geometric fidelity on a distilled base, so added clauses cost
silhouette accuracy (`tasks/ai-pipeline/research/a6-3-material-separation.md`,
`a5b-bakeoff-results.md`).

**4. `preprocess_prop.mjs`** — gltf-transform prune/dedup/resize:

```
node scripts/ai-pipeline/preprocess_prop.mjs <textured.glb> <final.glb>
```

Then the existing DDS bake, reused as-is (no new bake code):

```
node scripts/asset-pipeline/bake_textures.mjs gltf <final.glb>
```

Its `mr`/`ao`/`normal` slots pass `--ignore-srgb` to texconv: Blender's PNG
writer stamps an sRGB+gAMA+cHRM chunk on every image it saves regardless of
the image's own Non-Color tag, and texconv's WIC reader honors that chunk
over the requested output format, so converting to BC7_UNORM/BC5_UNORM
without the flag silently applied a real sRGB decode to already-linear mr/
normal data (measured: roughness mean 0.55 raw → 0.26 installed, matching
the sRGB EOTF at that value).

**5. `gen_prop.py`** — chain assembly, one candidate per invocation:

```
python scripts/ai-pipeline/gen_prop.py --asset <name> --seed N --out <dir> [--skip-concept <image.png>]
```

Runs concept → geometry → cleanup → texture → preprocess+bake → turntable →
chained `generation_manifest.json`; every stage is skipped if its output
already exists, so a second identical invocation exits in under a second
instead of regenerating. `--skip-concept <image.png>` bypasses concept
generation with a provided image — re-rolls geometry and everything
downstream of it without spending a new Z-Image concept. **One seed per
command:** one invocation is one candidate; a batch is the caller looping
seeds across separate foreground invocations, never one script call for N
candidates, so every command stays under the shell's timeout budget. Run
`check_weights.py` (above) once before starting a seed batch — a corrupted or
swapped Hi3DGen weight fails every candidate the same way, and the manifest
check is seconds, not a wasted geometry stage.
**VRAM sequencing (forced by the 10.6–12.3 GiB Hi3DGen peak above): ComfyUI must
never be up while a geometry stage runs.** Every ComfyUI stage owns its
server lifecycle (`comfy_run.server()`): the concept stage and the
multiview texture stage's generation passes each start a headless server
and stop it before returning, so the chain runs unattended and the rule
holds by construction. An already-running external server is refused, not
reused — the chain can't stop somebody else's server before geometry.

The asset's `view_res`/`texture_size` fields thread straight through to
`prop_texture.py` (above) via `--asset`; `--max-bytes` threads through to
`preprocess_prop.mjs`'s own flag (character-chain precedent below).
`preprocess_prop.mjs`'s `--max-dim` is driven automatically from the same
`texture_size` — no separate flag, so the two can no longer diverge.

### `asset_inspect` — lighting × channel × distance review

```
cargo run -p vordar-client --release --features offscreen --bin asset_inspect -- <glb|gltf> [--out DIR] [--size WxH] [--scale S] [--lighting studio,furnace,raking,dusk] [--channel beauty,albedo,rough,metal,normal,ao,clay] [--angles N] [--distance full,gameplay,macro] [--reference]
```

Renders one prop across every combination of lighting preset, shading debug
channel, and camera distance in a single invocation — the eyes-fix for a
review instrument that used to pass every AI-generated prop that then failed
in-game (`tasks/lessons/2026-07-23-review-in-engine-at-gameplay-framing.md`).
Four lighting presets (`studio` neutral/even so no colour cast can hide an
inpaint bleed, `furnace` uniform-radiance energy check, `raking`
camera-relative grazing light for normal/geometry readout, `dusk` the real
ship-gate HDRI + zone fog); seven channels (`beauty` the shipped shading,
`albedo`/`rough`/`metal`/`normal`/`ao` isolated shader debug outputs,
`clay` a uniform-grey material override); three distances (`full` a fitted
turntable shot, `gameplay` 2.3 m from the prop's near surface at eye height,
`macro` 0.6 m at the prop's mid-height). `--reference` renders
`rock_face_01` alongside at its shipped placement scale (4.0) for a
same-conditions material comparison.

**Per-frame PNGs at `--size` (default 1024²) are the evidence** —
`<out>/<lighting>_<channel>/<distance>_<angle>.png`. The stitched
`sheet_<lighting>_<channel>.png` beside them is only an index (tiles capped
at 512px, long edge near 1536px): a contact sheet gets downscaled by the
viewer before judging it, which is exactly the blindness that let eight bad
props ship. Judge `gameplay`/`macro` frames individually, at full
resolution, never from a sheet.

### `prop_audit.py` — measurement-only metric audit

```
python scripts/ai-pipeline/prop_audit.py [--asset <name-or-dir>] [--json]
```

Prints one fixed-width row per prop under `content/models/props/`, with
`rock_face_01` (the Poly Haven photoscan reference) sorted first. Reads each
prop's `*.textures/manifest.json` for its mesh path and image-slot map —
slots come from the manifest, never inferred from the glTF's own texture
indices, which can disagree (`crucero.glb`'s `baseColorTexture.index` is 0
while `img0.dds` is the normal map). Metrics: roughness/metallic/AO from the
ARM atlas, albedo luma percentiles and blown-highlight fraction, a
normal-map Laplacian std and dead-flat-texel fraction, atlas and
world-placed texel density (`atlas_px_per_m` and `placed_px_per_m` — the
latter divided by the prop's max `zones.ron` placement scale, since
`rock_face_01` ships at scale 4.0 and its raw atlas density alone overstates
how it actually reads in-game), the glTF's own
`roughnessFactor`/`metallicFactor`/`baseColorFactor`,
`blend_coverage`/`hole_frac` (the texture chain's `blend` stage record in
`generation_manifest.json`: `blend_coverage`/`hole_texels` under its
`measurements`, `texture_size` under its `params`), and a
tangent-space `baked_fraction` riding along as a relative corroborator (not
a physical light fit — the world-space depth renders it would need aren't
shipped).

**Measurement only: no thresholds, no gate, no non-zero exit.** Nobody yet
knows what quality this pipeline can reach, and every generated prop shipped
so far read as AI slop under an instrument that was itself broken — writing
a gate now would encode a guess from bad evidence. Metric names and slot
keys (`base_color`/`normal`/`metallic_roughness`/`occlusion`) match
`MaterialData`'s vocabulary in `content_lint.rs:259-265` so a later
promotion into a real test is a rename-free port, not a rewrite.

### Fixture

`content/models/props/candelabra_shrine/` — winner seed 2 (`cand_2`),
chosen over three review passes; full record (per-candidate notes, the
TRELLIS-baseline comparison, two texture-stage defects found and fixed
along the way): `tasks/ai-pipeline/a3.md` → "Decision log". Known gap: thin
iron members (posts, scroll arms) render as polished pewter rather than the
concept's weathered dark iron — a characterized ceiling of the projection
bake on thin-member-dominated props, ruled tolerable at game camera distance
for this fixture; Strategy 2 (Z-Image multi-view retexture) is the evidenced
escalation if a later art review rejects it.

## Character generation (Phase A4)

Image → 3D character pipeline: the same concept → Hi3DGen-geometry →
Blender-texture chain as Phase A3, extended with a rig stage that puts the
canonical Mixamo skeleton and its 10-action clip library onto the generated
mesh. A parametric CC0 route (MPFB2) shares the same rig/preprocess/bake/
review tail and is the current fixture winner. Full task-by-task record,
the texture-strategy ruling, the rig-gate failure/fix history, and the
decision-gate log live in `tasks/ai-pipeline/a4.md`.

### Rig design: transplant, not generate

The skeleton is never generated — it's fixed by architecture. Every
candidate gets the canonical `content/source/characters/mixamo/Character.fbx`
armature (Mixamo bone names, the rest pose the clip library animates
against) transplanted onto it; "rigging" reduces to fitting that armature
to the mesh and predicting skin weights over it, never inventing new
joints. UniRig was rejected as the auto-rigger: it predicts an arbitrary
skeleton per mesh with no template/retarget path, so its output can't play
the existing clips at all (evidence, the full rigging-candidate research,
and the contingency ladder: `tasks/ai-pipeline/a4.md`).

**Units invariant:** the imported armature's own object transform (FBX cm
scale) is never touched by any rig script — only the MESH moves into the
armature's space, by scale + translation. `client/vordar-client/src/
weapons.rs:204`'s socket matrix bakes exactly that cm→m scale; applying or
clearing the armature transform breaks weapon sockets silently.

**VRAM sequencing:** Hi3DGen peaks 10.6–12.3 GiB of 12 (shipped-prop spread,
the top of it already over the card) —
ComfyUI must be fully stopped before any geometry stage runs.
`gen_character.py` (below) owns this by construction: the concept stage and
the multiview texture stage each start and stop their own headless ComfyUI
server, so it's never up while Hi3DGen runs; the SkinTokens skin step also
uses the GPU (3.7 GiB peak) but never overlaps either.

### Pose kit + conditioning image

Committed: `scripts/ai-pipeline/assets/tpose_openpose.png` (+
`tpose_openpose.json` sidecar recording the figure's projected pixel
height) — a deterministic OpenPose-format 18-keypoint stick figure,
orthographically projected from `Character.fbx`'s own T-pose joint
positions. `char_concept.json`'s `LoadImage` node loads it and
`ControlNetApplyAdvanced` (strength ~0.8,
`xinsir/controlnet-openpose-sdxl-1.0`) locks every concept generation to
the canonical skeleton's proportions and rest pose — the fit in the rig
stage is then a near-rigid alignment, not a deformation solve. The
one-shot Blender-headless + Python probe that produced this asset (imports
`Character.fbx`, asserts the 65 `mixamorig:`-prefixed bones, projects the
stick figure) is not kept in the tree; only its frozen output ships.

### Concept, geometry, cleanup, texture — the A3 chain, character-tuned

`prop_hi3dgen.py` and `prop_cleanup.py` (above) run unchanged on
characters; the concept stage substitutes `char_concept.json`
(openpose-conditioned) for `prop_concept.json`. Cleanup targets `--height
1.744 --tri-budget 30000` (double the prop budget; no VQ triangle clause
exists, chosen under the 16 MB / 64-joint caps).

`prop_texture.py`'s per-view MaterialAnything decomposition (above)
estimates only the delit albedo per texel from the generated views — the
same machinery props use, not a character-specific mode. Roughness/metallic
ride the glTF scalar factors: the constant-dielectric contract (`metallic
0.0 / roughness 0.8`, matching every existing race model) is the
`character_skin` surface class's registry contract
(`content/models/surface_classes.json`, `proptex.registry`), same for
characters as for props, and the MPFB route's authored materials (below).

### `char_rig.py` — skeleton transplant + fit + weight adoption (Blender 5.2 headless)

```
& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" --background --python scripts/ai-pipeline/char_rig.py -- fit <textured.glb> <Character.fbx> <fit.glb>
& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" --background --python scripts/ai-pipeline/char_rig.py -- finish <textured.glb> <Character.fbx> <skinned.glb> <clips_dir> <out.glb> [--height M]
```

Two subcommands bracket SkinTokens' skin prediction (below). `fit`:
imports `Character.fbx`, keeps its armature exactly as imported, deletes
its meshes, imports the textured glb, fits the mesh into the armature's
space (uniform scale + translation from the skeleton's own T-pose
landmarks — crown/ground/hips/shoulder-span), prunes fingers, adds the
`handslot.r`/`handslot.l`/`head` socket bones, rigid-binds every vertex to
its nearest deform-bone segment (the minimal valid skin SkinTokens' input
shape needs), and exports `fit.glb`. `finish`: rebuilds the same scene from
the same inputs — not from `fit.glb` itself, since a glTF round-trip of the
armature object transform would break the units invariant above — adopts
SkinTokens' predicted weights under canonical bone names (matched by
descending both hierarchies in lockstep, not by nearest position: the
tokenizer's ~5 cm quantization error exceeds some bone spacings), trims the
three Mixamo end bones, runs the rig-quality gate (weightless-vertex
fraction over 0.5% is a structural failure, exits non-zero with the
numbers — never patched silently), then the shared `mixamo_rig.py`
machinery: clip stash, height/ground bake, glb export. Bone-heat solving
and its solver-warning gate are gone from this path entirely — SkinTokens
predicts every weight.

### Character auto-skinning (SkinTokens)

Install location: `C:\tools\SkinTokens\SkinTokens` —
`VAST-AI-Research/SkinTokens`, UniRig's successor (code MIT; weights MIT at
`VAST-AI/SkinTokens` on HF, ~1.6 GB GRPO-refined checkpoint
`experiments/articulation_xl_quantization_256_token_4/grpo_1400.ckpt`,
fetched by the repo's `download.py`). Venv: `C:\tools\SkinTokens\venv`
(Python 3.11, torch 2.7.0+cu128, prebuilt flash-attn 2.8.3 cu128 wheel —
flash-attn is hardcoded in their model code, no config escape — plus
`scipy`, which their `requirements.txt` omits).

**`char_skin.py`** — skin-only weight prediction (SkinTokens venv):

```
C:\tools\SkinTokens\venv\Scripts\python.exe scripts\ai-pipeline\char_skin.py <fit.glb> --out <skinned.glb> [--seed N]
```

Run with `cwd=C:\tools\SkinTokens\SkinTokens` (checkpoint path and their
`bpy_server.py` resolve from there). Predicts skin weights for `fit.glb`'s
mesh over the canonical skeleton it carries and transfers them onto that
same textured mesh. SkinTokens ships no seed control (sampling
generation), so the wrapper seeds torch/numpy/random itself and records
the seed in its JSON stats line. 3.7 GiB peak VRAM measured (`num_beams`
fixed at 4, the validated 12 GB setting).

### `preprocess_prop.mjs` — character caps

`--max-bytes N` and `--max-dim N` (both optional; defaults are the prop
caps, 32 MB / 2048) let the character chain raise the byte ceiling to
VQ-B2's 16 MB while keeping textures at 1024: `--max-bytes 16777216`. The
MPFB route also passes `--max-dim 1024` explicitly — at 2048 its two
4096² robe source textures push `final.glb` to ~19 MB, over the cap.

### `gen_character.py` — chain assembly

```
python scripts/ai-pipeline/gen_character.py --asset <name> --seed N --out <dir> [--skip-concept <image.png>] [--height M]
python scripts/ai-pipeline/gen_character.py --mpfb --out <dir> [--height M]
```

One invocation = one candidate, resumable stage by stage exactly like
`gen_prop.py`. The default (AI) chain: concept → geometry → cleanup →
texture (multiview, always) → rig (`char_rig.py fit` → `char_skin.py` →
`char_rig.py finish`) → preprocess+bake → review renders → chained
`generation_manifest.json`, landing under `<out>/cand_<seed>/`. `--mpfb`
swaps the first five stages for the parametric `char_mpfb.py` body (below;
the parser rejects `--asset`, `--seed`, or `--skip-concept` in this mode)
and lands under `<out>/cand_mpfb/`, with the manifest carrying `"mode":
"mpfb"` and no AI-chain keys. Both modes share the review stage: a static
8-angle turntable plus three animated sheets (`--clip walk`, `--clip
idle`, `--clip attack_slash`, default sample times).

### `turntable --clip <name> [--times t0,t1,…]` — animated review renders

```
cargo run -p engine-renderer --release --features offscreen --bin turntable -- <glb> --out <dir> --clip <name> [--times t0,t1,...] --size WxH [--hdri <path>]
```

Extends the static `--angles N` mode (above): holds a fixed 3/4-view
camera and samples the named clip instead of flattening to bind pose —
`--times` (comma-separated seconds) defaults to 5 evenly spaced samples
across the clip's duration. Exactly one of `--angles`/`--clip` is
required. This is what `gen_character.py`'s review stage and the fixture's
placement-proof renders use for the walk/idle/attack_slash sheets.

### MPFB2 (parametric CC0 body — `char_mpfb.py`, `--mpfb` mode)

Install location: `C:\tools\MPFB2\` — `makehumancommunity/mpfb2` 2.0.17
(nightly `mpfb2-20260720.zip`; code GPLv3, bundled assets CC0 1.0 —
license verdicts in `content/source/CREDITS.md`). Installs as a Blender
5.2 **extension**, not the legacy addon path — no separate venv,
CPU-only:

```python
bpy.ops.extensions.package_install_files(
    filepath="C:/tools/MPFB2/mpfb2-20260720.zip",
    repo="user_default", enable_on_install=True)
bpy.ops.wm.save_userpref()
```

then the two CC0 asset packs the fixture body uses:

```python
bpy.ops.mpfb.load_pack(filepath="<path>/makehuman_system_assets_cc0.zip")
bpy.ops.mpfb.load_pack(filepath="<path>/suits02_cc0.zip")
```

Module import name is `bl_ext.user_default.mpfb` — a bare `import mpfb`
fails; scripts resolve the package by scanning `sys.modules` for a name
ending in `.mpfb` (`char_mpfb.py`'s `resolve_mpfb()`).

`char_mpfb.py` builds the parametric MPFB2 monk — CC0 base body + the
`donitz_monk_robe`/`donitz_monk_robe_hood` garments + CC0 eyes +
`old_caucasian_male` skin — then brings it to the canonical T-pose and
binds it to the transplanted `Character.fbx` armature by MPFB's own
artist-authored, name-matched vertex groups: no bone-heat, no weight
prediction, no AI model of any kind on this route. `gen_character.py
--mpfb --out <dir>` runs it (no concept/geometry/texture/SkinTokens
stages, no asset/seed, CPU-only ~10 s) into `<out>/cand_mpfb/`, then the
shared preprocess/DDS-bake/turntable-review stages.

### Fixture

`content/models/characters/human_gen/` — the MPFB2 monk (`cand_mpfb/
final.glb`), installed as the Phase A4 fixture and held for Phase B4's
race swap; not wired into `content/races/human.ron`, which reverted to
the incumbent `human.glb` after the placement-proof lint run. Full
candidate history (the generative chain's candidate batch, the rig-gate
failure/fix pass, the TRELLIS baseline, and the user's decision-gate
ruling that selected the MPFB route): `tasks/ai-pipeline/a4.md` →
"Decision log".
