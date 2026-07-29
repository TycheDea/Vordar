# Plan: Solid-interior extraction — land the hollow-shell fix with a validation harness — 2026-07-28

Source: `docs/reviews/hi3dgen/reworks-hi3dgen-2026-07-28.md` finding 1.

Anchor conventions follow the source report: `fork:` = `C:/tools/Hi3DGen/Hi3DGen`
(branch `vordar-fixes`), unprefixed = vordar-repo relative. The Hi3DGen venv
interpreter is `C:\tools\Hi3DGen\venv\Scripts\python.exe`; Blender is
`C:\Program Files\Blender Foundation\Blender 5.2\blender.exe`.

## Ideal end state

Every mesh the fork extracts is a solid single shell: `fill_enclosed_sdf` (the
validated sign-flood from fork branch `750397b`) runs on the dense SDF before
marching cubes, flag-exposed as `fill_interior: bool = True` and gated off the
training path, with SDF-space floater removal in the same pass and the
extraction knobs (iso level, sdf bias, occupancy threshold) exposed and
recorded in the run manifest. A committed synthetic-topology harness in the
fork proves the fill (sphere solid, vessel mouth open, plate stacks
bit-identical), and a paired A/B on the three hard-topology props' saved
latents proves it on real fields. Success values: `interior_tris_removed`
falls from 24.1% of raw tris (crucero, 46170/191354) to ≤ 2% (hard gate 5%);
`two_crossing_ray_fraction` rises from 0.105 (column) to ≥ 0.3; peak VRAM
stays at the 7.41 GiB baseline (assert ≤ 8.0 — the fill is CPU-side).
`prop_cleanup.py`'s interior strip is untouched and becomes the standing
regression instrument, then the geometry-health stats flip to fail-loud gates.

## Design decisions

- **Adopt the sign-flood (`750397b`), close the morphology stack (`53472a1`).**
  `750397b` is 20 lines, carries measured validation (sphere 55.2%→0.0%
  inward-facing area, volume 0.14x→1.03x, vessel not sealed, plate stacks
  bit-identical, 0.25–0.29 s at 257³) and its rejected alternative's failure
  numbers (skip-cell flood seals an 8-cell mouth, 4.9x volume). `53472a1`
  stacks four unvalidated heuristics, its `behind_surface` claim fails on
  non-convex geometry, and it costs 3–5× the runtime. The two double-fill if
  merged; they are alternatives. Both branches are already pushed to the
  `fork` remote, so closure = delete the local branches; the record survives.
- **Fill placement: `SparseFeatures2Mesh.__call__`, not `get_dense_attrs`.**
  `750397b` buried the call inside `get_dense_attrs`, which has no `training`
  knowledge and no config. The policy holder is `SparseFeatures2Mesh`: ctor
  flags (`fill_interior=True`, `min_component_fraction`, `iso_level`,
  `sdf_bias`), applied in `__call__` under `self.fill_interior and not
  training`. `utils_cube.py` keeps only pure functions (`fill_enclosed_sdf`,
  `drop_solid_floaters`), which is also what makes the harness able to drive
  them directly on CPU with no sparse-backend or GPU dependency. Threading:
  `decoder_mesh.py` passes `representation_config.get(...)` with the same
  defaults (the checkpoint JSON carries only `use_color`, so defaults rule);
  per-run opt-out is a `--no-fill-interior` CLI flag in `prop_hi3dgen.py`
  setting the extractor attribute after `from_pretrained`. Rejected: env vars
  (invisible to the manifest), editing the checkpoint JSON (couples a run
  choice to a weights artifact).
- **Sealed-hollow inputs extract solid, accepted.** A cavity with no positive
  path to the grid boundary is indistinguishable from solid interior on the
  grid; the only escape hatch is `fill_interior=False`. The fill function's
  docstring states this as a constraint (no history, no finding numbers).
- **Floater removal in SDF space, 26-connectivity, default
  `min_component_fraction = 1e-4` of total solid voxels.** Runs after the fill
  so component sizes include filled interiors. 26-connectivity so thin
  diagonal features (chains, filigree) stay one component; the outside-flood in
  `fill_enclosed_sdf` keeps 6-connectivity (conservative for the flood
  direction). 1e-4 volumetric ≈ the ~5-voxel floaters `prop_cleanup.py`'s 2%
  bbox-diag heuristic targets, an order of magnitude under a thin real feature
  (a 2×2×40-voxel rod is 1.4e-3 of a typical prop). `prop_cleanup.py`'s
  fragment strip stays unchanged as the mesh-space backstop. If step 6 shows
  `fragments_removed > 0` surviving into cleanup, or a real component near the
  threshold, the default recalibrates (recorded), not the design.
- **Validation runs on saved real latents, not fresh generations.**
  `target/prop-latents/<prop>/cubefeats.pt` (all 7 props, dumped Jul 26) is
  exactly the tensor `decoder_mesh.to_representation` hands to
  `mesh_extractor`, with the hollow `raw.glb` extracted from the same features
  saved beside it. A new committed `scripts/ai-pipeline/prop_extract.py` runs
  the extraction stage alone (CPU) over those latents, giving a paired
  fill-on/fill-off A/B on identical fields — exact attribution, immune to
  rework 6's same-seed geometry nondeterminism, and nearly GPU-free. It also
  becomes the instrument rework 4 needs for iso/bias sweeps. The only GPU run
  in this plan is one end-to-end `prop_hi3dgen.py` smoke (~2 min wall, §8
  go-ahead at plan approval) to verify the manifest block and VRAM. The full
  7-prop regeneration sweep stays downstream (one sweep picks up this rework +
  finding 20 heights + rework 9 coverage re-bakes) and is NOT duplicated here;
  turntable/visual review rides that sweep.
- **Bake constants:** `BAKE_MAX_RAY_DISTANCE_M = 0.03` is re-derived by
  measurement (clean→hires p99 deviation + 0.01 m cage on solid meshes) — the
  inner-wall hazard it sat next to is gone, so it only needs to cover
  decimation deviation. `AO_DISTANCE_M = 0.15` is **unchanged by decision**:
  its stated rationale (occlusion locality at voussoir-joint scale) is
  independent of the shell defect; with the cavity gone the same bound simply
  stops integrating against it. No fake re-derivation.
- **Rework 5's gate is unaffected.** The fill adds 0.25–0.29 s CPU to
  extraction; marching cubes runs over the same 257³ grid either way, and the
  halved output mesh makes post-extraction CPU work cheaper. Finding 24's
  extraction-time measurement should simply run with the fill on (it is the
  new production path).
- **Licensing:** everything here is scipy/numpy/torch/skimage/trimesh — no
  nvdiffrast, kaolin, or FlexiCubes anywhere in the design.

## Findings (execution order)

### 1. Interior-fill harness in the fork, committed red on `vordar-fixes`

- **Evidence:** The fork has no test infrastructure (`fork:` root holds only
  `app.py`, `hi3dgen/`, `weights/`, requirements files; no `tests/`, no
  pytest in the venv). `750397b`'s validation exists only as commit-message
  prose. The production dense-field path the harness must drive is
  `fork:hi3dgen/representations/mesh/utils_cube.py`: `sparse_cube2verts`
  (line 62) → `get_dense_attrs` (line 72, stamps `dense_attrs[..., 0] = 1`
  then scatters onto written vertices only) → skimage
  `measure.marching_cubes(level=0.0, gradient_direction='ascent',
  allow_degenerate=False)` with faces flipped, as
  `fork:hi3dgen/representations/mesh/cube2mesh.py:145-150,178` does. All of
  that is pure torch/numpy/skimage — importable and runnable on CPU under the
  Hi3DGen venv (which has scipy 1.17.1, trimesh 4.12.2, no pytest).
- **Ideal:** `fork:tests/test_interior_fill.py`, a plain-Python assert-based
  module (no pytest dependency; `main()` runs every case, prints one PASS/FAIL
  line each, exits non-zero on any failure), committed on `vordar-fixes`,
  currently red because the fill does not exist there yet.
- **Gap:** No committed, re-runnable proof of the fill's contract exists; the
  branch could regress or be re-derived wrong with nothing to catch it.
- **Suggestion:** One shared helper builds production-shaped scatter-band
  fields from an analytic SDF `f` (in cell units) on a `res = 96` grid:
  coords = integer cells with `|f(center)| < 1.0` (the surface-voxel shell the
  SLat structure provides); per-cube feats = `f` at the 8 corners shaped
  `(N, 8, 1)` with the production bias `-1.0/res` added; then
  `sparse_cube2verts(coords, feats, training=False)` →
  `get_dense_attrs(v_pos, v_attrs, res=res+1, sdf_init=True)` → take channel
  0, reshape `(res+1)³` → `from hi3dgen.representations.mesh.utils_cube
  import fill_enclosed_sdf` and apply it (this import/call is what is red
  today) → `measure.marching_cubes(level=0.0,
  gradient_direction='ascent', allow_degenerate=False)`, faces flipped
  `[:, ::-1]`, into `trimesh.Trimesh` (no processing/repair:
  `process=False`). Metrics: `signed volume ratio` = `mesh.volume /
  analytic_volume`; `inward-facing area fraction` = area-weighted share of
  faces whose ray from `centroid + 1e-3·normal` along `normal` hits the mesh
  (trimesh ray casting). Set `os.environ` for `ATTN_BACKEND=xformers`,
  `SPCONV_ALGO=native`, `HF_HUB_OFFLINE=1` and `sys.path.insert` of the fork
  root before any `hi3dgen` import (the package `__init__` chain pulls
  `modules.sparse`), exactly as `scripts/ai-pipeline/prop_hi3dgen.py:20-52`
  does.
- **Outcome:** `8/10` — the contract the whole rework rests on becomes
  executable instead of prose.
- **Cost:** `3/10`.
- **Path:** Write `fork:tests/test_interior_fill.py` with cases:
  (a) **sphere** r=30: volume ratio in `[0.95, 1.10]`, inward-facing fraction
  `< 0.01`; (b) **vessel** (shell between r=24 and r=30 minus a radius-4
  cylinder bored from the center through +z to outside — CSG via
  min/max of the primitive SDFs): volume ratio in `[0.90, 1.30]` — the upper
  bound is the sealing detector, since welding the mouth adds the ~24³ cavity
  and jumps the ratio past 2; (c) **plate stack** (plates 2 cells thick at
  gaps 2, 4, 8, 16, 32): `torch.equal(fill_enclosed_sdf(field, coords),
  field)` — the fill relabels zero cells on sub-band structures; (d)
  **through-tunnel** (40×40×12 box with a 16-wide square tunnel through its
  thin axis, the chapel_arch analog): volume ratio in `[0.90, 1.15]`. Run
  `C:\tools\Hi3DGen\venv\Scripts\python.exe tests/test_interior_fill.py` from
  the fork root: expected result today is an `ImportError` on
  `fill_enclosed_sdf` — record that as the red state. Commit the harness on
  `vordar-fixes` (short message, e.g. "Add interior-fill extraction harness";
  no attribution trailers). The vordar cargo workspace is untouched.

### 2. Land the sign-flood fill behind `fill_interior`, close both topic branches

- **Evidence:** `fork:hi3dgen/representations/mesh/utils_cube.py:72-78`
  (`get_dense_attrs`) stamps the dense 257³ grid `sdf=+1` and scatters
  predictions onto surface-voxel corners only, so marching cubes extracts a
  second inner wall — every shipped mesh is a closed double-walled hollow
  shell. The fix exists on local branch `fix-hollow-shell-extraction`
  (`750397b`, `fill_enclosed_sdf`: `scipy.ndimage.label` over the positive
  field, border-connected components stay outside, positive+unreachable+
  never-written cells relabel to −1, predicted values never altered).
  Branch `solidify-shell-interior` (`53472a1`) is the rejected alternative.
  Both are pushed to the `fork` remote (`remotes/fork/...` exist). The fill
  currently runs unconditionally, including on the training path.
  `fork:hi3dgen/representations/mesh/cube2mesh.py:309-394` is
  `SparseFeatures2Mesh` (`__call__` receives `training`, computes `sdf_d` as
  a flattened `(res+1)³` vector at line 368/371);
  `fork:hi3dgen/models/structured_latent_vae/decoder_mesh.py:133` constructs
  it from `representation_config` (checkpoint carries only
  `{'use_color': True}`).
- **Ideal:** `vordar-fixes` carries `fill_enclosed_sdf` in `utils_cube.py`
  (verbatim from `750397b` including its constraint comment about the
  sign-based passability predicate, plus a docstring line stating that a
  fully sealed cavity extracts solid because nothing on the grid can
  distinguish it); `SparseFeatures2Mesh.__init__` gains
  `fill_interior: bool = True`; `__call__` applies
  `sdf_d = fill_enclosed_sdf(sdf_d.view(r, r, r), v_pos).reshape(-1)` (where
  `r = self.res + 1`) after unpacking `sdf_d` and before
  `get_defomed_verts`, only when `self.fill_interior and not training`;
  `decoder_mesh.py:133` passes
  `fill_interior=self.rep_config.get('fill_interior', True)`. `get_dense_attrs`
  stays untouched. The step-1 harness is green. Both local topic branches are
  deleted; the fork-remote copies remain as the record.
- **Gap:** The proven 20-line fix sits unmerged on an unbacked local branch
  while ~40–50% of every shipped prop's budget describes invisible interior
  wall.
- **Suggestion:** Cherry-pick `750397b` onto `vordar-fixes` (expect small
  context conflicts — `utils_cube.py` drifted: int32 grid verts, docstrings),
  then move the call site out of `get_dense_attrs` into `__call__` per the
  design decision, add the ctor flag and decoder threading, and squash to one
  commit. Do not copy the commit message's validation prose — the harness now
  carries it.
- **Outcome:** `10/10` — the rework's core.
- **Cost:** `3/10` — the fix is written; this step is placement and plumbing.
- **Path:** Land the change on `vordar-fixes` as one commit. Test: run
  `C:\tools\Hi3DGen\venv\Scripts\python.exe tests/test_interior_fill.py` from
  the fork root — all four step-1 cases must pass, and the specific
  assertions that flip are sphere volume ratio (`0.1712` un-filled red → within
  `[0.95, 1.10]` green) and sphere inward-facing fraction (`0.4970` red →
  `< 0.01` green), while vessel stays `< 1.30` (mouth not sealed) and plate
  stacks stay `torch.equal`. Then `git branch -D fix-hollow-shell-extraction
  solidify-shell-interior` after confirming
  `git branch -r --contains 750397b` and `--contains 53472a1` list
  `fork/...` refs. Vordar workspace untouched.

### 3. SDF-space floater removal in the same pass

- **Evidence:** Sparse-lattice floaters currently survive to Blender, where
  `scripts/ai-pipeline/prop_cleanup.py:372-390` strips loose mesh islands
  whose bbox diagonal is under 2% of the whole (`FRAGMENT_DIAG_FRACTION`,
  line 51) — a mesh-space heuristic that runs after the tri/UV budget already
  paid for the floaters once at extraction. After step 2 the dense solid
  field exists in `SparseFeatures2Mesh.__call__` right where components are
  one `ndimage.label` away.
- **Ideal:** `fork:hi3dgen/representations/mesh/utils_cube.py` gains
  `drop_solid_floaters(sdf, min_fraction)`: label the solid cells
  (`sdf < 0`) with **26-connectivity**
  (`ndimage.label(..., structure=ndimage.generate_binary_structure(3, 3))`),
  relabel to `+1` every component whose voxel count is below
  `min_fraction × total_solid_voxels`, return the field.
  `SparseFeatures2Mesh.__init__` gains `min_component_fraction: float = 1e-4`
  (0 disables); `__call__` applies it immediately after the interior fill,
  under the same `self.fill_interior and not training` guard;
  `decoder_mesh.py` threads `representation_config.get('min_component_fraction',
  1e-4)`. `prop_cleanup.py`'s fragment strip is not modified — it stays as
  the relaxed mesh-space backstop.
- **Gap:** Floaters cost extraction, decimation and atlas budget before the
  bbox heuristic ever sees them, and the bbox heuristic itself can be fooled
  by an elongated fragment.
- **Suggestion:** ~10 lines in `utils_cube.py` plus two ctor/threading lines;
  ordering after the fill is load-bearing (component sizes then include
  filled interiors, so a hollow floater is weighed whole).
- **Outcome:** `6/10`.
- **Cost:** `2/10`.
- **Path:** Implement, then extend `fork:tests/test_interior_fill.py` with
  two cases run through the same helper chain plus `drop_solid_floaters` at
  `min_fraction=1e-4`: (e) sphere r=30 plus a detached 2³ blob in a corner
  (≈8 voxels ≈ 7e-5 of solid) → extracted trimesh `body_count == 1` (blob
  gone); (f) sphere r=30 plus a detached 2×2×40 rod (≈1.4e-3 of solid) →
  `body_count == 2` (thin feature survives). Run the harness; all six cases
  green. Commit on `vordar-fixes`.

### 4. Expose the extraction knobs and record them in the run manifest

- **Evidence:** Three extraction constants are checkpoint accidents nobody
  can see or vary: iso level hardcoded `level=0.0` at
  `fork:hi3dgen/representations/mesh/cube2mesh.py:147`; `self.sdf_bias =
  -1.0 / res` at `cube2mesh.py:315`; occupancy cut hardcoded
  `decoder(z_s)>0` at `fork:hi3dgen/pipelines/hi3dgen.py:307`
  (`sample_sparse_structure`). `scripts/ai-pipeline/prop_hi3dgen.py:377-424`
  writes the run manifest and records sampler params but nothing about
  extraction; `staged_run` (line 146) calls `pipeline.sample_sparse_structure`
  directly. Rework 4's knob sweep is blocked on this exposure.
- **Ideal:** Fork: `EnhancedMarchingCubes.__init__` gains
  `iso_level: float = 0.0`, used in its `__call__`'s `marching_cubes` call;
  `SparseFeatures2Mesh.__init__` gains `iso_level: float = 0.0` (passed to
  the extractor) and `sdf_bias: Optional[float] = None` (None → `-1.0/res`),
  both threaded from `representation_config` in `decoder_mesh.py:133` with
  the same defaults; `Hi3DGenPipeline.sample_sparse_structure` gains
  `occupancy_threshold: float = 0.0` replacing the literal `>0`. Vordar:
  `prop_hi3dgen.py` gains a module constant `OCCUPANCY_THRESHOLD = 0.0`
  passed through `staged_run` into `sample_sparse_structure`, a
  `--no-fill-interior` flag that sets
  `pipeline.models['slat_decoder_mesh'].mesh_extractor.fill_interior = False`
  after `from_pretrained`, and a manifest `"extraction"` block read from the
  **live** extractor object (never re-stated constants):
  `{res, fill_interior, min_component_fraction, iso_level, sdf_bias,
  occupancy_threshold}`.
- **Gap:** The manifest cannot currently distinguish a solid-extraction run
  from a hollow one, and rework 4 has no knob to sweep.
- **Suggestion:** Pure parameter-threading; no behavior change at defaults.
- **Outcome:** `6/10` — manifest honesty plus rework 4's prerequisite.
- **Cost:** `2/10`.
- **Path:** Implement fork side, commit on `vordar-fixes`. Test: extend the
  harness with case (g): drive the sphere field through
  `EnhancedMarchingCubes(device='cpu', iso_level=0.0)` and
  `iso_level=-0.2` (calling its `__call__` with `voxelgrid_vertices=None`,
  `voxelgrid_colors=None`) and assert the −0.2 extraction's trimesh volume is
  strictly smaller than the 0.0 extraction's (the field is negative-inside,
  so a lower level shrinks the body) — proving the parameter reaches skimage.
  Harness green (7 cases). Then implement the vordar side
  (`prop_hi3dgen.py`); its manifest block is asserted by step 6's GPU smoke
  (no GPU run in this step). Vordar gate: the touched file is Python — run
  `python -m py_compile scripts/ai-pipeline/prop_hi3dgen.py` as the local
  check; cargo workspace untouched.

### 5. `prop_extract.py`: extraction-stage runner over saved latents

- **Evidence:** `target/prop-latents/<prop>/` exists for all 7 generated
  props (dumped 2026-07-26), each holding `cubefeats.pt` (coords `[N,4]`
  int32, feats `[N,101]` float32 — exactly the tensor
  `fork:hi3dgen/models/structured_latent_vae/decoder_mesh.py:186` hands to
  `self.mesh_extractor(x[i], training=self.training)`; 101 = 8 sdf + 24
  deform + 21 weights + 48 color, matching `use_color=True`, res
  64·4 = 256), plus the hollow `raw.glb` extracted from those same features
  and `dump_manifest.json`/`generation_manifest.json` with seeds, sha256s and
  face counts (chapel_arch: 773,576 faces). `SparseFeatures2Mesh.__call__`
  reads only `.coords[:, 1:]` and `.feats` from its argument and runs
  entirely on the device it was constructed with — CPU works (pure torch +
  skimage), and CPU float order is deterministic where the GPU
  `scatter_reduce` is not (reworks file finding 6).
- **Ideal:** A committed `scripts/ai-pipeline/prop_extract.py` (~90 lines,
  runs under the Hi3DGen venv): args `<latents_dir> --out <dir>
  [--no-fill-interior] [--device cpu|cuda]` (default cpu). It sets the same
  pre-import env vars and `sys.path` insertion as `prop_hi3dgen.py:20-52`,
  loads `cubefeats.pt`, wraps it in `types.SimpleNamespace(coords=...,
  feats=...)`, constructs `SparseFeatures2Mesh(device=args.device, res=256,
  use_color=True)` (flip `fill_interior` per flag), calls it with
  `training=False`, exports `to_trimesh(transform_pose=True)` to
  `<out>/raw_solid.glb` (or `raw_hollow.glb` with the flag), and prints one
  JSON stats line: vertex/face count, trimesh `volume`, `body_count`,
  `is_watertight`, fill flags, elapsed seconds, source `cubefeats.pt` sha256.
  This is the paired-A/B instrument for this rework and the sweep instrument
  for rework 4's iso/bias axes.
- **Gap:** The extraction stage can only be exercised today by re-running the
  full diffusion pipeline, which is GPU-bound and (finding 6) not
  seed-reproducible in geometry — every extraction change would be measured
  through sampler noise.
- **Suggestion:** Keep it a thin runner: no gates, no manifest chaining —
  stats line only, same one-JSON-line convention as `prop_cleanup.py`.
- **Outcome:** `7/10` — turns extraction A/Bs from GPU runs into deterministic
  CPU replays.
- **Cost:** `3/10`.
- **Path:** Implement. Test (behavioral, and the step's acceptance): run
  `C:\tools\Hi3DGen\venv\Scripts\python.exe scripts/ai-pipeline/prop_extract.py
  target/prop-latents/crucero --out target/prop-solid-validation/crucero
  --no-fill-interior` and assert its reported `face_count` matches the saved
  baseline within 0.5% (load `target/prop-latents/crucero/raw.glb` with
  trimesh for the reference count; the tolerance covers the GPU-vs-CPU
  scatter float-order drift finding 6 measured at ~0.01%) — this proves the
  runner reproduces the production extraction rather than approximating it.
  If the count differs by more than 0.5%, stop and report (the runner's
  config does not match the checkpoint's; do not tune tolerances to pass).
  Keep the produced stats JSON under `target/prop-solid-validation/`. Local
  gate: `python -m py_compile scripts/ai-pipeline/prop_extract.py`; cargo
  workspace untouched.

### 6. Paired hollow/solid validation on the three hard-topology props, plus one GPU smoke

- **Evidence:** Baselines on record: crucero `interior_tris_removed`
  46170/191354 raw tris (24.1%) with `blend_coverage` 0.7303→0.9759 after the
  strip (queue note, finding 15 at `1f32bbe`); column
  `two_crossing_ray_fraction` 0.105 while watertight with 0 boundary edges
  (finding 14) — manifold-closed yet double-walled; peak VRAM baseline
  7.41 GiB reserved / 42 s total per candidate (finding 17,
  `target/prop-batch/f17-after/hi3dgen_manifest.json`). Saved latents + hollow
  `raw.glb` exist for all three validation subjects: chapel_arch
  (through-opening), candelabra_shrine (separated arms), crucero (thin
  cross) under `target/prop-latents/<name>/`. Registry heights
  (`content/models/assets.json`): chapel_arch 5.497, candelabra_shrine 1.3,
  crucero 3.5. `prop_cleanup.py` invocation:
  `& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
  --background --python scripts/ai-pipeline/prop_cleanup.py -- <raw.glb>
  <clean.glb> --height <h>` (default `--tri-budget 15000`), printing one JSON
  stats line including `interior_tris_removed`, `fragments_removed` and the
  `geometry_health` fields. chapel_arch's full-pipeline concept is
  `target/prop-batch/b3/arch/cand_0/concept.png`, seed 0
  (`target/prop-latents/chapel_arch/generation_manifest.json`).
- **Ideal:** Measured proof, on real fields, that the fill converts hollow
  shells to solid single shells: per subject, a paired
  `prop_extract` fill-on vs the saved hollow baseline, then `prop_cleanup`
  on both, with all stats JSONs kept under `target/prop-solid-validation/`
  (this is the artifact trail future docs cite). One end-to-end
  `prop_hi3dgen.py` run proves the manifest `"extraction"` block and the VRAM
  claim.
- **Gap:** Steps 1–5 prove the mechanism on synthetic fields; a fix resting
  on a structural property of real input must be validated on real input, and
  the campaign's two success metrics (`interior_tris_removed`,
  `two_crossing_ray_fraction`) are only measurable through
  `prop_cleanup.py` on real props.
- **Suggestion:** Extraction replays are CPU; the single GPU item is the
  smoke run (~2 min wall including model load — the plan's only §8 item,
  named at approval).
- **Outcome:** `9/10` — this is where the rework's 10/10 outcome is either
  demonstrated or honestly parked.
- **Cost:** `4/10` — mostly Blender CPU minutes (six cleanup runs over
  0.4–1.1 M-tri meshes; the 64-ray strip dominates).
- **Path:** For each of chapel_arch, candelabra_shrine, crucero:
  (1) `prop_extract.py target/prop-latents/<name> --out
  target/prop-solid-validation/<name>` (fill on) → `raw_solid.glb`;
  (2) `prop_cleanup.py` on `target/prop-latents/<name>/raw.glb` (hollow
  baseline) and on `raw_solid.glb`, heights per registry above, saving both
  stats lines. Acceptance predicates, per subject: (a) `prop_extract`
  fill-on `face_count` is 30–55% below the hollow baseline's, and trimesh
  `volume`(solid) / `volume`(hollow) ≥ 1.5 — if the reduction is under 15%
  or the ratio under 1.2 on any subject, park and report the numbers (the
  shell premise fails there); (b) solid-run `interior_tris_removed /
  raw_tris ≤ 0.02` → success (≤ 0.05 → proceed but flag in the report;
  > 0.05 → park and report); (c) solid-run `two_crossing_ray_fraction ≥ 0.3`
  and ≥ 2× the paired hollow run's value → success (0.15–0.3 → record and
  instruct step 8 to gate on interior-fraction only; < 0.15 → park and
  report); (d) chapel_arch solid clean mesh `euler_number ≤ 0` (the
  through-opening survived as a handle; if it comes back 2, the arch was
  sealed — park and report); (e) `fragments_removed == 0` on solid runs
  (SDF-space pass caught floaters first; if > 0, record and note step 3's
  threshold as recalibration candidate — do not change it in this step).
  Then the GPU smoke: `C:\tools\Hi3DGen\venv\Scripts\python.exe
  scripts/ai-pipeline/prop_hi3dgen.py target/prop-batch/b3/arch/cand_0/concept.png
  --out target/prop-solid-validation/chapel_arch_e2e --seed 0` — assert the
  manifest contains the `"extraction"` block with `fill_interior: true`,
  `occupancy_threshold: 0.0`, `iso_level: 0.0`, and
  `vram.peak_reserved_gib ≤ 8.0` (baseline 7.41; the fill is CPU-side —
  if it exceeds 8.0, park and report, do not raise the bound). All stats
  files stay in `target/prop-solid-validation/`.

### 7. Re-derive `BAKE_MAX_RAY_DISTANCE_M` against solid hires meshes

- **Evidence:** `scripts/ai-pipeline/proptex/export.py:31-32` —
  `BAKE_CAGE_EXTRUSION_M = 0.01`, `BAKE_MAX_RAY_DISTANCE_M = 0.03` (no
  comment), used by `_bake_to` (line 40) for both the normal and AO
  selected-to-active bakes. The 0.03 bound sat at the same order as the
  hollow shells' 0.023–0.077 m wall thickness, so bake rays could land on the
  inner wall's wrong-facing normals; with solid hires meshes that hazard is
  gone and the bound's only remaining job is to exceed clean→hires surface
  deviation (decimation error) plus the 0.01 cage. `AO_DISTANCE_M = 0.15`
  (line 22) keeps its own locality rationale and is not changed by this plan
  (design decision).
- **Ideal:** The constant is derived from measurement on solid meshes: it
  covers p99 clean→hires deviation + cage with headroom, carries a
  constraint comment stating exactly that derivation (what bounds it — never
  history or finding numbers), and lands before the downstream regeneration
  sweep so the sweep bakes with it.
- **Gap:** 0.03 is inherited, not derived; nobody knows whether it clips
  legitimate bake rays on solid geometry or is generously safe.
- **Suggestion:** Measure with a scratchpad script (not committed): for each
  of the three step-6 subjects, load `clean.glb` and `clean_hires.glb` from
  `target/prop-solid-validation/<name>/` with trimesh, sample 20,000 surface
  points on the clean mesh (`trimesh.sample.sample_surface`), compute
  nearest-surface distance to the hires mesh
  (`trimesh.proximity.closest_point`), take p99 per subject; save the
  numbers as `target/prop-solid-validation/bake_ray_derivation.json`.
- **Outcome:** `5/10` — removes a silent texture-quality hazard and closes
  the rework's bake-constant clause.
- **Cost:** `2/10`.
- **Path:** Run the measurement. Decision rule: let `d = max over subjects of
  (p99 + 0.01)`. If `d ≤ 0.03` → keep `BAKE_MAX_RAY_DISTANCE_M = 0.03` and
  add the constraint comment stating the bound must exceed decimation
  deviation plus cage extrusion (cite the measured d in the step's final
  report, not in the comment). If `d > 0.03` → set the constant to `1.5 × d`
  rounded up to 0.005 and add the same comment. Verification: the constant's
  consumer is a Blender bake — no bake run here (the regeneration sweep
  verifies visually downstream); the step's test is the kept measurement JSON
  plus `python -m py_compile scripts/ai-pipeline/proptex/export.py` and one
  `import` check under Blender's python is unnecessary (the module only needs
  bpy at call time — py_compile suffices). Cargo workspace untouched.

### 8. Flip the geometry-health stats to fail-loud gates in `prop_cleanup.py`

- **Evidence:** `scripts/ai-pipeline/prop_cleanup.py:94-158`
  (`geometry_health`) and line 470 (`stats.update(geometry_health(me))`)
  currently report `is_watertight`, `boundary_edge_count`, `euler_number`,
  `component_count`, `two_crossing_ray_fraction` without acting on them;
  `interior_tris_removed` (line 399/459) likewise. The script's own contract
  (docstring lines 27-29) is that structural failures exit non-zero as
  decision-gate data. A hollow-shell regression now has a signature: the
  interior strip removes ~25–50% of raw tris and `two_crossing_ray_fraction`
  collapses toward 0.1.
- **Ideal:** `prop_cleanup.py` fails (via the existing `fail()`) any
  candidate whose stats carry the hollow-shell signature, so a regression in
  the fork's extraction can never silently re-enter the content path; the
  thresholds are calibrated from step 6's measured solid values, stated as
  constants with constraint comments.
- **Gap:** The instruments exist but gate nothing; a hollow regression would
  ship with only a JSON line to notice.
- **Suggestion:** Two gates after the stats dict is assembled and before it
  prints: (1) `interior_tris_removed / raw_tris > 0.05` → fail (solid
  extraction measures ≤ 0.02 per step 6; a hollow shell measures ≥ 0.24);
  (2) `two_crossing_ray_fraction <` a floor set to
  `round(0.75 × min(step-6 solid values), 2)` — but only if step 6's
  outcome (c) succeeded with all three subjects ≥ 0.3; if step 6 recorded
  the 0.15–0.3 band, install gate (1) only and state in the step report that
  the two-crossing gate is deferred to the regeneration sweep's data. Do not
  gate `is_watertight` (collapse decimation may legitimately open pinhole
  boundaries; it stays a reported stat).
- **Outcome:** `7/10` — converts the regression instrument into an actual
  gate, closing the rework's last Path clause.
- **Cost:** `3/10`.
- **Path:** Implement the gate(s) with constants near
  `FRAGMENT_DIAG_FRACTION` (constraint comments state what the thresholds
  separate — solid-extraction measurements vs the hollow-shell signature —
  no history/finding tags). Red test: build a hollow fixture with a
  scratchpad script — two concentric trimesh icospheres (outer r=1.0 normals
  out, inner r=0.9 normals flipped inward), exported to
  `target/prop-solid-validation/hollow_fixture.glb` — then run
  `blender --background --python scripts/ai-pipeline/prop_cleanup.py --
  target/prop-solid-validation/hollow_fixture.glb
  target/prop-solid-validation/hollow_fixture_clean.glb --height 1.0` and
  assert exit code non-zero with the interior-fraction gate named in stderr
  (the strip deletes the inner sphere ≈ 45% of tris, tripping gate 1).
  Green test: re-run `prop_cleanup.py` on
  `target/prop-solid-validation/crucero/raw_solid.glb` (step 6 output,
  height 3.5) and assert exit code 0 with the same stats step 6 recorded.
  Keep both fixture and outputs under `target/prop-solid-validation/`.
  Cargo workspace untouched; no content/ files change.
