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
~~finding 17~~ → finding 15 → finding 16 → **rework 1** →
finding 18 → finding 19 → finding 20 → finding 21 → finding 22 → finding 23 →
finding 24 → **rework 2** → **rework 3** → **rework 4**.
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
