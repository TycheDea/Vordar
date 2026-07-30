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

The reworks file's findings 24 and 25 (orientation-robust fidelity metric;
same-subject back/side noise floor) are **parked 2026-07-30, user ruling**:
no pending A/B consumes them since multi-view was rejected, so they wait for
the next orientation-sensitive experiment rather than entering this queue.
Campaign closed 2026-07-30 — aggregate regenerated at `1532b9d`, lesson-mining
pass done (2 lessons accepted: `tasks/lessons/2026-07-30-*`).

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
- **Probed 2026-07-29 — verdict: EXCISE.** Settled by measurement rather than by eyeballing, since the payload question has a parameter-free test: a normal field decodes to unit vectors and tracks geometry, albedo does neither.
  - **Channels 3-5 are the model's predicted surface normal.** Cosine against the geometric vertex normal averages **0.977** (candelabra_shrine) and **0.935** (chapel_arch); 95.4% / 88.4% of vertices exceed cos 0.9; the decoded-vs-geometric correlation matrix is diagonal (0.89-0.94) with off-diagonal ≤0.035. Decoded length is not tightly unit (mean 0.81, median 0.99) because trilinear interpolation happens on pre-sigmoid logits and shrinks off-lattice magnitude — the cosine and the identity correlation settle it regardless. This duplicates what the mesh already carries, and the pipeline already bakes a better normal map from the `_hires` mesh onto the clean mesh, at hires resolution rather than extraction resolution.
  - **Channels 0-2 are not albedo.** Inter-channel Pearson **0.997-0.999**, mean saturation 5.1% / 2.4%, decoded length within 1% of unit for only 1.7% / 0.8% of vertices, cosine against the normal 0.21 / 0.14. On candelabra_shrine — iron, wax and stone in one prop — there is **no material colour separation at all**; the renders are uniform ivory-grey. The only structure is a ~0.6-0.68 correlation with the normal's z, i.e. shading-like. What the head was trained as is unmeasured; that it carries no albedo on this normal-trained checkpoint is measured.
  - **Cost banked is larger than the finding claimed.** The +407 MB is exact and verified — the dense attribute grid is 257³×10×4B = 679.0 MB with colour against 271.6 MB without, and `voxelgrid_colors` measured 407,390,232 bytes — but it excludes the cube-corner `torch.cat` growing (N,8,4) → (N,8,10): a further **+181.3 MB** at chapel_arch's N=944,448 and +100.9 MB at candelabra_shrine's. Measured as tensor bytes on the CPU replay, not as a CUDA allocator delta; no GPU run was authorized.
  - Artifacts: `target/attr-probe/<prop>/` — vertex-coloured `.ply` for both channel groups, three orbit renders each, and `<prop>_attr_stats.json`. Probed on 2 of 7 props (the two with the most material variety and the hardest topology); the other five latents are untouched.

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
