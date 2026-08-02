# Geometry pipeline options — what to do about the decimation defect

2026-08-01. Research + decision report. No code or assets changed.

## The problem

Generated props do not read as stone. The cause is measured and located: Hi3DGen reconstructs
`chapel_arch` at 773,704 tri with all carving intact, and Blender's collapse decimation to 14,999 tri
destroys it — orthographic depth band-pass residuals show amplitude unchanged at every scale while
structure below ~3 cm decorrelates (Pearson 0.343 at σ≈8 mm, 0.585 at 17 mm, 0.764 at 34 mm, 0.940 at
135 mm), i.e. carving is not attenuated but *replaced* by equal-amplitude faceting noise. At 5.50 m the
arch carries 110 tri/m² over 136.7 m², a 14.5 cm mean triangle edge against 1–5 cm relief. A triplanar
detail layer was built and shipped against the earlier texture hypothesis and a blind test still picked
the photoscan control decisively, so the texture channel is not the lever. The measurements are recorded
inline in `tasks/todo.md:1019-1057` and `:1094-1106`; **`docs/reviews/town/decimation-attribution-2026-08-01.md`
does not exist** — the study was never written up as a standalone document.

## Corrections to the framing, before the options

Three facts change the shape of the decision and were not in the brief.

**(a) `tri_budget` is not unscaled — it is scaled by the wrong quantity.** The brief says the budgets are
hand-set "with no scaling by size". They were in fact derived from a 35-run measured sweep
(`docs/reviews/hi3dgen/reworks-hi3dgen-2026-07-28.md` §18; raw data at `target/prop-solid-validation/tribudget/`)
targeting p99 clean→hires deviation of 0.0015 **normalized by bounding-box diagonal**. That metric is
scale-*invariant*: a bigger prop is allowed proportionally bigger absolute error, so density collapses as
size grows. Hence chapel_arch at 167 tri/m² next to olive_stump at 2,832 tri/m² — a 17× density spread
produced *by the metric working as designed*. The defect is the choice of error metric, not the absence
of one. This matters because the fix is not "add a size term to a hand-set number"; it is "replace a
relative-error target with an absolute feature-scale target." Corroborating this from the outside: a
2026-07 perceptual study (below) measured that geometric distance metrics explain only 10–15% of human
preference between simplification results.

**(b) The shipped props were never built at those budgets.** Every shipped GLB's manifest records
`height_target: 1.8` and `clean_tris` 14,998–15,000 — the *old flat 15,000* default and a uniform 1.8 m
normalization. The per-asset budgets landed in `9e92cab` (2026-07-29); the props were generated
2026-07-28 and nothing has been re-run. So a re-decimation pass is **already owed** regardless of this
report's outcome, and folding the real fix into it costs nothing extra. `tasks/todo.md:1533` still lists
the in-engine validation of the 0.0015 target as "owed to the user".

**(c) Any geometry change costs a full texture re-run.** `prop_cleanup.py` unwraps with xatlas *after*
decimation (`scripts/ai-pipeline/prop_cleanup.py:475`) and the hires carries no UVs at all. Changing the
triangle count therefore changes the UV atlas, which invalidates the normal bake, the AO bake, and the
multiview albedo. There is no path that re-decimates without re-texturing all seven props. That is GPU
work and needs a go-ahead. It is also the strongest argument for probing before deciding: you get one
cheap shot at this, so make the decision once.

## THE REFRAMING QUESTION: what can this renderer afford?

**Not on disk. Any number I gave you would be a guess, and I am not going to give you one.**

What is on disk:

- **Zero GPU-side measurements anywhere in the repo.** The three recorded criterion benches
  (`joint_palette_40x64` 106.7 µs, `particle_fill_4096` 22.9 µs, `frustum_classify_552` 7.6 µs) are pure
  CPU math with no wgpu device. `benchmarks/benches/render_cpu.rs` never creates one. `docs/benchmarks/gate-log.txt`
  contains one line. No frame time, no fps, no draw-call counter, no triangle counter is recorded anywhere.
- The stated bar is `docs/visual-quality.md:125-127` **VQ-F1: 60 fps @ 1080p, 40 skinned characters + 2k
  particles, on the dev GPU (RTX 3080 Ti, 12 GB)** — checked "manually at phase boundaries", i.e. never,
  with no result written down. `docs/reviews/rendering/audit-rendering-2026-07-28.md` finding 8 flags this
  gap and is unimplemented.
- **The 15,000 figure was never a performance number.** It was a hardcoded `--tri-budget` default in
  `prop_cleanup.py` (origin `8c1b830`) that `gen_prop.py` never overrode. No rationale is recorded. The
  word "budget" in this pipeline has always meant *geometric fidelity*, never *frame time*.

What the architecture says, which is more informative than a guess:

- The start zone is **48 prop instances → 1,251 primitive instances → 547,424 triangles**, issued as
  **~708 unique primitives × 5 passes (3 shadow cascades + depth prepass + main) ≈ 3,540 `draw_indexed`
  calls per frame**, averaging ~437 tri/draw overall and **~41 tri/draw for the town kit**
  (`casa_corner.gltf` is 173 primitives for 7,166 triangles).
- **The seven generated props are 1 primitive each.** Raising their triangle count adds *zero* draw calls.
  This renderer is bound by submission overhead, not by primitive setup, and the props are on the
  free side of that line.
- Scale reference: a shipped UE5 *Valley of the Ancient* frame is ~5M triangles on 2021 hardware, over 90%
  of it software-rasterized. 7 props at 150k tri × ~20 instances ≈ 3M triangles of *large, hardware-rasterizable*
  geometry in one instanced draw each — the same order, on strictly easier terms.
- The multiplier that does bite: geometry is submitted **5×** (3 shadow cascades + prepass + main), so a
  3M-tri scene submits ~15M tri/frame. That is the number an LOD chain attacks first, by putting cascades
  on a coarse level.
- The `store.rs` no-dedup OOM referenced in the brief **is fixed** — `cac3c94` (2026-07-31) made
  `TextureCache` key on a content hash of the image, so the whole townkit is 15 unique 2048² BC7 textures
  ≈ 84 MB shared across every casa and the chapel, down from ~4.8 GB per casa instance. `tasks/todo.md:384-388`
  still describes the debt as open; **that note is stale and should be struck.** Residual: a *missing*
  texture slot still mints a fresh uncached 1×1 (`store.rs:132-134`), so casa_corner allocates 346 tiny
  textures plus 173 bind groups / uniform buffers / vertex buffers / index buffers. Memory is trivial;
  object count and per-draw bind-group churn are not.
- No stress-scene run has been recorded since the dedup landed either.

### The cheap probe that answers it

**Swap one prop's mesh for its 773,704-tri hires and read the GPU timers that already exist.**

`smirk/engine-renderer/src/gpu_timer.rs` already measures shadow / main / particles / bloom+tonemap / egui
and `frame.rs:178-182` prints them to the F3 overlay every 30 frames — nothing writes them to a file. The
hires GLBs are on disk for all seven props (`target/prop-batch/.../clean_hires.glb`, 4.4–18.6 MB, verified
present). The probe needs no generation, no texture run, no GPU inference: point the zone entry at the
hires, accept that it renders untextured (no UVs), and log the existing timers for the start zone at 15k
vs 773k. Sweep intermediate counts by re-running only `prop_cleanup.py`'s decimation step, which is
seconds of CPU.

Cost: an afternoon, mostly plumbing the timer values to a file (which finding 8 of the rendering audit
wants anyway). It converts the single largest unknown in this decision from a guess into a curve, and it
is a prerequisite for *every* option below being sized honestly. **Do this first.**

### The sampling argument, so the probe has a target to test

For a roughly uniform triangulation, mean edge `e` and density `D` relate as `D = 2.309 / e²` (equilateral
area `√3/4·e²`). Checking against the measurement: `e` = 14.5 cm → `D` = 110 tri/m². Matches the recorded
figure exactly, so the relation is sound.

| mean edge | tri/m² | what it resolves |
|---|---|---|
| 14.5 cm | 110 | current chapel_arch — nothing in the relief band |
| 10 cm | 231 | — |
| 5 cm | 924 | Nyquist floor for the *largest* relief only; features read as bumps, not carving |
| 3 cm | 2,566 | the measured decorrelation knee (Pearson crosses ~0.7 near 34 mm) |
| 2 cm | 5,772 | **the hires' own resolution** (773,704 / 136.7 m² = 5,660 tri/m² → e = 2.02 cm) |
| 1 cm | 23,090 | above the source; unreachable without regeneration |

Two consequences worth stating plainly. First, geometric detail needs roughly one edge per *half*
wavelength to read as form and closer to a quarter to read as carving, so 1–5 cm relief wants 1.5–2.5 cm
edges — 3,700–10,000 tri/m², **34–90× the current density.** Second, **the Hi3DGen source itself only
resolves ~2 cm**, so 5,660 tri/m² is a hard ceiling: there is no budget above the hires that buys anything.
The usable range is bounded on both ends, and it is narrow — somewhere between 900 and 5,700 tri/m².

Applying that to the seven props (hires surface areas from the §18 sweep; **flagged as a guess — the sweep
records 82.5 m² for chapel_arch while the attribution study records 136.7 m², and neither is stated at the
in-world scale from `content/zones/zones.ron`; measuring world-space area off each hires GLB is a
two-minute script and must precede any budget being written**):

| prop | hires area (m², sweep) | current | @924 tri/m² | @2,566 tri/m² | hires tri |
|---|---|---|---|---|---|
| candelabra_shrine | 2.3 | 15,000 | 2,100 | 5,900 | 187,022 |
| gravestone | 5.0 | 15,000 | 4,600 | 12,800 | 253,732 |
| olive_stump | 6.5 | 15,000 | 6,000 | 16,700 | 742,440 |
| broken_column | 7.1 | 15,000 | 6,600 | 18,200 | 326,406 |
| crucero | 15.5 | 15,000 | 14,300 | 39,800 | 191,354 |
| chapel_arch | 82.5 | 15,000 | 76,200 | 211,700 | 773,704 |
| cypress | 202.9 | 15,000 | 187,500 | 481,300 | 417,452 |
| **total** | **321.8** | **105,000** | **297,300** | **786,400** | **2,892,110** |

The table's own message: at a fixed density the small props barely move (several are *over*-budget today
and would go down), and essentially all of the cost is chapel_arch and cypress. Cypress is foliage and
belongs in a different regime — a tree's silhouette is alpha-cutout leaf cards, not carved relief, and
applying a stone-relief density to it is category error. Excluding cypress, the honest ask is
**~110k → ~305k triangles across six props**, i.e. the entire defect costs on the order of a quarter
million triangles.

---

## Option 1 — Raise the budget, derived from absolute feature scale

Replace the relative-deviation target (p99 deviation / bbox diagonal = 0.0015) with an absolute density
target `tri_budget = D × world_surface_area`, `D` chosen from the table above, clamped to the hires count.
Zero new tools. One edit to `content/models/assets.json` plus a small area-measurement step in
`gen_prop.py`. Also fixes the (b) discrepancy, since the props must be re-run anyway.

- **Outcome — high, and it is the only option that addresses the actual cause.** The measurement says
  relief is destroyed by insufficient sampling; only more samples fix that. At 2,566 tri/m² the mean edge
  drops from 14.5 cm to 3 cm, which is the band where the measured Pearson correlation is still 0.76 and
  climbing. It cannot exceed the hires' 2 cm ceiling, so it does not fully restore the concept — but the
  concept was never in the mesh.
- **Confidence — medium-high on the mechanism, low on the number.** The sampling relation is arithmetic
  and validates against the recorded 110 tri/m² / 14.5 cm pair. What is *not* established is (i) which `D`
  is perceptually sufficient, (ii) the true world-space areas, (iii) what the renderer affords.
  **Cheap probes**: the hires-swap timing probe above for (iii); a two-minute area script for (ii); for
  (i), decimate one prop at 5–6 densities from the hires already on disk and re-run only the *depth
  band-pass residual* measurement from the attribution study — no texture run needed, since the metric is
  geometric. That yields a correlation-vs-density curve and lets you pick `D` at the knee instead of by
  argument.
- **Cost — low in engineering, real in GPU.** The `assets.json` edit and area measurement are trivial. The
  unavoidable cost is one full re-texture of seven props (xatlas → normal/AO bake → multiview albedo →
  MaterialAnything), which needs a go-ahead and a wall-time estimate; Phase 3 budgets ~5.5 h GPU for a
  comparable batch. Triangle cost to the renderer is ~+200k in the start zone with **zero added draw calls**.

## Option 2 — Replace Blender Decimate with meshoptimizer

Blender's collapse is Garland–Heckbert QEM over **positions only**; UVs and normals never enter the cost
function. Two things in `bmesh_decimate_collapse.cc` (read on `main`, 2026-08-01) matter here: the UV-seam
guard is `// #define USE_SEAM`, literally commented out with the note "its not really that important";
and `USE_TOPOLOGY_FALLBACK` (`TOPOLOGY_FALLBACK_EPS 1e-12f`) switches collapse ordering from error-driven
to an edge-length heuristic wherever quadric cost is at float-noise level — which is exactly the regime of
shallow relief on a large prop. That is a plausible mechanism for "amplitude conserved, structure
decorrelated": QEM minimizes squared point-to-plane distance with facet *orientation* unconstrained, and
its quadric is rank-deficient on near-planar regions, so the surviving vertex slides freely within a null
space. Amplitude has nowhere to go but into facet orientation. Blender's own developers are replacing the
modifier — PR #158508 (Goudey, open since 2026-05-12) reimplements Simplify on meshoptimizer, "~10x faster
… and with better results."

meshoptimizer (**MIT**, v1.2, 2026-06-30) offers `simplifyWithAttributes` (normals/UVs in the quadric,
attribute error tracked across discontinuities since v0.22), `simplifyWithUpdate` (v0.25 — relocates
vertices to optimal positions rather than picking among originals; the only OSS option that does), and
`SimplifyPermissive` + `SimplifyVertex_Protect`, which matters because the README warns that faceted
meshes — exactly image-to-3D output — can otherwise stall before reaching the target count. All
simplification APIs went stable at v1.0 (2025-12-08).

The one rigorous comparison in the field: Lukáš Gallo, 80.lv, 2026-07-01 — 288 operations over 8 Arma
Reforger assets, ~2,800 pairwise comparisons, 150 judges. **meshoptimizer 57.6% preference and fastest by
a wide margin**; CGAL good at mild reduction and "degrades at aggressive levels"; Open3D "pillowed" on
hard surfaces (and drops UVs entirely); fast-simplification structurally destroyed geometry at 90%
reduction. Its central finding is the one that should change how this project measures: **Hausdorff-style
geometric metrics explained only 10–15% of human preference** — shading preservation, silhouette
integrity and feature retention dominated, and no standard metric captures them. That is a direct
external indictment of the 0.0015-deviation target.

Rejected on licence (the project's absolute gate): Simplygon ($42k/title/yr, free tier abolished 2024-03),
InstaLOD (from $11,880/yr; free tier requires attribution), RapidPipeline (batch CLI enterprise-gated),
Exoside Quad Remesher (the affordable Indie tier is non-commercial). MeshLab/PyMeshLab and CGAL are GPL-3
— usable as offline tools, not linkable, and neither beat meshoptimizer in the study. Quad remeshers
(Instant Meshes BSD-3, QuadriFlow MIT, Blender's remesh) are the **wrong tool** — they generate new
vertices and would destroy nothing here, since UVs are created after decimation, but they also do not
target a triangle budget and add a whole topology stage for no measured gain.

Note if the pipeline ever routes through glTF tooling: **gltfpack calls `simplifyWithAttributes`;
`gltf-transform`'s `simplify` calls plain `simplify()` and cannot reach the attribute-aware path at all.**

- **Outcome — medium. Real, but it is not the fix.** No simplifier recovers 1–5 cm relief at 110 tri/m²;
  that is a sampling limit and meshoptimizer is subject to it identically. What it plausibly buys is a
  *clean low-frequency base* instead of equal-amplitude faceting noise, which is precisely the carrier the
  normal map needs (see Option 4). Best value is as a multiplier on Option 1: at any given budget, better.
- **Confidence — medium.** The 80.lv study is genuinely rigorous but did not include Blender Decimate, so
  **no rigorous meshoptimizer-vs-Decimate benchmark exists anywhere**; the two claims that do are Goudey's
  PR text and a Blender Artists post. The `USE_TOPOLOGY_FALLBACK` mechanism is a source read, not a
  measurement. **Cheap probe**: run chapel_arch's 773k hires through Blender collapse and through
  `simplifyWithAttributes` at the same target and compare per-scale band-pass residual correlation with
  the attribution study's existing script. Same harness, no texture run, direct answer.
- **Cost — low-medium.** MIT, vendorable; a Rust binding or the `gltfpack` CLI both work. The pipeline
  complication is real though: meshoptimizer's attribute path wants UVs *going in*, and this pipeline
  creates UVs *after* decimation, so either the hires gets unwrapped first (changing stage order in
  `prop_cleanup.py`) or the attribute path degrades to position-only and most of the advantage is lost.
  Budget that restructuring, do not assume it is a drop-in.

## Option 3 — LOD chain instead of one budget

**No mesh LOD exists.** The only thing named LOD is animation pose-rate LOD (`smirk/engine-renderer/src/mesh/sync.rs:31`,
`LOD_POSE_DISTANCE = 40.0`), which halves pose updates beyond 40 m and still draws the full mesh. Geometry
LOD is explicitly deferred in `docs/visual-quality.md` "Future work".

Adding it is unusually cheap here. `MeshStore` is a flat `Vec<GpuMesh>` keyed by asset *path*
(`store.rs:236-242`), indices are append-only and stable by contract (`store.rs:260-262`), and draw lists
are rebuilt from scratch every frame by re-resolving `store.get_or_request(&mesh.asset)` per entity
(`sync.rs:319-388`). `pack_visible` then sorts by mesh index and emits contiguous instanced runs
(`sync.rs:398-399`). So: register `foo_lod0..lod2` as separate paths, pick the index at `sync.rs:323` from
a distance test — the exact `distance_squared` expression already exists at `sync.rs:362-363` — and the two
levels become two instanced runs automatically. There is no slot bookkeeping to invalidate.

The strongest argument for it is not memory, it is the **5× submission multiplier**: shadow cascades and the
depth prepass draw the same geometry four extra times, and cascades have no business at LOD0. The second is
sub-pixel triangles: measured across Haswell/Kepler/GCN, 1×1 px triangles run 10–20× slower than large ones
because the rasterizer's 2×2 quad granularity wastes three of four lanes; keeping triangles ≥ ~8×8 px is the
standing recommendation. A 300k-tri prop at distance violates that badly. Raising budgets without LOD
trades a near-field defect for a far-field one.

- **Outcome — medium-high, but as an enabler, not a fix.** It fixes nothing about the carving. It is what
  makes Option 1 safe at the top end, and it is the only thing that makes a genuinely aggressive density
  (≥2,566 tri/m²) affordable.
- **Confidence — high on the engineering cost (I read the submission path), low on the necessity.**
  Whether it is needed *at all* depends entirely on the unmeasured perf headroom. **The hires-swap probe
  settles this too**: if 773k-tri props at 48 instances hold 60 fps, LOD is optional polish; if they do
  not, LOD is mandatory and its distance thresholds fall out of the same measurement.
- **Cost — low-medium.** Renderer side is maybe a day given the architecture. Content side is the larger
  half: `prop_cleanup.py` must emit a chain, and each level needs its own UV atlas and bake unless levels
  share LOD0's atlas (they can, if simplification is constrained to preserve the seam set — which is
  precisely `meshopt_simplify`'s default behaviour and another argument for Option 2).

## Option 4 — Normal / displacement / parallax as the answer

**A normal map cannot fix a 14.5 cm silhouette, and this should not be litigated further.** Normal maps
perturb shading normals only: no silhouette change, no parallax, no self-shadowing, no self-occlusion. At
1–5 cm relief on a 5.5 m prop viewed at gameplay distance, parallax and self-shadowing are perceptually
load-bearing — this is the band where a normal map reads as decoration rather than carving. The blind
test already returned this verdict empirically.

The bake itself is, on inspection, **mostly correct**: `scripts/ai-pipeline/proptex/export.py` bakes
selected-to-active from the true hires (`clean_hires.glb`, the pre-decimation export at
`prop_cleanup.py:453`) onto the clean mesh, tangent space, `cage_extrusion = 0.01 m`,
`max_ray_distance = 0.01 + 0.004 × diag`, margin 8 px, AO at 128 samples / 0.15 m. On 1.8 m-normalized
meshes that gives ~1.8 cm of ray reach against a p99 clean→hires deviation of ~2–5 mm, so rays are
reaching. Three real but secondary defects: `cage_extrusion` is the one absolute constant in an otherwise
diagonal-relative scheme, applied identically to a 1.3 m candelabra and a 12 m cypress;
`BAKE_RAY_DIAG_FRACTION = 0.004` was re-derived from deviations at the *new* budgets that **no shipped prop
was ever built at**, so the shipped bakes ran at 0.006 against different geometry; and `UV_ATLAS_RESOLUTION`
is hardcoded 1024 (`prop_cleanup.py:71`) while three props bake at 2048, contradicting the comment at
`:67-70`. That last one wastes atlas efficiency rather than causing bleed, so it is minor.

The mechanistic explanation for "waxy" that fits the evidence: a tangent-space normal map is interpolated
over the low-poly's *interpolated vertex normals*, so a faceted base injects a low-frequency shading error
the map cannot cancel. The high-frequency signal is riding a wrong-shaped carrier — sheen without form.
That is a direct argument that **Option 2 improves the normal map's effectiveness even at an unchanged
budget**, by giving it a clean carrier.

POM is alive at AAA scale (Pearl Abyss's Black Space Engine uses screen-space displacement throughout
*Crimson Desert*, 80.lv 2026-06-01), with a predictable cost model — step count × pixel coverage — and
Tatarchuk's classic measurement of 0.7 ms perpendicular rising to 1.3 ms at grazing angles. But its
silhouette handling is a clip trick (masking overshooting pixels via opacity), not geometry, and it needs
a heightmap channel this pipeline does not currently produce, a new shader path, and a static-switch
variant to compile out at distance (distance-fade by lerp saves nothing). Tessellated displacement is back
narrowly as Nanite Tessellation (UE 5.4, Karis 2026-02) for hero/close-up work; that is an engine feature,
not a weekend.

The honest counter-argument to all of this is NVIDIA's *Appearance-Driven Automatic 3D Model
Simplification* (Hasselgren et al., EGSR 2021, arXiv:2104.03989), which jointly optimizes mesh and normal
map against image-space error and reports a 300k statue at 7k triangles. It does not refute the sampling
argument — it *gives up* on the geometry and migrates relief into shading deliberately, trading silhouette
and parallax rather than eliminating the trade. Which is the same conclusion.

- **Outcome — low as a primary fix, medium as an accompaniment.** Cannot restore silhouette or parallax.
- **Confidence — high that a normal map cannot fix silhouette (settled, uncontroversial). Low on whether
  the current bake has a *second*, independent defect.** **Cheap probe**: after re-decimating at a higher
  budget, render the same prop with the normal map on and off — if the on/off delta is large and the
  result still reads wrong, the bake is fine and the budget was the whole story.
- **Cost — POM is medium-high (heightmap generation + shader path + variant management) for a benefit that
  is strictly second-order to fixing the base mesh.**

## Option 5 — Virtualized geometry, mesh shaders, impostors

wgpu **does** have mesh shaders (v28.0.0, 2025-12-18; Vulkan/DX12/Metal), and the native feature set for a
Nanite-like path is present — multi-draw indirect, subgroups, `SHADER_INT64`, `TEXTURE_INT64_ATOMIC`.
Tracking issue #7197 still lists a "mesh shader redesign" (#9170), so budget for API churn. Available
building blocks: meshoptimizer's `meshopt_partitionClusters` / `buildMeshletsSpatial` / `clusterlod.h`
(MIT, but `clusterlod.h` ships under `demo/` — reference quality, not a stable contract); Bevy's
`bevy_pbr::experimental::meshlet` (MIT/Apache-2.0, Vulkan+Metal only, no DX12, MSAA must be off).

Two facts decide this. Bevy's meshlet path took three experienced graphics programmers roughly two years
across four release cycles, with meshoptimizer and METIS doing the offline work, and is still experimental
with open culling bugs and no streaming. And Bevy's own documentation states: *"Much greater base
overhead. Rendering will be slower and use more memory than Bevy's standard renderer with small amounts of
geometry."* This project's start zone is ~547k triangles in ~3,540 draws. That is roughly two orders of
magnitude below where virtual geometry begins to pay.

Impostors are the reverse-direction idea and are worth exactly one sentence: they solve far-field cost,
which is not the reported defect, and Option 3 covers the same ground for far less.

- **Outcome — high in the limit, irrelevant at this scale.**
- **Confidence — high.** Bevy's own docs state the crossover is on the wrong side.
- **Cost — months, and the offline DAG half is the harder half. Do not.**

---

## Recommended sequence

**First — measure, before deciding anything (no GPU generation, no go-ahead needed).**

1. **Plumb the existing GPU timers to a file** (`gpu_timer.rs` / `frame.rs:178-182` already produce the
   numbers) and record a start-zone baseline. This also closes rendering-audit finding 8.
2. **Hires-swap A/B**: point one zone entry at the on-disk 773,704-tri `clean_hires.glb` (untextured is
   fine) and re-read the timers. Sweep 15k / 50k / 150k / 300k / 773k by re-running only
   `prop_cleanup.py`'s decimation. This produces the affordability curve that is currently a guess.
3. **Measure world-space surface area per prop** off the hires GLBs at the `zones.ron` scale, and reconcile
   the 82.5 vs 136.7 m² discrepancy for chapel_arch. Two minutes; no budget may be written before it.
4. **Density-vs-correlation curve**: decimate chapel_arch at 5–6 densities and re-run the attribution
   study's band-pass residual measurement. Geometric metric only, so no texture run. Pick `D` at the knee.
5. **Same harness, Blender collapse vs `meshopt_simplifyWithAttributes`** at one matched budget. Settles
   Option 2 empirically and settles whether the `USE_TOPOLOGY_FALLBACK` mechanism is real.

All five are CPU-hours at most and together turn every load-bearing guess in this report into a number.

**Then — the actual fix, in one re-run (needs a go-ahead).**

6. Replace the relative-deviation target with `tri_budget = D × world_area`, clamped to the hires count,
   with cypress handled separately as foliage. Fold in the pending correction that no shipped prop was
   ever built at the current budgets.
7. Adopt meshoptimizer if step 5 says so — which requires moving the UV unwrap *before* simplification in
   `prop_cleanup.py` for the attribute path to mean anything.
8. Re-run the full texture pipeline for the seven props exactly once, at the settled budgets.

**Only if step 2 says the budget does not fit** — add the discrete LOD chain (Option 3). The renderer
architecture makes it cheap and the 5× shadow/prepass multiplier plus the sub-pixel-triangle penalty means
it will probably be wanted eventually regardless. But it is a consequence of the measurement, not a
prerequisite.

**Do not** build POM or tessellation, buy Simplygon/InstaLOD/RapidPipeline (all licence-disqualified with
no independent evidence they beat MIT-licensed meshoptimizer), introduce a quad remesher, or start on
virtualized geometry. And do not spend another cycle on the texture channel — that hypothesis has been
tested and refuted twice.

**Also strike, unrelated to this decision:** `tasks/todo.md:384-388`'s "no texture dedup" note is stale as
of `cac3c94`.

## What in this report is a guess

- **Every triangle-affordability statement.** No GPU measurement exists on disk. The architectural
  arguments (props are 1 draw each, 5× submission multiplier, the UE5 5M-tri comparison) are sound but are
  not a measurement of *this* renderer.
- **The per-prop surface areas** in the density table, taken from the §18 sweep, which disagrees with the
  attribution study on chapel_arch by 1.66× and states neither at in-world scale.
- **The choice of `D`.** 924 and 2,566 tri/m² are derived from a Nyquist argument, not from a perceptual
  measurement. Probe 4 replaces this.
- **The `USE_TOPOLOGY_FALLBACK` explanation** for the equal-amplitude noise. A source read of Blender's
  `bmesh_decimate_collapse.cc`, not a measurement. Probe 5 replaces this.
- **That meshoptimizer beats Blender Decimate.** No rigorous head-to-head exists in the literature; the
  80.lv study did not include Decimate. Directional only.
- **The GPU wall-time of a seven-prop re-texture.** Extrapolated from Phase 3's ~5.5 h estimate for a
  comparable batch, not measured.
- **Whether the 3-instance street OOM is actually resolved.** The dedup fix landed and is unit-tested, but
  no post-`cac3c94` street-scene run is recorded.

## Licence positions checked

meshoptimizer / gltfpack MIT (clear). Bevy meshlets MIT/Apache-2.0 (clear, but rejected on cost).
Instant Meshes BSD-3, QuadriFlow MIT (clear, but wrong tool). MeshLab/PyMeshLab and CGAL GPL-3 — offline
use only, not linkable. Simplygon, InstaLOD, RapidPipeline, Exoside Indie tier — **gated out**
(proprietary/NC/attribution-encumbered). No `content/source/CREDITS.md` rows added: that file's own rule
is to record an asset "when the asset lands", and nothing has been adopted yet. If meshoptimizer is
adopted in step 7, it earns a row at that point.
