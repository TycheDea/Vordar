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
finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
finding 7 → finding 8 → finding 9 → finding 10 → finding 11 → finding 12 →
finding 13 → finding 14 → finding 15 → finding 16 → finding 17 → **rework 1** →
finding 18 → finding 19 → finding 20 → finding 21 → finding 22 → finding 23 →
finding 24 → **rework 2** → **rework 3** → **rework 4**.
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
