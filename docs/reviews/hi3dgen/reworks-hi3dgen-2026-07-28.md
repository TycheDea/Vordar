# Hi3DGen Fork Reworks — 2026-07-28

Companion to `audit-hi3dgen-2026-07-28.md` (same anchor conventions: `fork:` =
`C:/tools/Hi3DGen/Hi3DGen`, `hub:` = the StableNormal torch-hub snapshot,
unprefixed = vordar-repo relative).

## Ideal end state

The fork produces solid, single-shell, floater-free meshes conditioned on
multiple views, exposed through a clean installable headless API, with every
extraction and guidance knob measured rather than inherited. The vordar-side
script is CLI + gates + manifest; all upstream-shaped knowledge lives in the
fork we own.

## Findings (implementation order)

Queue (single cross-file sequence, mirrored from the fixes file):
~~finding 1~~ → ~~finding 2~~ → ~~finding 3~~ → ~~finding 4~~ → ~~finding 5~~ →
~~finding 6~~ → ~~finding 7~~ → ~~finding 8~~ → ~~finding 9~~ → ~~finding 10~~ →
~~finding 11~~ → ~~finding 12~~ → ~~finding 13~~ → ~~finding 14~~ →
~~finding 17~~ → ~~finding 15~~ → ~~finding 16~~ → **rework 1** →
finding 18 → ~~finding 19~~ → ~~finding 20~~ → ~~finding 21~~ → ~~finding 22~~ → ~~finding 23~~ →
finding 24 → **rework 2** → **rework 3** → **rework 4**.
The findings numbered in *this* file (10–16) are discoveries from rework
execution and sit outside that mirrored queue; they are struck here.
Done 2026-07-29 (this file's findings 10, 11, 12 and 15; vordar `9b47c44`, fork
`cc29648`, `c99bf4b`, `fe17cc2`). Findings 10-12 were all written against
`fill_enclosed_sdf`, which rework 13 step 1 deleted; each premise was re-checked
against the replacement and survives — 10's baseline is fill-free, and 12's
padding belongs to `build_field`'s band plus the corner scatter, not to the fill.
Finding 10's pair reproduced exactly (`0.1712 / 0.4970`), so the plan's prose
takes the measured numbers; its line 31 quote of `750397b`'s own carried
validation is left attributed rather than overwritten, which is the "record the
helper variant" half of the Suggestion. Finding 15's in-band cavity fixture
landed with rework 13 step 2 and is the suite's first case that discriminates
between the two mechanisms (`body_count` 2 → 1, cavity confirmed
scatter-written).
**Correction, same day:** findings 11 and 15 were both superseded hours after
landing, when the direction-count sweep killed the fill and its deletion took
their subjects with it. Finding 15's fixture tested the fill's scatter-written
boundary and is gone; finding 11's `embreex`/`rtree` pins are un-landed, their
only consumer having been `inward_area_fraction`, the harness's hollow-shell
metric, which died with the fill contract tests. Neither is re-queued — both are
moot rather than pending. Findings 10 and 12 stand: 10 corrected a predecessor
plan's prose and 12 documents `build_field`'s scatter padding, which survives.
Done 2026-07-29 (this file's finding 14, commit `7d145cb`). `check_mesh` now
drops zero-area faces and records the count in the manifest instead of aborting
the run. The re-run measured the two assertions rework 1 step 6 left unmeasured:
manifest `extraction` block present, peak reserved VRAM 6.787 GiB (≤ 8.0).
premise-falsified in part: the re-run's mesh came out at 768804 faces against the
aborted run's 768462 at the same seed — the GPU non-determinism this campaign
already documents — and carried **0** zero-area faces, so it would have passed
the old gate too. The drop-and-record path is proven by unit test, not by this
run.
Done 2026-07-28 (findings 15, 19–23, commits `23c7063`..`f2015e7`; findings 19–23
run out of queue order, pulled forward as file-disjoint parallel work while the
GPU-bound items serialized). Finding 15 raised `blend_coverage` 0.7303→0.9759 on
crucero by deleting 46170 camera-unreachable interior tris. premise-falsified:
finding 19's `concept_rgba` is not dead output — `matte_concept()` feeds
`preprocess_image()`, so only the stale docstring was defective and the
"drop" branch would have deleted live code. Finding 20's per-metre re-baseline
is blocked on rework 9 (stale coverage bakes); its audit-side rescale of
`world_area_m2` was cut rather than kept, since regeneration — not a correction
factor — is what makes shipped height match the registry.
Done 2026-07-28 (finding 16, commit `5eea012`). premise-falsified: the ~64 s
fixed cost measured 28.9 s (25.2 s model load) on a warm page cache, so the
per-extra-candidate saving is 29-64 s depending on cache state, not a flat 64 s.
`--normal-model full` is refused with 2+ seeds: its normal map is seed-dependent,
so it cannot share one prediction across a batch.
Done 2026-07-28 (findings 1–14 + 17, commits `4e5dfaa`..`a77c156`). Measured
outcomes that diverged from the findings' premises: finding 11's sampler A/B
found cfg and SLAT-step changes indistinguishable (defaults kept); finding 13's
full StableNormal lost to turbo on both subjects (kept opt-out, flag retained);
finding 12's adopted 1024 normal resolution stands on resample-chain
cleanliness, not on its original top-octave evidence, which was invalid because
both arms denoise at 768. Finding 17 cut peak VRAM 15.57→7.41 GiB reserved and
wall time 39%. Two defects found in-path were fixed at `a77c156`.
Parked: rework 5 (gate: finding 24's measurement shows extraction is a
dominant wall-clock share).
**CLOSED 2026-07-29: reworks 1 and 13 both — SDF-space solidification is not
viable on real prop fields.** Not parked; the whole approach family is
eliminated by measurement, and no third member is worth trying.

Rework 1 (`plan-rework1-solid-interior-2026-07-28.md`, approved `3c35a7b`)
reached step 6 of 8, where its paired validation failed the premise:
`fill_enclosed_sdf` moved chapel_arch -0.021% and crucero -0.033% in face count
(volume ratios 1.0002/1.0000) against a required 30-55% reduction. Rework 13
(`plan-rework13-winding-solidification-2026-07-28.md`, approved `5a43db9`)
replaced it with 26-direction exposure and landed green at `fe17cc2`, 9/9
harness. Step 3's real-field replay then parked chapel_arch at 2.43% face
reduction against a 15% floor while volume rose 3.214× and bodies went 16 →
3,700 — volume climbing without surface, which is what welding an *open*
concavity looks like, not what filling a sealed core looks like.

The settling measurement is a direction-count sweep (`029f59e`, full table in
rework 13's plan). Refining 26 → 1330 directions collapses the filled-cell count
from 690,882 to **7** on chapel_arch and 283,407 to **6** on crucero. It does not
converge to a plateau; the limit is zero. A true straight-line visibility test
fills nothing on these props because **none of them has an enclosed interior** —
the network emits a genuine hollow shell whose inner wall is real predicted
surface, not a `get_dense_attrs` stamping artifact. Every criterion in the family
("find the enclosed interior in the SDF grid and fill it") therefore has nothing
to find, and any direction count is a tuning knob silently setting how much
exterior concavity gets welded shut.

The mechanism is deleted from the shipping path (`fill_interior` defaulted to
`True`, so the welding was live): fork `5d4c9b0`, vordar `b4db6c6`, -347 lines. `drop_solid_floaters`, `iso_level`,
`sdf_bias` and `occupancy_threshold` survive — they are independent of the fill.

**What survives of rework 1.** Step 7 (re-derive `BAKE_MAX_RAY_DISTANCE_M`) is
untouched by this result. Step 8 (flip the geometry-health stats to fail-loud
gates) is now *more* relevant, not less: it is the gate that would refuse
chapel_arch. Both are re-queued on their own merit rather than behind a
solidification that will not arrive.

**Where the effort goes instead** (user ruling 2026-07-29, both the more
ambitious option of two): (1) attack the network's hollow output at source —
the inner wall is ~a third of every extraction and is stripped downstream, so
recovering it is worth the investigation even though tractability is unknown;
(2) chase chapel_arch's 3,824-component / non-watertight cleanup output as a
ship-blocker now. Lead on (2): raw extraction reports 16 bodies and
`cleanup_hollow.json` reports 3,824 components, so the shredding is a
`prop_cleanup.py` defect rather than a generation defect.

Artifact trail throughout: `target/prop-solid-validation/`. Step 6's GPU smoke
aborted (rework 14, fixed at `7d145cb`); the re-run measured both assertions it
had blocked — manifest `extraction` block present, peak reserved VRAM **6.787
GiB** against the `≤ 8.0` bound and the 7.41 baseline. Reworks 10-12 were queued
from steps 1 and 3 and are done; rework 15 landed inside rework 13 step 2.
Reordered 2026-07-28 by user decision, after finding 13's code half measured
peak VRAM at 16.74 GiB reserved on a 12 GiB card (every stage spilling to
system memory, wall time 40.8 min vs turbo's 2.6): finding 17 runs before
finding 13's A/B, which cannot be measured on a thrashing card. Remaining
order is finding 17 → finding 13 A/B → finding 14 → finding 15 → finding 16 →
**rework 1** → …. This also resolves the note's conflict with finding 16's own
Path, which requires finding 17 to land first.

### 1. Solid-interior extraction: land the hollow-shell fix with a validation harness
- **Evidence:** Every mesh the fork ships is a closed double-walled hollow shell: `fork:hi3dgen/representations/mesh/utils_cube.py:78` stamps the dense 257³ grid `sdf=+1` ("outside") and scatters predicted SDF onto surface-voxel corners only, so interior cells stay "outside" and marching cubes extracts an inner wall. Measured on real output: ray-crossing histograms dominated by 4 (not 2); implied wall thickness ~3.3 voxels; enclosed-interior area 37–50% of every shipped prop (audit finding 15). Two fork branches attack this: `fix-hollow-shell-extraction` (`750397b`, +20 lines, `scipy.ndimage.label` reachability flood on the positive field, commit message carries full validation — 55.2%→0.0% inward-facing area, volume 0.14x→1.03x on a synthetic sphere, plate-stack regression bit-identical, 0.25–0.29 s cost, plus a rejected alternative documented with its 4.9x-volume failure numbers) and `solidify-shell-interior` (`53472a1`, +68 lines, four stacked morphology heuristics, zero validation data, `behind_surface`'s "nothing outside satisfies this" claim fails on non-convex geometry like the void between a character's legs, ~3–5× the runtime). The two merge textually clean but would double-fill the field — they are alternatives, not complements. Neither has a committed test; `750397b`'s harness exists only as commit-message prose. The fill also runs unconditionally on the training path (no flag, no `training` guard), and a genuinely sealed hollow input now extracts solid (documented behaviour change with no escape hatch).
- **Ideal:** One validated interior-fill approach on `main`, flag-exposed (`fill_interior: bool = True`), recorded in the manifest, with a committed regression harness covering the hard topology cases we already own (chapel_arch's through-opening, candelabra's separated arms, crucero's thin cross), floaters removed in the same scipy pass, and the downstream bake constants re-derived against solid hires meshes.
- **Gap:** The single highest-leverage defect in the prop pipeline has a proven 20-line fix sitting unmerged on an unbacked local branch, while ~40–50% of every shipped prop's triangle and texel budget goes to invisible interior wall (and `BAKE_MAX_RAY_DISTANCE_M = 0.03` at `scripts/ai-pipeline/proptex/export.py:31-32` sits at the same order as the 0.023–0.077 m wall thickness, so bake rays can land on the inner wall's wrong-facing normals; `AO_DISTANCE_M = 0.15` integrates against the cavity).
- **Suggestion:** Adopt `fix-hollow-shell-extraction` as primary; close `solidify-shell-interior` as superseded (push for the record, annotate, delete locally), extracting its ideas only as measured follow-ups if the flood's assumptions ever fail. Expose the fill as a constructor flag threaded from `representation_config`, plus the other hardcoded extraction knobs (iso level 0.0, `sdf_bias = -1/res`, occupancy cut `>0` at `fork:hi3dgen/pipelines/hi3dgen.py:301`) so the manifest can record them. Add component-based floater removal (~3 lines in the same `ndimage` pass — label negative-SDF components, drop below-fraction ones; strictly better than `prop_cleanup.py`'s bbox heuristic at `scripts/ai-pipeline/prop_cleanup.py:236-247`, which stays as a relaxed backstop). Commit the sphere/vessel/plate-stack harness as a test module asserting signed-volume ratio and inward-facing-area fraction. Gate the fill off the training path.
- **Outcome:** `10/10` — upstream of tri budget, atlas density, texture coverage, AO, and collision; converts every downstream stage's arithmetic from shell to solid.
- **Cost:** `6/10` — the fix is written; the harness, knob exposure, branch closure, re-extraction of validation seeds, and bake-constant re-derivation (`BAKE_MAX_RAY_DISTANCE_M`, `AO_DISTANCE_M`) are the work. Re-extractions are GPU runs — bundle the go-ahead at plan approval.
- **Path:** plan via /plan-rework: harness first (committed, red on stock main) → land `750397b` + flag + knobs → harness green → close branch B → floater pass → regenerate validation seeds → re-derive bake constants → audit finding 14's stats flip to gates.

### 2. Multi-view conditioning — stop hallucinating every back side
- **Evidence:** `fork:hi3dgen/pipelines/hi3dgen.py:446-479` (`run_multi_image`) and `fork:hi3dgen/pipelines/hi3dgen.py:389-444` (`inject_sampler_multi_image`) implement two conditioning modes (`stochastic` round-robin, `multidiffusion` per-view averaging with CFG) — complete, correct-looking, and unreachable: app.py's "Multiple Images" tab is a stub, and we call single-image `run()` at `scripts/ai-pipeline/prop_hi3dgen.py:169`. Every prop and character back-side is invented from a single silhouette.
- **Ideal:** Characters and asymmetric props condition on ≥3 views (front/back/side) so the far half is generated, not guessed — the dominant fidelity ceiling for characters (`scripts/ai-pipeline/gen_character.py:186`).
- **Gap:** Needs a multi-view concept source (Z-Image multiview workflow, or a turntable of an approved single-view candidate), per-view matte + normal prediction, and a mode choice (`multidiffusion` is the quality option).
- **Suggestion:** Design pass: opt-in `--views front.png back.png side.png` path through per-view matte/normal into `run_multi_image(mode="multidiffusion")`; decide the concept-side view-generation recipe; A/B against single-view on one character and one asymmetric prop.
- **Outcome:** `9/10`
- **Cost:** `8/10` — concept-stage work + fork plumbing + evaluation.
- **Path:** plan via /plan-rework after rework 1 (solid meshes first, so the A/B measures conditioning, not shell artifacts).

### 3. A real headless API in the fork; shrink prop_hi3dgen.py to CLI + gates + manifest
- **Evidence:** Of `scripts/ai-pipeline/prop_hi3dgen.py`'s 211 lines, the majority is upstream-shaped workaround: pre-import env vars, `sys.path` insertion (the package isn't installable), `preload_birefnet` patching `fork:hi3dgen/pipelines/hi3dgen.py:198-205`'s hardcoded `'weights/BiRefNet'` path at a distance, `matte_concept` re-implementing the fork's own preprocess minus the crop, the hub-load try/except copied from app.py, and the run+export block copied from app.py. Genuinely vordar's own: `check_matte`, the manifest, the CLI.
- **Ideal:** The fork exposes `hi3dgen.headless.generate(image, out_dir, seed, …) -> dict` holding env setup, weight resolution, model lifecycle (including the stage-offload from audit finding 17), seeded normal prediction, mesh validation, and export; shipped with a `pyproject.toml` so the venv `pip install -e`s it. The vordar script drops to ~60–70 lines.
- **Gap:** Every upstream quirk is currently patched at a distance from the code that has it, so workaround and cause drift independently; the fork carries none of our knowledge.
- **Suggestion:** Move the corrected code (after audit findings 3–17 land, so what moves is already right) into `fork:hi3dgen/headless.py`; delete the vordar-side duplicates per the swap rule.
- **Outcome:** `8/10`
- **Cost:** `5/10`
- **Path:** plan via /plan-rework; explicitly sequenced after the fixes it absorbs.

### 4. Knob-sweep harness: measure the extraction and guidance parameters nobody has ever varied
- **Evidence:** Thin-feature survival (chains, blade edges, filigree — the dark-fantasy vocabulary) is governed by knobs that are pure checkpoint accidents: occupancy cut hardcoded `>0` (`fork:hi3dgen/pipelines/hi3dgen.py:301`), `cfg_interval [0.5,1.0]` and `rescale_t 3.0` from `fork:weights/trellis-normal-v0-1/pipeline.json` (consumed at `fork:hi3dgen/pipelines/samplers/flow_euler.py:130-131,196-197`, overridable via the sampler-params dicts we already pass).
- **Ideal:** A bounded sweep (fixed seeds × 3 representative subjects) with defined evaluation criteria; winners adopted as recorded defaults.
- **Gap:** Needs a design pass on evaluation criteria before any GPU is spent; the occupancy threshold needs the knob exposure from rework 1.
- **Suggestion:** Plan the criteria (silhouette fidelity vs concept, thin-feature survival count, component/watertight stats from audit finding 14), then a sweep harness over `cfg_interval` lower bound, `rescale_t`, occupancy threshold.
- **Outcome:** `5/10`
- **Cost:** `6/10` — mostly GPU sweep time; §8 go-ahead required with named wall-time.
- **Path:** plan via /plan-rework after rework 1 and audit findings 11–13 (their A/Bs establish the evaluation muscle this reuses).

### 5. GPU iso-surface extraction — PARKED
- **Gate:** activate only if audit finding 24's measurement shows CPU marching cubes is a dominant share of per-candidate wall time under batch mode (where it can otherwise overlap the next candidate's GPU work).
- **Evidence:** `fork:hi3dgen/representations/mesh/cube2mesh.py:136-147` — 68 MB GPU→CPU round trip, single-threaded skimage over 17 M voxels, GPU idle meanwhile. This is the fork's license-driven replacement for FlexiCubes; any GPU alternative must be permissively licensed — nvdiffrast/kaolin/FlexiCubes remain banned by the standing ruling.
- **Ideal:** Extraction is not a meaningful share of candidate wall time.
- **Gap:** Unmeasured; parked without a queue position until the gate is evaluated.
- **Suggestion:** If activated: evaluate permissively-licensed GPU marching cubes implementations; otherwise rely on batch-mode overlap and strike this rework.
- **Outcome:** `7/10`
- **Cost:** `7/10`
- **Path:** gate first (audit finding 24) → strike or plan.

### 6. Same-seed geometry is not reproducible; every A/B in this queue is reading noise it has not bounded
- **Evidence:** Measured while verifying audit finding 17. Three turbo runs of `scripts/ai-pipeline/prop_hi3dgen.py` on the same concept (`target/prop-batch/candelabra-z/cand_5/concept.png`) at `--seed 5`: 541220, 541286, 541242 vertices — and the last two came from byte-identical code, so the spread is not the fix under test. The normal map is bit-identical across all three (`normal_sha256 b70414b1…`), so the divergence is entirely in the geometry stage: `torch.scatter_reduce` in `fork:hi3dgen/representations/mesh/utils_cube.py:cubes_to_verts` and the sparse convs accumulate in nondeterministic float order, which shifts the SDF field enough to move the marching-cubes iso-surface.
- **Ideal:** A seed pins the mesh, so an A/B between two configurations measures the configuration. Failing that, the queue knows the size of the noise floor and reports differences against it.
- **Gap:** `prop_hi3dgen.py` already reseeds the normal stage specifically so a same-seed re-run reproduces (`prop_hi3dgen.py`, comment above `torch.manual_seed(seed)`), which sets an expectation the geometry stage does not meet. The A/B reports in this folder (`ab-sampler-*`, `ab-conditioning-*`) compare single runs per arm with no repeat baseline, so any effect smaller than ~0.01% of vertex count — and, more importantly, any unquantified effect on the visual metrics — is indistinguishable from run-to-run drift.
- **Suggestion:** Two parts, decide which. (a) Force determinism: `torch.use_deterministic_algorithms(True)` plus `CUBLAS_WORKSPACE_CONFIG`, and check whether spconv's `native` algo and `scatter_reduce` have deterministic paths at acceptable cost — if they do, the seed becomes a real pin. (b) If determinism is unaffordable, measure the noise floor once (N repeats on 2–3 subjects, per metric the A/Bs use) and require every future A/B to report its delta against that floor.
- **Outcome:** `7/10` — every remaining comparison in this campaign depends on it.
- **Cost:** `4/10` — (a) is a flag plus a compatibility check; (b) is GPU time (§8 go-ahead) plus a convention.
- **Path:** try (a) first on one candidate — if `use_deterministic_algorithms` runs at all, repeat 3× and confirm identical vertex counts; fall back to (b). Sequence before the next A/B in the queue.

### 7. `--normal-resolution` never reaches the denoiser: both normal pipelines process at 768 internally
- **Evidence:** Measured while running audit finding 13's A/B. `hub:hubconf.py`'s `Predictor.__call__` resizes the input with `resize_image(img, resolution)` and then calls `self.model(img, match_input_resolution=…, **kwargs)`, where `kwargs` carries `num_inference_steps` only — `processing_resolution` is never passed. Both pipelines fall back to their own `default_processing_resolution`, which is `768` in the constructor signature of `hub:stablenormal/pipeline_yoso_normal.py:159` and `hub:stablenormal/pipeline_stablenormal.py:246`, and is not overridden by either checkpoint's `model_index.json`. So at `--normal-resolution 1024` the conditioning image is resized to 1024, downsampled by the pipeline to 768, denoised at 768, and upsampled back — the denoiser never sees more than 768 px in either arm. Corroborating measurement: with one instrument over all cells, the r768 arm carries *more* top-octave energy than r1024 (candelabra 0.0306 vs 0.0060, crucero 0.0112 vs 0.0019), the opposite of `ab-conditioning-2026-07-28.md`'s ordering — consistent with LANCZOS upsample ringing rather than with resolved detail.
- **Ideal:** `--normal-resolution` sets the resolution the normal is actually denoised at, so the knob the queue adopted a default for is the knob it measured.
- **Gap:** Finding 12's adopted `--normal-resolution 1024` default currently buys only a different resample chain, not a higher-resolution prediction. The genuine 1024 prediction has never been run, so the knob's real quality ceiling is unmeasured; the 768 cap is also a candidate cause of the full predictor's speckle at higher step counts (`ab-normal-model-2026-07-28.md`).
- **Suggestion:** Pass `processing_resolution` through from `prop_hi3dgen.py` — either by calling the pipeline directly instead of via `Predictor.__call__`, or by carrying the fix in the fork/hub snapshot. Then re-run the 768-vs-1024 A/B, since its adopted conclusion rests on cells that never differed in denoising resolution. Check VRAM: 1024 denoising is ~1.8x the pixels at the stage that already peaks the process.
- **Outcome:** `7/10` — a quality knob the queue believes it has already tuned, and does not have.
- **Cost:** `4/10` — small plumbing change, plus a re-run of finding 12's A/B under §8.
- **Path:** plumb `processing_resolution` → confirm the manifest resolution matches the denoiser's actual working size → re-run the 768/1024 grid on the same two subjects.
- **Status (2026-07-28):** CONFIRMED — `--normal-resolution` never reaches the
  denoiser; both arms of every past grid denoised at 768. Still open: the Path
  above (plumb `processing_resolution` through, then re-run the 768/1024 grid
  with the angular instrument and ≥2 repeats per cell). The instrument dispute
  raised alongside this finding is settled: the radial-spectrum top-octave
  reading in `ab-conditioning-2026-07-28.md` was measuring resample artifact,
  not denoised detail, and is corrected there; the angular-domain suite
  (mean/p95 angular difference, detail-pixel angular gradient, speckle
  fraction) is the instrument for future normal-map comparisons. The default
  stays `--normal-resolution 1024` meanwhile — it is the strictly cleaner
  resample chain around the same 768 denoise, independent of this finding's
  outcome — and is not a defense against the re-run above.

### ~~8. The local normal-predictor load in `prop_hi3dgen.py` is dead code; every run silently takes the network fallback~~
- **Evidence:** Measured while running audit finding 13's A/B. `scripts/ai-pipeline/prop_hi3dgen.py:290-305` calls `torch.hub.load(<local snapshot>, …, source="local", pretrained=True, …)`, but neither `hub:hubconf.py` entrypoint (`StableNormal`, `StableNormal_turbo`) accepts a `pretrained` argument — the call raises `TypeError: StableNormal_turbo() got an unexpected keyword argument 'pretrained'` on every invocation, is swallowed by the bare `except Exception`, and the fallback `torch.hub.load("hugoycj/StableNormal", …, trust_repo=True)` runs instead. Reproduced directly: dropping `pretrained=True` makes the local branch load successfully.
- **Ideal:** The offline-pinned local snapshot is what loads; the network branch is a real fallback that never fires in normal operation.
- **Gap:** The intended offline path has never executed. `HF_HUB_OFFLINE=1` guards HF hub fetches but not `torch.hub`'s GitHub resolution, so the fallback is one cache eviction away from a network fetch (or a hard failure) inside a pipeline that is supposed to be reproducible offline. The bare `except Exception` is what hides it.
- **Suggestion:** Drop the `pretrained=True` kwarg. Then decide whether the fallback should exist at all — if the snapshot is pinned in `models.sha256`, a missing snapshot is a setup error and should fail loudly rather than silently reach the network. If it stays, narrow the `except` and log which branch was taken; the manifest should record it.
- **Outcome:** `5/10` — reproducibility/offline guarantee, no output change today.
- **Cost:** `1/10`
- **Path:** delete the kwarg → run one candidate and confirm no `Using cache found in` / network resolution for the StableNormal repo → decide the fallback's fate.
- **Done (2026-07-28):** `pretrained=True` and the network-fallback try/except both deleted; the local `source="local"` load now succeeds unconditionally (verified: `StableNormal_turbo` loads clean, no network resolution). The snapshot's `.py` files are now pinned in `models.sha256` under `Hi3DGen/StableNormal-hub/` with a matching `check_weights.py` root (54/54 OK).

### 9. `prop_audit.py` can't measure 6 of 7 generated props: coverage-sweep data is stale against current UV islands
- **Evidence:** Measured while implementing audit finding 20 (`height_m`). Running `python scripts/ai-pipeline/prop_audit.py` (unmodified, no code involved from finding 20) aborts immediately: `holes_broken_column.png island misses 8.8% of the rasterized UV island (must be >= 98% contained)`. Per-asset re-runs show the same failure for `candelabra_shrine` (22.0%), `crucero` (30.8%), `cypress` (34.3%), `gravestone` (19.5%), `olive_stump` (7.6%) — every generated prop except `chapel_arch`, which passes clean. `covered_mask`'s containment check (`prop_audit.py`) compares the glb's current, freshly-rasterized UV island against `target/prop-coverage/holes_<name>.png`, a Blender-baked coverage map from an earlier `prop_coverage_sweep.py` run. `prop_cleanup.py` gained an interior-face strip at `1f32bbe` (before finding 20's changes), which removes faces and therefore reflows the xatlas unwrap; the six affected props' `holes_*.png` predate that topology change, `chapel_arch`'s manifest post-dates it.
- **Ideal:** `target/prop-coverage/` reflects the UV layout of the props currently on disk, so `prop_audit.py` can measure every shipped prop, not just whichever one happens to have a fresh coverage bake.
- **Gap:** Six of seven generated props are unmeasurable until `prop_coverage_sweep.py` re-runs against current geometry. Finding 20's per-metre density re-baseline could only be demonstrated on `chapel_arch` (and the downloaded `rock_face_01` reference) as a result.
- **Suggestion:** Re-run `prop_coverage_sweep.py --asset <name>` for the six stale props (or all seven, for a clean baseline) so `target/prop-coverage/coverage.json` and `holes_*.png` match current geometry, then re-run `prop_audit.py` for the full density re-baseline finding 20's Path calls for.
- **Outcome:** `6/10` — unblocks measuring 6/7 generated props; no other consumer of `target/prop-coverage/` is affected.
- **Cost:** `3/10` — `prop_coverage_sweep.py` is a Blender multiview render pass (§8 go-ahead), ~7 props.
- **Path:** go-ahead for the render pass → `prop_coverage_sweep.py` per stale asset → `prop_audit.py` full sweep → compare against the pre-fix, fictional-height density numbers already on record from finding 20.

### 10. `plan-rework1-solid-interior`'s hollow-baseline reference numbers are ~20% off the spec-faithful helper

- **Evidence:** Measured while implementing that plan's finding 1 (interior-fill harness). The plan states the un-filled sphere baseline as `volume ratio ≈ 0.14, inward-facing area fraction ≈ 0.55`. Building the field exactly as the plan's Suggestion specifies — `res = 96`, band `|f(cell centre)| < 1.0`, corner samples with the `-1.0/res` production bias, `sparse_cube2verts` → `get_dense_attrs(res=97, sdf_init=True)` → channel 0 → `measure.marching_cubes(level=0.0, gradient_direction='ascent', allow_degenerate=False)`, faces `[:, ::-1]`, `Trimesh(process=False)` — yields `volume_ratio=0.1712 inward_fraction=0.4970` for the r=30 sphere, reproducibly. A bounded sweep of the two plausible helper knobs did not close the gap and moved the two metrics in opposite directions: band 0.5 gives `0.1191 / 0.5109`; clamping corner samples to ±1 gives `0.1656 / 0.4921`; clamping to ±0.5 gives `0.1539 / 0.4930`. No variant reaches `0.14` and `0.55` together, and production applies no clamp (`fork:hi3dgen/representations/mesh/cube2mesh.py:360` adds `sdf_bias` to the raw decoder output and nothing else).
- **Ideal:** The plan's quoted baseline is reproducible from the plan's own construction recipe, so a later reader can re-derive the defect the fill is meant to remove.
- **Gap:** The defect signature is unambiguous either way — a double wall, ~6x volume deficit, ~half the surface area facing inward — but the two quoted constants are not reproducible from the recipe as written. Whatever helper produced `0.14 / 0.55` differed from the plan text in a way the text does not record. No downstream assertion depends on them: the harness's thresholds bracket the *filled* result (`[0.95, 1.10]`, `[0.90, 1.30]`, `[0.90, 1.15]`), and each case's un-filled baseline (sphere `0.1712`, vessel `0.6181`, through-tunnel `0.4467`) sits well outside its band, so every case discriminates.
- **Suggestion:** Replace the plan's `≈ 0.14 / ≈ 0.55` with the measured `0.1712 / 0.4970`, or record the helper variant that produced the original pair. Prefer the former unless the original helper can be recovered — the measured pair is reproducible from `fork:tests/test_interior_fill.py`'s `build_field`.
- **Outcome:** `3/10` — documentation accuracy only; the harness and the fill contract are unaffected.
- **Cost:** `1/10`.
- **Path:** re-run `build_field(sphere_sdf)` + `extract` without the fill → confirm `0.1712 / 0.4970` → correct the plan's prose.

### 11. The interior-fill harness needs a trimesh ray backend the fork's requirements do not pin

- **Evidence:** Measured while implementing `plan-rework1-solid-interior` finding 1. The harness's inward-facing metric uses trimesh ray casting, as that finding's Suggestion specifies. The Hi3DGen venv shipped neither `rtree` nor `embreex`, so `mesh.ray` raised `ModuleNotFoundError: No module named 'rtree'` on first call. With `rtree` alone, trimesh selects the pure-Python `ray_triangle.RayMeshIntersector`: the sphere case's ~64k rays did not finish in 600 s. With `embreex` 4.4.0 installed, trimesh selects `ray_pyembree.RayMeshIntersector` and the same query takes 0.06 s. Both packages were installed into `C:\tools\Hi3DGen\venv` to land the harness; neither is recorded in `fork:requirements.txt` or `fork:requirements.lock.txt`.
- **Ideal:** A clean venv rebuilt from the fork's pinned requirements can run `tests/test_interior_fill.py` without a manual install, and gets the fast intersector rather than the 600s-plus one.
- **Gap:** The harness is currently reproducible only on this machine's venv. A rebuild silently regresses to either a hard `ModuleNotFoundError` or a run slow enough to read as a hang. The harness docstring states the requirement, which is not the same as pinning it.
- **Suggestion:** Add `embreex` (and `rtree`, which trimesh's fallback path needs) to `fork:requirements.txt` and to `fork:requirements.lock.txt` at the versions installed — `embreex==4.4.0`, `rtree==1.4.1`. Test-only dependencies in the main requirements file are the fork's existing convention (it has no separate dev-requirements file); introducing one is the alternative if that is unwanted.
- **Outcome:** `5/10` — makes the fill's contract re-runnable off this machine, which is the whole point of committing the harness.
- **Cost:** `1/10`.
- **Path:** pin both packages in the fork's requirements files → rebuild or `pip install -r` a clean venv → `python tests/test_interior_fill.py` selects `ray_pyembree` and each case's ray query stays sub-second.

### 12. `box_sdf` voxel count through the production scatter chain does not equal the box's nominal edge length cubed

- **Evidence:** Measured while implementing `plan-rework1-solid-interior` finding 3's floater cases. A `box_sdf` fixture spanning `(2,2,2)` to `(4,4,4)` (edge length 2, intended as a "2^3 blob") run through `build_field` → `fill_enclosed_sdf` rasterizes to 27 solid voxels (a 3x3x3 block), not 8: the `|f(cell centre)| < 1.0` band plus the cube-corner scatter in `sparse_cube2verts` pads roughly one grid cell onto each face. An edge-length-1, integer-aligned box `(2,2,2)`-`(3,3,3)` rasterizes to exactly 8 solid voxels instead. Confirmed with a direct voxel-count script against `hi3dgen.representations.mesh.utils_cube.fill_enclosed_sdf`.
- **Ideal:** A test author can predict a `box_sdf` fixture's realized solid-voxel count directly from its nominal span, so threshold-relative assertions (e.g. "below `min_fraction × total`") can be sized without a calibration run.
- **Gap:** The nominal-to-realized voxel count relationship depends on both box size and grid alignment (phase relative to the integer cell-center lattice), undocumented anywhere near `build_field` or `box_sdf`. `tests/test_interior_fill.py`'s floater cases had to be sized empirically rather than from the finding text's stated "2^3 ≈ 8 voxels", which turned out to require a differently-dimensioned box than the literal "2^3" phrasing suggested.
- **Suggestion:** Add a one-line note to `build_field`'s docstring (or `box_sdf`'s) stating the padding behavior, so a future fixture author sizes boxes correctly on the first try instead of needing a calibration script.
- **Outcome:** `2/10` — test-authoring friction only; no production or harness-correctness impact, both floater cases pass with the recalibrated fixture.
- **Cost:** `1/10`.
- **Path:** add the docstring note → no behavior change, no re-run required.

### 13. The sealed-cavity premise fails on real prop fields: `fill_enclosed_sdf` changes almost nothing on 2 of 3 hard-topology props

- **Evidence:** Measured executing `plan-rework1-solid-interior` finding 6 (paired hollow/solid validation); full artifact trail under `target/prop-solid-validation/` (`summary.json` plus per-subject `extract_*.json` / `cleanup_*.json`). Fill-on CPU replay vs the saved hollow `raw.glb` baseline: chapel_arch face count 773576 → 773414 (**-0.021%**), trimesh volume ratio **1.0002**; crucero 341880 → 341766 (**-0.033%**), volume ratio **1.0000**; candelabra_shrine 334942 → **359880** (a **7.4% increase**), volume ratio **1.5244**. Device-matched `--no-fill-interior` CPU replays reproduce the GPU baselines to within 0.003% (773566 / 334938 / 341878), so this is not a CPU-vs-GPU confound — the fill genuinely has near-zero effect on two subjects. Downstream `prop_cleanup.py` confirms it: solid-run `interior_tris_removed / raw_tris` is **0.3409** (chapel_arch), **0.3113** (candelabra_shrine), **0.3608** (crucero) against a `≤ 0.02` success bar and a `> 0.05` park bar, and each is within 0.001 of its own hollow pair except candelabra_shrine, where the solid run strips *more* interior (0.3113 vs 0.2615). `two_crossing_ray_fraction` is flat across the pairs (0.405 vs 0.415; 0.28 vs 0.28; 0.415 vs 0.41 — ratios 0.98/1.00/1.01 against a required `≥ 2×`). chapel_arch's solid `euler_number` is **864**, not `≤ 0`, with `component_count` 3824. The mechanism is wired and does run (`fork:hi3dgen/representations/mesh/cube2mesh.py:380-385`; `body_count` moves 16→12 on chapel_arch, 11→34 on candelabra_shrine, 15→6 on crucero), so this is a premise failure, not a plumbing failure.
- **Ideal:** The interior fill converts these props' hollow double walls into solid single shells, which is what `interior_tris_removed → 0` and a rising `two_crossing_ray_fraction` would show.
- **Gap:** `fill_enclosed_sdf` fills only outside-valued cells with **no positive path to the grid boundary**, and additionally clears every cell the sparse scatter wrote (`fork:hi3dgen/representations/mesh/utils_cube.py:83-91`). On these real fields the cavity between the two walls is evidently not sealed in that sense at res 256 — either it drains to the boundary through the props' open mouths and through-openings (an arch, a wayside cross, a shrine are all topologically open), or the inter-wall gap is thin enough to be entirely covered by the active-voxel band the scatter wrote and therefore excluded by line 90. The synthetic harness cases all have genuinely sealed cavities, which is why they pass while real input does not. candelabra_shrine is the one subject where a real cavity existed (+52% volume) — and even there the face count rose, so the fill closed volume without removing inner wall.
- **Measured (direction (ii), settled):** Instrumenting `fill_enclosed_sdf` on the real 257³ field (16,974,593 cells) counts, per subject, the cells that are positive-and-boundary-unreachable *before* line 90 masks anything:

  | | chapel_arch | candelabra_shrine | crucero |
  |---|---|---|---|
  | `n_unreachable` (pre-line-90) | **49** | **142,571** | **11** |
  | of which scatter-written (line 90 clears) | 22 | 28,363 | 11 |
  | `n_filled` today | 27 | 114,208 | 0 |
  | largest unreachable components | 27, 11, 2, 1… | 116680, 12438, 6201, 6138… | 3, 2, 2, 1… |
  | interior cells a solid fill must claim | **758,977** | 172,745 | **288,055** |

  The anchor row is positive cells sandwiched by solid cells on all three axes, cross-checked against trimesh volume (hollow chapel_arch 0.02057 → 345k cells vs `n_negative` 410,607). chapel_arch is off by a factor of **15,000**, crucero by **26,000**, and both find confetti (largest components 27 and 3 cells) rather than a cavity. Deleting line 90 and re-running the three CPU replays confirms it: chapel_arch 773414 → 773250 faces (0.04% below hollow, volume ratio 1.0002), crucero 341766 → 341660 (0.06%, ratio 1.0000). Only candelabra_shrine moves — 359880 → **262342** faces (**21.7%** below hollow, volume ratio **1.583**, `is_watertight` true, `body_count` 34→5) — and that is the subject whose cavity was already sealed and already being filled, not a masked one. Harness stays 7/7 with line 90 deleted, but **bit-identically**, so the suite never exercises line 90 at all (filed as finding 15).
- **Suggestion:** Do not tune `min_component_fraction`, `iso_level` or `sdf_bias` to force these numbers — the reachability criterion itself is what does not match the defect. Direction (ii) (dropping the line-90 exclusion) is now measured and eliminated as a fix: it buys 22 and 11 cells on the two failing props, 0.003% and 0.004% of what a solid interior needs. What remains is (i) — replace boundary-reachability with a signed-distance / generalized-winding-number solidification that does not depend on the cavity being sealed. The reason is structural rather than a tuning miss: an arch and a wayside cross are topologically open, so their inter-wall gap has a positive path to the grid boundary and **no reachability criterion at any resolution** will classify it as enclosed. Direction (ii)'s one-line deletion is independently worth keeping for candelabra_shrine, but it belongs to whatever replaces `fill_enclosed_sdf`, not ahead of it.
- **Outcome:** `9/10` — this decides whether the solid-interior rework can land at all, or whether the hollow-shell defect needs a different instrument.
- **Cost:** `5/10` — an extraction-stage redesign; direction (ii) is spent.
- **Path:** plan direction (i) as its own rework — a solidification pass whose sign test does not consult the grid boundary — then re-run the three CPU replays and the six `prop_cleanup.py` pairs and re-evaluate predicates (a)-(e). `plan-rework1-solid-interior` steps 7-8 stay blocked until those predicates move.

### 14. `prop_hi3dgen.py`'s zero-tolerance degenerate-face gate aborts a run over 2 faces in 768k

- **Evidence:** Measured executing `plan-rework1-solid-interior` finding 6's GPU smoke. `C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_hi3dgen.py target/prop-batch/b3/arch/cand_0/concept.png --out target/prop-solid-validation/chapel_arch_e2e --seed 0` exited with `prop_hi3dgen: cand_0: 2/768462 zero-area (degenerate) faces in raw mesh`. `check_mesh` (`scripts/ai-pipeline/prop_hi3dgen.py:286-290`) raises on `n_degenerate != 0` with no tolerance, and the abort happens before `raw.glb` and `hi3dgen_manifest.json` are written (`:438-505`), so only `concept_rgba.png` and `normal.png` landed. The fill is not the cause: zero-area face counts on the three fill-on CPU replays are **0**/773414, **0**/359880, **0**/341766. Record in `target/prop-solid-validation/gpu_smoke.json`.
- **Ideal:** A 2-in-768462 zero-area face — 0.00026% of the mesh, and well inside what `prop_cleanup.py`'s decimation and xatlas pass absorbs — does not cost a full GPU generation run.
- **Gap:** The gate's stated purpose (docstring at `:271-275`) is to refuse geometry that would surface as a confusing Blender abort three stages downstream. A handful of exactly-coincident vertices out of three-quarters of a million is not that class of failure; the GPU `scatter_reduce` path's nondeterministic float order (noted in `scripts/ai-pipeline/prop_extract.py:2-7`) makes such a face an expected occasional artifact rather than a broken candidate. As written the gate converts it into a lost run, and it blocked finding 6's manifest `"extraction"`-block and `vram.peak_reserved_gib ≤ 8.0` assertions, which remain unmeasured.
- **Suggestion:** Decide a tolerance policy rather than silently relaxing the check: either drop degenerate faces from the mesh before export and record the dropped count in the manifest (preferred — downstream gets clean geometry and the artifact is still on record), or admit a small absolute/fractional allowance and keep failing above it. The count must reach the manifest either way, so a rising trend is visible.
- **Outcome:** `6/10` — restores the ability to complete an end-to-end run and unblocks the two unmeasured smoke assertions.
- **Cost:** `2/10` — one function in `prop_hi3dgen.py` plus one manifest field; verification is a single ~2 min GPU run.
- **Path:** choose drop-and-record vs allowance → implement in `check_mesh` → re-run the smoke command above → assert `hi3dgen_manifest.json` contains the `"extraction"` block (`fill_interior: true`, `occupancy_threshold: 0.0`, `iso_level: 0.0`) and `vram.peak_reserved_gib ≤ 8.0` against the 7.41 GiB baseline.

### 15. `test_interior_fill.py` never exercises the scatter-written exclusion the fill's central comment defends

- **Evidence:** Measured executing finding 13's direction (ii). Deleting line 90 of `fill_enclosed_sdf` (`fork:hi3dgen/representations/mesh/utils_cube.py:90`) leaves all 7 harness cases passing with **bit-identical** metrics — `sphere` 1.0002/0.0000, `vessel` 1.0033/0.3916, `through_tunnel` 1.0049/0.1379, `plate_stack` 0 relabelled, `floater_blob_dropped` body=1, `floater_rod_survives` body=2, `iso_level` unchanged. The set `unreachable ∩ scatter-written` is empty in every synthetic fixture, so the line is inert under test. On real fields it is not inert: it clears 22 / 28,363 / 11 cells on the three props.
- **Ideal:** The comment at `:79-82` states a behavioral claim — that flooding cells the scatter skipped cannot pass through an open mouth made of predicted outside values, so the exclusion is what keeps a through-hole from welding shut. A claim that load-bearing has a case that fails when it is violated.
- **Gap:** Every fixture's sealed cavity sits wholly outside the active-voxel band, which is the one configuration where line 90 cannot fire. The suite therefore certifies neither that the exclusion is needed nor that removing it is safe; finding 13 had to measure on production latents to learn which. `through_tunnel` and `vessel` were written to catch exactly this and do not, because their walls are thin enough that the band never overlaps the cavity.
- **Suggestion:** Add a fixture whose sealed cavity lies *inside* the scatter band — a thin-walled shell whose interior gap is narrower than the active-voxel width — and assert the line-90 behavior in whichever direction the finding-13 successor settles on. Do not delete the existing cases; they cover the disjoint configuration.
- **Outcome:** `4/10` — closes a blind spot that already cost one wrong hypothesis, and any replacement for `fill_enclosed_sdf` inherits the same untested boundary.
- **Cost:** `2/10` — one fixture in the existing plain-assert harness, seconds to run.
- **Path:** build the thin-wall fixture → confirm it fails with line 90 in the state the successor rework rejects → land alongside that rework, not before it.

### 16. `min_component_fraction`'s denominator collapsed with the interior fill, so `1e-4` drops fewer floaters than it was calibrated to

- **Evidence:** Measured 2026-07-29 while deleting the interior-fill mechanism (successor to finding 13; the direction-count sweep in `plan-rework13-winding-solidification-2026-07-28.md` killed the fill, and `solidify_hidden_interior` is now gone from the fork). `drop_solid_floaters` (`fork:hi3dgen/representations/mesh/utils_cube.py`) thresholds every solid component at `min_fraction * total_solid_voxels`, and that total used to include the filled interior. With the fill gone it is the predicted shell alone. On the harness's r=30 sphere the total solid voxel count is **18,640** and the detached 8-voxel blob is **4.29e-4** of it, the 369-voxel rod **1.94e-2**. Directly measured consequence: `case_floater_blob_dropped`, green at `1e-4` for the entire life of the fill, came back `body_count=3` (blob surviving) on the first post-deletion run and only returns green at a fraction above 4.29e-4 — it now runs at `FLOATER_FRACTION = 3e-3` in `fork:tests/test_extraction_contract.py`, the geometric mean of the two fixtures' shares. On real props the same denominator shrink is the fill's measured volume inflation, 1.671x (candelabra_shrine) to 3.214x (chapel_arch) per that plan's step-3 table.
- **Ideal:** `min_component_fraction`'s shipped default is calibrated against the solid voxel count the extractor actually produces, so the floater sizes it deletes on real props are a stated absolute range rather than an accident of what the interior fill used to add to the denominator.
- **Gap:** `1e-4` was chosen when the denominator carried a filled interior. Nothing re-derived it when the fill was deleted, so the absolute size of a dropped component silently fell by the per-prop fill ratio (1.7-3.2x) — the mechanism still runs, still gates on `min_component_fraction > 0`, and still drops *something*, which is exactly why nothing catches the shift.
- **Suggestion:** Measure the per-component solid voxel counts on the three saved latents under `target/prop-latents/<name>/` via `scripts/ai-pipeline/prop_extract.py`'s CPU replay (no GPU), then pick the default from the gap between real debris and real geometry rather than from the synthetic fixtures. Do not move the harness's `FLOATER_FRACTION` to match whatever is chosen: it is a bar placed to straddle two fixtures, not a copy of the production value, and coupling them would hide the next such drift. `plan-rework13-...` already listed this recalibration as a recorded candidate (`fragments_removed` 11 / 5 / 9 surviving into cleanup) and deferred it as out of scope.
- **Outcome:** `4/10` — restores a calibrated floater drop; it is the only surviving grid-space cleanup after the fill's deletion, so its threshold is now load-bearing on its own.
- **Cost:** `2/10` — three CPU extraction replays plus a one-line default; no GPU, no Blender.
- **Path:** replay the three latents recording per-component solid voxel counts → identify the debris/geometry gap → set the default in `fork:hi3dgen/representations/mesh/cube2mesh.py` (and `decoder_mesh.py`'s `rep_config` fallback) → re-run `fork:tests/test_extraction_contract.py` (3/3, unchanged: its fraction is passed explicitly) → re-run the three replays and record `body_count`.
