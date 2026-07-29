# Plan: Multi-view conditioning — stop hallucinating every back side — 2026-07-28

Source: `docs/reviews/hi3dgen/reworks-hi3dgen-2026-07-28.md` finding 2.
Written 2026-07-29. Anchors: `fork:` = `C:/tools/Hi3DGen/Hi3DGen` (branch
`vordar-fixes`, HEAD `c7389f5`), unprefixed = vordar-repo relative. Fork venv:
`C:\tools\Hi3DGen\venv\Scripts\python.exe`.

## Ideal end state

`prop_hi3dgen.py` accepts extra conditioning views (`--view back.png --view
side.png`) and routes them through per-view matte → per-view StableNormal
prediction → `get_cond([normals])` → the fork's already-written
`inject_sampler_multi_image` contexts, with the mode and every view's
provenance in the manifest. An A/B on one asymmetric prop and one character
subject measures whether the extra views actually move the far half of the
geometry toward the provided back/side images — with the same-seed GPU noise
floor bounded *before* any difference is claimed. Both arms produce hollow
shells (rework 1's closure: the SLat representation cannot hold a solid, every
prop is a double-walled shell permanently), so every metric here is chosen to
be valid on hollow meshes. The verdict decides whether production wiring
(concept stages emitting view sets) is queued; that wiring is explicitly not
this plan.

## Design decisions

**1. Integrate in vordar's staged path; reuse the fork's injection unchanged.**
The finding says "we call single-image `run()` at `prop_hi3dgen.py:169`" — that
is stale. Since finding 17, `prop_hi3dgen.py` reimplements the pipeline as
`staged_cond()` / `staged_sample()` (`prop_hi3dgen.py:157-188`) so each model's
weights are resident only during its stage (peak 6.79 GiB vs 15.57 before), and
it passes `occupancy_threshold` explicitly. Calling `fork:run_multi_image`
(`hi3dgen.py:455-487`) would throw all of that away. The right seam is:
`inject_sampler_multi_image` (`fork:hi3dgen/pipelines/hi3dgen.py:397-452`) is a
self-contained contextmanager that monkey-patches `sampler._inference_model`;
we wrap vordar's existing `sample_sparse_structure` / `sample_slat` calls in it
and change nothing in the fork's pipeline code. Verified compatible: both
stages use `FlowEulerGuidanceIntervalSampler`
(`fork:weights/trellis-normal-v0-1/pipeline.json:11,23`), whose
`_inference_model` signature `(model, x_t, t, cond, neg_cond, cfg_strength,
cfg_interval, **kwargs)` (`guidance_interval_mixin.py:33`) matches the
multidiffusion replacement's signature exactly (`hi3dgen.py:429`).
`run_multi_image` itself stays unused; rework 3 (headless API) absorbs or
deletes it later.

**2. Conditioning views are per-view normal maps.** The pipeline conditions on
the StableNormal bridge output, not RGB (`prop_hi3dgen.py:424`:
`staged_cond(hi3dgen_pipeline, normal_image)`). So a view = matte
(`matte_concept` + `check_matte`) → `preprocess_image(resolution=1024)` →
turbo normal prediction → DINOv2 encoding. Turbo's prediction is the
deterministic image latent (the reason multi-seed batching already shares one
prediction), so per-view predictions add ~1 s each (measured `elapsed_s.normal`
= 0.92 s, `target/prop-solid-validation/chapel_arch_e2e/cand_0/hi3dgen_manifest.json`).
`--normal-model full` stays legal with `--view` under a single seed — its
existing multi-seed refusal (`prop_hi3dgen.py:332-339`) is unchanged and
sufficient.

**3. The noise floor is bounded by a determinism probe first, repeats second —
and the A/B is paired by seed.** Rework 6 (unresolved) measured three same-seed
runs at 541220/541286/541242 vertices from byte-identical code; a single-run
A/B is unreadable against that. `CUBLAS_WORKSPACE_CONFIG` is already exported
(`prop_hi3dgen.py:34`) but `torch.use_deterministic_algorithms` is called
nowhere — the flag has never been tried. Whether it (a) errors on
`scatter_reduce`/spconv ops, (b) runs but leaves spconv nondeterministic, or
(c) pins the mesh, is a hypothesis whose test is step 4, priced at 3–6 runs of
~1 min. If (c): the floor is exactly 0 and every later arm is single-run. If
(a)/(b): the floor per metric is measured from the same 3 same-seed repeats,
and the A/B claim rule becomes: per-seed delta (arm − baseline) must have the
same sign at all 3 seeds AND its minimum magnitude must exceed that metric's
measured floor. No expected magnitudes are pre-registered; if deltas land
inside the floor the verdict is "indistinguishable at 3 seeds" and the next
spend goes to the user, not to a quiet threshold move.

**4. Metrics valid on hollow meshes, instrument validated analytically.** Both
arms are hollow shells, so no volume, no watertightness, no interior criteria
(`two_crossing_ray_fraction` and `is_watertight` no longer exist — deleted at
`feacbb0`/`cbecd72`). Primary metric: **bbox-normalized silhouette IoU** of the
raw mesh's orthographic projection against each concept view's matte — a pure
outer-silhouette measure the inner wall cannot touch. Secondary: raw-mesh
connectivity stats (component count, boundary-edge count, main-island face
fraction) computed by the same script — hollow-valid because they describe the
closed raw surface, which rework 1 established is watertight-ish on the main
island before our own cleanup cuts it. The instrument is a ~150-line
pure-Python script (trimesh + cv2, both pinned in `fork:requirements.txt:22,25`
for the Hi3DGen venv): rotate vertices by (azimuth, elevation=15°, the
pipeline's `MV_ELEVATION_DEG` convention at `proptex/views.py:20`), project
orthographically, rasterize all faces in one `cv2.fillPoly` call. Yaw is fitted
per mesh by scanning azimuth 0..355° in 5° steps for max front-view IoU, then
back = fit+180°, side = max(fit±90°) — applied identically to every arm, so
the scan granularity and the fixed elevation cannot bias the comparison. The
projection itself is validated against an analytic ground truth (a box mesh
whose silhouette aspect and fill are known exactly), not against its own
output.

**5. Concept-side view-generation recipe — USER DECISION.** Four options,
each with independent outcome (as if free) and cost weights:

- **A. Z-Image per-view prompt-only** (reuse `workflows/prop_concept.json`
  with `view_hint` text per view, same subject, per-view seeds). Outcome
  **3/10** — nothing ties the views to one object instance; the back view will
  be a different design, and conditioning on contradictory views teaches the
  A/B nothing about the mechanism. Cost **1/10**.
- **B. One Z-Image sheet, split** (single generation: "three views of the same
  X side by side — front, side, back", 1536×512, equal-thirds crop). In-image
  consistency is the model's strength; the panels genuinely describe one
  object, and the back is independent information (the image model designs it,
  not the mesh). All three model files (`z_image_turbo_bf16`,
  `qwen_3_4b_fp8_mixed`, `ae.safetensors`) are already pinned in
  `models.sha256` — no new tool, no licensing work. Whether Z-Image-Turbo lays
  out a clean 3-panel sheet is unmeasured; its test is a seconds-scale ComfyUI
  probe inside step 5, retried across seeds. Outcome **6/10** (7/10 for
  characters — turnaround sheets are in-distribution; weaker for props). Cost
  **2/10**.
- **C. Depth-ControlNet turntable of the approved single-view candidate**
  (render back/side depth of the single-view mesh via `proptex/views.py`, feed
  `workflows/prop_multiview.json` — all tooling exists in
  `proptex/generate.py`). Rejected *for the A/B*: the back views' silhouettes
  are derived from the single-view arm's own hallucinated mesh, so the
  back-IoU metric would be grading the instrument's own input — circular. It
  remains a legitimate production recipe for texture-consistent detail later.
  Outcome **4/10** for this A/B (6/10 as a production refinement pass), cost
  **3/10**.
- **D. Adopt a dedicated multiview image model** (MV-Adapter, Era3D,
  Wonder3D-class: front image → posed consistent novel views). The technically
  strongest independent-back source and the likely production endgame. Requires
  a recency-constrained research pass with head-to-head/practitioner evidence
  (lesson: research-needs-experience-sources) and a licensing gate (strict NC
  ruling — NC tools never touch the shipping asset path). Outcome **8/10**,
  cost **6/10**.

**Recommendation: B for this A/B**, with D queued as the follow-up *if* the
A/B proves multi-view conditioning moves geometry at all — B answers the
mechanism question for two orders of magnitude less work, and D's research is
wasted if the mechanism is a null result. The plan's steps 5–7 are written for
B; if the user picks another option, step 5 is replanned and steps 6–8 consume
whatever view set it produces (they only require "≥3 matted view images per
subject on disk").

**6. Conditioning mode: measure both, don't pre-decide.** `stochastic`
round-robins views across steps; `multidiffusion` averages per-view
predictions under CFG each step (the quality option per TRELLIS, ~2× the
sparse-structure sampling cost — trivial against a 17 s geometry stage). Cost
does not force a choice and no quality evidence exists, so both are A/B arms
(three arms total: single-view, mv-stochastic, mv-multidiffusion). If the A/B
separates them, the winner becomes the flag default; if not, the choice goes to
the user with multidiffusion recommended (upstream's quality mode, cost
trivial). The flag default until then is `multidiffusion`.

**7. Scope: A/B verdict, not production wiring.** Wiring `gen_prop.py` /
`gen_character.py` concept stages to emit view sets (and `char_concept.json`'s
SDXL→Z-Image migration, which predates the Z-Image ruling) is queued as
follow-up work by the verdict step, contingent on a positive result. The
character A/B subject is an evaluation-only concept chosen autonomously
(standing model-asset autonomy; AA dark-fantasy direction), not shipped
content. No character asset exists in `content/models/assets.json` today, and
none is registered by this plan.

## Findings (execution order)

### 1. Fork: contract test for `inject_sampler_multi_image`

- **Evidence:** `fork:hi3dgen/pipelines/hi3dgen.py:397-452` implements the two
  conditioning modes as an untested contextmanager that monkey-patches
  `sampler._inference_model`. Nothing in `fork:tests/` exercises it — the only
  test module is `tests/test_extraction_contract.py` (plain asserts, no
  pytest, run as `C:\tools\Hi3DGen\venv\Scripts\python.exe
  tests/test_extraction_contract.py` from the fork root). Both production
  samplers are `FlowEulerGuidanceIntervalSampler`
  (`fork:weights/trellis-normal-v0-1/pipeline.json:11,23`); its base
  `_inference_model` multiplies `t` by 1000 and calls `model(x_t, t, cond,
  **kwargs)` (`fork:hi3dgen/pipelines/samplers/flow_euler.py:61-63`), and the
  guidance-interval mixin computes `(1+cfg)·pred − cfg·neg_pred` inside
  `cfg_interval` (`fork:hi3dgen/pipelines/samplers/guidance_interval_mixin.py:33-39`).
- **Ideal:** A committed CPU-only test pins the injection's three behavioral
  claims — stochastic round-robin order, multidiffusion's averaged-CFG
  formula, and restoration on context exit — so step 2's plumbing rests on a
  proven seam.
- **Gap:** The mechanism this whole rework routes through has zero test
  coverage; a silent signature drift (the injected functions duplicate the
  mixin signatures by hand) would corrupt geometry rather than crash.
- **Suggestion:** New `fork:tests/test_sampler_injection.py`, plain asserts,
  same header/env conventions as `test_extraction_contract.py`
  (`ATTN_BACKEND=xformers`, `SPCONV_ALGO=native`, `HF_HUB_OFFLINE=1`,
  `sys.path` insert). No GPU, no weights: build a bare pipeline via
  `p = Hi3DGenPipeline()` (its `__init__` returns early when `models is None`,
  `fork:hi3dgen/pipelines/hi3dgen.py:57-58`), then
  `p.sparse_structure_sampler = FlowEulerGuidanceIntervalSampler(sigma_min=1e-5)`.
  Dummy model: a function `model(x_t, t, cond, **kw)` that appends
  `float(cond[0,0,0])` to a call log and returns
  `torch.full_like(x_t, float(cond[0,0,0]))`. Cond tensor `(3, 2, 4)` with
  `cond[i] = i+1` everywhere; `neg = torch.zeros(1, 2, 4)`; `x_t =
  torch.zeros(1, 2, 4)`.
- **Path:**
  1. **Stochastic:** inside
     `p.inject_sampler_multi_image('sparse_structure_sampler', num_images=3,
     num_steps=6, mode='stochastic')`, call `sampler.sample(model, noise=x_t,
     cond=cond, neg_cond=neg, steps=6, cfg_strength=5.0, cfg_interval=(0.5,
     1.0), verbose=False)`. Assert the call log's *positive*-cond entries (the
     nonzero ids) are exactly `[1.0, 2.0, 3.0, 1.0, 2.0, 3.0]` — one per step,
     round-robin — and that every logged cond had batch size 1.
  2. **Multidiffusion:** inside a fresh context with `mode='multidiffusion'`,
     call `sampler._inference_model(model, x_t, t=0.75, cond=cond,
     neg_cond=neg, cfg_strength=5.0, cfg_interval=(0.5, 1.0))` directly.
     Expected: per-view preds are 1, 2, 3 → mean 2; neg pred 0; inside the
     interval the result is `(1+5)·2 − 5·0 = 12`. Assert
     `torch.allclose(out, torch.full_like(x_t, 12.0))`. Repeat at `t=0.3`
     (outside `[0.5, 1.0]`): assert `allclose(out, full_like(x_t, 2.0))`.
  3. **Restoration:** after each context exits, assert
     `not hasattr(sampler, '_old_inference_model')` and that
     `sampler._inference_model(model, x_t, t=0.75, cond=cond[:1],
     neg_cond=neg, cfg_strength=5.0, cfg_interval=(0.5, 1.0))` returns
     `full_like(x_t, 6.0)` — the original mixin math `(1+5)·1 − 5·0` — i.e.
     behavior, not object identity (the contextmanager legitimately leaves a
     bound-method instance attribute behind).
  4. Run `C:\tools\Hi3DGen\venv\Scripts\python.exe
     tests/test_sampler_injection.py` from `C:/tools/Hi3DGen/Hi3DGen` — the
     new file must pass; also run `tests/test_extraction_contract.py`
     (unchanged, 3/3) so the fork test suite stays green. Commit on
     `vordar-fixes`. No vordar-repo files change; no cargo gate applies.

### 2. `prop_hi3dgen.py`: opt-in multi-view CLI, per-view matte/normal, injected sampling

- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py` is single-view end to
  end: one `matte_concept`/`check_matte` (`:376-381`), one
  `preprocess_image(resolution=1024)` (`:394`), one normal prediction
  (`:402-405`), `staged_cond(pipeline, normal_image)` encoding exactly one
  image (`:157-165`, `:424`), and `staged_sample` (`:168-188`) calling
  `sample_sparse_structure`/`sample_slat` bare. The fork's
  `run_multi_image` shows the multi-image recipe: `get_cond(images)`, then
  `cond['neg_cond'] = cond['neg_cond'][:1]`, then each sampler wrapped in
  `inject_sampler_multi_image(name, len(images), steps, mode)`
  (`fork:hi3dgen/pipelines/hi3dgen.py:476-487`). vordar's merged param dicts
  already carry `steps` explicitly (`:422-423`), so the injection's
  `num_steps` is `ss_params["steps"]` / `slat_params["steps"]`.
- **Ideal:** `prop_hi3dgen.py <front.png> --view back.png --view side.png
  --mv-mode multidiffusion --out D --seed N` runs the full staged path with
  three conditioning views; a run without `--view` is byte-for-byte the
  current single-view behavior; the manifest records every view's provenance
  and the mode.
- **Gap:** No CLI, no per-view processing loop, no injection call sites, no
  manifest fields.
- **Suggestion:** Changes confined to `prop_hi3dgen.py`:
  - **CLI:** `--view PATH` (`action="append"`, `dest="extra_views"`,
    `type=Path`, default `[]`) — the positional `image` is view 0 (front);
    `--mv-mode` `choices=("stochastic", "multidiffusion")`,
    `default="multidiffusion"` (only consulted when extra views exist).
  - **Per-view loop:** build `views = [args.image] + args.extra_views`
    (resolve each). For each view k: open RGBA → `matte_concept` →
    `check_matte` (abort names the failing view) → save `concept_rgba.png`
    for k=0 (unchanged name — downstream consumers keep working) and
    `concept_rgba_v{k}.png` for k≥1 → conditioning source
    (`full_res_conditioning_source` per view when `--crop-from-original`) →
    `preprocess_image(resolution=1024)`. Free BiRefNet after the *last*
    view's matte (move the existing `del` after the loop). Then one
    `torch.manual_seed(seeds[0])` (as today, `:401`) and a normal prediction
    per view in view order — turbo's latent is the deterministic image
    latent, so the shared-across-seeds property is untouched; save
    `normal.png` (k=0) / `normal_v{k}.png` (k≥1).
  - **Conditioning:** generalize `staged_cond` to take a list:
    `pipeline.get_cond(images)` then `cond["neg_cond"] =
    cond["neg_cond"][:1]` unconditionally (identity for the single-view
    case — `get_cond` builds `neg_cond` as `zeros_like(cond)`,
    `fork:hi3dgen.py:271-276`).
  - **Sampling:** `staged_sample` gains `n_views: int` and `mv_mode: str`;
    when `n_views > 1` wrap the `sample_sparse_structure` call in
    `pipeline.inject_sampler_multi_image('sparse_structure_sampler', n_views,
    ss_params["steps"], mode=mv_mode)` and the `sample_slat` call in the same
    for `'slat_sampler'` with `slat_params["steps"]` — each context *inside*
    its existing `staged(...)` block so weight residency is unchanged.
  - **Manifest:** add `"views"`: list of `{path, input_sha256,
    concept_rgba_sha256, normal_sha256}` (one entry per view, view 0 first)
    and `"mv_mode"`: the mode string when `len(views) > 1`, else `None`.
    Existing top-level fields (`input_image`, `concept_rgba`, `normal`, their
    hashes) keep describing view 0.
  - Duplicate `--view` paths are allowed (the identity smoke below depends on
    it); repeated seeds stay refused as today.
- **Path:**
  1. Implement the above.
  2. Single-view regression (no GPU): `python -c` import-and-argparse is not a
     behavioral test — instead rely on the GPU smoke, which exercises both
     modes of the changed code.
  3. **GPU identity smoke (~3 min, §8: two ~1 min candidates + shared
     ~30 s loads):** run
     `C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_hi3dgen.py
     target/prop-batch/b3/arch/cand_0/concept.png --out target/mv-ab/smoke-sv
     --seed 0`, then the same with `--view
     target/prop-batch/b3/arch/cand_0/concept.png --out target/mv-ab/smoke-dup
     --mv-mode multidiffusion`. Assertions: both exit 0; `smoke-dup`'s
     manifest has `mv_mode == "multidiffusion"` and `len(views) == 2` with
     both `input_sha256` equal; `normal_v1.png` exists and its sha256 equals
     `normal.png`'s (turbo is deterministic per image); and — the behavioral
     core — with duplicated identical views, multidiffusion's averaged
     prediction is mathematically the single-view prediction, so
     `smoke-dup`'s `vertex_count` must be within 0.1% of `smoke-sv`'s (the
     rework-6 same-seed spread measured 66 vertices in 541k ≈ 0.012%; 0.1%
     is an order of magnitude above that floor and two below any real
     conditioning change). Record both counts in the step's summary. If step
     4 later proves determinism, this pair is re-assertable as bit-identical
     `raw.glb` hashes — do not wait on that here.
  4. Single-view arm check rides the same smoke: `smoke-sv`'s manifest must
     carry `"mv_mode": null` and a one-entry `views` list.

### 3. `mv_ab_metrics.py`: silhouette-IoU and raw-stats instrument

- **Evidence:** No tool measures a raw mesh against a concept view. The
  camera convention to mirror is `proptex/views.py:54-67` (`mv_view`:
  direction `d = [sin az·cos el, −cos az·cos el, sin el]`, right `s =
  normalize(cross(f, [0,0,1]))`, up `u = cross(s, f)`, `f = −d`) and
  `MV_ELEVATION_DEG = 15.0` (`views.py:20`). The Hi3DGen venv pins `trimesh
  4.12.2`, `opencv-python-headless`, `numpy` (`fork:requirements.txt:19-25`),
  so the script runs there with no new dependency. Concept masks come from
  production artifacts: `concept_rgba*.png` alpha, thresholded at
  `> 0.8·255` — the same cut `preprocess_image`'s bbox test and
  `check_matte` use (`fork:hi3dgen.py:143`, `prop_hi3dgen.py:229`).
- **Ideal:** `C:\tools\Hi3DGen\venv\Scripts\python.exe
  scripts/ai-pipeline/mv_ab_metrics.py <raw.glb> --front F.png [--back B.png]
  [--side S.png] --out metrics.json [--masks-dir DIR]` writes one JSON with
  fitted yaw, per-view bbox-normalized IoU, and raw connectivity stats, in
  seconds, deterministically.
- **Gap:** The script does not exist.
- **Suggestion:** Pure Python, ~150 lines:
  - **Projection:** load `trimesh.load(path, force='mesh')`; for (az, el):
    build `s, u` per the `mv_view` formulas above; project `x = V·s`,
    `y = V·u`; map the fixed world window (center = bbox center, half-width =
    `norm(hi−lo)/2 · 1.05`, matching `mv_camera_rig`'s rig at
    `views.py:70-78`) onto a 512² canvas; rasterize every face with a single
    `cv2.fillPoly(canvas, polys, 255)` call (`polys` = the int32 triangle
    array, one entry per face).
  - **Mask compare:** binarize concept alpha at `> 204`; crop each mask to
    its nonzero bbox, letterbox to square, `cv2.resize` to 256² with
    `INTER_NEAREST`; IoU = `(a & b).sum() / (a | b).sum()`.
  - **Yaw fit:** IoU against the front mask at az = 0..355 step 5, el = 15;
    `yaw* = argmax`; report `iou_front = max`, `iou_back` at `yaw*+180`,
    `iou_side = max(yaw*±90)` (a single "side view" panel does not say which
    side, so take the better — stated in the JSON as `side_azimuth`).
  - **Raw stats** (hollow-valid, no volume/watertight fields):
    `component_count` = `len(trimesh.graph.connected_components(
    mesh.face_adjacency, min_len=0, nodes=np.arange(len(mesh.faces))))`;
    `boundary_edge_count` = rows of `mesh.edges_sorted` appearing exactly
    once (`trimesh.grouping.group_rows(mesh.edges_sorted, require_count=1)`);
    `main_face_fraction` = largest component / total faces;
    `vertex_count`, `face_count`.
  - **JSON** also records the instrument parameters (`elevation_deg`,
    `scan_step_deg`, `canvas_px`, `norm_px`) so a future reader can see the
    fixed constants; `--masks-dir` dumps the rendered and normalized mask
    PNGs for eyeball checks.
- **Path:**
  1. Implement `scripts/ai-pipeline/mv_ab_metrics.py`.
  2. Companion plain-assert test `scripts/ai-pipeline/test_mv_ab_metrics.py`
     (run under the Hi3DGen venv), two cases, no GPU:
     - **Analytic projection truth (non-circular):** build
       `trimesh.creation.box(extents=[1, 2, 3])`; render at az=0, el=0 via
       the script's own projection entry point. The view direction is −Y, so
       the silhouette is the box's XZ face: assert the mask's nonzero bbox
       aspect (height/width) is within 1% of 3.0 and the fill fraction
       inside that bbox is ≥ 0.999 (a rectangle fills its bbox exactly).
     - **Yaw-fit behavior:** render
       `target/prop-solid-validation/chapel_arch_e2e/cand_0/raw.glb` at
       az=20, el=15; write that mask as an RGBA PNG (mask as alpha); run the
       full CLI with it as `--front`. Assert `metrics.json`'s fitted yaw ==
       20 exactly (20 is on the 5° grid) and `iou_front > 0.99`.
     Both assertions are invariants, not calibrated bands.
  3. Run the test file; it must pass. This step touches only new Python
     files — no cargo gate.

### 4. Determinism probe, and the measured noise floor

- **Evidence:** Rework 6 (`reworks-hi3dgen-2026-07-28.md` finding 6,
  unresolved): three same-seed turbo runs gave 541220/541286/541242 vertices
  from byte-identical code; the normal map was bit-identical, so the noise is
  entirely in the geometry stage (`scatter_reduce` + sparse convs).
  `CUBLAS_WORKSPACE_CONFIG=":4096:8"` is already exported before CUDA init
  (`prop_hi3dgen.py:31-34`) — set up *for* `use_deterministic_algorithms`,
  which is called nowhere in the repo (grep verified). Whether the flag
  errors, runs-but-doesn't-pin (spconv kernels are outside torch's flag), or
  pins the mesh is unknown and unknowable without a GPU run.
- **Ideal:** The A/B knows, per metric, the size of same-seed run-to-run
  noise — either exactly 0 (determinism proven) or a measured floor from
  repeats — before any cross-arm difference is read.
- **Gap:** The flag is untried, and no A/B metric has ever been computed
  twice on the same configuration.
- **Suggestion:** Add `--deterministic` to `prop_hi3dgen.py`: when set, call
  `torch.use_deterministic_algorithms(True)` immediately after the torch
  import block (before any model load), and record `"deterministic": true`
  in the manifest. Then probe.
- **Path:**
  1. Implement the flag (a guarded call plus one manifest field).
  2. **Probe (§8: 3 GPU runs ≈ 3–4 min):** run
     `prop_hi3dgen.py target/prop-batch/b3/arch/cand_0/concept.png
     --deterministic --seed 0 --out target/mv-ab/det-r{1,2,3}` three times
     (separate `--out` dirs so nothing skips). Three outcomes:
     - **Errors** (torch raises `RuntimeError` naming a nondeterministic
       op): record the op name in the probe summary, proceed to 3.
     - **Runs, hashes differ:** flag insufficient (spconv); proceed to 3.
     - **Runs, `raw.glb` sha256 identical across all three:** determinism
       holds. Record the wall-time overhead against the 17.8 s
       non-deterministic candidate baseline
       (`chapel_arch_e2e/cand_0` manifest `elapsed_s.candidate`). The floor
       is 0; steps 6–7 run every arm `--deterministic`, one run per
       (arm, seed). Skip 3–4.
  3. **Floor fallback (§8: 3 GPU runs ≈ 3–4 min):** rerun the same three
     commands without `--deterministic` into `det-nf{1,2,3}`.
  4. Compute `mv_ab_metrics.py` on each repeat with `--front
     target/prop-solid-validation/chapel_arch_e2e/cand_0/concept_rgba.png`
     (its matching concept matte). Write
     `target/mv-ab/noise_floor.json`: per metric (`iou_front`,
     `component_count`, `boundary_edge_count`, `main_face_fraction`,
     `vertex_count`), the three values and the max pairwise absolute
     difference. That max-spread is the floor steps 6–7 test deltas against.
     Three repeats is a coarse floor — the claim rule in step 6 compensates
     by also requiring sign consistency across three independent seeds, and
     a result landing near the floor is reported as indistinguishable, never
     resolved by adjusting the rule.
  5. Assertion that makes this step's success visible: `noise_floor.json`
     exists and every listed metric has 3 recorded values (or, on the
     determinism outcome, the probe summary records 3 identical hashes and
     the measured overhead). Update nothing else; rework 6's queue entry is
     updated by step 8 with whichever outcome occurred.

### 5. Concept view sheets for the two A/B subjects — GATED ON THE USER'S RECIPE CHOICE

- **Evidence:** Recommended recipe is option B (Design decisions §5); this
  step is written for it and is replanned if the user picks otherwise.
  Machinery: `comfy_run.server()` + `comfy_run.run_workflow(workflow, dir)`
  (`scripts/ai-pipeline/comfy_run.py`), the `{subject}` placeholder
  convention (`gen_prop.py:117-123` — text replace, then every `seed`/
  `noise_seed` input overwritten), and `workflows/prop_concept.json` as the
  Z-Image-Turbo template (UNET `z_image_turbo_bf16`, CLIP
  `qwen_3_4b_fp8_mixed` as lumina2, `ae.safetensors` VAE, 8-step
  `res_multistep`, cfg 1.0, `EmptySD3LatentImage` sizing — all models
  already pinned in `scripts/ai-pipeline/models.sha256`). Subjects: prop =
  the registered olive_stump subject line ("gnarled dead olive tree stump,
  twisted weathered grey trunk, deep cracked bark",
  `content/models/assets.json`) — the most asymmetric generated prop
  (rework-1 close-out: deep bark crevices, largest interior-strip share);
  character = an evaluation-only dark-fantasy subject, e.g. "hooded pilgrim
  monk, weathered dark robes, rope belt, leather satchel, semi-realistic
  dark fantasy" (autonomous pick per the standing asset-autonomy ruling).
- **Ideal:** For each subject, three matting-ready panels on disk —
  `target/mv-ab/<subject>/view_front.png`, `view_side.png`, `view_back.png`
  — cropped from one Z-Image sheet whose three panels visibly depict the
  same object, plus the uncut `sheet.png` and a `sheet_meta.json` with the
  comfy manifest and seed.
- **Gap:** No sheet workflow exists; whether Z-Image-Turbo produces a clean
  3-panel layout without ControlNet is unmeasured — that hypothesis's test
  is this step, priced in seconds per attempt.
- **Suggestion:** New `scripts/ai-pipeline/workflows/mv_sheet.json`: copy of
  `prop_concept.json` with `EmptySD3LatentImage` at width 1536 / height 512
  and prompt text `"three views of the same {subject}, side by side on one
  sheet, left: front view, center: side view, right: back view, identical
  object in all three, flat even neutral studio lighting, plain grey
  background"`. New `scripts/ai-pipeline/mv_sheet.py` (plain system
  Python, `comfy_run` import): substitute subject, set seed, own the server
  (`comfy_run.server()`), pull the PNG, split into equal thirds with cv2,
  write the three panels + meta.
- **Path:**
  1. Implement workflow + script.
  2. **Generation (§8: ComfyUI server start + a handful of 8-step 1536×512
     generations, ~3–5 min per subject including retries):** run for both
     subjects, a few seeds if the first sheet is poor.
  3. Gate before any geometry spend: the sheets are *images*, so the
     consistency judgment is visual — surface `sheet.png` for both subjects
     to the user (with the panels' crops) and get an explicit go/no-go per
     subject; do not self-approve (visual judgment does not run at this
     tier). A sheet where the three panels are plainly different objects
     after ~5 seeds is a measured negative for option B — report it and
     stop; the fallback recipe is the user's call, not an autonomous swap.
  4. Behavioral check that is scriptable: each cropped panel, pushed through
     `matte_concept`/`check_matte` (which step 6's runs do anyway), must
     pass the matte gate — a panel whose matte fails would abort the A/B
     run, so pre-flight each panel with a tiny driver:
     `python -c` invoking `prop_hi3dgen.py`'s matte path is not available
     standalone; instead simply let step 6's first run be the check and
     re-cut the sheet if a view's matte aborts (the abort names the view,
     per step 2).

### 6. A/B on the asymmetric prop (olive_stump subject)

- **Evidence:** Arms and rule fixed by Design decisions §3/§6. Inputs from
  step 5: `target/mv-ab/olive_stump/view_{front,side,back}.png`. Per-run
  cost measured: shared load ≈ 29 s warm, geometry ≈ 17 s/candidate
  single-view (`chapel_arch_e2e/cand_0` manifest); multidiffusion multiplies
  the 50-step sparse-structure stage's model evals by (3 views + 1 neg)/2 ≈
  2×, so budget ≈ 35 s/candidate; +2 normal predictions ≈ +2 s per process.
- **Ideal:** A per-seed paired table showing, for each metric, whether
  multi-view conditioning moved the mesh toward the provided back/side
  views, readable against the step-4 floor.
- **Gap:** No multi-view geometry has ever been generated.
- **Suggestion:** Three arms × seeds {0, 1, 2}, one `prop_hi3dgen.py`
  process per arm (seeds batched, sharing load + normals):
  - SV: `prop_hi3dgen.py view_front.png --out target/mv-ab/olive_stump/sv
    --seed 0 --seed 1 --seed 2`
  - MV-S: same + `--view view_back.png --view view_side.png --mv-mode
    stochastic --out .../mv-stoch`
  - MV-M: same views with `--mv-mode multidiffusion --out .../mv-multi`
  All three with `--deterministic` iff step 4 proved it; otherwise as-is
  (the floor bounds the read). View order is front, back, side — view 0
  must be the front for yaw/orientation comparability.
- **Path:**
  1. **Runs (§8: 9 candidates ≈ 8–12 min GPU total, three processes).**
  2. Metrics: `mv_ab_metrics.py` per candidate with `--front view_front`'s
     matte, `--back view_back`'s, `--side view_side`'s — masks are the
     `concept_rgba*.png` files the MV runs wrote (BiRefNet is deterministic
     per image, so every arm shares one mask set; take them from the MV-M
     run dir). Collect into `target/mv-ab/olive_stump/ab.json`.
  3. **Claim rule (fixed before looking):** for each metric and each MV arm,
     compute per-seed delta vs SV at the same seed. A difference is claimed
     only if all 3 deltas share a sign AND min |delta| > that metric's
     step-4 floor (floor = 0 under determinism, where the sign-consistency
     requirement still stands). Everything else is reported as
     indistinguishable at N=3. No threshold moves, no post-hoc metric
     additions.
  4. Visual artifact: new tiny Blender script
     `scripts/ai-pipeline/mv_ab_render.py` (run `blender --background
     --python ... -- raw.glb out_dir --yaw <fitted>`) reusing
     `proptex.views.mv_camera_rig` + `normal_setup` + `render_exr` to write
     4 camera-space-normal renders (fitted yaw +0/90/180/270, el 15) and an
     `cv2.hconcat` contact sheet per candidate (~30 s each, 9 candidates).
     Untextured geometry reads best in normal shading, and it reuses the
     package's exact camera math.
  5. Deliverable: `ab.json` + 9 contact sheets + a summary block (written
     into step 8's report skeleton). The per-arm hollow caveat is stated in
     the JSON header: both arms are double-walled shells; metrics are
     outer-silhouette and connectivity only.

### 7. A/B on the character subject

- **Evidence:** Identical harness to step 6; inputs
  `target/mv-ab/<character>/view_{front,side,back}.png` from step 5. The
  character is the finding's stated motivation
  (`gen_character.py:183-211` routes character geometry through the same
  `prop_hi3dgen.py`, so a positive result here transfers to the character
  chain without further mechanism work). Characters are where the back is
  most *designed* (hair, hood, satchel, cloak) and where a single silhouette
  under-constrains most.
- **Ideal:** The same paired table for the character subject, so the verdict
  rests on both an organic-asymmetric prop and a character.
- **Gap:** As step 6.
- **Suggestion:** Repeat step 6's arms, seeds, commands, metrics, claim rule
  and renders verbatim with the character's view set under
  `target/mv-ab/<character>/`. One character-specific addition: record
  which azimuth the yaw fit chose per candidate — if MV arms systematically
  fit a different yaw than SV arms, that itself is a conditioning effect
  (orientation following the view set) and must be reported, not silently
  normalized away.
- **Path:**
  1. **Runs (§8: 9 candidates ≈ 8–12 min GPU, three processes).**
  2. Metrics into `target/mv-ab/<character>/ab.json`, same masks convention,
     same fixed claim rule.
  3. Renders via `mv_ab_render.py`, 9 contact sheets.
  4. Deliverable: `ab.json` + contact sheets, feeding step 8.

### 8. Verdict, adoption question, and queue bookkeeping (docs-only)

- **Evidence:** Steps 4, 6, 7 leave `noise_floor.json` (or a determinism
  record), two `ab.json` tables, 18 contact sheets. The campaign's A/B
  report convention is `docs/reviews/hi3dgen/ab-<topic>-<date>.md`
  (`ab-sampler-2026-07-28.md`, `ab-conditioning-2026-07-28.md`). The queue
  note in `reworks-hi3dgen-2026-07-28.md:23` lists **rework 2** unstruck;
  rework 6's entry (finding 6) carries no probe result yet.
- **Ideal:** One report that a later reader can re-derive the verdict from:
  the floor, every per-seed value, the claim rule as pre-registered, the
  contact sheets referenced, the hollow-arms caveat stated, and the
  decisions that follow — mode default, production wiring, option D
  research — each either taken (when forced by the evidence) or written as
  a user question with weighted options.
- **Gap:** Nothing records the A/B or closes the queue entry.
- **Suggestion:** Write `docs/reviews/hi3dgen/ab-multiview-2026-07-29.md`:
  noise-floor/determinism section (including the probe's op-name if it
  errored — that is rework 6 evidence), per-subject tables, claimed vs
  indistinguishable metrics under the fixed rule, visual-review outcome
  (user/opus reads the contact sheets — the metric table alone does not
  clear a visual deliverable), and the decision block. Then:
  - strike **rework 2** in the reworks file's queue note (standing
    "mark done" ruling) with a one-line result;
  - append the determinism-probe outcome to finding 6's entry (its Path's
    part (a) is now measured either way);
  - if the verdict is positive: queue "production wiring — concept stages
    emit view sets; `char_concept.json` SDXL→Z-Image migration rides along"
    and, if the user opted for it, the option-D research pass, as new
    queue entries; if negative or indistinguishable: record that the
    mechanism is plumbed, opt-in, and dormant, and what evidence would
    reopen it;
  - if MV-S vs MV-M was not separated by the evidence, put the default-mode
    choice to the user (multidiffusion recommended; cost measured in step
    6's wall-times).
- **Path:** write the report → apply the two reworks-file edits → verify the
  queue note renders struck → no source code changes, no test, no gate.
  (docs-only)

## §8 wall-time roster (one approval covers exactly these)

| step | runs | est. wall |
|---|---|---|
| 2 | 2 GPU candidates (identity smoke) | ~3 min |
| 4 | 3 GPU runs deterministic probe (+3 fallback repeats if it fails) | 3–8 min |
| 5 | Z-Image sheet generations, 2 subjects × ≤5 seeds | ~5–10 min ComfyUI |
| 6 | 9 GPU candidates (3 arms × 3 seeds, batched per arm) + 9 Blender renders | ~10–15 min |
| 7 | 9 GPU candidates + 9 Blender renders | ~10–15 min |

Total ≈ 35–50 min of GPU/Blender across the plan; no single run exceeds ~4 min.
