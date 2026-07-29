# Plan: A real headless API in the fork; shrink prop_hi3dgen.py to CLI + gates + manifest — 2026-07-29

Source: `docs/reviews/hi3dgen/reworks-hi3dgen-2026-07-28.md` finding 3.

## Ideal end state

The fork (`C:/tools/Hi3DGen/Hi3DGen`, branch `vordar-fixes`, HEAD `c7389f5`) is a
pip-installable package whose `hi3dgen.headless` module owns everything
upstream-shaped: backend env pins, weight resolution (including the BiRefNet
path fixed at its source), model lifecycle with the finding-17 stage offload,
seeded normal prediction, per-seed sampling with the rng digest, and the
extraction metadata incl. `last_extract_s` that finding 24's manifest split
reads. `scripts/ai-pipeline/prop_hi3dgen.py` is reduced to argparse CLI,
resume/skip, the two refusal gates (`check_matte`, `check_mesh`), export and
manifest writing — no env vars, no `sys.path` insertion, no `torch` import, no
re-implementation of fork internals. `prop_extract.py` and the fork's own tests
lose the same `sys.path` workaround. The per-candidate manifest schema is
byte-for-byte the same key set the current script writes.

## Design decisions

- **Session class, not a single `generate() -> dict`.** The finding's Ideal
  sketches `hi3dgen.headless.generate(image, out_dir, seed, …) -> dict`, but its
  own title keeps gates + manifest vordar-side, and two later findings shape the
  call pattern: finding 16's batching (one model load + one matte + one turbo
  normal prediction shared across N seeds) and the gate points (`check_matte`
  must refuse *before* the normal prediction is paid; `check_mesh` before
  export). A single function would have to absorb the gates and the manifest or
  give up batching. The API is therefore a `Session` with three stage methods —
  `matte()`, `prepare()`, `sample(seed)` — mirroring the shared-setup /
  per-candidate split the CLI already has. Rejected alternatives: single
  `generate()` (loses gate points and batching); free functions with explicit
  model handles (re-exposes the lifecycle the finding wants hidden).
- **Packaging: PEP 660 editable install with `dependencies = []`.** The venv's
  GPU stack is hand-pinned from custom indexes (torch 2.7.1+cu128, xformers
  0.0.31.post1, local cumm/spconv wheels — `scripts/ai-pipeline/README.md:409-431`);
  any dependency list in `pyproject.toml` would invite pip to resolve against
  PyPI and clobber it. So the project declares **no** dependencies
  (`requirements.txt` stays the human-managed environment recipe) and the
  install command additionally passes `--no-deps`. **Measured 2026-07-29:** a
  probe package with this exact pyproject shape installed editable into
  `C:\tools\Hi3DGen\venv` (Python 3.11.9, pip 24.0, setuptools 65.5.0), imported
  from an arbitrary cwd, and uninstalled cleanly with `pip uninstall -y`,
  leaving no trace; default build isolation worked (network fetch of the build
  backend succeeds on this box). Rollback contract is in step 1.
- **Root causes move, workarounds die.** Three of the vordar script's
  workarounds are fixed at their origin instead of being relocated:
  `_lazy_load_birefnet`'s hardcoded `'weights/BiRefNet'` (a path nothing ever
  populated) becomes an offline pinned-revision load from the HF cache — the
  knowledge `preload_birefnet` holds at a distance today; `matte_concept`'s
  duplication of `preprocess_image`'s matte steps is dissolved by extracting a
  `matte_image()` method inside the pipeline that `preprocess_image` itself
  calls; the pre-import env pins become package defaults —
  `os.environ.setdefault("SPCONV_ALGO", "native")` at the top of
  `hi3dgen/__init__.py` (the only backend whose code default, `'auto'`, differs
  from the operating value; `ATTN_BACKEND`'s code defaults are already
  `'xformers'` in both `modules/attention/__init__.py:27` and
  `modules/sparse/__init__.py:29`, so no setdefault is added for it — the
  Session's fail-loud assert remains the guard) and
  `HF_HUB_OFFLINE`/`CUBLAS_WORKSPACE_CONFIG` setdefaults at the top of
  `headless.py`. `app.py`'s own `os.environ['SPCONV_ALGO'] = 'native'` is
  superseded by the `__init__` default and deleted (swap rule).
- **The ~60–70-line target is superseded; the responsibility split is the
  invariant.** The finding was written against a 211-line script; the file is
  541 lines today because the gates, batching, resume and manifest all grew
  under audit findings 1–24. Post-refactor the script lands near ~220 lines —
  every one of them CLI, gate, manifest or print. No upstream-shaped line
  survives; that predicate, not a line count, is the acceptance test.
- **Manifest schema is frozen through the refactor.** `gen_prop.py` /
  `gen_character.py` read `hi3dgen_manifest.json` downstream and the campaign's
  A/B tooling compares its fields. The refactor must not change the key set the
  current script writes (top-level keys as in
  `target/prop-solid-validation/chapel_arch_e2e/cand_0/hi3dgen_manifest.json`,
  plus `elapsed_s.extraction` from finding 24, minus `extraction.fill_interior`
  which died with the fill at fork `5d4c9b0`). The step-4 smoke asserts this.
- **Gates stay vordar-side and unchanged.** `check_matte` / `check_mesh` are
  product policy; their signatures, exceptions and unit tests
  (`scripts/tests/test_prop_hi3dgen.py`) survive verbatim. `Candidate` carries
  both the raw `mesh_result` (for `.success`) and the converted trimesh so
  `check_mesh(mesh_result, trimesh_mesh)` needs no signature change.
- **Timing split is preserved via returned timings.** `prepare()` returns its
  internal `{"preprocess", "normal", "cond"}` wall times and `sample()` returns
  `extract_s` read from `SparseFeatures2Mesh.last_extract_s`, so the manifest's
  `elapsed_s` keys — including finding 24's `extraction` sub-interval — keep
  their exact meanings.
- **Rollback is layered and was probed.** (a) `pip uninstall -y hi3dgen`
  removes only the editable `.pth`/finder + dist-info (probe-verified; nothing
  else in site-packages is touched because nothing else is installed). (b) The
  old scripts' `sys.path` insertion and the editable install resolve to the
  same files, so any partial state — install present with old scripts, or
  install absent with new fork code — still runs. (c) Absolute restore of
  today's working pair, independent of pip state:
  `git -C C:/tools/Hi3DGen/Hi3DGen checkout c7389f5` and vordar
  `git checkout 1d5c681 -- scripts/ai-pipeline/prop_hi3dgen.py scripts/ai-pipeline/prop_extract.py`.

## Findings (execution order)

### 1. Make the fork pip-installable and install it editable into the venv

- **Evidence:** `C:/tools/Hi3DGen/Hi3DGen` (branch `vordar-fixes`, HEAD
  `c7389f5`) has no `pyproject.toml`/`setup.py`; top level is `LICENSE`,
  `README.md`, `app.py`, `assets/`, `hi3dgen/`, `requirements.txt`,
  `requirements.lock.txt`, `tests/`, `weights/`, `.gitignore` (one line:
  `__pycache__`). Every consumer reaches the package by `sys.path.insert`:
  `scripts/ai-pipeline/prop_hi3dgen.py:56-57`, `scripts/ai-pipeline/prop_extract.py:36-37`,
  `fork tests/test_extraction_contract.py:24`. Venv `C:\tools\Hi3DGen\venv`:
  Python 3.11.9, pip 24.0, setuptools 65.5.0, no `wheel`, no `hi3dgen` dist.
  Measured 2026-07-29: a probe package with the pyproject below installed
  editable (`pip install -e . --no-deps`, default build isolation), imported
  from `/`, and uninstalled cleanly.
- **Ideal:** `import hi3dgen` works under the venv from any cwd with no
  `sys.path` manipulation, via an editable install that can never touch the
  hand-pinned GPU stack; uninstalling restores today's state exactly.
- **Gap:** No package metadata exists; the venv resolves `hi3dgen` only through
  each script's path hack.
- **Suggestion:** Add `C:/tools/Hi3DGen/Hi3DGen/pyproject.toml`:

  ```toml
  [build-system]
  requires = ["setuptools"]
  build-backend = "setuptools.build_meta"

  [project]
  name = "hi3dgen"
  version = "0.1.0"
  requires-python = ">=3.11"
  # Intentionally empty: the venv's GPU stack (torch/xformers/cumm/spconv) is
  # hand-pinned from custom indexes per requirements.txt; a dependency list
  # here would let pip resolve against PyPI and clobber it. Install with
  # --no-deps.
  dependencies = []

  [tool.setuptools.packages.find]
  include = ["hi3dgen*"]
  ```

  Extend the fork's `.gitignore` with `*.egg-info/` (setuptools writes
  `hi3dgen.egg-info/` into the project root during the editable build). Then:

  ```
  C:\tools\Hi3DGen\venv\Scripts\python.exe -m pip install -e C:\tools\Hi3DGen\Hi3DGen --no-deps
  ```

  Commit `pyproject.toml` + `.gitignore` on `vordar-fixes`. Rollback for this
  step and every later one: `C:\tools\Hi3DGen\venv\Scripts\python.exe -m pip
  uninstall -y hi3dgen` (probe-verified clean removal); the existing scripts
  keep working throughout because their `sys.path` insertion resolves to the
  same files whether or not the install exists.
- **Path:** Write `pyproject.toml` and the `.gitignore` line → run the install
  command above → test: from cwd `C:\` (not the fork root, no `PYTHONPATH`),
  `C:\tools\Hi3DGen\venv\Scripts\python.exe -c "import hi3dgen, hi3dgen.pipelines; print(hi3dgen.__file__)"`
  must succeed and print a path under `C:\tools\Hi3DGen\Hi3DGen\hi3dgen`
  (fail-first: run the same command *before* the install and confirm it fails
  with `ModuleNotFoundError` — that is the red state the install turns green)
  → `pip show hi3dgen` reports the editable location → full gate: extraction
  harness `C:\tools\Hi3DGen\venv\Scripts\python.exe C:\tools\Hi3DGen\Hi3DGen\tests\test_extraction_contract.py`
  prints `3/3 cases passed`; vordar workspace untouched (no Rust sources in
  this rework; cargo gate unaffected throughout).

### 2. Fix BiRefNet loading at its source and extract `matte_image()` from `preprocess_image`

- **Evidence:** `fork hi3dgen/pipelines/hi3dgen.py:198-205`
  (`_lazy_load_birefnet`) loads the cwd-relative path `'weights/BiRefNet'`,
  which was never populated — the weights live in the standard HF cache
  (snapshot `e2bf8e4`, per `scripts/ai-pipeline/README.md:463`). The vordar
  script patches this at a distance: `prop_hi3dgen.py:191-207`
  (`preload_birefnet`) pre-sets `pipeline.birefnet_model` from
  `ZhengPeng7/BiRefNet` rev `e2bf8e4460fc8fa32bba5ea4d94b3233d367b0e4` with
  `local_files_only=True`. Separately, `prop_hi3dgen.py:241-256`
  (`matte_concept`) re-implements `preprocess_image`'s RGB branch
  (`hi3dgen.py:119-136`: convert RGB → LANCZOS resize to ≤1024 →
  `_get_birefnet_mask` → alpha = mask×255) minus the crop, to get a matte at
  the concept's own framing.
- **Ideal:** The pipeline itself loads BiRefNet from the HF cache at the pinned
  revision, offline; it exposes `matte_image(image) -> RGBA` (matte at
  input-aligned framing, no crop), and `preprocess_image`'s RGB branch is
  implemented *as* `matte_image` followed by the shared crop/pad/resize path —
  one implementation, zero drift.
- **Gap:** Loader path is dead; the matte logic exists twice, one copy per
  repo.
- **Suggestion:** In `fork hi3dgen/pipelines/hi3dgen.py`: add module constants
  `BIREFNET_REPO = "ZhengPeng7/BiRefNet"`,
  `BIREFNET_REVISION = "e2bf8e4460fc8fa32bba5ea4d94b3233d367b0e4"`; rewrite
  `_lazy_load_birefnet` to
  `AutoModelForImageSegmentation.from_pretrained(BIREFNET_REPO, revision=BIREFNET_REVISION, trust_remote_code=True, local_files_only=True).to(self.device)`
  + `.eval()` (fail-loud offline load — no fallback). Add method
  `matte_image(self, image: Image.Image) -> Image.Image`: convert RGB, LANCZOS
  resize if longest side > 1024, lazy-load BiRefNet if
  `getattr(self, 'birefnet_model', None) is None`, `_get_birefnet_mask`, write
  mask×255 into the alpha of the (possibly resized) RGBA — exactly
  `matte_concept`'s body. Rewrite `preprocess_image`'s `else` branch (lines
  119-136) to `output = self.matte_image(input)`; the downstream bbox/crop/pad
  path is untouched. `app.py` and the current vordar script keep working: the
  vordar `preload_birefnet` still pre-sets the attribute so the lazy branch
  never fires, and RGBA inputs bypass the matte entirely.
- **Path:** Implement the two changes → add fork test
  `C:\tools\Hi3DGen\Hi3DGen\tests\test_preprocess_contract.py` (plain asserts,
  no pytest — the venv has none; same pattern as `test_extraction_contract.py`).
  Construct `p = Hi3DGenPipeline()` (bare — `models=None` returns early), set
  `p.birefnet_model = object()` (sentinel skips the lazy load) and
  `p._get_birefnet_mask = <stub>` returning a fixed 0/1 array with an offset
  square of foreground. Three cases: (a) `matte_image` on a 1400×700 RGB
  returns a 1024×512 RGBA whose alpha equals stub-mask×255 exactly
  (`np.array_equal`) and whose RGB equals the LANCZOS-resized input; (b)
  `matte_image` on a 800×600 RGB returns 800×600 (identity framing — the
  property `matte_concept` exists for); (c)
  `np.array_equal(np.asarray(p.preprocess_image(rgb_img, resolution=1024)), np.asarray(p.preprocess_image(p.matte_image(rgb_img), resolution=1024)))`
  is True — the RGB branch routes through `matte_image`, so the two entries
  can never drift. Run the new test + extraction harness (`3/3 cases passed`)
  under `C:\tools\Hi3DGen\venv\Scripts\python.exe` → commit on `vordar-fixes`.

### 3. `hi3dgen/headless.py`: Session, identity, and package-owned env defaults

- **Evidence:** Everything upstream-shaped in `scripts/ai-pipeline/prop_hi3dgen.py`
  today: env pins before import (lines 22-34), fork-internal imports for the
  backend assert and manifest (lines 58-70), weight/entrypoint constants
  (72-98), `hi3dgen_id()` git identity (107-119), `vram_peaks`/`resident_gib`
  (122-138), the finding-17 stage offload `staged`/`staged_cond`/`staged_sample`
  (141-188), the normal-predictor hub load (361-371), the BiRefNet free after
  its last consumer (388-390), the checkpoint-param merge (422-423), and the
  extractor handle for `last_extract_s` (358, 508). `fork
  hi3dgen/modules/sparse/conv/__init__.py:53` defaults `SPCONV_ALGO = 'auto'`;
  `fork app.py:32` hard-sets it to `'native'`; `hi3dgen/__init__.py:33-36`
  imports the submodules that read these env vars, so any consumer's pin must
  precede `import hi3dgen`.
- **Ideal:** `from hi3dgen.headless import Session` is the fork's one headless
  entry: constructing a `Session` loads and stages every model, `matte()` /
  `prepare()` / `sample(seed)` run the three phases with the models resident
  only during their windows, `identity()` hands the manifest its
  toolchain/weights/backends/versions block, and importing the package alone
  yields working backend defaults (`SPCONV_ALGO=native` unless the environment
  overrides).
- **Gap:** No such module exists; the knowledge lives in the vordar script.
- **Suggestion:** (1) At the very top of `fork hi3dgen/__init__.py`, before the
  submodule imports: `import os` + `os.environ.setdefault("SPCONV_ALGO",
  "native")`, with a comment stating the constraint (the conv backend reads
  this at import time; `native` is the deterministic algo every consumer of
  this fork runs). Delete `app.py:32`'s hard-set (superseded;
  `python -m py_compile app.py` still passes). No `ATTN_BACKEND` setdefault —
  the code defaults are already `xformers`. (2) New `fork
  hi3dgen/headless.py`, content moved from `prop_hi3dgen.py`:
  - Module top: `os.environ.setdefault("HF_HUB_OFFLINE", "1")` and
    `os.environ.setdefault("CUBLAS_WORKSPACE_CONFIG", ":4096:8")` with their
    existing constraint comments (offline guard against silent network
    fallback; cuBLAS var must precede CUDA context creation).
  - `REPO_DIR = Path(__file__).resolve().parents[1]` (self-locating — no
    hardcoded `C:\tools` path survives in the package), `GEOMETRY_WEIGHTS =
    REPO_DIR / "weights" / "trellis-normal-v0-1"`, and the constants moved
    verbatim: `NORMAL_WEIGHTS_REPO`, `YOSO_VERSION`, `STABLE_NORMAL_FULL_REPO`,
    `STABLE_NORMAL_DIFFUSION_VERSION`, `NORMAL_ENTRYPOINTS`,
    `STABLE_NORMAL_HUB_SNAPSHOT`, `SS_SAMPLING_STEPS_DEFAULT = 50`,
    `SLAT_SAMPLING_STEPS_DEFAULT = 6`, `SS_CFG_DEFAULT = 5.0`,
    `SLAT_CFG_DEFAULT = 5.0`, `NORMAL_RESOLUTION_DEFAULT = 1024`,
    `OCCUPANCY_THRESHOLD_DEFAULT = 0.0`, `GIB`.
  - `checkout_id() -> dict` (= `hi3dgen_id()`, `git -C REPO_DIR`),
    `vram_peaks() -> dict`, `_resident_gib()` — moved verbatim with their
    comments.
  - `@dataclass Prepared`: `normal_image: Image.Image`, `elapsed_s: dict`
    (keys `preprocess`, `normal`, `cond` — wall times measured inside).
  - `@dataclass Candidate`: `mesh_result` (the raw `MeshExtractResult`),
    `mesh: trimesh.Trimesh` (`to_trimesh(transform_pose=True)` applied),
    `rng_state_sha256: str`, `extract_s: float`
    (`extractor.last_extract_s` — finding 24's sub-interval, must stay
    reachable), `sampler_params: dict`
    (`{"sparse_structure": …, "slat": …}` — the merged dicts the samplers
    actually ran with), `extraction: dict` (`res`, `min_component_fraction`,
    `iso_level`, `sdf_bias` off the extractor + the `occupancy_threshold`
    passed in).
  - `class Session`: `__init__(self, normal_model="turbo", device="cuda")` —
    validate `normal_model in NORMAL_ENTRYPOINTS` (raise `ValueError` before
    any load), assert `hi3dgen.modules.sparse.ATTN == "xformers"` and
    `hi3dgen.modules.sparse.conv.SPCONV_ALGO == "native"` (the fail-loud
    guards, moved with their comments), `Hi3DGenPipeline.from_pretrained(GEOMETRY_WEIGHTS)`,
    set `pipeline.device`, keep `self.extractor =
    pipeline.models["slat_decoder_mesh"].mesh_extractor`, eager-load BiRefNet
    via the step-2 `pipeline._lazy_load_birefnet()` (so the cost sits in the
    load window, as today), `torch.hub.load` the normal predictor from
    `STABLE_NORMAL_HUB_SNAPSHOT` with `local_cache_dir=str(REPO_DIR /
    "weights")`, record `self.resident = {"after_load": _resident_gib()}`.
    `matte(self, image) -> Image` — `pipeline.matte_image(image)`.
    `prepare(self, image, matte, *, normal_resolution=NORMAL_RESOLUTION_DEFAULT,
    normal_steps=None, crop_from_original=False, seed) -> Prepared` — free
    BiRefNet (`del` + `empty_cache`, record `after_birefnet_free`), build the
    conditioning source (`matte`, or the moved `full_res_conditioning_source`
    logic when `crop_from_original`), `preprocess_image(source,
    resolution=1024)`, `torch.manual_seed(seed)` (the full predictor draws
    from the ambient RNG — comment moves with the code), predict the normal,
    free the predictor (record `after_normal_free`), staged
    `get_cond([normal_image])`, return `Prepared` with the three timings.
    `sample(self, seed, *, ss_steps=…, slat_steps=…, ss_cfg=…, slat_cfg=…,
    occupancy_threshold=OCCUPANCY_THRESHOLD_DEFAULT) -> Candidate` — merge
    checkpoint sampler params exactly as `prop_hi3dgen.py:422-423` does, then
    the moved `staged`/`staged_sample` body: `torch.manual_seed(seed)`, rng
    digest, staged sparse-structure → slat → mesh decode, `to_trimesh`,
    return `Candidate`. `identity(self) -> dict` — `{"hi3dgen":
    checkout_id(), "weights": {"geometry": str(GEOMETRY_WEIGHTS), "normal":
    NORMAL_WEIGHTS_REPO, "normal_diffusion": <full repo or None>, "birefnet":
    BIREFNET_REPO}, "backends": {"attn": …, "sparse_attn": …, "spconv_algo":
    …}, "versions": {torch, cuda, xformers, spconv, trimesh, skimage,
    numpy}}` — the exact blocks the manifest carries today, sourced from the
    fork-internal imports that then leave the vordar script.
  This step adds the module and its cheap tests only; `prop_hi3dgen.py` is
  untouched and keeps working (its hard env sets coexist with the new
  setdefaults; its `preload_birefnet` still pre-empts the lazy load). The
  GPU-path behavior of `Session` is proven by step 4's smoke — the load/sample
  path is too heavy for a per-step unit test and would duplicate that smoke.
- **Path:** Implement (1) and (2) → add fork test
  `C:\tools\Hi3DGen\Hi3DGen\tests\test_headless_contract.py` (plain asserts):
  (a) in a subprocess with `SPCONV_ALGO` removed from the environment,
  `C:\tools\Hi3DGen\venv\Scripts\python.exe -c "import hi3dgen; from hi3dgen.modules.sparse import conv; print(conv.SPCONV_ALGO)"`
  prints `native` — the package default reaches the conv module through the
  real import chain; (b) same subprocess pattern with `SPCONV_ALGO=auto`
  exported prints `auto` — the environment still overrides (setdefault, not
  set); (c) `headless.checkout_id()["rev"]` equals the output of
  `git -C <REPO_DIR> rev-parse HEAD` run by the test, and `["dirty"]` is a
  bool; (d) `Session(normal_model="bogus")` raises `ValueError` (and returns
  in well under a second — it must refuse before loading 5 GB of weights).
  Run the new test + extraction harness (`3/3 cases passed`) → `python -m
  py_compile C:\tools\Hi3DGen\Hi3DGen\app.py` → commit on `vordar-fixes`.

### 4. Rewrite `prop_hi3dgen.py` as CLI + gates + manifest over `hi3dgen.headless`

- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py` (541 lines) — everything
  listed in step 3's Evidence is still in the file, plus the genuinely-vordar
  parts that stay: argparse CLI (311-339), resume/skip over
  `CANDIDATE_OUTPUTS` (341-351), `check_matte` (218-238), `check_mesh`
  (271-308), the per-candidate manifest (455-517), prints and the VRAM warning
  (519-536). `scripts/tests/test_prop_hi3dgen.py` unit-tests the gates through
  `import prop_hi3dgen` and needs no change (the editable install from step 1
  now satisfies its `from hi3dgen.pipelines import Hi3DGenPipeline`).
  Reference run for parity, on disk:
  `target/prop-solid-validation/chapel_arch_e2e/cand_0/hi3dgen_manifest.json`
  — input `target/prop-batch/b3/arch/cand_0/concept.png` (exists, sha
  `2bd695fe…`), seed 0, turbo, resolution 1024, steps None, crop False,
  `normal_sha256 = 822b22e5c2529af6e601ceffc813a1120b73d90cd00265ed6b8d7e7965e98f8f`,
  768,804 faces, wall ~48 s warm (load 28.4 + candidate 17.8). The fork
  commits since that manifest's rev (`cf718c6` → `c7389f5`) touched only the
  extraction stage, and finding 6 measured the turbo normal map bit-identical
  across same-input runs, so `normal_sha256` is a cross-refactor invariant;
  face count moves by GPU noise (measured spread 768,462↔768,804 ≈ 0.04%)
  plus the fill deletion's measured −0.021%.
- **Ideal:** The script is CLI + gates + manifest only: no `os.environ`, no
  `sys.path`, no `import torch`, no fork-internal imports — its only fork
  import is `from hi3dgen import headless`. Behavior, CLI surface, output
  files and manifest key set are unchanged.
- **Gap:** The script still carries every workaround the fork now owns.
- **Suggestion:** Delete: the env block (22-34), `REPO_DIR`/`sys.path` (51-61),
  the fork-internal imports and the `ATTN` assert (58-70, now inside
  `Session`), the constants that moved (72-98 — the CLI takes its argparse
  defaults from `headless.SS_SAMPLING_STEPS_DEFAULT` etc.), `hi3dgen_id`,
  `vram_peaks`, `resident_gib`, `staged`, `staged_cond`, `staged_sample`,
  `preload_birefnet`, `matte_concept`, `full_res_conditioning_source`, and
  the imports their deletion orphans (`torch`, `skimage`, `spconv`,
  `xformers`, `contextmanager`, `subprocess`). Keep: docstring (updated: drop
  the "plain repo checkout" and env-var rationale), `CANDIDATE_OUTPUTS`, the
  exceptions, `check_matte`, `check_mesh`, `main()`. New `main()` flow, with
  timing points preserved: parse args (same flags; `--seed` repeat rules and
  the `--normal-model full` multi-seed refusal stay as `parser.error`s) →
  resume/skip → `t0`; `session = headless.Session(normal_model=args.normal_model)`;
  `t_loaded` → `concept_rgba = session.matte(image)`; `check_matte` gate
  (exit before any normal prediction is paid); save `concept_rgba.png` per
  pending candidate; `t_matted` → `prepared = session.prepare(image,
  concept_rgba, normal_resolution=…, normal_steps=…, crop_from_original=…,
  seed=seeds[0])`; save `normal.png` per candidate → `shared_elapsed_s =
  {"load": t_loaded - t0, "preprocess": (t_matted - t_loaded) +
  prepared.elapsed_s["preprocess"], "normal": prepared.elapsed_s["normal"],
  "cond": prepared.elapsed_s["cond"]}` → per pending seed: time
  `cand = session.sample(seed, ss_steps=…, slat_steps=…, ss_cfg=…,
  slat_cfg=…)` as `geometry`; `check_mesh(cand.mesh_result, cand.mesh)` gate;
  export `raw.glb`; manifest exactly as today with `**session.identity()`
  supplying `hi3dgen`/`weights`/`backends`/`versions`,
  `cand.rng_state_sha256`, `cand.sampler_params`, `cand.extraction`,
  `"extraction": cand.extract_s` inside `elapsed_s` (keeping the
  sub-interval comment), and `vram = headless.vram_peaks()` +
  `session.resident`. Prints and the 0.9-fill VRAM warning unchanged.
- **Path:** Rewrite → run the vordar unit suite under the venv:
  `C:\tools\Hi3DGen\venv\Scripts\python.exe -m unittest discover -s scripts/tests -t .`
  from the vordar root — all `test_prop_hi3dgen.py` cases pass (gates
  unchanged; the import now resolves through the editable install) → **GPU
  parity smoke (heavy compute, named here for the go-ahead: one turbo
  candidate, ~1-2 min wall on the measured 48 s warm baseline; the only GPU
  run in this plan):**
  `C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_hi3dgen.py target/prop-batch/b3/arch/cand_0/concept.png --out target/prop-solid-validation/rework3-smoke --seed 0`
  then assert, in a small check script or by inspection of the written
  manifest: (a) `normal_sha256 ==
  822b22e5c2529af6e601ceffc813a1120b73d90cd00265ed6b8d7e7965e98f8f` — the
  normal path reproduced bit-exact through the refactor; (b) top-level key
  set equals the reference manifest's
  (`target/prop-solid-validation/chapel_arch_e2e/cand_0/hi3dgen_manifest.json`),
  `elapsed_s` keys equal the reference's plus `extraction`, and
  `extraction` block keys == `{res, min_component_fraction, iso_level,
  sdf_bias, occupancy_threshold}`; (c) `0 < elapsed_s.extraction <
  elapsed_s.geometry` (the finding-24 sub-interval survives); (d)
  `face_count` within 1% of 768,804 — a sanity bound an order of magnitude
  above the measured 0.04% GPU noise and the fill's −0.021%, not a tuned
  gate; (e) `grep`-level predicate on the new file: no `sys.path`, no
  `os.environ`, no `import torch` remain. Commit vordar-side.

### 5. Delete the workaround block from `prop_extract.py` and the fork tests' path hacks

- **Evidence:** `scripts/ai-pipeline/prop_extract.py:19-37` carries the same
  env pins + `REPO_DIR`/`sys.path` insertion as the old runner, though its
  replay loads no weights and touches no network or CUDA (CPU device default;
  `SparseFeatures2Mesh` is pure computation); `fork
  tests/test_extraction_contract.py:24` self-inserts its parent onto
  `sys.path` and its docstring pins "Run from the fork root". Step 3 added
  `tests/test_headless_contract.py` and step 2 `tests/test_preprocess_contract.py`
  in the same pattern. Measured replay baseline on record (fork `973df9e`,
  reconfirmed unchanged by `c7389f5` which only added timing):
  `target/prop-latents/candelabra_shrine/cubefeats.pt` extracts to exactly
  167,479 vertices / 334,938 faces on CPU in ~13 s.
- **Ideal:** Both repos' scripts import the installed package directly; the
  only env pin that remains anywhere is the package-owned setdefault chain.
- **Gap:** Three files still carry the superseded workaround (swap rule:
  delete, no compatibility path).
- **Suggestion:** In `prop_extract.py`: delete lines 19-37 (env block,
  `REPO_DIR`, `sys.path` insert) and the now-stale "plain repo checkout"
  comment; keep `MESH_RESOLUTION` and everything else. The env vars are
  genuinely dead here: the replay reads no HF weights (`HF_HUB_OFFLINE`
  moot), runs on CPU (`CUBLAS` moot), and the backend defaults now come from
  `hi3dgen/__init__` (`SPCONV_ALGO=native`) with `xformers` already the code
  default. In the fork tests: delete each `sys.path.insert` line and update
  the docstrings' run instructions to drop the "from the fork root"
  constraint (any cwd now works).
- **Path:** Make both edits → CPU replay gate:
  `C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_extract.py target/prop-latents/candelabra_shrine --out <scratch dir> --device cpu`
  run from a cwd outside both repos; assert the printed JSON has
  `vertex_count == 167479` and `face_count == 334938` — the deterministic
  CPU replay reproduces the recorded baseline exactly, proving the import
  path swap changed nothing → fork harness from a non-fork cwd:
  `C:\tools\Hi3DGen\venv\Scripts\python.exe C:\tools\Hi3DGen\Hi3DGen\tests\test_extraction_contract.py`
  prints `3/3 cases passed` → commit fork-side and vordar-side.

### 6. Update the README's install and runner sections (docs-only)

- **Evidence:** `scripts/ai-pipeline/README.md:399-431` documents the fork
  install as clone + venv + pinned wheels with no package install;
  `README.md:421-427` states "Runtime env vars (same convention as TRELLIS,
  required for every run): ATTN_BACKEND=xformers SPCONV_ALGO=native";
  `README.md:452-454` says importing `Hi3DGenPipeline` works "from
  C:\tools\Hi3DGen\Hi3DGen, with ATTN_BACKEND=xformers SPCONV_ALGO=native
  set"; `README.md:501-520` describes `prop_hi3dgen.py` including "this
  script's own `preprocess_image` step".
- **Ideal:** The README's fresh-machine recipe includes the editable install
  and states the new contract: backends default from the package
  (`SPCONV_ALGO=native` via setdefault, still overridable), no per-run env
  vars, no path hacks; the runner description reflects that preprocessing and
  model lifecycle live in `hi3dgen.headless`.
- **Gap:** Four sections describe the superseded arrangement.
- **Suggestion:** In the install block after the pip lines, add:
  `C:\tools\Hi3DGen\venv\Scripts\python.exe -m pip install -e C:\tools\Hi3DGen\Hi3DGen --no-deps`
  with one sentence on why `--no-deps` (hand-pinned GPU stack; pyproject
  declares no dependencies). Replace the "Runtime env vars … required for
  every run" paragraph with the package-default contract. Fix the
  `Hi3DGenPipeline` import note (no cwd/env preconditions; venv only). In the
  `prop_hi3dgen.py` section, re-attribute the matte/preprocess/lifecycle to
  `hi3dgen.headless` and change "this script's own `preprocess_image` step"
  to the fork's. Do not touch the weights table (still accurate) or the
  version-divergence history block.
- **Path:** Edit the four sections → verify every command shown is one the
  earlier steps actually ran verbatim (install line from step 1, runner line
  unchanged from step 4) and that no `ATTN_BACKEND`/`SPCONV_ALGO` "required"
  claim survives (`grep -n "ATTN_BACKEND" scripts/ai-pipeline/README.md`
  shows only the package-default note). Full gate: none beyond review — no
  source or content changed in this step.
