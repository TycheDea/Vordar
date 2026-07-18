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
