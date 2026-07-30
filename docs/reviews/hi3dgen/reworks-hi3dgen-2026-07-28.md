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

> **~~finding 1~~ → ~~finding 2~~ → ~~finding 3~~ → ~~finding 4~~ → ~~finding 5~~ →
> ~~finding 6~~ → ~~finding 7~~ → ~~finding 8~~ → ~~finding 9~~ → ~~finding 10~~ →
> ~~finding 11~~ → ~~finding 12~~ → ~~finding 13~~ → ~~finding 14~~ →
> ~~finding 17~~ → ~~finding 15~~ → ~~finding 16~~ → ~~rework 1~~ →
> ~~finding 18~~ → ~~finding 19~~ → ~~finding 20~~ → ~~finding 21~~ →
> ~~finding 22~~ → ~~finding 23~~ → ~~finding 24~~ →
> ~~rework 2~~ → ~~rework 3~~ → ~~rework 4~~ → ~~rework 18~~.**

Rework 2 closed 2026-07-29, negative verdict, no code change beyond what
step 2 already landed: `docs/reviews/hi3dgen/ab-multiview-2026-07-29.md`.
Multi-view conditioning is not adopted; `--view`/`--mv-mode` stay plumbed,
opt-in, and dormant, not wired into `gen_prop.py`/`gen_character.py`
defaults.

This file's findings 24 and 25 (orientation-robust fidelity metric;
same-subject back/side noise floor) are **parked 2026-07-30, user ruling**:
no pending A/B consumes them since multi-view was rejected, so they wait for
the next orientation-sensitive experiment rather than entering this queue.
Campaign closed 2026-07-30 — aggregate regenerated at `1532b9d`, lesson-mining
pass done (2 lessons accepted: `tasks/lessons/2026-07-30-*`).

The findings numbered in *this* file (10–17, 19) are discoveries from rework
execution and sit outside that mirrored queue; they are struck here. Where the
two numberings collide, this file's own are written "this file's finding N".
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

**Lead (2) settled and fixed 2026-07-29 (`c9c695b`), this file's finding 17.**
Two ordering defects, not one. The raw mesh carries duplicate vertices at shared
corners, so neighbouring faces share no edge and every island count was reading
vertex bookkeeping rather than shape — welding at a sub-voxel epsilon drops
chapel_arch's raw count 3,824 → 1,012 with zero geometric change. And the
loose-fragment cull ran *before* `strip_interior_faces`, the only stage that
maroons fragments: where it sat it caught 14 islands / 204 tris, moved after the
strip it catches **3,575 / 9,563**. Correct order is weld → strip → cull
→ normalize. chapel_arch ends at **151 components**, 98% of faces kept, and
the 15,000-tri budget no longer spends 21% of itself on dust. The residual 150
islands each exceed the cull's own 158 mm threshold and were left alone rather
than chased with a second threshold. `strip_interior_faces` is untouched: it
remains the source of 100% of the fragmentation, and whether its 64-ray
escape-to-infinity test is merely noisy or systematically biased is the separate
open question a camera-visibility discriminator is measuring.

`geometry_health` also gained `boundary_edges_per_face` (hole count normalized by
main-island faces) as a **stat, not a gate** — the input rework 1 step 8 needs.
Caveat for that step: it is computed on the *final decimated* mesh, where
chapel_arch reads 1.0378 because the interior strip leaves a lace-like shell; the
hires figure is 0.0747 against a 0.1138 baseline. A gate must name which mesh it
reads, and 15k-tri decimation is not that mesh.

Done 2026-07-29 (finding 18, fork `973df9e`, vordar `482d41f`). Probe verdict
**excise** — neither channel group is a texture prior (measurements in the
finding). The dense attribute grid drops 257³×10 → 257³×4 (679.0 → 271.6 MB) and
the cube-corner concat (N,8,10) → (N,8,4); CPU extraction time falls 40.6 s →
26.5 s on chapel_arch and 22.5 s → 12.9 s on candelabra_shrine, a saving the
finding did not anticipate. Replay geometry is unchanged exactly (386,614 v /
773,518 f and 167,479 v / 334,938 f), harness 3/3, −76 lines.
**Architectural constraint found in-path:** `SLatMeshDecoder.out_layer` is trained
at `[101, 96]` in `slat_dec_mesh_swin8_B_64l8m256c_fp16.safetensors`, and
`decoder_mesh.py` sizes it off `feats_channels`. Deleting the layout entry would
have shrunk that to 53 and broken checkpoint loading. `feats_channels` therefore
stays 101 — the frozen network keeps emitting the columns; only the scatter,
interpolation and concat of them are gone, which is where the memory and time
went. No training-path consumer exists in this fork.
Done 2026-07-29 (this file's finding 16, **no code change** — the measurement
cleared the default). Per-component solid-voxel counts computed on all seven
saved latents through the production `sparse_cube2verts`/`get_dense_attrs` chain
with the same 26-connectivity `drop_solid_floaters` uses
(`target/prop-solid-validation/component_counts.json`). The debris ceiling is
**5.48e-5** (a 52-voxel speck on broken_column); the real-geometry floor is
**2.10e-4** (olive_stump's 268-voxel piece). `1e-4` sits at 1.07e-4, the geometric
midpoint of that gap — where a fresh calibration on the shell-only denominator
would land it anyway. The finding's premise (the denominator shrank) is true; its
conclusion (the default is now miscalibrated) is **premise-falsified**, because the
debris/geometry gap is an order of magnitude wide on both sides and the collapse
did not move `1e-4` out of it. Tightest margin is broken_column at 1.8x. Harness
still 3/3; CPU replays unchanged.

Regression-checked across all seven props (`c9c695b`, `4310640`): every one
processes, heights land, stats reconcile exactly. Fragment yields vary by two
orders of magnitude — gravestone 2,292, olive_stump 1,807, chapel_arch 3,575,
candelabra_shrine 13 — confirming the cull was catching almost nothing where it
sat. `boundary_edges_per_face` on the final mesh ranges 0.0899 (crucero) to
1.1944 (olive_stump); **no prop is watertight after cleanup**, which the next
note explains.

**Lead (1) answered 2026-07-29: the network's hollow output cannot be attacked at
source. The representation cannot hold a solid.** Read-only investigation on the
saved latents and the fork's own code:

- **The SLat latent is a surface band and nothing else.** TRELLIS defines the
  latent only on voxels *intersecting the surface*; coords come from thresholded
  occupancy of the sparse-structure flow (`fork:hi3dgen/pipelines/hi3dgen.py:309`) and
  the decoder emits SDF only at 4³ subdivisions of those (`decoder_mesh.py:133`,
  res 64 × 4 = **256³**, not 512³). Measured on `slat.pt`: chapel_arch's active
  set is 14,757 voxels, 99.3% at chessboard depth 1; gravestone 100% at depth 1.
  broken_column has **8,429 interior cells fully enclosed by its active shell and
  carrying no latent at all**. Cells without a latent are stamped `+1` outside at
  `utils_cube.py:95`. Solid is unrepresentable end to end — there is no tensor to
  put it in.
- **The head is a truncated SDF supervised only by renders.** `decoder_mesh.py:147`
  is a bare `SparseLinear`, no distance normalization; TRELLIS trains it with L1
  between rendered depth/normal maps and ground truth. Nothing in the loss ever
  observes interior sign. Measured: corner values in `cubefeats.pt` occupy a
  tight band [−0.137, 0.139] — a TSDF band, not a global distance field. Render
  supervision cannot distinguish hollow from solid from outside, so even solid
  training assets exert no pressure toward solid interiors.
- **The inner wall carries no recoverable signal.** It is a parallel inward offset
  of the outer wall — 4-crossing dense-SDF columns read wall 3 / cavity 18 / wall
  3 voxels in the medians, uniform. chapel_arch's main component encloses 0.0206
  units³ of material across 4.56 units² of surface: a skin. Discarding it is
  correct; there is no solid in the model to extract.

**The consequence that matters, and it inverts the problem.** The raw extraction
is **already watertight**: chapel_arch's largest component is 99.96% of the mesh,
**zero boundary edges, genus 86** — inner and outer wall are *one closed surface*
welded through 86 handle tunnels. crucero likewise (genus 5, 99.95%). So
connectivity cannot separate the inner wall (everything outside the main
component is **308 faces** on chapel_arch, against the 259,061 `strip_interior_faces`
deletes), and the non-watertightness, the boundary edges and the fragmentation are
**all manufactured by `strip_interior_faces` itself**: cutting a face subset out of
a closed genus-86 surface necessarily leaves open rims and islands. The pipeline
takes a watertight mesh and shreds it. The open question is therefore not "how do
we solidify" but "should the inner wall be deleted by per-face ray voting at all".

**This constrains rework 1 step 8 before its data arrives.** Step 8 was to flip
the geometry-health stats into fail-loud gates, `is_watertight` among them. But
watertightness on the *cleaned* mesh is not a generation-health signal — it is a
statement about our own cleanup, and it is false by construction for every prop
(all seven measured non-watertight, `boundary_edges_per_face` 0.0899 crucero to
1.1944 olive_stump) because the raw surface is closed and we cut faces out of it.
Gating it would gate our own design decision. The signal is real one stage
earlier: the *raw extraction* is watertight, and a raw mesh that came back open
would be genuine generation failure. So step 8's watertight gate moves to the raw
mesh or is dropped; it cannot stay where it is. This holds whichever strip
survives, and is only escapable by closing the rims the strip opens — an approach
nobody has measured and which carries its own free parameter.

**Measured 2026-07-29 — both offered options are refuted, and the real defect is
a third thing.** The discriminator replayed `strip_interior_faces` read-only
(deleted counts reproduce bit-exact: chapel_arch 263,759, crucero 123,308) and
tested the deleted set against the bake rig read out of `prop_texture.py` itself.

*Bias is small.* Of what the 64-ray test deletes, only **0.574%** (chapel_arch,
1,513 faces) and **0.224%** (crucero, 276) is seen by any registry bake camera —
0.20% and 0.08% of the whole mesh. There is no large camera-visible population
being destroyed. It is not pure noise either: chapel_arch's 1,513 form **115
contiguous patches** (against 1,469 clusters / 1,427 singletons for a random
control of the same size), median facing cosine 0.89, the largest a ~50 cm patch
of squarely-visible wall at springing height. A small *structured* leak where the
concavity is deepest.

*The camera-visibility replacement is refuted outright.* The declared bake set is
four azimuths at a **single +15° elevation**. A face facing none of them is
deleted before occlusion is even considered — **17,500 chapel_arch faces and
32,491 crucero faces, 100% of them down-facing**. The arch soffit is in that set.
Against the literal camera set the replacement would delete **123,398** faces
(24.2%) to rescue 1,513 — 81:1 against — and is the one criterion guaranteed to
destroy every downward surface on the model. Only the runtime-picked
`MV_EXTRA_CANDIDATE_ELEVATIONS` grid covers the sphere, and a set picked per
asset at runtime is not a fixed criterion at all. **Do not build this.**

*The actual defect: `INTERIOR_RAY_COUNT = 64` is an unconverged knob.* Full
re-runs at 64 / 256 / 1024 rays:

| rays | chapel_arch deleted | of raw | crucero deleted | of raw |
|---|---|---|---|---|
| 64 | 263,759 | 34.1% | 123,308 | 36.1% |
| 256 | 198,982 | 25.7% | 117,837 | 34.5% |
| 1024 | 128,284 | **16.6%** | 105,999 | 31.0% |

chapel_arch **halves** and the per-4×-refinement drop is not decaying (−8.4 pp
then −9.1 pp of raw); crucero is still falling and *accelerating* (−1.6, −3.5).
This is the 2026-07-29 SDF direction-count sweep again, one refinement generation
slower. `interior_tris_removed` is a knob reading, not a measurement — which
matters directly, because rework 13's declared success metric was built on its
0.3409 baseline. Caveat raised by the measurer and worth keeping: `_hemisphere_dirs`
draws `z` then `phi` from one seed-0 stream, so the 256-direction set is **not** a
superset of the 64-direction set; unlike the SDF sweep's nested sets, monotonicity
here is only statistically expected, and some row-to-row movement is draw luck.

*Why it cannot converge, and this is the root cause.* The raw surface is closed
and genus 86. Its inner wall is connected to the outside through 86 handle
tunnels, so inner-wall faces genuinely **are** reachable by straight rays from
outside — just rarely. More rays find more tunnels and keep more inner wall, so
the limit of "escapes to infinity" is not "outer wall", it is "nearly everything".
64 lands near the true inner-wall share (~1/3) by luck, not by principle. **No
ray-count refinement of an escape-to-infinity test can separate the walls**,
because on this topology the property it tests is not the property we want.

*Also measured:* `bpy.ops.mesh.select_interior_faces()` returns **0 faces** on
both subjects. The topological pre-filter contributes nothing; every deletion
comes from the ray test alone. Dead code.

*Two refinements from the fuller pass.* First, the discriminator replayed the
**pre-weld** pipeline (it started before `c9c695b`), so its 263,759 is the old
ordering's deleted count; with the weld in front the same prop deletes 259,061,
because 16,021 duplicate-corner triangles no longer reach the strip. All
fractions shift by well under 1% and no conclusion moves. Second, and against the
stated suspicion: down-facing faces (`n_z < −0.5`) are **under**-represented in
the visible-deleted set — 7.6% of it against 16.3% of the deleted set overall.
They have to be, since a camera at +15° elevation can never face a downward
normal. **The current test is not punching holes in the soffit.** The soffit only
appears as a casualty of the *replacement*, which is where it would be fatal.

*Where this leaves the design.* Deleting the inner wall is still correct in
principle — finding 15 measured `blend_coverage` 0.7303 → 0.9759 on crucero from
exactly this deletion, so keeping it and paying the tri and atlas budget is the
worse option. But the criterion must be one that converges.

**Decided 2026-07-29 and in flight: delete only what no view the baker is allowed
to pick could see.** Taken autonomously under the user's standing "best outcome"
instruction; reversible, and neither scope nor licensing.

The criterion is the *candidate* view set, not the picked one: the per-asset
azimuths at `MV_ELEVATION_DEG` (`registry.py:19`, `views.py:20`) union
`MV_EXTRA_CANDIDATE_AZIMUTHS × MV_EXTRA_CANDIDATE_ELEVATIONS` plus the 75° top
(`coverage.py:51-52`). `prop_texture.py` picks its actual bake views at runtime as
a **subset** of that set, so deleting only what *no* candidate view sees can never
delete a face the bake would have textured. That is what makes it sound rather
than approximate, and it sidesteps the ordering problem entirely — the strip does
not need to know which views were picked, only which ones were available.

It also has no free parameter. The candidate set is the baker's own fixed
enumeration, not a sample of a continuum, so there is nothing to refine and no
convergence question to answer. That is the property both rejected options
lacked.

Measured support, from the discriminator's grid columns rather than assumed: the
orientation floor — faces deleted purely for facing no candidate view — is **0**
on both subjects, because the grid carries −35° elevation precisely for
down-facing texels. The soffit is safe, which is exactly what killed the
four-camera variant. Against the grid, currently-deleted-but-visible falls to
9,215 (3.5%) on chapel_arch and 772 (0.63%) on crucero, so the 1,513-face
structured leak is closed; and 41,609 currently-kept faces (8.2%) that no
candidate view can reach are reclaimed. Expected new deleted count ~296k (38%)
against the old 259k — recorded as a cross-check, **not** as a target to hit.

**Landed `323c55c`, verified on all seven props.** chapel_arch measured 290,652
(37.6% of raw) against the ~296k cross-check; nothing was adjusted to close the
gap. The orientation floor is **0 faces on all seven**, confirming the soundness
premise the design rests on.

| prop | components | boundary edges / main face | interior tris removed |
|---|---|---|---|
| chapel_arch | 151 → **109** | 1.0378 → **0.3977** | 259,061 → 290,652 |
| olive_stump | 85 → **38** | 1.1944 → **0.4134** | 245,926 → 374,574 |
| gravestone | 31 → **12** | 0.1264 → **0.0772** | 306,953 → 311,199 |
| crucero | 21 → **8** | 0.0899 → **0.0495** | 122,174 → 123,699 |
| broken_column | 14 → **7** | 0.2605 → **0.2041** | 412,934 → 417,019 |
| candelabra_shrine | 6 → 6 | 0.1136 → 0.1101 | 86,621 → 88,852 |
| cypress | 3 → 3 | 0.2149 → **0.3016** | 27,103 → 31,405 |

cypress is the one regression — its boundary edges per face rise 40% — and it is
the prop whose deleted share is smallest (10.5% of raw). Not chased; recorded.
olive_stump's deleted count jumps 52%, the largest move on the board: a gnarled
stump has deep bark crevices no orthographic view can reach, and the strip's own
premise is that what the baker cannot reach is not worth carrying. That premise
is now load-bearing on a prop where it was not before, and is worth a look when
olive_stump is next reviewed in engine.

Implementation notes worth carrying: the worker found the facing condition in
this note's own framing (`n·(−d) > 0`) sign-ambiguous against
`atlas.view_weight`, and resolved it by reusing `view_weight`'s literal
expression on `mv_camera_rig`'s own `v["f"]` rather than re-deriving the formula
— the right call, and the reason the two stages cannot drift. `prop_cleanup.py`
gains a **required `--asset`**; `gen_prop.py` and `gen_character.py` thread it
through, and a `kind="downloaded"` asset is refused outright since it declares no
azimuths.

**Rework 1 closed 2026-07-29 (steps 7 and 8, commits `feacbb0`, `cbecd72`).**
Step 8 is refuted at its premise and step 7 changed shape. The measurement that
settled both also deleted a pipeline stage.

*The weld was doing nothing, and was doing harm.* Finding 17 landed two changes
together — welding coincident vertices, and moving the fragment cull after the
strip — and credited the pair with chapel_arch's 3,824 → 1,012 component drop.
Re-run as a clean A/B across all seven props with the cull already in its correct
place, the weld's contribution is **zero**: raw component counts are identical on
6 of 7 (cypress 233 vs 235) and final component counts are identical on all
seven. What it does contribute is damage. On chapel_arch's main island it
manufactures 36 boundary edges and 27 non-manifold edges out of a mesh that
arrives with 4 and 0; final boundary edges per face halve without it, 0.2297 →
0.1471, and every prop improves. It also collapsed 2–4% of the triangles the
budget is spent on (olive_stump 40,329). Its stated premise — that Hi3DGen
exports duplicate corners so neighbouring faces share no edge — is false: the
glTF arrives already sharing vertices, which is why the eps=0 arm of the sweep
below is the *best* arm rather than the degenerate one. Deleted, −53 lines.

Weld-epsilon sweep (chapel_arch main island; epsilon as a fraction of the bbox
diagonal). The knob had never been swept:

| eps | boundary edges | non-manifold edges | tris collapsed |
|---|---|---|---|
| 0 | 4 | 0 | 0 |
| 2.5e-5 | 6 | 1 | 4,099 |
| 5e-5 | 25 | 14 | 8,179 |
| 1e-4 (shipped) | 40 | 27 | 16,021 |
| 2e-4 | 75 | 62 | 31,910 |
| 4e-4 | 172 | 150 | 64,546 |
| 8e-4 | 370 | 424 | 129,826 |

Monotone in the wrong direction at every refinement.

*`geometry_health` was reporting a mixed quantity.* It counted boundary edges
over the whole mesh and divided by the **main island's** face count, so a debris
speck's rim read as a hole in the prop — which is why `raw_boundary_edges_per_face`
first came back 0.0 on broken_column while the mesh carried 45 boundary edges.
Main-island quantities are now computed on that island alone, `main_euler_number`
with them (where it states a genus rather than a mixture), and the same stats are
read a second time on the mesh as imported — the only point that still describes
the network's output instead of our own cutting. `is_watertight` is gone as a
field: it restated `boundary_edge_count == 0`.

*Step 8's gate is not writable, and the reason is not missing data.* The gate was
specified to catch "a hollow-shell regression". Lead (1) established that the
hollow shell is architectural and permanent — every prop is one, always — so
there is no regression for it to catch. The fallback of gating raw watertightness
fails on its own measurement: the raw main island is closed on only 2 of 7 props
(0, 0, 4, 12, 42, 68, 366 boundary edges), and its face fraction runs 0.658
(candelabra_shrine, whose arms are genuinely separate bodies) to 1.0. Calibrating
a fail-loud threshold across that spread from seven passing samples and zero
failures is the guessed band `~/.claude/CLAUDE.md:14` forbids. **No threshold is
installed.** What would make one writable is a corpus of failed generations to
put a floor under; until that exists the corrected stats are the deliverable and
they gate nothing.

Post-weld-deletion baseline, all seven props
(`target/prop-solid-validation/r1s78/`):

| prop | raw main boundary edges | raw main face fraction | components | boundary edges / main face |
|---|---|---|---|---|
| broken_column | 0 | 0.9908 | 7 | 0.0784 |
| candelabra_shrine | 0 | 0.6581 | 6 | 0.0395 |
| chapel_arch | 4 | 0.9996 | 109 | 0.1471 |
| crucero | 12 | 0.9995 | 8 | 0.0135 |
| cypress | 366 | 0.9114 | 3 | 0.2839 |
| gravestone | 68 | 1.0 | 12 | 0.0249 |
| olive_stump | 42 | 0.9951 | 38 | 0.2079 |

Every final-mesh figure improves against the pre-deletion table recorded above.

*Step 7 found a live defect and changed the constant's shape.* Clean→hires
deviation, measured at 20k/80k/320k surface samples per prop (p99 stable to 3e-4
across that refinement, so the sample count is not deciding the answer):
`BAKE_MAX_RAY_DISTANCE_M = 0.03` **clips on cypress**, which needs 0.0454 at p99
and 0.0582 at p99.9 — roughly 1% of its normal-bake texels have been falling back
to flat. candelabra_shrine needs 0.0111. A flat bound raised to cover cypress
hands the smallest prop five times the reach it needs, and the spread is not
noise: deviation tracks prop size because the triangle budget does not — every
prop decimates to the same 15,000. Replaced by `BAKE_RAY_DIAG_FRACTION = 0.006`
of the prop's own bbox diagonal, added to the cage extrusion at the call site;
every prop clears its own p99.9 by ≥1.6×. Overshoot is safe by construction —
Cycles takes the first hit, so extra ray length can only turn a miss into a hit,
never corrupt one.

| prop | bbox diag | p99 | p99.9 | needed (p99.9 + cage) | supplied |
|---|---|---|---|---|---|
| candelabra_shrine | 1.853 | 0.00068 | 0.00105 | 0.0111 | 0.0211 |
| gravestone | 1.906 | 0.00184 | 0.00264 | 0.0126 | 0.0214 |
| olive_stump | 2.047 | 0.00400 | 0.00553 | 0.0155 | 0.0223 |
| broken_column | 2.317 | 0.00211 | 0.00324 | 0.0132 | 0.0239 |
| crucero | 4.247 | 0.00431 | 0.00629 | 0.0163 | 0.0355 |
| chapel_arch | 7.877 | 0.01188 | 0.01884 | 0.0288 | 0.0573 |
| cypress | 13.414 | 0.03541 | 0.04822 | 0.0582 | 0.0905 |

**Decided autonomously under the standing "best outcome" instruction, and worth
the user's eye:** the approved plan's step 7 said to keep a single metre constant
and scale it by 1.5 if the measurement demanded it. Measurement made that *form*
indefensible, not just that value, so the constant became size-relative — the
idiom `prop_cleanup.py` already uses for its own tolerances. Reversible; neither
scope nor licensing.

The deviation spread is itself a symptom: the flat `--tri-budget 15000` is what
makes a 12 m cypress and a 1.2 m stump deviate by 52×. The per-asset triangle
budget already queued as a quality finding is the root fix, and this constant
should be re-derived once it lands. Deriving it now was still correct — cypress
is clipping today.

Artifacts: `target/prop-solid-validation/r1s78/` (per-prop `cleanup.json`,
`bake_ray_derivation.json`) and `r1s78-noweld/` (the A/B arm).

Done 2026-07-29 (finding 24, fork `c7389f5`, vordar `1d5c681`). A `perf_counter`
around the extractor call, surfaced as `SparseFeatures2Mesh.last_extract_s` and
recorded as `elapsed_s.extraction` in the per-candidate manifest — a
**sub-interval** of `elapsed_s.geometry`, not a sibling, so the two must never be
summed. Confirmed populated on a CPU replay; the number that decides rework 5's
gate is the extraction share under the normal GPU path, which the next routine
candidate run reports. Rework 5 stays parked until then.

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
- **Part (b) delivered 2026-07-29** (`a40dad8`, `docs/reviews/hi3dgen/noise-floor-2026-07-29.md`): 3 repeats × 3 subjects, per-metric floor with the three raw values, deviation floor at a 20k/80k/320k stability triple. Vertex-count floor 0.0089–0.0291%, with this finding's own 0.012% inside that range. Part (a) survives but its target is now localized: the three `ss_logits.npy` are **byte-identical per subject** (sha256 verified), so the sparse-structure stage is already bit-reproducible at a fixed seed and every measured spread enters downstream of it, in the SLat sampler and the extractor. A determinism attempt therefore only has to cover `scatter_reduce` and the SLat convs, not the whole chain. Also measured: `body_count`, `boundary_edge_count` and `main_euler_number` are **unresolved at this noise** — identical runs moved 9→11 bodies and 12→16→8 boundary edges — so no A/B may claim a topology-count effect below those spreads.

**Part (a) measured 2026-07-29** (`docs/reviews/hi3dgen/ab-multiview-2026-07-29.md`,
`target/mv-ab/noise_floor.json` `determinism_probe`): `torch.use_deterministic_algorithms(True)`
is only partially effective. Three `--deterministic --seed 0` runs on
chapel_arch — `det-r1` and `det-r2` byte-identical `raw.glb` (`fa35dc9748...`),
`det-r3` differs (`a5d846e91e...`). 2 of 3 byte-identical; spconv sparse
convolutions run outside `torch.use_deterministic_algorithms`, so the seed is
still not a full pin. All 18 multi-view A/B candidates ran without
`--deterministic` as a result, sharing one non-deterministic regime with the
noise floor that adjudicates them.

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
- **Evidence:** Measured while running audit finding 13's A/B. The load site — `fork:hi3dgen/headless.py:210` since rework 3 moved the model lifecycle out of `prop_hi3dgen.py` — calls `torch.hub.load(<local snapshot>, …, source="local", pretrained=True, …)`, but neither `hub:hubconf.py` entrypoint (`StableNormal`, `StableNormal_turbo`) accepts a `pretrained` argument — the call raises `TypeError: StableNormal_turbo() got an unexpected keyword argument 'pretrained'` on every invocation, is swallowed by the bare `except Exception`, and the fallback `torch.hub.load("hugoycj/StableNormal", …, trust_repo=True)` runs instead. Reproduced directly: dropping `pretrained=True` makes the local branch load successfully.
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

### ~~16. `min_component_fraction`'s denominator collapsed with the interior fill, so `1e-4` drops fewer floaters than it was calibrated to~~

- **Evidence:** Measured 2026-07-29 while deleting the interior-fill mechanism (successor to finding 13; the direction-count sweep in `plan-rework13-winding-solidification-2026-07-28.md` killed the fill, and `solidify_hidden_interior` is now gone from the fork). `drop_solid_floaters` (`fork:hi3dgen/representations/mesh/utils_cube.py`) thresholds every solid component at `min_fraction * total_solid_voxels`, and that total used to include the filled interior. With the fill gone it is the predicted shell alone. On the harness's r=30 sphere the total solid voxel count is **18,640** and the detached 8-voxel blob is **4.29e-4** of it, the 369-voxel rod **1.94e-2**. Directly measured consequence: `case_floater_blob_dropped`, green at `1e-4` for the entire life of the fill, came back `body_count=3` (blob surviving) on the first post-deletion run and only returns green at a fraction above 4.29e-4 — it now runs at `FLOATER_FRACTION = 3e-3` in `fork:tests/test_extraction_contract.py`, the geometric mean of the two fixtures' shares. On real props the same denominator shrink is the fill's measured volume inflation, 1.671x (candelabra_shrine) to 3.214x (chapel_arch) per that plan's step-3 table.
- **Ideal:** `min_component_fraction`'s shipped default is calibrated against the solid voxel count the extractor actually produces, so the floater sizes it deletes on real props are a stated absolute range rather than an accident of what the interior fill used to add to the denominator.
- **Gap:** `1e-4` was chosen when the denominator carried a filled interior. Nothing re-derived it when the fill was deleted, so the absolute size of a dropped component silently fell by the per-prop fill ratio (1.7-3.2x) — the mechanism still runs, still gates on `min_component_fraction > 0`, and still drops *something*, which is exactly why nothing catches the shift.
- **Suggestion:** Measure the per-component solid voxel counts on the three saved latents under `target/prop-latents/<name>/` via `scripts/ai-pipeline/prop_extract.py`'s CPU replay (no GPU), then pick the default from the gap between real debris and real geometry rather than from the synthetic fixtures. Do not move the harness's `FLOATER_FRACTION` to match whatever is chosen: it is a bar placed to straddle two fixtures, not a copy of the production value, and coupling them would hide the next such drift. `plan-rework13-...` already listed this recalibration as a recorded candidate (`fragments_removed` 11 / 5 / 9 surviving into cleanup) and deferred it as out of scope.
- **Outcome:** `4/10` — restores a calibrated floater drop; it is the only surviving grid-space cleanup after the fill's deletion, so its threshold is now load-bearing on its own.
- **Cost:** `2/10` — three CPU extraction replays plus a one-line default; no GPU, no Blender.
- **Path:** replay the three latents recording per-component solid voxel counts → identify the debris/geometry gap → set the default in `fork:hi3dgen/representations/mesh/cube2mesh.py` (and `decoder_mesh.py`'s `rep_config` fallback) → re-run `fork:tests/test_extraction_contract.py` (3/3, unchanged: its fraction is passed explicitly) → re-run the three replays and record `body_count`.

### 17. `prop_cleanup.py` measured island topology before welding, and culled fragments before the stage that creates them
- **Evidence:** `scripts/ai-pipeline/prop_cleanup.py` ran the loose-fragment cull immediately after import, ahead of `strip_interior_faces`; `geometry_health` flood-filled vertex connectivity on an unwelded mesh. Hi3DGen exports duplicate vertices at shared corners, so adjacent faces share no edge. chapel_arch: raw extraction reports 16 bodies, `cleanup_hollow.json` reports 3,824 components.
- **Ideal:** one cull, placed where fragments exist; every topology measure taken on a mesh whose coincident vertices have been merged.
- **Gap:** the cull caught 14 islands / 204 tris out of 773,566 where it sat. The 3,824 count was ~2/3 vertex bookkeeping and ~1/3 real marooned islands, and the tri budget was spending 21% of itself describing dust that decimation then preserved.
- **Suggestion:** weld at a sub-voxel epsilon first, then strip, then cull, then normalize.
- **Path:** DONE `c9c695b`. `weld_vertices` at `WELD_EPS_FRACTION = 1e-4` of the mesh bbox diagonal (0.79 mm at chapel_arch's 7.881 m, ~1/20 of the 512³ extraction voxel, so it can only merge what the generator emitted twice at one position). `cull_loose_fragments` reimplemented in bmesh over the flood fill `geometry_health` already had, rather than moving the `bpy.ops.mesh.separate(type="LOOSE")` block — separating 3,575 islands into Blender objects and rejoining them is that operator's pathological case. Measured on chapel_arch at `--height 5.497`: components 3,770 → **151**, face retention across the cull **98.0%**, stats reconcile exactly (773,574 − 16,021 weld − 259,061 strip − 9,563 cull = 488,929 hires).
- **premise-falsified:** the predicted landing point was 46 components; the real one is 151, the residual islands all exceeding the cull's own threshold. No threshold was moved to reach the prediction. `uv_charts` moved only 4,519 → 4,096 — xatlas segments on curvature, not on island count — so the chart-count half of the expected win does not exist.
- **Half of this finding was refuted 2026-07-29 (`feacbb0`).** The fix landed two
  changes in one commit and credited the pair. Isolated as an A/B across all
  seven props — cull already in its correct place, weld on vs off — the weld's
  contribution is zero: identical raw component counts on 6 of 7 props and
  identical final counts on all 7. The reordering was doing all the work. The
  weld's stated premise is also false; the glTF arrives already sharing
  vertices, so `remove_doubles` was merging genuinely distinct geometry, which
  is why it manufactured 27 non-manifold edges and 36 boundary edges on
  chapel_arch's main island out of a mesh that arrives with 0 and 4, and
  collapsed 2–4% of the triangles. `weld_vertices` and `WELD_EPS_FRACTION` are
  deleted. Numbers in the Path bullet above that include the weld (the 16,021
  term, the 151 components, the 98.0% retention) are superseded by the table in
  this file's queue note.

### ~~18. Per-asset triangle budget (user-decides): the flat 15,000 over-serves small props and starves large ones~~
- **Evidence:** `scripts/ai-pipeline/prop_cleanup.py`'s `--tri-budget` defaults to a flat 15,000 for every prop, and `gen_prop.py` never overrides it. Measured through the pipeline itself on all seven props at 5k/15k/30k/60k/120k (35 runs, `target/prop-solid-validation/tribudget/`, p99 clean→hires deviation at 80k surface samples per run): at the shipped 15,000 the deviation normalized by bbox diagonal spans **0.000369 (candelabra_shrine) to 0.002633 (cypress), a 7.1× range**. The same measurement is what forced `BAKE_RAY_DIAG_FRACTION` to become size-relative — this is that defect's root cause rather than its symptom.
- **Ideal:** Every prop is decimated to the budget its own geometry needs to hold a chosen deviation, so the triangle budget buys the same visual fidelity everywhere instead of an accident of prop size.
- **Gap:** The budget is uniform and the quality is not. candelabra_shrine currently gets 4× more fidelity than it needs while cypress gets less than half; nobody chose that split.
- **Suggestion:** Per-asset `tri_budget` in `content/models/assets.json` (alongside `height_m`, same `_GENERATED_FIELDS` treatment audit finding 20 describes), threaded through `gen_prop.py`. **Do not derive it from a formula.** Measured budget needed for a uniform deviation, against prop size:

| prop | bbox diag | hires area | needed @0.0015 | tris/m² implied |
|---|---|---|---|---|
| candelabra_shrine | 1.85 m | 2.3 m² | 4,993 | 2,140 |
| gravestone | 1.91 m | 5.0 m² | 7,837 | 1,577 |
| olive_stump | 2.04 m | 6.5 m² | 18,383 | 2,832 |
| broken_column | 2.32 m | 7.1 m² | 11,170 | 1,574 |
| crucero | 4.25 m | 15.5 m² | 9,531 | 615 |
| chapel_arch | 7.88 m | 82.5 m² | 13,756 | 167 |
| cypress | 13.44 m | 202.9 m² | 24,182 | 1,467 |

  Neither diagonal nor surface area predicts it: olive_stump at 2.04 m needs
  nearly twice crucero at 4.25 m, and the implied triangle density spans 17×.
  The driver is geometric complexity — gnarled bark against a smooth cross —
  which no size formula carries. The budget is therefore a per-asset
  measurement, not a computed field.
- **THE USER'S DECISION — the deviation target.** It is a free parameter and a visual one, so it is not the implementer's to pick. Totals across the seven props, against today's 105,000:

| target p99/diag | total tris | vs today | what changes |
|---|---|---|---|
| 0.0020 | 66,322 | −37% | cypress and olive_stump improve; candelabra_shrine drops 15,000 → 4,051 |
| 0.0015 | 89,852 | −14% | every prop at or better than today's worst; candelabra_shrine → 4,993 |
| 0.0010 | 138,350 | +32% | every prop at or better than today's best except candelabra_shrine |
| 0.0005 | 292,235 | +178% | diminishing — candelabra_shrine already measures 0.000014 at 120k |

  Recommendation: **0.0015**. It is the only row that is both cheaper than today
  in total and no worse than today on any prop, because the flat budget's waste
  on the small props pays for the large ones. The reason it still needs the
  user's eye is that it cuts candelabra_shrine to a third of its triangles on the
  strength of a distance metric, and whether that reads at gameplay framing is
  not a headless judgement.
- **Outcome:** `6/10` — uniform fidelity per triangle spent, and it removes the size dependence that `BAKE_RAY_DIAG_FRACTION` now works around.
- **Cost:** `2/10` — the measurement is already done and kept; the change is a registry field plus threading. Re-deriving `BAKE_RAY_DIAG_FRACTION` afterwards is part of it.
- **Path:** user picks the target → write the per-asset budgets from the measured curve (never a formula) → thread `tri_budget` through `gen_prop.py` → re-run the seven props → re-derive `BAKE_RAY_DIAG_FRACTION` against the new deviation spread → in-engine look at candelabra_shrine and olive_stump before the budgets are considered settled.

### 19. `plan-rework4-knob-sweep-2026-07-28.md` step 1's pre-measured `--iso-level 0.03` assertions don't reproduce
- **Evidence:** Step 1's Path pre-registered two CPU replays of `target/prop-latents/candelabra_shrine` on the production chain. The defaults replay reproduced exactly: `vertex_count 167479, face_count 334938, boundary_edge_count 0, component_count 11, main_face_fraction 0.6581, main_euler_number -8, sdf_bias -0.00390625` — all eight fields bit-for-bit, including the 0.6581 cross-instrument figure against `geometry_health`'s recorded `raw_main_face_fraction`. The `--iso-level 0.03` replay did not: measured `vertex_count 314625, face_count 629870, boundary_edge_count 1896, component_count 252, main_euler_number -1705` against the plan's registered `299364 / 600368 / 938 / 307 / -1869`. Reran twice with identical output both times (`elapsed_s` differed, every topology field did not), so the mismatch is not run-to-run nondeterminism.
- **Ideal:** The plan's pre-registered assertions for every knob setting reproduce exactly against the fork as it stands today, the same way the defaults case does.
- **Gap:** Only the default-parameter path was cross-validated live; the `--iso-level 0.03` numbers were carried from an earlier measurement (possibly a different fork revision, a different `EnhancedMarchingCubes`/`measure.marching_cubes` version, or a different `candelabra_shrine` latent) and no longer match the code this plan's step 1 lands against.
- **Suggestion:** Re-measure the `--iso-level 0.03` (and any other non-default) assertions fresh against current `target/prop-latents/candelabra_shrine` and the current fork checkout before later knob-sweep steps depend on them as ground truth.
- **Outcome:** `4/10` — the sweep driver itself is unaffected (defaults path, which is what proves `topo_stats`/`geometry_health` agreement, is exact), but any later step that trusts the 0.03 numbers as a regression baseline would be trusting stale figures.
- **Cost:** `1/10` — one more CPU replay (~15s) plus updating the plan's registered numbers.
- **Path:** rerun `prop_extract.py --iso-level 0.03` against `target/prop-latents/candelabra_shrine` on the current fork → record the fresh vertex/face/boundary/component/euler numbers → update `plan-rework4-knob-sweep-2026-07-28.md` step 1 (or whichever later step reads them) to the fresh values before relying on them as a gate.

### 20. The sparse-structure stage is not bit-reproducible across a *changed call pattern*, so the duplicated-view identity band is unmeasured
- **Evidence:** `plan-rework2-multiview-conditioning-2026-07-28.md` finding 2 step 3's identity smoke was run as specified. P1 and P2 both hold exactly: `smoke-sv`'s `normal_sha256 == 822b22e5c2529af6e601ceffc813a1120b73d90cd00265ed6b8d7e7965e98f8f`, `face_count 768730` (0.0096% from the reference 768,804), every reference manifest key present, `elapsed_s` carrying the seven reference keys plus `extraction 0.731 < geometry 15.46`, extra keys exactly `{ss_active_voxels, views, mv_mode}`, `mv_mode: null`, one-entry `views`. `smoke-dup` (the same image passed twice, `--mv-mode multidiffusion`) is structurally correct too: `mv_mode "multidiffusion"`, two `views` with equal `input_sha256`, `normal_v1.png` present with sha256 equal to `normal.png`'s, identical `sampler_rng_state_sha256`. The two behavioral predicates fail: `ss_active_voxels` is **14588 (sv) vs 14591 (dup)**, not identical, and `vertex_count` is **384222 vs 383197 — 0.267%** apart against a 0.1% band. The algebra is identical by inspection: with two equal conditioning rows `sum(preds)/len(preds)` is exact in IEEE, and `inject_sampler_multi_image`'s multidiffusion branch applies the same `(1 + cfg) * pred - cfg * neg_pred` as `guidance_interval_mixin.py:33-39`. `fork:hi3dgen/headless.py:16-19` sets `CUBLAS_WORKSPACE_CONFIG` but nothing in the package ever calls `torch.use_deterministic_algorithms`, so nondeterministic kernels are unconstrained.
- **Ideal:** A duplicated-view multidiffusion run and its single-view twin agree at the sparse-structure stage exactly (that stage's noise and algebra are identical), and their meshes agree inside the same-seed floor rework 6 measured (0.0089–0.0291% on vertex count).
- **Gap:** Rework 6's "the sparse-structure stage *is* bit-reproducible" was measured between two runs of the *same* call pattern. Multidiffusion makes three model calls per step where the bare guidance-interval sampler makes two, which changes allocator and kernel-selection state; with deterministic algorithms never enabled, that is enough to move three voxels out of 14,588 (0.02%), and 50 sparse-structure steps of divergence then land 0.267% apart on vertex count. So the 0.1% band was derived from a same-call-pattern floor and does not describe a cross-call-pattern comparison — the identity check has no valid threshold today, not a failing one.
- **Suggestion:** Establish the cross-call-pattern floor before any multi-view A/B trusts a mesh-count comparison: either enable `torch.use_deterministic_algorithms(True)` in the fork's headless entry (the `CUBLAS_WORKSPACE_CONFIG` line already anticipates it) and re-run the pair to see whether `ss_active_voxels` then matches exactly, or accept nondeterminism and measure the floor directly by running the duplicated-view case three times and reporting its own spread. Do not widen the 0.1% band by fiat; replace it with a measured number, and prefer comparing `ss_active_voxels` distributions over vertex counts, which amplify upstream drift by an order of magnitude.
- **Outcome:** `5/10` — the multi-view path itself is landed and structurally verified; what is missing is the yardstick that would let a real conditioning change be told apart from kernel noise, which every later multi-view arm depends on.
- **Cost:** `2/10` — one flag plus a re-run of the ~3 min smoke pair, or three ~1 min candidates for the empirical floor.
- **Executed by** `plan-rework2-multiview-conditioning-2026-07-28.md` step 4, which already rosters the same flag and the same probe — do not schedule this separately. That step measures a *same*-call-pattern floor, so if its determinism outcome is "three identical `raw.glb` hashes" this finding dissolves; if the flag proves insufficient, the cross-call-pattern gap below survives its floor and still needs the duplicated-view repeats.
- **Path:** add `torch.use_deterministic_algorithms(True)` behind the existing env guard in `fork:hi3dgen/headless.py` → re-run the `smoke-sv` / `smoke-dup` pair → if `ss_active_voxels` matches exactly, keep the flag and re-assert the identity smoke at the rework-6 floor; if it does not, revert the flag, run the duplicated-view case three times, and record the measured spread as the band `plan-rework2-multiview-conditioning-2026-07-28.md` finding 2 step 3 should have used.

### 21. `mv_ab_metrics.py`'s planned single `cv2.fillPoly` call over all faces cancels a closed mesh's silhouette instead of union-filling it
- **Evidence:** `plan-rework2-multiview-conditioning-2026-07-28.md` finding 3's Suggestion specified rasterizing every face with one `cv2.fillPoly(canvas, polys, 255)` call. Implementing it exactly and running the finding's own analytic test (`trimesh.creation.box(extents=[1, 2, 3])` at az=0, el=0) gave a fill fraction of 0.035 inside the bbox, not the expected ≥0.999. Isolated repro in the Hi3DGen venv (`cv2` 4.11.0): a single triangle alone fills its bbox correctly (25806 px for a 130x390 half-box rectangle); two *exactly* overlapping triangles passed to one `cv2.fillPoly` call together fill only 909 px, regardless of matching or opposite vertex winding. `cv2.fillPoly`'s multi-contour fill is an edge-parity (even-odd-style) algorithm, not a union: overlapping contours in the same call cancel. For any closed/watertight manifold, a straight line through the interior along the view axis crosses the surface an even number of times (front face + back face, at minimum) at every interior silhouette point, so this cancellation is not specific to the axis-aligned test box — it is the generic case for any closed mesh rasterized this way, at any azimuth (confirmed off-axis: az=5/el=0 and az=1/el=1 on the same box both still returned only ~3000-3700 px instead of a filled silhouette).
- **Ideal:** The instrument's rasterizer computes the true union of all face projections regardless of how many surfaces overlap at a pixel, so it works for both closed test primitives and open/hollow raw Hi3DGen output.
- **Gap:** No single-call `cv2.fillPoly` invocation over a full face list has union semantics; the finding's Suggestion assumed one did.
- **Suggestion:** Already applied in the landed `scripts/ai-pipeline/mv_ab_metrics.py`: paint each face with its own `cv2.fillConvexPoly` call in a loop, so every face independently ORs 255 into the canvas (idempotent, order-independent, no cancellation). Benchmarked at ~3.8s per 512x512 view on the 768,804-face `chapel_arch_e2e/cand_0/raw.glb` fixture — acceptable for this instrument's occasional-use A/B role, but worth revisiting (e.g. bounding-box-limited rasterization, or a vectorized scan-conversion) if it is ever driven at a tighter loop cadence (a per-candidate sweep across many props, or a finer azimuth scan step).
- **Outcome:** `8/10` — without this fix the instrument silently reports near-zero silhouette coverage for any closed mesh (including its own analytic self-test), which would have made every later multi-view A/B reading on this yardstick meaningless.
- **Cost:** `0/10` — already implemented and tested as part of landing finding 3; nothing further required unless the per-view runtime becomes a bottleneck for a future sweep.
- **Path:** none outstanding — recorded for provenance; revisit only if a later step's call volume makes the per-face-loop runtime a bottleneck.

### 22. `mv_ab_metrics.py` measured silhouettes in the wrong frame: `trimesh.load` keeps glTF Y-up, but `view_axes` assumes Blender's Z-up
- **Evidence:** `view_axes` mirrors `proptex/views.py`'s `mv_view`, which runs inside Blender and is correct there because Blender's glTF importer converts glTF Y-up to Blender Z-up on load. `mv_ab_metrics.py` instead loads the `.glb` with `trimesh.load`, which keeps the raw glTF Y-up frame, so the mirrored camera math treated the mesh's Z axis (depth) as "up" and looked down its real height axis. Measured on `target/mv-ab/det-nf1/cand_0/raw.glb` against `target/prop-solid-validation/chapel_arch_e2e/cand_0/concept_rgba.png` (extents X 0.9997 / Y 1.0008 / Z 0.2551 — Z is wall thickness, Y is the arch's real height): as-is `fitted_yaw=0, best_iou=0.3076` (a squat wide slab); converting vertices `(x, y, z) -> (x, -z, y)` before rendering gave `fitted_yaw=155, best_iou=0.8807`, matching the concept's pointed gothic arch. The existing analytic test (`trimesh.creation.box(extents=[1,2,3])`, rendered directly with `render_mask` in the same call) is structurally blind to this class: it never crosses the glTF/Blender frame boundary `trimesh.load` introduces, so it validates the camera math against its own convention regardless of which "up" axis is right.
- **Ideal:** `mv_ab_metrics.py`'s silhouettes are computed in the same frame `proptex/views.py`'s Blender-side renders use, so the module's own claim of apples-to-apples comparison with the ControlNet-depth stage is actually true.
- **Gap:** No frame conversion existed between `trimesh.load` and `view_axes`; the module docstring and `view_axes`'s docstring asserted convention parity with `proptex/views.py` that only holds inside Blender.
- **Suggestion:** Already applied. A single `load_mesh(path)` owns the conversion — `trimesh.load` plus `(x, y, z) -> (x, -z, y)`, returning a mesh already in the frame `view_axes` assumes — and is the only load path in the module; `main()` and the tests all go through it, so the string `-v[:, 2]` occurs exactly once in the workspace. No flag is threaded through the render path and no as-is code path is kept. Both docstrings (module header, `view_axes`) state the Z-up assumption and the glTF-Y-up-vs-Blender-Z-up fact. Added `test_gltf_y_up_box_renders_tall_not_wide` to `scripts/ai-pipeline/test_mv_ab_metrics.py`: builds a box tall in glTF Y (`extents=[1, 3, 1]`), exports it to `.glb`, and drives the CLI as a subprocess so the assertion runs against the shipped entry point rather than an in-process helper call, asserting the rendered silhouette's height/width aspect exceeds 2. Reverting the conversion was verified to fail this test (measured aspect 1.02) and to fail the pre-existing `test_yaw_fit_recovers_grid_azimuth` the other direction; restoring it passes all three. Putting the conversion inline in `main()` was the first fix attempted and was rejected: it forced the same two lines into the test's own load site, which is how a convention silently acquires a second copy.
- **Outcome:** `9/10` — every prior `mv_ab_metrics.py` reading (including `target/mv-ab/noise_floor.json`'s `iou_front` floor, now recomputed to 0.8806-0.8807 at `fitted_yaw_deg=155`) was measuring the wrong silhouette; any future multi-view A/B on this instrument would have compared a real render against a spuriously bad geometric baseline.
- **Cost:** `0/10` — already implemented, tested, and the noise floor recomputed; nothing further required.
- **Path:** none outstanding.

### 23. Panel-viewpoint distinctness on a generated concept sheet is not measurable by any cheap geometric instrument — the judgment is semantic
- **Evidence:** A background-subtraction silhouette was tried first for telling whether a Z-Image concept sheet's three panels are genuinely different viewpoints or the same view drawn twice, and was rejected: its threshold is a free parameter, and the sheets with real viewpoint change also carry cast shadows, which subtraction counts as object -- biasing the instrument against exactly the case it must detect. `scripts/ai-pipeline/panel_matte_ab.py` instead runs each panel through `hi3dgen.headless.Session.matte` (the same BiRefNet pass `prop_hi3dgen.py` uses) and builds each panel's silhouette from that matte's own alpha cut, `mv_ab_metrics.ALPHA_THRESHOLD` (`0.8 * 255`, `preprocess_image`'s own bbox test) -- no threshold chosen by this script. Measured over `target/mv-ab/olive_stump/seed{1,2,3}` and `target/mv-ab/pilgrim_monk/seed{1,2,3}` (all 18 panels passed `prop_hi3dgen.check_matte`, no refusals), pairwise IoU between panels' normalized silhouettes: olive_stump seed1 front-side `0.8452`, front-back `0.9761`, side-back `0.8441`; seed2 front-side `0.7449`, front-back `0.9039`, side-back `0.7461`; seed3 front-side `0.8412`, front-back `0.9682`, side-back `0.8499`. pilgrim_monk seed1 front-side `0.6439`, front-back `0.9659`, side-back `0.6431`; seed2 front-side `0.6224`, front-back `0.9537`, side-back `0.6224`; seed3 front-side `0.5960`, front-back `0.9501`, side-back `0.5874`. Front-back IoU is consistently far higher (0.90-0.98) than front-side/side-back (0.59-0.85) across every seed of both subjects.
  That reading does **not** mean the front and back panels are the same view, and the finding's first framing that it did was wrong. `pilgrim_monk` seed1 is a plainly genuine turnaround on inspection -- the front carries a face and a cross pendant, the centre panel is a true profile, the right panel shows the hood from behind with the satchel strap crossing the back -- and it scores front-back `0.9659`. A standing figure's front and back silhouettes are near-identical by anatomy, so a high front-back IoU is what a *correct* turnaround produces. The metric cannot separate that from a duplicated panel.

  A third instrument was then tried and also fails: RGB mean-absolute-difference inside the matte, each panel cropped to its alpha bbox and resized to 256. Front-back separation runs 19-34 against front-side 37-66 for every seed of both subjects -- but `pilgrim_monk` seed1 (a real turnaround) scores front-back `22.72` and `olive_stump` seed1 (front and back visibly near-copies) scores `22.60`. The two cases are indistinguishable at the same number.

  The confound is intrinsic, not a defect of any one instrument: in a genuine turnaround the front and back panels share framing, lighting and — for a bilaterally symmetric subject — outline, differing only in *what the surface depicts*. That is a semantic property. No silhouette or pixel statistic separates "the back of this object" from "the front of this object again".
- **Ideal:** The question a concept sheet's acceptance turns on -- does the third panel depict the object's back -- is routed to a judge that can answer it, rather than to a geometric proxy that cannot.
- **Gap:** Three instruments were built and spent against a question none of them can reach. `plan-rework2-multiview-conditioning-2026-07-28.md` step 5's Path already said so ("the sheets are *images*, so the consistency judgment is visual ... do not self-approve"); the probes re-derived that at cost instead of taking it.
- **Suggestion:** Keep `scripts/ai-pipeline/panel_matte_ab.py` for what it does measure and what step 5's Path item 4 actually asks for -- the matte gate. All 18 panels passed `check_matte` with no refusals, so every panel is matting-ready regardless of which sheets are accepted. Its opaque fractions also carry a real result for the A/B's design: `pilgrim_monk`'s side panels sit at 0.16-0.17 against 0.24 front/back (a profile is markedly narrower), while `olive_stump`'s sit at 0.30-0.33 against 0.31-0.37 -- the stump is close to radially symmetric, so a side view of it is inherently weak new information and the prop subject is the harder case for multi-view conditioning on its merits. Do not add a fourth geometric proxy for panel semantics.
- **Outcome:** `6/10` -- the matte gate and the symmetry reading are worth keeping; the headline claim the instrument was built for is not obtainable this way, and recording that stops the next attempt from re-spending it.
- **Cost:** `0/10` -- implemented and run; nothing further required.
- **Path:** none outstanding.

### 24. Silhouette IoU cannot resolve front from back — an orientation-robust fidelity metric is needed
- **Evidence:** `docs/reviews/hi3dgen/ab-multiview-2026-07-29.md` (rework 2 step 8). `fit_yaw`'s argmax is degenerate: across the 18-candidate multi-view A/B, the gap between IoU at the fitted argmax and IoU at (argmax + 180 deg) runs 0.0014-0.1053, with 7 of 18 candidates below 0.01 — the same order as the cross-arm effects being claimed (0.0037-0.0081). All three `pilgrim_monk` `sv` candidates fit ~180 deg off (confirmed visually: the "front" render shows the monk's back), which inverted step 7's reported "MV beats sv on iou_front" into a front-vs-back comparison. This is the third instrument in this domain to fail on a front/back or panel-viewpoint distinction that turns out to be semantic rather than geometric (finding 23, `panel_matte_ab.py`, made the same discovery for concept-sheet panels: no silhouette or pixel statistic separates "the back of this object" from "the front of this object again").
- **Ideal:** A fidelity metric for a multi-view A/B (or any future orientation-sensitive comparison) resolves which side of a near-symmetric silhouette it is looking at, so cross-arm deltas are not confoundable with which of two tied peaks the yaw fit happened to pick.
- **Gap:** No orientation-robust instrument exists in this pipeline. Silhouette IoU is blind to it by construction — a standing figure's front and back silhouettes are near-identical by anatomy, and an arch or a stump reads similarly from opposite sides too.
- **Suggestion:** Correlate rendered camera-space normals against Hi3DGen's own predicted `normal.png` (already written to every candidate directory) to break the 180-degree tie. The silhouette fit already localizes the peak pair (argmax and argmax+180), so only those two candidates need normal correlation, not a full azimuth sweep — 2 renders per candidate, not a re-run of the whole scan.
- **Outcome:** `6/10` — unblocks any future orientation-sensitive A/B in this pipeline (multi-view conditioning, concept-sheet turnaround checks) that silhouette IoU cannot currently adjudicate.
- **Cost:** `3/10` — two camera-space normal renders per candidate through the existing `proptex.views`/`mv_camera_rig` machinery, plus a correlation function; no new GPU generation.
- **Path:** implement normal-map correlation in `mv_ab_metrics.py` at the argmax and argmax+180 candidates → validate on the 7 already-ambiguous candidates from this A/B (known ground truth: `pilgrim_monk` sv is back-fitted, MV arms are front-fitted) → only then would a re-run of the multi-view A/B be worth the GPU time.

### 25. Same-subject noise floor covering `iou_back` / `iou_side`
- **Evidence:** `docs/reviews/hi3dgen/ab-multiview-2026-07-29.md` (rework 2 step 8). `target/mv-ab/noise_floor.json`'s determinism probe only ran `--front`, so `iou_back` and `iou_side` have no measured noise floor at all — every value for those two metrics in both `ab.json` files was reported raw, with no claim rule applicable. Back/side fidelity is the rework's actual question and was never adjudicable by metric as a result; the verdict rests entirely on visual review instead (finding 24's front/back degeneracy separately voided `iou_front` too).
- **Ideal:** A same-subject noise floor exists for `iou_back` and `iou_side`, so a future A/B can claim a back- or side-fidelity effect the way this one claimed `vertex_count`.
- **Gap:** No repeat-candidate data has ever been collected with `--back`/`--side` mattes supplied to `mv_ab_metrics.py`.
- **Suggestion:** 3 same-seed repeat candidates per subject (mirroring `noise_floor.json`'s existing 3-repeat design), run through `mv_ab_metrics.py --front --back --side` so `max_pairwise_abs_diff` is computed for all three IoU axes, not just front. Do on at least one prop and one character subject, since `pilgrim_monk`'s floor is currently borrowed cross-subject from chapel_arch.
- **Outcome:** `5/10` — without it, no back/side metric claim is possible for any future multi-view (or other orientation-sensitive) A/B in this pipeline.
- **Cost:** `2/10` — ~2 min GPU per repeat candidate, 6 candidates total (3 per subject) if both subjects are covered; reuses the existing `noise_floor.json` harness with `--back`/`--side` added to the metrics call.
- **Path:** run 3 same-seed repeats per subject → `mv_ab_metrics.py --front --back --side` per candidate → record `max_pairwise_abs_diff` for `iou_back`/`iou_side` alongside the existing `iou_front` floor → update `noise-floor-2026-07-29.md`'s pre-registered thresholds if the two new axes need their own adjudication bar.
