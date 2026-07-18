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
| `controlnet\` | `controlnet-openpose-sdxl-1.0.safetensors`, `controlnet-depth-sdxl-1.0.safetensors` |
| `text_encoders\` | `clip_l.safetensors`, `t5xxl_fp8_e4m3fn.safetensors` |
| `vae\` | empty |
| `diffusion_models\` | empty |

`flux1-schnell-fp8.safetensors` is the Comfy-Org all-in-one checkpoint
(UNet+CLIPs+VAE bundled) — load it with `CheckpointLoaderSimple`, not the
modular UNet/CLIP/VAE loader chain. The standalone `clip_l`/`t5xxl` text
encoders are kept for a future modular path; that path also needs
`ae.safetensors`, which sits behind a gated HF repo (401 anonymous) and has
not been fetched.

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
| `models` | Every `*Loader` node's filename input, cross-referenced against `models.sha256` for its sha256 (`null` if not in the manifest) |
| `outputs` | Each saved output file: node id, output kind, filename, subfolder, type, local save path |

## TRELLIS (image → 3D)

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

**Known blocker (carried to Phase A3):** the installed xformers wheel has no
CUDA kernels, so real mesh generation is not yet functional — needs a
CUDA-enabled xformers or triton build before A3 can run it for real. A0 only
verified the environment installs and CUDA is visible (`torch.cuda.is_available()`
and `from trellis.pipelines import TrellisImageTo3DPipeline` both succeed);
no generation smoke test was required or run.

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
anything the engine visibly uses (kloppenheim's real 36416 peak sits above
the clamp; evening_road's soft-sun peak is ~20).

The highlight-expansion curve is calibrated against `cv2`-probed stats of
the two committed CC0 references — `evening_road_01_puresky_2k.hdr`: peak
20.1 / median 0.554; `kloppenheim_02_puresky_1k.hdr`: peak 36415.9 / median
0.097 (full table in `tasks/ai-pipeline/a2.md`'s reference calibration
section) — so a lit dusk sky lands near evening_road's register and a
hard-sun sky lands near kloppenheim's.

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
(`evening_road_01_puresky_2k.hdr`). This is the engine-side gate for any
generated HDRI — the full `load_environment_hdr` → `decode_hdr` → f16
upload → IBL bake → sky + lit render path, exercised end to end.

### Fixture

`content/textures/env/castilian_plateau_dusk_2k.hdr` (+ its
`.manifest.json`) is the Phase A2 fixture: Path 3, seed 7, sun az
263.1°/el 8°. Provenance and shippability note: `content/source/CREDITS.md`
→ "Castilian Plateau Dusk HDRI, 2k" row.
