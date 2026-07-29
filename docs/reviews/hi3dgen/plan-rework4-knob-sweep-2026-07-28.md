# Plan: Knob-sweep harness — measure the extraction and guidance parameters nobody has ever varied — 2026-07-29

Source: `docs/reviews/hi3dgen/reworks-hi3dgen-2026-07-28.md` finding 4.

Anchor conventions as in the source report: `fork:` = `C:/tools/Hi3DGen/Hi3DGen`
(branch `vordar-fixes`, HEAD `c7389f5`, verified), unprefixed = vordar-repo
relative. Venv: `C:\tools\Hi3DGen\venv\Scripts\python.exe`.

## Ideal end state

Every inherited knob has a measurement on record: `iso_level` and `sdf_bias`
(extraction side — swept for free on CPU replays of the seven saved latents),
`occupancy_threshold`, and the sparse-structure and SLAT samplers'
`cfg_interval` lower bound and `rescale_t` (sampler side — swept on bounded GPU
runs against a measured same-seed noise floor). Each knob's default is either
confirmed or changed by user-approved evidence; the manifest already records
all of them, so the adopted values are self-documenting. "All defaults
confirmed" is a success outcome — the finding's Ideal is *measured* defaults,
not *moved* ones.

## Design decisions

**1. The plan splits along the saved-latent line.** `scripts/ai-pipeline/prop_extract.py`
replays `SparseFeatures2Mesh` deterministically on CPU from
`target/prop-latents/<prop>/cubefeats.pt` (7 props; baseline replays reproduce
exactly — candelabra_shrine 167,479 v / 334,938 f re-confirmed in this plan's
own probes). `iso_level` and `sdf_bias` act inside that replay, so their sweep
costs zero GPU and carries zero noise. `occupancy_threshold` acts upstream of
the saved latents (`fork:hi3dgen/pipelines/hi3dgen.py:309`, coords from
`decoder(z_s) > threshold` feed SLAT sampling), and `cfg_interval`/`rescale_t`
are sampler params (`fork:hi3dgen/pipelines/samplers/flow_euler.py:197,195`) —
those need real generation runs. CPU work runs first and stands alone.

**2. Criteria register — the free-parameter discipline is the plan's spine.**
This campaign was burned three times by criteria whose free parameter was never
swept: an SDF direction count (26 → 1330 collapsed 690,882 filled cells to 7),
an interior-ray count (64/256/1024 gave 34.1%/25.7%/16.6% and never converged),
and a weld epsilon (monotone-wrong at every refinement; stage deleted).
A knob-sweep harness is a machine for producing that failure at scale, so:
**every evaluation criterion below either carries no free parameter, or its
answer must be shown stable as that parameter is refined toward its limit
BEFORE any knob result computed with it is believed.** No threshold, arm value,
or acceptance bound may ever be moved so a metric passes. The register:

| criterion | free parameter | status / obligation |
|---|---|---|
| vertex/face counts, `trimesh.volume`, `body_count` | none — exact functions of the mesh | usable as-is |
| topology set: `boundary_edge_count` (edges bordering exactly 1 face), `component_count` (vertex-connectivity islands), `main_face_fraction`, `main_boundary_edge_count`, `main_euler_number` (V−E+F on the largest island alone), `boundary_edges_per_face` (main island's boundary edges / its faces) | none — exact counts | trimesh mirror of `prop_cleanup.py:148` `geometry_health`, validated this pass: it reproduces the production-recorded candelabra raw `main_face_fraction` 0.6581 exactly |
| `ss_active_voxels` (coords count), occupancy logit→count curve | none — exact per threshold | new manifest field / dump, step 3 |
| sampled surface deviation arm↔baseline (mean, p99) | **sample count** | report at 20k/80k/320k; believe only values stable across that refinement. Probe measurement: mean 0.007084→0.007122, p99 0.02063→0.02028 across 20k→80k, query cost 3.2 s/7.3 s — the instrument is stable and cheap on this data |
| turntable renders | n/a | descriptive evidence for the user's eye only, never a numeric gate; any visually-judged adoption is the user's call |
| GPU same-seed noise floor | **repeat count (3)** | the floor is a lower-bound estimate of run spread, not a gate constant. Effects at or near it are reported "unresolved at this noise", full stop — repeats are never added until an effect clears, and no arm is re-run to improve its number |

**Refused criteria** (divergence from the finding's Suggestion, which predates
the campaign's instrument lessons): (a) a numeric "thin-feature survival count"
— any such detector needs a thinness radius / ray count and would be a fresh
parameterized instrument grading the very sweep that justifies building it;
thin-feature judgment stays with topology counts plus the user's eye on
renders. (b) silhouette-IoU vs the concept image — mesh↔concept camera
alignment (fov, distance, orientation, resolution) is a stack of free
parameters with no calibration corpus; misalignment would be read as knob
effect. (c) the finding's "component/watertight stats from audit finding 14" —
`is_watertight` and `two_crossing_ray_fraction` no longer exist (deleted with
rework 1's close); the topology set above is their successor. The finding's
"needs the knob exposure from rework 1" is satisfied: `iso_level`, `sdf_bias`,
`occupancy_threshold` are all threaded and manifest-recorded at fork `c7389f5`
/ `prop_hi3dgen.py:481-487`.

**3. `iso_level` and `sdf_bias` are two real axes, not one.** Measured this
pass on the production chain (candelabra_shrine latent, CPU): at the *same*
effective raw-SDF level (surface where `raw_sdf = iso_level − sdf_bias`), the
two knobs give meshes 5% apart — `drop_solid_floaters` classifies solid at a
fixed `sdf < 0` (`fork:hi3dgen/representations/mesh/utils_cube.py:74-77`), which
`sdf_bias` shifts and `iso_level` does not, and the `+1`-stamped band edge
(`utils_cube.py:95`) interpolates differently under each. Probe table
(baseline = iso 0.0, bias −1/256 = −0.00390625):

| arm | raw level | verts | faces | volume | bodies |
|---|---|---|---|---|---|
| baseline | +0.0039 | 167,479 | 334,938 | 0.015485 | 11 |
| iso +0.030, bias default | +0.0339 | 299,364 | 600,368 | 0.025779 | 307 |
| iso 0, bias −0.03390625 | +0.0339 | 284,600 | 570,834 | 0.027053 | 313 |
| iso −0.030, bias default | −0.0261 | 185,029 | 367,484 | 0.017469 | 464 |

So the sweep is one-factor-at-a-time on each axis. And the response is steep —
±0.03 explodes body count 11 → 307/464 and manufactures 938 boundary edges on
a raw mesh that arrives with 0 — so the sweep arms sit within ±0.02 of the
default, not at ±0.03.

**4. GPU arms are full `prop_hi3dgen.py` runs, not a new stage-pinned replay
harness.** Rework 6 (unresolved) measured same-seed GPU vertex spread at
~0.012% (541220/541286/541242); whether topology stats and `ss_active_voxels`
are similarly tight is exactly what the noise-floor step measures. Building a
z_s-pinning SLAT-replay harness to suppress noise that may already be
negligible is speculative machinery; if the measured floor turns out to swallow
every sampler-knob effect, "indistinguishable at the measured noise" is the
honest result (it was ab-sampler's), and the harness question goes back to the
user with the floor table as evidence. Every GPU A/B in this plan reports
deltas against the floor; nothing is claimed from a single pair.

**5. Occupancy arms come from a measured curve, not from guesses.** The
sparse-structure latent z_s was never saved (the latent dumps start at slat),
so the occupancy-logit distribution cannot be read from disk today. The fork
gains a `last_ss_logits` attribute (precedent: `SparseFeatures2Mesh.last_extract_s`,
`fork:.../cube2mesh.py:291`), the runner a `--dump-ss-logits` flag; the
noise-floor runs dump it, and the threshold arms are chosen from the resulting
active-count-vs-threshold curve — recorded in the report *before* the arms run.

**6. Winners are the user's call.** ab-sampler precedent ("the winner is the
user's call") plus the standing visual-judgment rule. Each sweep step ends in a
report with stats tables, floor-relative deltas, and render contact sheets; the
adoption step executes only user-approved changes. Sampler-side adopted values
land as `prop_hi3dgen.py` flag defaults (the existing `SS_CFG_DEFAULT` idiom —
`weights/.../pipeline.json` stays byte-pristine because `models.sha256` pins
it); extraction-side values land as `SparseFeatures2Mesh` constructor defaults
in the fork.

**7. Bounded budget, §8.** OFAT only, no cross-products. Named GPU spend, each
gated at its own step: smoke ~5 min (2 runs), noise floor ≤25 min (9), occupancy
≤30 min (12), SS guidance ≤45 min (18), SLAT guidance ≤20 min (8), adoption
confirmation ≤20 min (≤7, only if a sampler-side default changes). Worst-case
total ≈ 2.5 h GPU wall (per-run anchor: a single candidate process measured
"~2 min" end-to-end, source report finding 14; warm setup 28.9 s, finding 16).
CPU spend: extraction sweep ≤45 min, finalist `prop_cleanup.py` runs ≤1 h
(chapel_arch measured ≈16.5 min by log timestamps, `r1s78/chapel_arch/cleanup.log`).
Approval of this plan is the §8 go-ahead for exactly these runs.

**8. Subjects and seeds** (fixed throughout, chosen for the thin-feature
vocabulary the finding names): chapel_arch (large architectural, through-opening;
concept `target/prop-batch/b3/arch/cand_0/concept.png`, seed 0),
candelabra_shrine (thin scrolled arms, the established hard case; concept
`target/prop-batch/candelabra-z/cand_4/concept.png`, seed 4), crucero (thin
freestanding cross; concept `target/prop-batch/b3/crucero/cand_21/concept.png`,
seed 21). All three concept files verified present; seeds are each subject's
saved-latent generation seed, so the saved latents are the seed-matched
baselines. Sweep artifacts live under `target/knob-sweep/` (gitignored).

## Findings (execution order)

### 1. Expose the extraction knobs and the topology stat set on `prop_extract.py`

- **Evidence:** `scripts/ai-pipeline/prop_extract.py:46-53` parses only
  `latents_dir`, `--out`, `--device`; line 67-69 constructs
  `SparseFeatures2Mesh(device=args.device, res=MESH_RESOLUTION)` with default
  knobs, although the constructor (`fork:hi3dgen/representations/mesh/cube2mesh.py:276-278`)
  accepts `min_component_fraction`, `iso_level`, `sdf_bias`. The stats line
  (lines 80-90) emits vertex/face counts, volume, `body_count`,
  `is_watertight` — none of the topology set `prop_cleanup.py`'s
  `geometry_health` (line 148) established as the campaign's raw-mesh health
  vocabulary.
- **Ideal:** `prop_extract.py` is the extraction-stage sweep instrument: it
  accepts `--iso-level` (default 0.0), `--sdf-bias` (default: omitted → the
  constructor's `-1/res`), `--min-component-fraction` (default 1e-4), records
  all three in its stats JSON, and emits the parameter-free topology set
  alongside the existing fields.
- **Gap:** the knobs exist in the fork but cannot be driven from the replay
  CLI; the replay stats cannot see the defects (boundary edges, fragmentation)
  that the sweep must score.
- **Suggestion:** Add the three `argparse` options and pass them through to the
  constructor. Add a module-level `topo_stats(mesh: trimesh.Trimesh) -> dict`
  (importable — later sweep drivers reuse it on GPU-produced `raw.glb` files)
  implementing exactly: `edges = mesh.edges_sorted`; unique rows with counts
  via `np.unique(edges, axis=0, return_counts=True)`; boundary edge = count
  == 1; components = `trimesh.graph.connected_components(uniq_edges,
  nodes=np.arange(len(mesh.vertices)), min_len=1)` sorted descending by
  length; main island = largest; `main_faces` counted by membership of
  `faces[:, 0]`, `main_edges`/`main_boundary` by membership of `uniq[:, 0]`
  (faces and edges never span islands, so first-vertex membership is full
  membership — same shortcut `geometry_health` uses at
  `prop_cleanup.py:163-167`); fields `boundary_edge_count`,
  `component_count`, `main_face_fraction` (round 4),
  `main_boundary_edge_count`, `main_euler_number` = |main verts| − main_edges
  + main_faces, `boundary_edges_per_face` = main_boundary / main_faces
  (round 4). Merge `topo_stats(trimesh_mesh)` plus
  `{"iso_level": ..., "sdf_bias": ..., "min_component_fraction": ...}` into
  the printed stats dict. Delete nothing existing.
- **Path:** implement → verify with two CPU replays of
  `target/prop-latents/candelabra_shrine` (venv
  `C:\tools\Hi3DGen\venv\Scripts\python.exe`), which this plan pre-measured on
  the production chain so the assertions are exact:
  (1) defaults: stats line must show `vertex_count == 167479`,
  `face_count == 334938`, `boundary_edge_count == 0`, `component_count == 11`,
  `main_face_fraction == 0.6581`, `main_euler_number == -8`,
  `iso_level == 0.0`, `sdf_bias == -0.00390625`;
  (2) `--iso-level 0.03`: `vertex_count == 299364`, `face_count == 600368`,
  `boundary_edge_count == 938`, `component_count == 307`,
  `main_euler_number == -1869`, `iso_level == 0.03`.
  The 0.6581 figure equals the production `geometry_health`'s recorded
  `raw_main_face_fraction` for this prop
  (`target/prop-solid-validation/r1s78/candelabra_shrine/cleanup.json`), which
  is the cross-instrument check. Each replay ≈15 s CPU. Gate: no Rust touched
  (workspace trivially green); fork untouched.

### 2. CPU sweep: `iso_level` and `sdf_bias` axes on three subjects

- **Evidence:** the defaults (`iso_level = 0.0`, `sdf_bias = -1/256`,
  `fork:hi3dgen/representations/mesh/cube2mesh.py:277-284`) are checkpoint
  accidents inherited from TRELLIS, never varied. This plan's probe measured
  the response steep and two-axis (Design decision 3's table): ±0.03 in level
  fragments candelabra 11 → 307/464 bodies in *both* directions, so the
  default sits near a body-count minimum whose neighborhood has never been
  mapped.
- **Ideal:** a measured curve per axis per subject — exact topology set,
  counts, volume, and refinement-stable surface deviation vs the default-arm
  replay — plus render sheets, ending in a keep/change recommendation the user
  rules on.
- **Gap:** no arm between 0 and ±0.03 has ever been extracted; nothing is
  known about where fragmentation and boundary-edge growth begin, in either
  axis or either direction.
- **Suggestion:** OFAT arms, all run with step 1's `prop_extract.py` on the
  saved latents of chapel_arch, candelabra_shrine, crucero
  (`target/prop-latents/<prop>/`, `--device cpu`):
  `--iso-level` ∈ {−0.02, −0.01, −0.005, +0.005, +0.01, +0.02} at default
  bias; `--sdf-bias` ∈ {0.0, −0.0078125, −0.015625} at default iso (the 0.0
  arm tests whether the inherited −1/256 earns its place at all). 27 replays +
  3 baseline replays ≈ 15-25 min CPU (measured 13-27 s each, subject-dependent).
  For each arm, a driver (scratchpad script; exact command lines recorded in
  the report) computes sampled surface deviation arm↔same-subject baseline via
  `trimesh.sample.sample_surface(arm, n, seed=0)` +
  `trimesh.proximity.ProximityQuery(baseline).on_surface(...)`, reported at
  n = 20k/80k/320k — a deviation number is quoted only where mean and p99 are
  stable across that refinement (probe: 3.2 s/7.3 s at 20k/80k, ~30 s at
  320k). Render every arm with the existing offscreen turntable
  (`cargo run -p engine-renderer --bin turntable --features offscreen --
  <raw.glb> --out <dir> --size 512x512 --angles 4`, the ab-sampler precedent)
  and stitch per-subject contact sheets.
- **Path:** run the grid → write
  `docs/reviews/hi3dgen/ab-extraction-level-<date>.md` with: per-subject
  tables (all topology fields, counts, volume, deviation with its three-n
  column so stability is visible on the page), the probe table from this plan
  as the ±0.03 outer anchors, contact-sheet paths, and a recommendation.
  Verification predicate: the report's baseline rows must equal step 1's
  pinned values exactly (CPU determinism — any drift is an instrument bug, not
  noise), and every quoted deviation must show its 20k/80k/320k triple.
  Artifacts under `target/knob-sweep/extraction/<subject>/<arm>/`. **User
  checkpoint: keep or change `iso_level`/`sdf_bias` — the user rules on the
  report + sheets; no default changes in this step.** Gate: no source changed
  (measurement step); no Rust touched.

### 3. Sampler-knob and occupancy CLI on `prop_hi3dgen.py`, plus the ss-logits dump

- **Evidence:** `scripts/ai-pipeline/prop_hi3dgen.py:98` hardcodes
  `OCCUPANCY_THRESHOLD = 0.0` (no flag); lines 422-423 merge
  `{**pipeline.sparse_structure_sampler_params, "steps": ..., "cfg_strength": ...}`
  so `cfg_interval` [0.5, 1.0] and `rescale_t` 3.0 always ride in from
  `fork:weights/trellis-normal-v0-1/pipeline.json` with no override path; the
  manifest already records the post-merge dicts (line 480) and the extraction
  block (lines 481-487). `fork:hi3dgen/pipelines/hi3dgen.py:309` computes
  `decoder(z_s)` and immediately thresholds it — the logit grid is never
  observable. The manifest records no active-voxel count.
- **Ideal:** every sweep axis drivable from the CLI, every run
  self-documenting: `--occupancy-threshold`, `--ss-cfg-interval-lo`,
  `--slat-cfg-interval-lo`, `--ss-rescale-t`, `--slat-rescale-t`,
  `--dump-ss-logits`, and an `ss_active_voxels` manifest field.
- **Gap:** the three sampler-side axes cannot be varied without editing
  source; occupancy arms cannot be placed without the logit distribution,
  which no artifact holds.
- **Suggestion:** Fork (one commit, branch `vordar-fixes`): in
  `sample_sparse_structure` (`fork:hi3dgen/pipelines/hi3dgen.py:278-311`),
  bind `logits = decoder(z_s)`, set
  `self.last_ss_logits = logits.detach().float().cpu()` (64³ ≈ 1 MB;
  attribute precedent `SparseFeatures2Mesh.last_extract_s`), threshold from
  `logits`. Vordar side: add the five flags — occupancy default
  `OCCUPANCY_THRESHOLD`, the four sampler flags default `None` meaning "do
  not override" so the checkpoint values ride exactly as today; when set,
  merge as `"cfg_interval": (args.ss_cfg_interval_lo, 1.0)` /
  `"rescale_t": args.ss_rescale_t` into `ss_params`/`slat_params` at lines
  422-423 (the manifest then records them with zero extra work). Thread
  `args.occupancy_threshold` to `staged_sample` (line 438) and into the
  manifest's extraction block (line 486). Record
  `"ss_active_voxels": int(coords.shape[0])` (surface it from `staged_sample`,
  which holds `coords` at line 184) in the extraction block. With
  `--dump-ss-logits`, save `pipeline.last_ss_logits` to
  `<cand_dir>/ss_logits.pt`.
- **Path:** implement → GPU smoke, two runs on candelabra_shrine
  (`target/prop-batch/candelabra-z/cand_4/concept.png`, `--seed 4`,
  ≈2 min each, §8 share ~5 min):
  (1) all new flags omitted + `--dump-ss-logits`: assert manifest
  `sampler_params.sparse_structure == {"steps": 50, "cfg_strength": 5.0,
  "cfg_interval": [0.5, 1.0], "rescale_t": 3.0}` (byte-equal semantics to
  today's runs — the no-override contract), `ss_logits.pt` exists, and
  `int((torch.load("ss_logits.pt") > 0.0).sum()) == manifest
  extraction.ss_active_voxels` (dump↔run consistency);
  (2) `--occupancy-threshold 0.5`: assert
  `extraction.occupancy_threshold == 0.5` and `ss_active_voxels` differs from
  run 1's (behavioral: the flag reaches the sampler). Gate: fork suite
  `C:\tools\Hi3DGen\venv\Scripts\python.exe fork:tests/test_extraction_contract.py`
  stays 3/3; no Rust touched.

### 4. GPU noise floor: three identical repeats per subject, and the occupancy curve

- **Evidence:** rework 6 (source report finding 6, unresolved): three
  byte-identical same-seed runs gave 541220/541286/541242 vertices — GPU float
  order is nondeterministic. The spread of every *other* metric this plan
  scores (topology set, volume, `ss_active_voxels`, sampled deviation) has
  never been measured, and ab-sampler's single-run-per-arm grid is explicitly
  noise-unbounded ("the cheapest next experiment … is the same configs at a
  second seed, to size the noise floor").
- **Ideal:** a per-subject, per-metric floor table every later A/B row is
  reported against, plus the occupancy logit curves that place step 5's arms.
- **Gap:** without the floor, every sampler-knob delta in steps 5-7 is
  uninterpretable; without the logit curves, occupancy arm values would be
  guesses.
- **Suggestion:** For each subject (chapel_arch seed 0, candelabra_shrine
  seed 4, crucero seed 21 — concepts per Design decision 8), run
  `prop_hi3dgen.py <concept> --out target/knob-sweep/floor/<subject>/r<k>
  --seed <seed> --dump-ss-logits` for k = 1, 2, 3 (distinct `--out` per
  repeat defeats the resume-skip). 9 runs ≈ ≤25 min GPU (§8). A driver script
  (importing `prop_extract.topo_stats`) computes per repeat: vertex/face
  counts, volume, `body_count`, the topology set, `ss_active_voxels` (from
  the manifest), and pairwise sampled deviation between the three repeat
  meshes at 80k samples (all three pairs; one pair also at 20k/320k for the
  stability triple). The floor per metric = max − min across the repeats.
  From each repeat's `ss_logits.pt`: active-count-vs-threshold curve
  (`(logits > t).sum()` over a dense grid of t spanning the logit range —
  exact, no free parameter), all three repeats overlaid so the curve's own
  run-to-run jitter is visible.
- **Path:** run → write `docs/reviews/hi3dgen/noise-floor-<date>.md` holding
  the floor table (metric × subject), the deviation-floor values with their
  stability triple, and the three occupancy curves. Then choose step 5's four
  threshold arms from the curves — two below 0.0, two above, at values where
  the active count moves by clearly more than its measured repeat jitter —
  and record the chosen values plus their implied voxel counts in this report
  *before step 5 runs* (arm placement is thereby data-driven and
  pre-registered, and may never be revised after step 5's results are seen).
  Verification predicate: the report states, per metric, the three raw repeat
  values (not just the spread), and the vertex-count floor row must be
  consistent in order of magnitude with rework 6's measured ~0.012% — a 10×
  larger spread means the harness changed something and must be chased, not
  averaged over. Gate: no source changed; no Rust touched.

### 5. Occupancy-threshold sweep

- **Evidence:** the cut is hardcoded `> 0` at
  `fork:hi3dgen/pipelines/hi3dgen.py:309` and has never been varied; it decides
  which 64³ cells become the sparse structure that SLAT sampling fills — for
  chapel_arch, 14,757 active voxels at threshold 0
  (`target/prop-latents/chapel_arch/dump_manifest.json`). Thin-feature
  survival at the resolution limit is plausibly sensitive to it in both
  directions — that hypothesis is exactly what this step tests, with no
  pre-registered expectation of magnitude or direction.
- **Ideal:** measured meshes at four thresholds per subject, scored against
  step 4's floor, with the default confirmed or a change recommended to the
  user.
- **Gap:** zero arms exist at any threshold other than 0.0.
- **Suggestion:** For each subject (chapel_arch/0, candelabra_shrine/4,
  crucero/21), run the four thresholds chosen and pre-registered in step 4's
  report: `prop_hi3dgen.py <concept> --out
  target/knob-sweep/occupancy/<subject>/t<value> --seed <seed>
  --occupancy-threshold <value>`. 12 runs ≈ ≤30 min GPU (§8). Score each arm
  with the same driver as step 4 (counts, volume, topology set,
  `ss_active_voxels`, deviation vs the same subject's `floor/r1` mesh at
  20k/80k/320k), report every delta alongside that metric's floor row, and
  render each arm (turntable, 4 × 512², contact sheet per subject).
- **Path:** run → write `docs/reviews/hi3dgen/ab-occupancy-<date>.md`: per-
  subject tables with a floor column; deltas within the floor are labelled
  "unresolved at measured noise", never adjudicated. Verification predicate:
  each arm's manifest `extraction.occupancy_threshold` equals its directory's
  nominal value, and each arm's `ss_active_voxels` is consistent with step
  4's pre-registered curve reading for that threshold (within the curve's own
  measured repeat jitter) — a mismatch means the dump and the run diverged
  and invalidates the arm. **User checkpoint on the recommendation.** Gate:
  no source changed; no Rust touched.

### 6. Sparse-structure guidance sweep: `cfg_interval` lower bound and `rescale_t`

- **Evidence:** both ride in from `fork:weights/trellis-normal-v0-1/pipeline.json`
  (`cfg_interval [0.5, 1.0]`, `rescale_t 3.0`), consumed at
  `fork:hi3dgen/pipelines/samplers/flow_euler.py:183-220` (interval mixin) and
  `:130-131` (`t_seq = rescale_t * t / (1 + (rescale_t − 1) t)`), and have
  never been varied on either stage. ab-sampler-2026-07-28.md's conclusion
  points here: with the SLAT stage's own cfg-strength/steps knobs measured
  inert, "any further Hi3DGen quality work should aim at the sparse-structure
  stage".
- **Ideal:** six OFAT arms per subject, floor-bounded verdicts, default
  confirmed or a user-approved change.
- **Gap:** the SS stage's guidance schedule is entirely unmeasured.
- **Suggestion:** Arms (OFAT, defaults elsewhere): `--ss-cfg-interval-lo` ∈
  {0.0, 0.25, 0.75} (default 0.5 is the baseline from step 4's floor runs;
  interval upper bound stays 1.0), `--ss-rescale-t` ∈ {1.0, 2.0, 4.5}
  (default 3.0; 1.0 is the un-rescaled identity schedule). Three subjects ×
  6 arms = 18 runs ≈ ≤45 min GPU (§8), out dirs
  `target/knob-sweep/ss-guidance/<subject>/<knob>-<value>`. Same scoring
  driver and reporting discipline as step 5 (floor column, deviation
  stability triples, turntable sheets). Note `ss_active_voxels` per arm — a
  guidance change that moves the active set is visible there first, exactly
  and noise-cheaply.
- **Path:** run → write `docs/reviews/hi3dgen/ab-ss-guidance-<date>.md`, same
  predicate structure as step 5: manifest `sampler_params.sparse_structure`
  must carry the arm's `cfg_interval`/`rescale_t` (the no-override default
  rides for the other knob), every delta reported against its floor row,
  sub-floor deltas labelled unresolved. **User checkpoint on the
  recommendation.** Gate: no source changed; no Rust touched.

### 7. SLAT guidance mini-sweep: `cfg_interval` lower bound and `rescale_t`

- **Evidence:** same checkpoint params on the SLAT sampler
  (`fork:weights/trellis-normal-v0-1/pipeline.json` slat_sampler block,
  consumed via `fork:hi3dgen/pipelines/samplers/flow_euler.py:183-220`).
  ab-sampler measured the SLAT stage's *other* knobs (`cfg_strength` 3 vs 5,
  `steps` 6/12/25) inert to <1% — but `cfg_interval` and `rescale_t` were in
  neither grid, and an unmeasured knob stays unmeasured however inert its
  neighbors (the finding's whole premise). The prior null result justifies a
  smaller allocation, not omission.
- **Ideal:** four arms on two subjects closing the last unmeasured sampler
  axes, floor-bounded.
- **Gap:** no SLAT run has ever varied either param.
- **Suggestion:** Arms: `--slat-cfg-interval-lo` ∈ {0.0, 0.75},
  `--slat-rescale-t` ∈ {1.0, 4.5}; subjects candelabra_shrine (seed 4) and
  chapel_arch (seed 0) — the hard case and the big case. 8 runs ≈ ≤20 min GPU
  (§8), out dirs `target/knob-sweep/slat-guidance/<subject>/<knob>-<value>`.
  Same scoring driver, floor discipline, and render sheets as steps 5-6.
- **Path:** run → write `docs/reviews/hi3dgen/ab-slat-guidance-<date>.md`,
  same predicates: manifest `sampler_params.slat` carries the arm's value,
  deltas vs floor, sub-floor labelled unresolved. **User checkpoint on the
  recommendation.** Gate: no source changed; no Rust touched.

### 8. Adopt the user-approved defaults and confirm on all seven props

- **Evidence:** after steps 2 and 5-7, every knob has a user ruling: keep or
  change. Extraction defaults live at
  `fork:hi3dgen/representations/mesh/cube2mesh.py:276-284`
  (`SparseFeatures2Mesh.__init__`); sampler-side defaults belong in
  `scripts/ai-pipeline/prop_hi3dgen.py` module constants/flag defaults (the
  `SS_CFG_DEFAULT` idiom, lines 88-98 — `pipeline.json` stays pristine under
  its `models.sha256` pin). The manifest records whatever runs
  (`prop_hi3dgen.py:480-487`), so adoption is self-documenting from the first
  post-change run.
- **Ideal:** the approved values are the defaults, verified across all seven
  props, with the fork suite green — the finding's "winners adopted as
  recorded defaults", where confirming every default is an equally complete
  outcome (then this step's diff is the reports' verdict lines plus the queue
  strike, and no confirmation runs are spent).
- **Gap:** none until the rulings exist; this step is their executor.
- **Suggestion:** Apply exactly the approved changes, nothing else. Then
  confirm: if an extraction default changed — 7 CPU replays
  (`prop_extract.py`, one per `target/prop-latents/<prop>/`, ≈5 min total)
  asserting the new defaults appear in each stats line and no prop's
  `component_count` or `boundary_edge_count` regresses past its own
  default-arm value from the step 2/4 record; if a sampler-side default
  changed — 7 GPU runs at each prop's saved-latent seed (≤20 min, §8, ask
  bundled at the ruling checkpoint) with the same predicate on the topology
  set vs each prop's recorded baseline. For the changed-knob subjects, run
  `prop_cleanup.py` (Blender headless, `--asset <name>`, heights from each
  prop's registry entry; chapel_arch measured ≈16.5 min, three subjects ≤1 h
  CPU) and report the downstream stats (`interior_tris_removed`,
  `component_count`, `boundary_edges_per_face`) beside the r1s78 baseline
  table in the source report — descriptive, not gated.
- **Path:** apply approved diffs → fork suite
  `C:\tools\Hi3DGen\venv\Scripts\python.exe fork:tests/test_extraction_contract.py`
  3/3 (its fractions are passed explicitly, so extraction-default changes must
  not move it — a failure here means the change leaked somewhere it should
  not) → run the applicable confirmation set above → append the adopted
  values and confirmation table to the last sweep report → strike rework 4 in
  the source report's queue note. Gate: fork suite 3/3; no Rust touched;
  workspace green.
