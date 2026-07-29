# Hi3DGen Fork Audit — 2026-07-28

First audit of the `hi3dgen` domain: our fork of Stable-X/Hi3DGen, the
image→3D geometry stage of the asset pipeline.

**Anchor conventions.** `fork:` prefixes paths under the fork checkout at
`C:/tools/Hi3DGen/Hi3DGen` (remote `fork` = `github.com/TycheDea/Tyche3DGen`,
remote `origin` = upstream `Stable-X/Hi3DGen`). `hub:` prefixes the StableNormal
torch-hub snapshot under `~/.cache/torch/hub/hugoycj_StableNormal_main`.
`venv:` prefixes `C:/tools/Hi3DGen/venv`. Unprefixed anchors are vordar-repo
relative.

**Fork state at audit time.** `main` = `c29f668` = upstream HEAD, zero local
commits; the only fork-side work is two unmerged, unpushed local branches:
`fix-hollow-shell-extraction` (`750397b`) and `solidify-shell-interior`
(`53472a1`). Every asset shipped so far came from stock upstream extraction.

**Licensing verdict (hard gate).** No non-commercial or unclear license is
reachable from the shipping-asset path — verified against `fork:requirements.txt`
AND the actual venv package list. `nvdiffrast`, `kaolin`, `flexicubes`,
`diffoctreerast`, `flash-attn` are absent from fork, requirements, and venv;
the fork's marching-cubes extractor is scikit-image (BSD-3). Weights: Trellis
MIT, YOSO Apache-2.0, BiRefNet MIT, DINOv2 Apache-2.0. Residual risks are
paperwork/pinning gaps (findings 3–5), not license violations.

## Ideal end state

The fork is an owned, pinned, offline-capable, headless mesh generator: every
model loads from a hash-verified local path, every sampler and extraction knob
is explicit and recorded in a manifest that alone reproduces the mesh, output
meshes are solid single shells validated at the stage that produces them, a
batch of seeded candidates amortizes one model load, and peak VRAM is the
largest single stage rather than the sum of all stages. The vordar-side script
shrinks to CLI + gates + manifest; everything upstream-shaped lives in the fork
we own.

## Findings (implementation order)

Queue (single cross-file sequence; reworks live in
`reworks-hi3dgen-2026-07-28.md`):
~~finding 1~~ → ~~finding 2~~ → ~~finding 3~~ → ~~finding 4~~ → ~~finding 5~~ →
~~finding 6~~ → ~~finding 7~~ → ~~finding 8~~ → ~~finding 9~~ → ~~finding 10~~ →
~~finding 11~~ → ~~finding 12~~ → ~~finding 13~~ → ~~finding 14~~ →
~~finding 17~~ → ~~finding 15~~ → ~~finding 16~~ → **rework 1** →
finding 18 → ~~finding 19~~ → ~~finding 20~~ → ~~finding 21~~ → ~~finding 22~~ → ~~finding 23~~ →
finding 24 → **rework 2** → **rework 3** → **rework 4**.
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
**PARKED: rework 1** at step 6 of 8 (`plan-rework1-solid-interior-2026-07-28.md`,
approved at `3c35a7b`). Steps 1-5 landed and are green — fork `vordar-fixes`
carries the harness (`e62ca75`), the sign-flood fill (`64f54ad`), SDF floater
removal (`32572cd`) and the extraction knobs (`cf718c6`), 7/7 harness cases;
vordar carries `prop_extract.py` (`839763d`) and the manifest extraction block
(`18ae931`). Step 6's paired validation on real fields FAILED the premise:
`fill_enclosed_sdf` moves chapel_arch -0.021% and crucero -0.033% in face count
(volume ratios 1.0002/1.0000) against a required 30-55% reduction, and
`interior_tris_removed / raw_tris` stays at 0.34/0.31/0.36 against a `≤ 0.02`
bar. Artifact trail: `target/prop-solid-validation/`. Queued as rework 13, which
decides whether steps 7-8 can proceed at all. Its direction (ii) is now measured
and eliminated: chapel_arch has 49 boundary-unreachable cells and crucero 11,
against the 758,977 / 288,055 a solid interior needs, so the cavities are open
rather than masked and no reachability criterion can find them. Direction (i) —
solidification whose sign test does not consult the grid boundary — is the only
remaining path. Steps 7 and 8 stay blocked until its predicates move. Step 6's
GPU smoke aborted (rework 14, fixed at `7d145cb`); the re-run measured both
assertions it had blocked — manifest `extraction` block present, peak reserved
VRAM **6.787 GiB** against the `≤ 8.0` bound and the 7.41 baseline. Reworks 10-12
were queued from steps 1 and 3, rework 15 from rework 13's measurement.
Reordered 2026-07-28 by user decision, after finding 13's code half measured
peak VRAM at 16.74 GiB reserved on a 12 GiB card (every stage spilling to
system memory, wall time 40.8 min vs turbo's 2.6): finding 17 runs before
finding 13's A/B, which cannot be measured on a thrashing card. Remaining
order is finding 17 → finding 13 A/B → finding 14 → finding 15 → finding 16 →
**rework 1** → …. This also resolves the note's conflict with finding 16's own
Path, which requires finding 17 to land first.

### 1. Push the fork's work to the Tyche3DGen remote — nothing is backed up
- **Evidence:** `git branch -r` in the fork lists only `origin/HEAD` and `origin/main`; zero `fork/*` refs exist. `git log origin/main..main` is empty, so 100% of our work is the two local-only commits `750397b` and `53472a1` — including the measurement work in `750397b`'s commit message that would be expensive to redo. Losing this machine loses the fork's entire delta.
- **Ideal:** `main` and both topic branches exist on `TycheDea/Tyche3DGen`.
- **Gap:** The fork remote is configured but has never been pushed to or fetched from.
- **Suggestion:** `git push fork main fix-hollow-shell-extraction solidify-shell-interior`. Highest value-per-second item in this audit; do it before anything else touches the fork.
- **Outcome:** `9/10` — the fork's only real engineering is durably backed up.
- **Cost:** `1/10`
- **Path:** one push command; verify `git branch -r` shows the three `fork/*` refs.

### 2. Fork repo hygiene: ignore weights/target, purge stray output, neutralize the LFS phantom dirt
- **Evidence:** `fork:.gitignore` is one line (`__pycache__`); `weights/` (5.0 GB) shows as untracked, one `git add -A` from staging. `fork:target/` contains vordar's `prop-batch`/`char-batch` directory tree replayed inside the fork by a past relative `--out` (currently empty of files, so git hides it). 71 `assets/*.png` files show permanently modified: `fork:.gitattributes:36-38` declares `*.png filter=lfs`, but the committed blobs are raw PNGs (`git lfs ls-files` → 0 entries) — the installed LFS clean filter synthesizes a 131-byte pointer at compare time against the 936 KB blob. Committing them would replace real images with pointers to objects that exist on no LFS server.
- **Ideal:** `git status` in the fork is empty, so our own edits are visible and `git add -A` is safe.
- **Gap:** Permanent noise masks real changes; two multi-GB trees are unprotected from accidental staging.
- **Suggestion:** Add `weights/`, `target/` to `fork:.gitignore`; delete the stray `target/` tree; override the LFS attribute locally (`.git/info/attributes` line `*.png -filter -diff -merge` or `git config filter.lfs.clean/smudge cat`) rather than editing the tracked `.gitattributes`. Never `git add` the PNGs.
- **Outcome:** `6/10` — clean status is the precondition for safely owning a fork.
- **Cost:** `2/10`
- **Path:** ignore entries → delete stray tree → local attribute override → `git status` empty.

### 3. Fix the DINOv2 loader: missing `import os` forces the network fallback every run
- **Evidence:** `fork:hi3dgen/pipelines/hi3dgen.py:96` calls `os.path.join(...)` but the module's import block (`fork:hi3dgen/pipelines/hi3dgen.py:32-43`) never imports `os` → `NameError`, swallowed by the bare `except:` at line 97 → `torch.hub.load('facebookresearch/dinov2', ...)` over the network on every invocation, despite the local snapshot existing. The torch-hub trusted list contains only the StableNormal repo, so an untrusted-repo warning fires each run — consistent with the local branch never once succeeding.
- **Ideal:** The local snapshot loads; failures surface with a reason.
- **Gap:** A one-token upstream bug makes every candidate pay a GitHub round-trip, breaks offline operation, and the bare `except:` hides any genuine failure forever. It is also the vector for finding 5's NC-contamination risk.
- **Suggestion:** In the fork: `import os` at the top of `fork:hi3dgen/pipelines/hi3dgen.py`, narrow `except:` → `except Exception` with a printed reason. Cheapest possible first fork commit.
- **Outcome:** `7/10` — offline-capable, deterministic conditioning-model resolution.
- **Cost:** `1/10`
- **Path:** two-line fork commit on a new working branch → one smoke invocation confirms the local branch is taken (no hub warning in stderr).

### 4. Load geometry weights from the pinned local copy, offline
- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py:43` sets `GEOMETRY_WEIGHTS = "Stable-X/trellis-normal-v0-1"`; `fork:hi3dgen/pipelines/base.py:52` tests `os.path.exists(<id>/pipeline.json)` relative to cwd → false → `hf_hub_download` (same miss ×4 checkpoints in `fork:hi3dgen/models/__init__.py:72-83`). So every run loads 2.65 GB from the HF hub cache with 9 unpinned `revision="main"` HEAD requests to huggingface.co, while the copy that `scripts/ai-pipeline/models.sha256:72-82` hash-pins (`fork:weights/trellis-normal-v0-1`) is **never read**. `fork:app.py:270` uses the local path; we don't. `scripts/ai-pipeline/README.md:461` documents the local copy as the load location — wrong today.
- **Ideal:** The bytes we hash-pin are the bytes that load, offline, one copy on disk.
- **Gap:** 2.65 GB duplicated; the integrity manifest guards a dead copy (a swapped HF-cache blob is undetectable); network in the hot path; an upstream retag would silently swap the model under a green hash check.
- **Suggestion:** `GEOMETRY_WEIGHTS = REPO_DIR / "weights" / "trellis-normal-v0-1"` (absolute, so cwd-proof), set `HF_HUB_OFFLINE=1` in the subprocess env as a hard guard, delete the HF cache entry, fix README:461.
- **Outcome:** `8/10` — pinning becomes real; hot path goes offline.
- **Cost:** `2/10`
- **Path:** one-line constant change + env var → smoke run → delete `models--Stable-X--trellis-normal-v0-1` from the HF cache → README fix.

### 5. Pin and ledger DINOv2 + BiRefNet; fix the README attribution
- **Evidence:** DINOv2 (`dinov2_vitl14_reg`, 1.22 GB) is Hi3DGen's own `image_cond_model` (`fork:weights/trellis-normal-v0-1/pipeline.json`), loaded every geometry run — yet `content/source/CREDITS.md` has no DINOv2 row (grep for `dinov2|DINO` → nothing), and `scripts/ai-pipeline/README.md:466-468` misattributes it to the YOSO predictor. `scripts/ai-pipeline/models.sha256` pins neither DINOv2 nor BiRefNet (grep → 0 hits). BiRefNet loads with `trust_remote_code=True` (`scripts/ai-pipeline/prop_hi3dgen.py:63-66`) against an unpinned snapshot — 92 KB of arbitrary Python executing per run with no revision pin. The DINOv2 hub snapshot now ships NC-licensed siblings (`LICENSE_CELL_DINO_MODELS` "FAIR Noncommercial Research License", `LICENSE_XRAY_DINO_MODEL`) beside the Apache-2.0 root license; our model is under the Apache root, but an unpinned re-pull is a live NC-contamination vector.
- **Ideal:** Every model and every piece of executed remote code in the asset path is hash-pinned, revision-pinned, and has a ledger row with a verdict.
- **Gap:** 1.66 GB of weights and executing remote code sit outside the pinning discipline the rest of the pipeline enforces; the mandatory conditioning model was never license-cleared on paper.
- **Suggestion:** Append DINOv2's `.pth` and BiRefNet's `model.safetensors` + `birefnet.py` + config to `models.sha256`; pass `revision="e2bf8e4460fc8fa32bba5ea4d94b3233d367b0e4"` at `scripts/ai-pipeline/prop_hi3dgen.py:65`; add a DINOv2 CREDITS row (Apache-2.0, Cleared) explicitly noting the NC siblings are out of scope; fix README:466-468; with finding 3 landed, consider deleting the network fallback in the fork outright.
- **Outcome:** `8/10` — closes the only live NC-ingress vector and the ledger gap.
- **Cost:** `2/10`
- **Path:** hash + pin + ledger row + README fix; no GPU needed.

### 6. Nothing verifies the Hi3DGen lines in models.sha256 — add a verify mode
- **Evidence:** `scripts/ai-pipeline/models.sha256:72-92` holds 21 `Hi3DGen/…` lines; the file's only consumer, `scripts/ai-pipeline/comfy_run.py:31`, uses it for ComfyUI cache keys and never touches the `Hi3DGen/` prefix. `scripts/ai-pipeline/check_models.py` queries a running ComfyUI server and cannot cover these weights. The hashes were written once by hand and never re-checked.
- **Ideal:** The weight manifest is an assertion, re-run mechanically, covering the paths that actually load (after finding 4).
- **Gap:** A corrupted or swapped weight file passes silently forever.
- **Suggestion:** Add a `--verify` mode (to `check_models.py` or a tiny `check_weights.py`) that maps manifest prefixes to roots (`Hi3DGen/` → `fork:weights/`) and re-hashes. Offline, no GPU.
- **Outcome:** `5/10`
- **Cost:** `2/10`
- **Path:** script + one clean run; wire into the pipeline docs as the pre-batch check.

### 7. The manifest is not a re-run recipe — record everything that shaped the mesh
- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py:182-201` records seed, steps, model ids, torch/xformers versions, vert/face counts, `peak_vram_allocated_gb`. It omits: the fork commit that produced the mesh (checking out either topic branch would change every mesh with zero trace); the effective sampler params — `cfg_strength 5.0`, `cfg_interval [0.5,1.0]`, `rescale_t 3.0` arrive silently from `fork:weights/trellis-normal-v0-1/pipeline.json` via the merge at `fork:hi3dgen/pipelines/hi3dgen.py:290`; the normal map (computed at line 167, consumed at 170, discarded — the one artifact that bisects "normal stage" vs "geometry stage" failures); elapsed time (no timing instrumentation at all; the ~64 s load / ~45 s inference split exists only as file mtimes); resolved `ATTN_BACKEND`/`SPCONV_ALGO`; skimage/trimesh/spconv versions (the three that determine mesh geometry); and the VRAM field is decimal GB while three docstrings quote "11.5 GiB" — the shipped spread is actually 10.60–12.29 GiB, including one candidate (olive_stump, 13.203 GB recorded) that exceeded the 12 GiB card and fell back to system memory unnoticed. The normal stage's determinism additionally hangs on nothing consuming the global RNG between `torch.manual_seed(seed)` at line 166 and the predictor call at 167 (hub `Predictor.__call__` has no `generator=` parameter) — invisible, unasserted.
- **Ideal:** The manifest alone reproduces the mesh (the hard requirement at `tasks/ai-pipeline/a0.md:220`): fork rev + dirty flag, post-merge sampler dicts read back off the pipeline object, saved+hashed `normal.png`, `elapsed_s` with load/inference split, backends, dep versions, `peak_vram_allocated_gib` + `peak_vram_reserved_gib` with an over-90% warning.
- **Gap:** Finding 11's silent cfg drift is exactly the class this would have caught; the card-overflow event was recorded in a unit nobody compared.
- **Suggestion:** Add a `hi3dgen_id()` mirroring `comfy_id()` (`scripts/ai-pipeline/comfy_run.py:36-45`); record the merged params, normal.png sha, timing split, backends, versions; convert VRAM fields to GiB and add `max_memory_reserved`; update the three GB/GiB docstrings (`scripts/ai-pipeline/prop_hi3dgen.py`, `scripts/ai-pipeline/gen_prop.py:20-21`, `scripts/ai-pipeline/gen_character.py:29-30`) from the real spread.
- **Outcome:** `7/10` — every later A/B in this queue becomes attributable.
- **Cost:** `2/10`
- **Path:** manifest block rewrite + docstring corrections → one smoke run to see the new fields → this is the instrumentation gate for findings 10–15.

### 8. Write `hi3dgen_manifest.json` directly — kill the rename dance
- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py:201` writes `generation_manifest.json`; `scripts/ai-pipeline/gen_prop.py:155` and `scripts/ai-pipeline/gen_character.py:192` immediately `.replace()` it to `hi3dgen_manifest.json` because the orchestrators reserve the original name for the chained manifest. If the process dies in the window, the resume path (`scripts/ai-pipeline/gen_prop.py:146` skips when `raw.glb` + `concept_rgba.png` exist) ships the candidate with placeholder provenance — seed, cfg, VRAM permanently lost.
- **Ideal:** Two producers, two filenames, no rename, no loss window.
- **Gap:** Three silent failure modes for zero benefit.
- **Suggestion:** `prop_hi3dgen.py` writes `hi3dgen_manifest.json` directly; delete the rename from both orchestrators; make the geometry skip-check also require the manifest so an interrupted run regenerates.
- **Outcome:** `6/10`
- **Cost:** `1/10`
- **Path:** rename at the source + two deletions + skip-check tightening; resume-path smoke via a dry `--through geometry` on an existing candidate dir.

### 9. Make prop_hi3dgen.py cwd-independent
- **Evidence:** The script runs under `cwd=fork` by contract; `scripts/ai-pipeline/prop_hi3dgen.py:126` mkdirs a possibly-relative `--out` with no guard, and the stray `fork:target/` tree (finding 2) proves a relative path already leaked once. The only genuine cwd dependency is `local_cache_dir="./weights"` at `scripts/ai-pipeline/prop_hi3dgen.py:138`. `scripts/ai-pipeline/gen_character.py` resolves paths per-call-site rather than once.
- **Ideal:** The script is indifferent to cwd; the `cwd=HI3DGEN_REPO` argument in both orchestrators becomes unnecessary.
- **Gap:** A caller mistake writes into a git checkout silently.
- **Suggestion:** Resolve `args.out`/`args.image` at parse time; replace `./weights` with an absolute `REPO_DIR / "weights"`.
- **Outcome:** `5/10`
- **Cost:** `1/10`
- **Path:** three lines + one smoke run from a different cwd.

### 10. Gate the mesh output; reject degenerates at extraction
- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py:177-180` exports unconditionally; the fork already computes `MeshExtractResult.success` (`fork:hi3dgen/representations/mesh/cube2mesh.py:55`) and nobody reads it. No check for NaN vertices, zero bbox, or zero area — failures surface three stages later as a confusing Blender abort in `scripts/ai-pipeline/prop_cleanup.py:206-259`. Separately, `fork:hi3dgen/representations/mesh/cube2mesh.py:143-147` leaves skimage's `allow_degenerate` at its default `True`: measured 15–50 zero-area faces per raw mesh, which poison quadric decimation and xatlas chart seeding. And `fork:hi3dgen/representations/mesh/cube2mesh.py:111` passes `face_normals` shaped `(F,3,3)`, which trimesh silently discards and recomputes — a latent trap.
- **Ideal:** Symmetric gating: `check_matte` guards the input (`scripts/ai-pipeline/prop_hi3dgen.py:72-92`), a `check_mesh` guards the output; extraction emits no degenerate faces; no export argument is silently ignored.
- **Gap:** The stage that produces the artifact everything downstream trusts is the only one with no refusal gate.
- **Suggestion:** `check_mesh()`: assert success flag, `isfinite(vertices).all()`, bbox extent > 0 on all axes, face area > 0, exit non-zero with measured numbers. In the fork: `allow_degenerate=False`, and pass `face_normal[:, 0, :]` or drop the argument with a comment.
- **Outcome:** `6/10`
- **Cost:** `2/10`
- **Path:** vordar-side gate + two one-line fork edits → verify a known-good candidate still passes and face counts drop by the degenerate count.

### 11. Own the sampler parameters: explicit CFG + per-stage steps, then A/B
- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py:2-6` claims to transcribe app.py's recipe; it transcribed the step sliders (50/6 at `scripts/ai-pipeline/prop_hi3dgen.py:52-53`) but not the guidance sliders — `fork:app.py:87-88` runs both stages at `cfg_strength=3`, while the merge at `fork:hi3dgen/pipelines/hi3dgen.py:290` keeps `pipeline.json`'s 5.0. Every shipped prop was generated at ~1.67× the demo-validated guidance, invisible in provenance. The SLAT stage — the one that carries surface detail — runs at 6 steps (a Gradio-latency compromise) vs the checkpoint's trained default of 25; the SS stage runs 50 vs default 25.
- **Ideal:** Both knobs are named constants, passed explicitly, recorded (finding 7), and their values chosen by measurement for an unattended quality-first pipeline, not inherited from a demo's latency budget.
- **Gap:** The single most output-shaping parameter is an accident of a weights file; high CFG on flow models over-sharpens and amplifies silhouette artifacts — a plausible contributor to the blocky surfaces and floaters `prop_cleanup.py` mops up.
- **Suggestion:** Add `--ss-cfg`/`--slat-cfg`/`--ss-steps`/`--slat-steps` with explicit defaults; then A/B cfg 3.0 vs 5.0 and SLAT steps 6/12/25 on fixed seeds (one prop, one character). The A/B is a generation run — name the wall-time and get the go-ahead per CLAUDE.md §8 before running it.
- **Outcome:** `8/10` — likely direct geometry-quality gain plus closed provenance hole.
- **Cost:** `3/10` — code trivial; the A/B needs a few GPU candidate runs (~2 min each).
- **Path:** constants + CLI + manifest (rides finding 7) → gated A/B → adopt winner as recorded default.

### 12. Un-soften the conditioning chain: resolution round-trips and the premultiply fringe
- **Evidence:** The sole conditioning signal passes through five resizes: concept capped to ≤1024 **before** the object bbox crop (`scripts/ai-pipeline/prop_hi3dgen.py:105-107`), crop upsampled to 1024² (`fork:hi3dgen/pipelines/hi3dgen.py:186`), downsampled to 768 for the normal predictor (`scripts/ai-pipeline/prop_hi3dgen.py:167`, copying `fork:app.py:96`; hub default is 1024), LANCZOS back up to 1024, then 518² for DINOv2. `fork:hi3dgen/pipelines/hi3dgen.py:189-194` premultiplies RGBA against black after resizing introduced fractional alpha, then the hub predictor composites the same image onto white — a dark fringe exactly at the silhouette the normal model must resolve. Threshold mismatch: our matte gate uses `alpha > 0.1*255` (`scripts/ai-pipeline/prop_hi3dgen.py:83`) vs the fork's bbox test `> 0.8*255` (`fork:hi3dgen/pipelines/hi3dgen.py:145`), and if no pixel clears 0.8 the fork silently returns the raw unmatted frame.
- **Ideal:** One resize to the working resolution, one background convention, crop taken from full-resolution pixels, and the silent raw-frame fallback raises instead.
- **Gap:** Real resolution and edge fidelity lost upstream of everything; needs measurement, not assumption (DINOv2's 518² input caps part of the benefit).
- **Suggestion:** With finding 7's saved `normal.png`: A/B normal `resolution` 768 vs 1024, and crop-from-original vs current; fix the premultiply in the fork (composite onto white once); align the matte-gate threshold to `0.8*255`; make the no-alpha fallback raise.
- **Outcome:** `6/10`
- **Cost:** `3/10`
- **Path:** instrumented A/B (small, gated) → land winners → threshold + raise fix regardless.

### 13. Offer the full StableNormal predictor as the quality normal bridge
- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py:133-148` loads `StableNormal_turbo` (1-step YOSO). The hub snapshot also exports the full two-stage `StableNormal` pipeline with multi-step refinement (`hub:hubconf.py`), plus an unused `num_inference_steps` passthrough. Geometry is entirely mediated by this normal map (`fork:hi3dgen/pipelines/hi3dgen.py:383` conditions on nothing else).
- **Ideal:** The highest-fidelity normal predictor available feeds an offline AA pipeline; turbo remains the fast path for coverage sweeps.
- **Gap:** Single-step normals are soft on high-frequency detail — cloth folds, ornament, blade edges — exactly the dark-fantasy vocabulary that matters.
- **Suggestion:** Fetch `Stable-X/stable-normal-v0-1` into `fork:weights/`, extend `models.sha256`, add `--normal-model turbo|full` + `--normal-steps`, A/B on fixed seeds.
- **Outcome:** `9/10` — highest-leverage pure-quality knob short of rework 1.
- **Cost:** `5/10` — weights fetch + plumbing + gated A/B runs.
- **Path:** fetch + pin → CLI plumb → gated A/B → adopt default.

### 14. Record geometry-health stats in cleanup — the instrument for rework 1
- **Evidence:** Raw meshes are not reliably watertight (measured: boundary edges 4–12, Euler numbers 7 / −137 / 78 across three candidates; components 9–80 per mesh) yet `scripts/ai-pipeline/prop_cleanup.py:307-320`'s stats JSON records none of it — no `is_watertight`, `boundary_edge_count`, `euler_number`, `component_count`, no 2-crossing-ray fraction. The fragment filter at `scripts/ai-pipeline/prop_cleanup.py:236-247` cuts on a single bbox-diagonal fraction (0.02) with no volume criterion — on one column, ~11 fragments between 2% and 10% survive and weld into the prop.
- **Ideal:** Every cleanup run emits the geometry numbers that would show rework 1 working (or regressing) in production, next to `prop_audit.py`'s texture rows.
- **Gap:** The hollow-shell fix has no production instrument; record-only now (hard thresholds would fail pre-rework-1), gate after.
- **Suggestion:** Add the five stats above to the cleanup stats JSON so the chained manifest carries them; leave thresholds for after rework 1.
- **Outcome:** `6/10`
- **Cost:** `2/10`
- **Path:** stats block + one re-run on an existing raw.glb to see the numbers.

### 15. Interior-face stripping in prop_cleanup — cheap insurance and immediate budget reclaim
- **Evidence:** Measured on shipped assets: enclosed-interior area fraction 0.372–0.505 (broken_column 0.415, crucero 0.372, chapel_arch 0.505 — area-weighted normal-ray occlusion test). `scripts/ai-pipeline/prop_cleanup.py:285-301` decimates to the 15k tri budget and unwraps xatlas over the whole shell, so ~40–50% of every prop's triangles and texels describe surfaces no camera can see. `scripts/ai-pipeline/proptex/coverage.py:53-54` misattributes the uncovered texel set to "undersides" — the dominant uncovered set is unreachable by any camera, which is why the extra-view search predicts only ~5% gain.
- **Ideal:** Only exterior surface consumes budget: `blend_coverage` ≥ 0.9, atlas density roughly doubles at unchanged size, 15k tris buys 15k visible triangles.
- **Gap:** Neither side strips interior geometry; the fragment filter cannot catch an inner wall welded to the outer at silhouette edges. Rework 1 removes the wall at the source, but this pass pays off on the very next regeneration and stays as a regression instrument (its `interior_tris_removed` should drop to ~0 once rework 1 lands).
- **Suggestion:** Between fragment strip and normalization in `prop_cleanup.py`: ray-visibility test (~64 rays per face from `+eps·n`, delete faces with zero escaping rays; `select_interior_faces` is the cheaper first try), report `interior_tris_removed` in stats.
- **Outcome:** `9/10` — reclaims roughly half the geometry/texel budget of every generated prop.
- **Cost:** `3/10`
- **Path:** Blender pass + stats field → regenerate one candidate through cleanup and compare coverage/density numbers.

### 16. Multi-seed batch mode — stop re-paying ~64 s of model load per candidate
- **Evidence:** Measured mtime split on a real candidate: 109 s total, ~64 s fixed cost (interpreter + 6.94 GB weight I/O + CUDA init) before any inference. One process = one candidate by design (`scripts/ai-pipeline/README.md:643-645`; `scripts/ai-pipeline/gen_prop.py:150`). `fork:hi3dgen/pipelines/hi3dgen.py:360-387` takes `seed` as a plain argument and re-seeds internally — N seeds in one process is a loop, not a redesign. Matte + normal prediction are seed-independent for a fixed concept, so a multi-seed run computes them once.
- **Ideal:** A batch of N candidates loads models once; candidates-reviewed-per-hour roughly doubles for 4-seed sweeps.
- **Gap:** A 4-candidate sweep burns ~4 minutes re-reading identical bytes. Caveat: a batch-of-N run will not bit-reproduce a batch-of-1 at the same seed unless per-sample seeding is handled explicitly — manifest must record batch context (finding 7).
- **Suggestion:** Repeatable `--seed` writing `cand_<seed>/` outputs each, load hoisted; orchestrators keep per-candidate resume semantics. Do after finding 17 (a warm process holding 8 GB resident is a bad neighbour; post-offload it holds ~2 GB between candidates). A persistent stdin worker is the further step only if batch mode proves insufficient.
- **Outcome:** `7/10`
- **Cost:** `4/10`
- **Path:** CLI + loop + per-seed manifests → 2-seed smoke batch → wire `gen_prop.py`/`gen_character.py` to batch when multiple seeds are queued.

### 17. Cut peak VRAM: free finished helpers, stage the models, drop dead buffers
- **Evidence:** Usage windows are strictly disjoint, yet everything stays resident: BiRefNet (0.44 GB) after the matte, StableNormal (2.63 GB fp16) after the normal map (`scripts/ai-pipeline/prop_hi3dgen.py:129-167`, nothing ever `.cpu()`d), DINOv2 (1.13 GB fp32), both diffusion stages. Resident-weight floor ≈ 8.3 GB against a 12 GiB card; olive_stump recorded 13.203 GB allocated — past physical VRAM, silently absorbed by driver system-memory fallback. Dead weight inside the extractor: `fork:hi3dgen/representations/mesh/cube2mesh.py:313-315` allocates `reg_v`+`reg_c` at **construction** time — 1.48 GiB of int64, of which `reg_c` (1.07 GB) is never read anywhere, and index range fits int32. The 6-channel color interpolation adds +407 MB at exactly the peak moment for data that is discarded (finding 18 decides its fate first).
- **Ideal:** Peak ≈ largest single stage (~5–6.5 GiB), unblocking the "ComfyUI must never be up during geometry" sequencing rule (`scripts/ai-pipeline/gen_prop.py:16-21`) and making batch mode a good neighbour.
- **Gap:** ~3 GB of finished helpers plus ~1.5 GB of dead/oversized buffers ride the peak; one shipped candidate already overflowed the card unnoticed.
- **Suggestion:** Free BiRefNet after matte and StableNormal after normal prediction (`del` + `empty_cache()`); a small `staged_run()` moving each Hi3DGen stage to GPU around its call; in the fork, delete `reg_c`, cast `reg_v` to int32, construct lazily. Verify against finding 7's reserved+allocated GiB fields on one candidate regeneration.
- **Outcome:** `9/10` — headroom for every quality experiment in this queue, on 12 GiB hardware.
- **Cost:** `4/10` — mechanical edits + one gated verification run.
- **Path:** helper frees → fork buffer fixes → staged_run → one candidate run confirms the new peak.

### 18. Probe the discarded 6-channel vertex attributes, then exploit or excise
- **Evidence:** The mesh decoder is configured `use_color: true` and interpolates 6 sigmoid channels per vertex ("including normal map", `fork:hi3dgen/representations/mesh/cube2mesh.py:325-329`, `365-373`) into `MeshExtractResult.vertex_attrs`; `to_trimesh` (`fork:hi3dgen/representations/mesh/cube2mesh.py:92-115`) never reads them. `raw.glb` carries only POSITION+NORMAL. Computing them costs the +407 MB grid growth noted in finding 17.
- **Ideal:** Either the model-native per-vertex prior feeds the texture stage (whose measured coverage is 0.44–0.65 and whose main complaint is inpaint filler), or we stop paying for it.
- **Gap:** Unknown payload — this checkpoint is normal-trained, so the channels may be normal-space rather than albedo. One export answers it.
- **Suggestion:** Flag-gated `vertex_colors=` export of one candidate; inspect. If useful → design a texture-stage consumer (goes to the reworks file as a follow-on). If not → drop `color` from the concat and skip `_interpolate_colors`, banking the 407 MB.
- **Outcome:** `5/10` — either a free texture prior or a free VRAM cut.
- **Cost:** `3/10`
- **Path:** probe export → eyeball → exploit-or-excise decision recorded in the manifest either way.

### 19. Decide `concept_rgba.png`'s fate — its stated consumer does not exist (user-decides)
- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py:95-102` justifies the matte output as what `prop_texture.py` projects; `scripts/ai-pipeline/prop_texture.py:284-293`'s full argument list takes no image. Repo-wide, `concept_rgba` is only a skip sentinel and sha source; the projection design it cites exists only in `tasks/ai-pipeline/research/a6-1-mr-contract.md:115` as an unimplemented proposal.
- **Ideal:** Either the a6-1 projection consumer exists, or the ~30 lines producing a second matte don't.
- **Gap:** A misleading docstring and dead output in the middle of the geometry stage.
- **Suggestion:** User call on the a6-1 plan: (a) **drop** — delete `matte_concept`, run `check_matte` on the fork's own preprocess intermediate, switch the skip sentinel to the manifest (outcome `5/10`, cost `2/10`); (b) **implement** the projection consumer (outcome `6/10`, cost `7/10` — it is a texture-stage design).
- **Outcome:** `4/10`
- **Cost:** `2/10`
- **Path:** ask at queue launch (user-decides batching) → execute the chosen branch.

### 20. Per-asset `height_m` — stop shipping every prop at 1.8 m
- **Evidence:** Raw meshes are normalized to ~unit box (no scale is possible from image-to-3D); `scripts/ai-pipeline/prop_cleanup.py:195` defaults `--height` to 1.8 and `scripts/ai-pipeline/gen_prop.py:171` never passes it. Measured shipped heights: broken_column 1.800, gravestone 1.802, crucero 1.799, olive_stump 1.805 — and the "towering Italian cypress" at 1.799. Only chapel_arch (5.497) escaped, via an off-chain manual invocation. `prop_audit.py`'s per-metre density stats are computed against fictional sizes.
- **Ideal:** Real-world height is a required registry field, per the registry's own no-defaults doctrine (`scripts/ai-pipeline/proptex/registry.py`); `zones.ron` scale corrects placement, not model size.
- **Gap:** One field violates the doctrine and every density metric inherits the fiction.
- **Suggestion:** Add `height_m` (and optionally `tri_budget`) to `content/models/assets.json` + `_GENERATED_FIELDS`; refuse on absence; pass from `gen_prop.py`. Re-run `prop_audit.py` to re-baseline densities.
- **Outcome:** `7/10`
- **Cost:** `2/10`
- **Path:** registry field + plumbing → per-asset values (choose from concept briefs) → audit re-baseline.

### 21. Assert the sparse-attention backend at startup
- **Evidence:** Sparse attention accepts only xformers/flash_attn (`fork:hi3dgen/modules/sparse/__init__.py:49`); flash_attn has no Windows wheel, so xformers is a hard single-wheel dependency whose only diagnostic is an import-time print. Setting `ATTN_BACKEND=sdpa` would break the sparse import while dense attention silently degrades.
- **Ideal:** A misconfiguration fails loudly at second 0, not 90 seconds into a batch.
- **Gap:** A torch/xformers wheel mismatch (the README records how carefully this stack was assembled) takes the geometry stage down with no clear signal.
- **Suggestion:** Startup assertion in `prop_hi3dgen.py` that `hi3dgen.modules.sparse.ATTN == "xformers"`; record it in the manifest (rides finding 7). A real sparse-sdpa fallback is upstream-shaped work — only if the wheel ever actually breaks.
- **Outcome:** `4/10`
- **Cost:** `1/10`
- **Path:** one assert + manifest field.

### 22. Freeze the real environment; shed the demo dead weight
- **Evidence:** `fork:requirements.txt` diverges from the venv on ~6 packages including every GPU-critical one: lists `gradio`/`triton` (never installed), `timm==0.6.7` (actual 1.0.28), omits torch/xformers/spconv/cumm entirely; the real install list lives only in `scripts/ai-pipeline/README.md:411-415` prose and installed versions (spconv 2.3.8, xformers 0.0.31.post1) match neither source. `fork:app.py` (282 lines) cannot even run in this venv (no gradio); `fork:assets/` is 41 MB of demo images (also the LFS-dirt source in finding 2).
- **Ideal:** One machine-readable lock that reproduces the environment shipped assets were made in, mechanically re-auditable for licensing.
- **Gap:** The declared and real dependency sets diverge exactly where geometry is determined.
- **Suggestion:** Commit `requirements.lock.txt` (venv freeze) to the fork; record skimage/trimesh/spconv/numpy versions in the manifest (rides finding 7); optionally delete `assets/` + `app.py` from the working tree (recoverable from origin) — weigh against keeping the tree upstream-identical.
- **Outcome:** `4/10`
- **Cost:** `2/10`
- **Path:** freeze + commit; the deletion decision rides the finding-2 cleanup.

### 23. Refresh the upstream baseline
- **Evidence:** `origin/main` has never been fetched since the 2026-07-19 clone (no FETCH_HEAD, no remote-ref log); its tip `c29f668` is dated 2025-07-02. "Level with upstream" currently means level with a year-old tip observed nine days ago.
- **Ideal:** Divergence assessed against a current fetch; upstream fixes (if any) consciously adopted or declined.
- **Gap:** A year of possible upstream activity is invisible — though the year-old tip suggests dormancy.
- **Suggestion:** `git fetch origin` after finding 1's push, then `git log HEAD..origin/main --oneline`.
- **Outcome:** `4/10`
- **Cost:** `1/10`
- **Path:** fetch + log + one-line note in the fork's README or this report's successor.

### 24. Measure extraction wall-time — the gate for the parked GPU-MC rework
- **Evidence:** `fork:hi3dgen/representations/mesh/cube2mesh.py:136-147` round-trips a 68 MB volume to CPU and runs single-threaded skimage marching cubes over 17 M voxels while the GPU idles; plausibly the largest non-sampler wall-clock item, but unmeasured. Both topic branches add further CPU scipy passes to the same region.
- **Ideal:** Known per-stage timing; if extraction dominates, it overlaps the next candidate's GPU work under batch mode (finding 16) or moves to a permissively-licensed GPU extractor (parked rework 5). FlexiCubes/kaolin/nvdiffrast remain banned regardless.
- **Gap:** Cannot scope an optimization nobody has measured.
- **Suggestion:** Timer around the extraction block, recorded via finding 7's timing split; read the number off the next routine candidate run — no dedicated GPU run needed.
- **Outcome:** `5/10` — converts a parked rework's gate into data.
- **Cost:** `2/10`
- **Path:** timer + read result → strike or activate rework 5's gate.

## Carried forward from previous report

None — first audit of this domain.

## Resolved since last report

None — first audit of this domain.
