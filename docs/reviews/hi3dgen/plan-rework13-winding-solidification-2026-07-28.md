# Plan: Solidify by directional exposure, not by boundary reachability — 2026-07-29

Source: `docs/reviews/hi3dgen/reworks-hi3dgen-2026-07-28.md` finding 13 (with
finding 15 landing inside it, step 2).

Anchor conventions follow the source report: `fork:` = `C:/tools/Hi3DGen/Hi3DGen`
(branch `vordar-fixes`, clean at `cf718c6`), unprefixed = vordar-repo relative.
The Hi3DGen venv interpreter is `C:\tools\Hi3DGen\venv\Scripts\python.exe`;
Blender is `C:\Program Files\Blender Foundation\Blender 5.2\blender.exe`.

Predecessor: `plan-rework1-solid-interior-2026-07-28.md`, approved at `3c35a7b`,
parked at step 6 of 8. Its steps 1–5 stand; its step 6 measured the premise
failure this plan answers; its steps 7–8 stay blocked until step 4 below moves
the predicates.

## Ideal end state

`SparseFeatures2Mesh` solidifies the dense SDF grid by **directional exposure**:
a cell keeps its outside value only if some straight line from it leaves the
grid without entering material; every other outside-valued cell is relabelled
`-1` before marching cubes. `fill_enclosed_sdf` and its scatter-written
exclusion are deleted — not flagged off, not kept as a second path — because
exposure strictly subsumes them. The extracted mesh is a single watertight
solid whose inner surfaces never existed, so `prop_cleanup.py`'s
camera-reachability strip becomes a near-no-op backstop
(`interior_tris_removed / raw_tris` ≤ 0.02) instead of deleting a third of every
raw mesh and leaving 14,838 boundary edges behind. The fork harness pins the new
contract in both directions: an opening you can see through stays open, a cavity
no straight line reaches becomes solid — including a cavity that lies wholly
inside the scatter band, which the old mechanism could not fill and the old
harness never exercised.

## Design decisions

### The finding's diagnosis is incomplete, and the corrected one changes the fix

Finding 13 concluded that chapel_arch and crucero have cavities that "drain to
the boundary through the props' open mouths". Measured on the real 257³ fields
rebuilt from the saved latents (read-only probes over
`target/prop-latents/<prop>/cubefeats.pt`, this planning pass):

| | chapel_arch | crucero | candelabra_shrine |
|---|---|---|---|
| written (scatter) cells | 1,126,189 | 478,115 | 617,221 |
| negative (material) cells | 410,607 | 227,061 | 269,689 |
| material EDT max / p50 (cells) | 5.74 / 1.0 | 5.39 / 1.0 | 7.00 / 1.41 |
| band half-thickness p50 / p99 | 2.0 / 5.39 | 2.0 / 5.39 | 2.24 / 6.00 |
| free cells enclosed by the **band** | 108 | **0** | 124,408 |
| free-space components (whole grid) | 5 | **1** | 19 |
| inward-ray first hit, p50 (cells) | 2.47 | 3.71 | 16.77 |
| raw hollow mesh: components / main-body volume | 17 / +0.020575 | 15 / +0.011912 | 11 / +0.022671 (plus a −0.007279 inner shell) |

Three facts fall out, and together they falsify the finding's mechanism:

1. **There is no unwritten cavity on chapel_arch or crucero.** The complement
   of the scatter band is *one* connected region on crucero and one plus four
   27-cell specks on chapel_arch. Nothing is enclosed by the band, so no cell
   was ever left at `get_dense_attrs`'s `+1` initialiser inside those props.
   The premise that `get_dense_attrs` stamps an interior it should not — the
   whole basis of rework 1 — simply does not fire on them.
2. **Their material is a thin sheet, ~2.5–3.7 cells thick**, and the band
   (~4–5 cells) spans it. So the extracted "inner wall" is not an extraction
   artefact at the `+1` boundary: it is genuine predicted surface. The network
   generated a hollow shell. On candelabra_shrine, whose material is 16.8 cells
   thick at the median, the band *cannot* span it — that is why a genuine
   `+1` cavity exists there (124,408 cells), why it is the one subject
   `fill_enclosed_sdf` moved, and why its raw mesh carries a separate
   −0.007279-volume inner component while the other two do not.
3. **Therefore generalized winding number cannot fix chapel_arch or crucero on
   its own.** GWN answers "is this point inside the surface". The surface is a
   correctly-oriented, watertight, thin shell; GWN over it returns exactly that
   shell. To solidify, something must first decide that the inner sheet is not
   wanted — and no field-level or winding-level signal distinguishes the inner
   sheet from the outer one, because both are predicted geometry with identical
   provenance. GWN is only usable *after* that decision, as a smoother, at the
   cost of a second marching-cubes pass, an inner-sheet classification
   heuristic, ~15 M queries against ~770 k triangles, and a new dependency.

This is a wall in the direction the user selected, so it is reported rather than
shimmed: **direction (i) is planned as a signed-distance/visibility
determination, not as a winding-number one.** The user's binding requirement —
"a sign test that does not consult the grid boundary" — is satisfied in the
sense that matters: exposure uses the grid boundary only as a ray terminator,
never as a connectivity seed, so a single leak lets in one narrow cone instead
of draining an entire interior. That catastrophic single-leak behaviour is
exactly what killed reachability (49 cells found where 690,882 were needed).

### The mechanism: directional exposure over the 26 integer directions

`solidify_hidden_interior(sdf)`: material `M = {sdf < 0}`; a cell is *exposed*
if, for at least one of the 26 directions `d ∈ {-1,0,1}³ \ {0}`, the ray from
that cell in direction `d` leaves the grid without entering `M`; every
non-exposed non-material cell is relabelled `-1`. Integer directions make each
sweep exact — no rasterization, no interpolation — and each is a single
recurrence over 257 slabs (`E[i] = free[i] & shift(E[i+s])`, with cells shifted
in from outside the grid counting as escaped).

Measured on the real fields in this planning pass (pure numpy, CPU):

| | chapel_arch | crucero | candelabra_shrine |
|---|---|---|---|
| interior cells, 6 directions | 758,977 | 288,055 | 172,745 |
| interior cells, 18 directions | 719,139 | 283,857 | 164,222 |
| **interior cells, 26 directions** | **690,882** | **283,407** | **162,787** |
| largest interior component (26) | 690,845 (99.99%) | 283,402 (99.998%) | 116,680 |
| interior components (26) | 9 | 3 | 28 |
| implied volume ratio (cell count) | 2.68 | 2.25 | 1.60 |
| wall time, all 26 directions | 1.0 s | 0.9 s | 0.9 s |

Against `fill_enclosed_sdf`'s 27 / 0 / 114,208 cells filled, this is a factor of
~25,000 on chapel_arch and unbounded on crucero, and it lands as **one coherent
cavity** rather than the confetti (largest components 27 and 3) reachability
found. The 6→26 direction sweep moves the count by only 9% / 1.6% / 5.8%, so
the criterion is not perched on a tuning knob; 26 is chosen because it is the
complete integer-direction set (no arbitrary count) and costs 1.0 s versus
0.5 s for 6. Rejected: exposing the direction count as a ctor knob — the
measured insensitivity across the whole 6..26 range does not earn a parameter,
and finding 13 explicitly warned against adding knobs to force these numbers.

**Cost and licensing.** Pure numpy + the existing torch/scipy imports; 1.0 s CPU
against `fill_enclosed_sdf`'s 0.25–0.29 s, on a stage that already costs tens of
seconds. `libigl` is **not** in the Hi3DGen venv (verified: `pip list` shows
numpy 1.26.4, scipy 1.17.1, scikit-image 0.26.0, trimesh 4.12.2, embreex,
rtree — no winding-number package of any kind), so the GWN route would have
required a new install. This design adds **no dependency**, so the nvdiffrast /
kaolin / FlexiCubes ban and the NC-licensing gate are untouched by construction.

### `fill_enclosed_sdf` is deleted, not kept alongside

Exposure strictly subsumes reachability: a cell with no positive path to the
boundary also has no straight line to it, so `Interior_reachability ⊆
Interior_exposure`. The candelabra_shrine win the finding insisted must not be
dropped is carried and exceeded — 162,787 cells claimed against the
line-90-deleted flood's 142,571, with the same dominant components (116,680,
12,438, 6,201, 6,138 all reappear in the exposure decomposition). Keeping both
would be two parallel paths for one defect, so the swap rule applies: the
function, its scatter-written exclusion, its harness usages and its docstring
contract all go. The ctor flag keeps the name `fill_interior` — it is already
threaded through `decoder_mesh.py:133`, `prop_hi3dgen.py`'s `--no-fill-interior`
and the manifest `"extraction"` block, and renaming it would churn three files
for no behavioural gain.

The scatter-written exclusion must not survive in any form. Its purpose was to
stop a flood entering through an open mouth made of predicted outside values;
exposure has no flood to protect. Worse, keeping it would defeat the fix: the
band's positive skirt against an inner sheet is *written*, so excluding written
cells would leave a 2–4 cell positive rind inside every cavity and marching
cubes would re-extract the inner wall. Predicted values on the *exposed* side
are untouched, which is what preserves the outer surface's sub-voxel placement.

### The harness contract changes, in both directions

The new contract is "an opening you can see through stays open; a cavity no
straight line reaches becomes solid", replacing "a fully sealed cavity extracts
solid". Consequences pinned by tests:

- `plate_stack` (gaps 2–32, open on all four lateral sides) stays
  **bit-identical** — every gap cell sees daylight along ±x and ±y. Unchanged.
- `through_tunnel` (the chapel_arch analogue, 16-wide straight bore) stays
  **open** — every tunnel cell sees daylight along the bore axis. Unchanged.
- `sphere` stays solid, inward-facing fraction < 0.01. Unchanged.
- `vessel` (r=24..30 shell with an r=4 bore to +z) now **seals**: cavity cells
  off the bore axis are blocked in all 26 directions. Its old `≤ 1.30` upper
  bound was the seal *detector*; it becomes the seal *assertion*. This is the
  intended policy for a prop pipeline and is the same policy
  `prop_cleanup.py:172-210` already applies in mesh space with 64 hemisphere
  rays — this plan moves it upstream, it does not invent it.

  **Measured 2026-07-29, and the "all 26 directions" clause above is wrong.**
  The bore admits a visibility bundle: 94.8% of the 57,747 `r<24` cells fill,
  and the residual is one connected component of 3,011 cells anchored at the
  bore mouth (volume ratio 2.0029 against a 2.05 full-fill ceiling — no inner
  wall survives). Those cells have a clear line of sight out along at least one
  of the 26 directions, so they are exposed *by definition*; sealing them is
  exactly what `bowl_stays_open` and `through_tunnel` forbid. **Step 1's second
  assertion `inward_area_fraction < 0.05` is struck.** It is not relaxed and not
  replaced: it measures 0.3916 under `fill_enclosed_sdf` and 0.2005 under
  exposure, failing in both states, so it never discriminated — while the volume
  ratio separates them 1.0033 → 2.0029 against the `≥ 1.5` bar. `solid_case`'s
  own docstring already excludes `inward_max` from shapes with a bore; the plan
  applied it anyway. No substitute assertion is added, because every candidate
  either passes under the broken mechanism too (the unfilled cavity is also one
  component at the bore) or duplicates `bowl_stays_open`. Inward-facing area is
  gated on real props at step 4, not on a bored synthetic.
- A new `bowl` case (wide-mouth hemispherical shell) must stay **open**, taking
  over the openness guard `vessel` used to carry. Without it, nothing would
  catch a regression that fills every concavity.

### Finding 15's fixture

A thin-walled box whose interior gap is narrower than the band: a 2-cell-thick
slab cavity inside 3-cell walls, so every cavity cell sits within 0.5 cells of
material and is therefore scatter-written (`build_field` activates cubes with
`|f(centre)| < 1.0`, so a cavity thicker than ~2 cells would leave unwritten
cells in its middle and stop being the configuration finding 15 asks for). Under
`fill_enclosed_sdf` line 90 that cavity is unreachable *and* excluded, so it is
never filled; under exposure it is blocked in all 26 directions, so it is. The
fixture is therefore a genuine discriminator between the two mechanisms — which
is precisely what finding 15 says the suite never had (`unreachable ∩
scatter-written` is empty in all 7 existing fixtures, so deleting line 90 left
them bit-identical). The existing cases are kept; they cover the disjoint
configuration.

### `two_crossing_ray_fraction` does not discriminate — escalated, not decided

The predecessor's success bar `tcrf ≥ 0.3` was set from broken_column's 0.105.
All three subjects already sit at 0.415 / 0.41 / 0.28 in **hollow** form, and
the paired runs move them 0.976× / 1.012× / **1.000×**. The decisive datum is
candelabra_shrine: the one subject where the fill genuinely worked (volume
×1.52, 114,208 cells filled) shows `tcrf` 0.28 → 0.28, unchanged to three
digits. The instrument is inert on this subject class — rays through an arch,
a cross or a candelabra legitimately cross 4+ times when the prop is perfectly
solid, so `tcrf` measures silhouette complexity as much as hollowness.

What discriminates, measured today and re-measured in step 4:

1. `interior_tris_removed / raw_tris` — 0.3409 / 0.3608 / 0.3113 hollow, target
   ≤ 0.02. Primary; unchanged from the predecessor and not renegotiated.
2. Clean-mesh `boundary_edge_count` — 14,838 / 1,745 / 949 today. The strip
   currently shreds the mesh; a solid raw makes the strip a no-op and this
   should collapse.
3. Clean-mesh `component_count` — 3,824 / 207 / 22 today, same cause.
4. Raw face-count reduction and trimesh volume ratio from `prop_extract.py`
   (predicate (a)), which measure the extraction directly rather than through
   two downstream stages.

**Ruled by the user 2026-07-29: `tcrf ≥ 0.3` is struck from the success criteria
and kept as a recorded stat; (2) and (3) become gates in its place.** (1) is
untouched. The replacement is not a downward renegotiation — (2)/(3) gate a
defect (a shredded, 3,824-component clean mesh) that nothing currently catches.
The same ruling struck `tcrf ≥ 0.3` from the predecessor plan's Ideal end state,
which step 5 records.

### Scope boundaries

- **No GPU run in this plan.** The end-to-end smoke that would assert the
  manifest `"extraction"` block and `vram.peak_reserved_gib ≤ 8.0` is blocked by
  rework 14 (`check_mesh`'s zero-tolerance degenerate-face gate aborted it over
  2 faces in 768,462). It belongs to rework 14 and is not duplicated here. The
  VRAM bound is unaffected by this change on its face: the sweep runs in numpy
  on host memory, exactly as `fill_enclosed_sdf` and `drop_solid_floaters`
  already do.
- `drop_solid_floaters` is unchanged and still runs immediately after the
  solidification, so component sizes include filled interiors.
- `prop_cleanup.py` is unchanged. Its strip staying in place as a backstop is
  what makes `interior_tris_removed` a valid measurement of the fork-side fix.
- `min_component_fraction` recalibration (step 6 recorded `fragments_removed`
  11 / 5 / 9 surviving into cleanup) stays a recorded candidate, out of scope.
- `prop_extract.py` needs no change; it already drives `fill_interior`.

## Findings (execution order)

### 1. Replace `fill_enclosed_sdf` with a directional-exposure solidification

- **Evidence:** `fork:hi3dgen/representations/mesh/utils_cube.py:74-91` is
  `fill_enclosed_sdf(sdf, coords)`: `ndimage.label` over `sdf > 0`, border
  components stay outside, `enclosed` cells relabel to `-1`, and line 90
  excludes every scatter-written cell. Measured on real 257³ fields it fills 27
  cells on chapel_arch and 0 on crucero against the 690,882 / 283,407 a solid
  interior needs, because their material is a 2.5–3.7-cell sheet fully covered
  by the ~4–5-cell scatter band: nothing is enclosed by the band at all
  (free-space components = 1 on crucero, 5 on chapel_arch with the four
  non-trivial ones 27 cells each). Its only call site is
  `fork:hi3dgen/representations/mesh/cube2mesh.py:380-385`
  (`if self.fill_interior and not training:` → `fill_enclosed_sdf(sdf_d.view(r,
  r, r), v_pos)` → `drop_solid_floaters` → `reshape(-1)`, `r = self.res + 1`).
  Other references: `utils_cube.py:97` (a docstring line in
  `drop_solid_floaters` naming the ordering), and
  `fork:tests/test_interior_fill.py` lines 7, 37, 154, 174, 189, 195. Nothing in
  the vordar repo names the function; `scripts/ai-pipeline/prop_hi3dgen.py:360`
  and `:486` touch only the `fill_interior` attribute, which is kept.
- **Ideal:** `utils_cube.py` carries
  `solidify_hidden_interior(sdf: torch.Tensor) -> torch.Tensor` and no longer
  carries `fill_enclosed_sdf`. The new function computes, for each of the 26
  directions `d ∈ {-1,0,1}³ \ {(0,0,0)}`, the boolean field "the ray from this
  cell in direction `d` leaves the grid without entering `sdf < 0`", ORs them,
  and returns `sdf.masked_fill(~material & ~exposed, -1.0)`. It takes no
  `coords` argument — there is no scatter-written exclusion. `cube2mesh.py:382`
  becomes `sdf_d = solidify_hidden_interior(sdf_d.view(r, r, r))`;
  `drop_solid_floaters` and the `fill_interior and not training` guard are
  unchanged. The docstring states the contract as a constraint (an opening a
  straight line passes through stays open; a cavity no straight line reaches
  becomes solid) with no history, no finding numbers, no "used to be".
- **Gap:** The shipped mechanism cannot see the defect on 2 of 3 hard-topology
  props, and its scatter-written exclusion would actively defeat any replacement
  by leaving a positive rind inside every cavity.
- **Suggestion:** Implement each direction as one exact integer sweep, no
  interpolation. Pick the first non-zero component of `d` as the slab axis `a`
  with sign `s`, move that axis to the front, and iterate the 257 slabs in the
  order that visits `i + s` before `i`. At each slab
  `E[i] = free[i] & shift2d(E[i+s], du, dv)` where `du, dv` are `d`'s other two
  components and `shift2d` fills `True` outside the source bounds (a ray leaving
  the grid laterally has escaped); the first slab visited uses an all-`True`
  source. Work in numpy on the host (`sdf.cpu().numpy()`), matching how
  `fill_enclosed_sdf` and `drop_solid_floaters` already operate, and return via
  `torch.from_numpy(...).to(sdf.device)`. Whole-grid bool arrays are 17 MB each;
  keep one accumulating `exposed` array, not 26.
- **Path:**
  1. Write the function and rewire the call site on `vordar-fixes`; delete
     `fill_enclosed_sdf` entirely and fix `drop_solid_floaters`'s docstring
     reference at `utils_cube.py:97` to name the new function.
  2. Update `fork:tests/test_interior_fill.py`: change the import and the four
     call sites (lines 154, 174, 189, 195) to `solidify_hidden_interior(field)`
     — note the `coords` argument is gone: `solid_case` becomes
     `extract(solidify_hidden_interior(field))`, `case_plate_stack` becomes
     `solidify_hidden_interior(field)`, and the two floater cases become
     `drop_solid_floaters(solidify_hidden_interior(build_field(...)[0]), 1e-4)`.
     `build_field` itself is unchanged and still returns `(field, coords)`;
     step 2 needs `coords`. Rewrite the module docstring's second
     paragraph to state the new contract. Re-target `case_vessel`: rename it
     `case_vessel_seals`, and assert `mesh.volume / analytic_volume(vessel_sdf)
     >= 1.5` — the cavity is claimed. The second assertion this step originally
     carried, `inward_area_fraction(mesh) < 0.05`, is struck; see the Ideal end
     state's `vessel` entry for the measurement that struck it. Leave
     `case_sphere` (`[0.95, 1.10]`,
     inward < 0.01), `case_plate_stack` (`torch.equal`), `case_through_tunnel`
     (`[0.90, 1.15]`), both floater cases and `case_iso_level` untouched in
     their assertions.
  3. Run `C:\tools\Hi3DGen\venv\Scripts\python.exe tests/test_interior_fill.py`
     from the fork root. **Expected: 7/7 PASS.** The assertions that carry this
     step are `vessel_seals` (volume ratio ≥ 1.5 — it was `< 1.30` under the old
     mechanism, so this line is red before the change and green after) and
     `plate_stack` (`torch.equal`, proving laterally-open gaps are still not
     touched). Run the file once **before** step 1.2's edits to record the red
     state: it fails with `ImportError: cannot import name 'fill_enclosed_sdf'`
     once step 1.1 lands.
  4. Outcome branches: if `case_through_tunnel` or `case_plate_stack` fails, the
     sweep's lateral escape handling is wrong (a ray leaving the grid sideways
     must count as escaped) — fix `shift2d`'s fill value, do not relax the
     assertion. If `case_sphere`'s volume ratio exceeds 1.10, the sweep is
     filling exterior cells — check the slab iteration order (`i + s` must be
     visited before `i`). If `case_vessel_seals`'s ratio lands in `[1.3, 1.5)`,
     record the value and proceed; the bore cone legitimately survives and the
     bound is not load-bearing. Do not tune the direction set to move any of
     these.
  5. Commit on `vordar-fixes`, one commit, short message, no attribution
     trailers. The vordar cargo workspace is untouched — no cargo gate applies.

### 2. Harness fixtures for the two contract edges: a wide mouth stays open, an in-band cavity fills

- **Evidence:** `fork:tests/test_interior_fill.py` has 7 cases, all of whose
  sealed cavities lie wholly *outside* the scatter band. Finding 15 measured
  that `unreachable ∩ scatter-written` is empty in every one of them, so
  deleting the scatter-written exclusion left all 7 bit-identical (`sphere`
  1.0002/0.0000, `vessel` 1.0033/0.3916, `through_tunnel` 1.0049/0.1379,
  `plate_stack` 0 relabelled, floaters body=1 / body=2, `iso_level` unchanged).
  On real fields that exclusion was not inert — it cleared 22 / 11 / 28,363
  cells on the three props. After step 1 the exclusion no longer exists, and
  after step 1's re-targeting of `case_vessel` nothing in the suite asserts that
  a *visible* cavity survives. The fixture builder is
  `build_field(sdf, res=96)` at `tests/test_interior_fill.py:105-117` (cells
  with `|f(centre)| < 1.0` become active cubes, corner samples get the
  production `-1/res` bias); helpers `solid_case`, `extract`,
  `analytic_volume`, `inward_area_fraction`, `body_count` are in the same file.
- **Ideal:** Two more cases in `CASES`, 9 total, both driven through the same
  `build_field` → `solidify_hidden_interior` → `extract` chain:
  `bowl_stays_open` and `in_band_cavity_fills`.
- **Gap:** The suite certifies neither that visible concavities survive nor that
  a cavity inside the band is claimed — the two edges the new mechanism is
  defined by, and the second is exactly the blind spot that cost finding 13 a
  wrong hypothesis.
- **Suggestion:** `bowl_sdf`: the `vessel_sdf` shell (`max(d - 30, 24 - d)`)
  with the entire upper half removed — `max(shell, p[..., 2] - CENTER)` keeps
  only `z <= CENTER`, leaving a hemispherical bowl whose r=24 cavity is open to
  the whole +z hemisphere. Note the sign: `vessel_sdf`'s bore uses
  `CENTER - p[..., 2]`, which keeps the *upper* half, so this is the mirrored
  expression, not a copy of it. `in_band_cavity_sdf`: a hollow slab,
  `max(box_sdf(p, (28,28,44), (68,68,52)), -box_sdf(p, (30,30,47), (66,66,49)))`
  — a 40×40×8 outer box around a 36×36×**2** cavity, walls 2 cells laterally and
  3 cells on the thin axis. The cavity being only 2 cells thick is what puts it
  inside the band: every cavity cell lies 0.5 cells from material, so
  `build_field`'s `|f(centre)| < 1.0` activation writes all of them. Analytic
  cell volumes: outer box 12,800, cavity 2,592, shell 10,208 — so a filled
  result is 1.25× the shell, which is what separates the two outcomes.
- **Path:**
  1. Add both SDFs and both cases to `fork:tests/test_interior_fill.py` and
     append them to `CASES`.
  2. `case_bowl_stays_open`: `field, _ = build_field(bowl_sdf)`; extract
     `solidify_hidden_interior(field)`; assert
     `mesh.volume / analytic_volume(bowl_sdf) <= 1.15`. The predicate is the
     volume ratio and the expected value is "at most 1.15" — a mechanism that
     filled the bowl would land near 2.0, since the r=24 half-cavity is
     comparable to the shell itself.
  3. `case_in_band_cavity_fills`: `field, coords = build_field(in_band_cavity_sdf)`;
     assert first that the cavity is genuinely scatter-written — build the
     written mask from `coords` (the same index expression `build_field` feeds
     `get_dense_attrs`) and assert every cell in the index range
     `[31:66, 31:66, 47:49]` is written, so the fixture is the configuration
     finding 15 asks for and not a duplicate of `sphere`. Then extract
     `solidify_hidden_interior(field)` and assert **both**
     `body_count(mesh) == 1` (the inner wall is gone; the unfilled shell
     extracts as 2 bodies) and
     `mesh.volume >= 1.15 * analytic_volume(in_band_cavity_sdf)` (the cavity is
     claimed: filled is 1.25× the shell, unfilled is 1.00×, so the bar sits in
     the middle of a 25% gap).
  4. Run `C:\tools\Hi3DGen\venv\Scripts\python.exe tests/test_interior_fill.py`
     from the fork root. **Expected: 9/9 PASS.**
  5. Outcome branches: if the written-mask precondition fails, the cavity is not
     inside the band — thin the cavity from 2 cells to 1 and re-run; do not
     weaken the precondition, since a cavity outside the band makes the fixture a
     duplicate of `sphere` and re-opens finding 15's blind spot. If
     `in_band_cavity_fills` reports `body_count == 2` with the precondition
     holding, the solidification is not reaching scatter-written cells — check
     that step 1 dropped the exclusion entirely rather than adjusting the
     assertion. If
     `bowl_stays_open` exceeds 1.15, report the measured ratio and **park**: the
     26-direction set is welding a hemisphere-wide opening, which would mean the
     sweep is wrong, not that the bound is too tight.
  6. Commit on `vordar-fixes`, one commit. Vordar cargo workspace untouched.

### 3. Deterministic CPU extraction replays on the three hard-topology props

- **Evidence:** `scripts/ai-pipeline/prop_extract.py` replays
  `SparseFeatures2Mesh` over a saved `cubefeats.pt` on CPU (deterministic where
  the GPU `scatter_reduce` path is not) and prints one JSON stats line
  (`vertex_count`, `face_count`, `volume`, `body_count`, `is_watertight`,
  `fill_interior`, `device`, `elapsed_s`, `cubefeats_sha256`, `out_glb`).
  Latents exist for all three subjects under `target/prop-latents/<name>/`.
  Recorded hollow baselines (`target/prop-solid-validation/summary.json`):
  chapel_arch 773,576 faces / volume 0.0205709; crucero 341,880 / 0.0119119;
  candelabra_shrine 334,942 / 0.0154855. Device-matched hollow CPU replays
  reproduce those to within 0.003% (773,566 / 341,878 / 334,938), so any
  difference measured here is attributable to the solidification alone. The
  previous solid run under `fill_enclosed_sdf` produced 773,414 / 341,766 /
  359,880 — the numbers this step must move.
- **Ideal:** Three fill-on CPU replays with the step-1 mechanism, their JSON
  stats saved under `target/prop-solid-validation/<name>/extract_solid_v2.json`,
  showing a large face-count reduction and a large volume increase on all three.
- **Gap:** Steps 1–2 prove the mechanism on synthetic fields; a fix that rests
  on a structural property of real input has to be measured on real input —
  that is precisely how rework 1 failed at this stage.
- **Suggestion:** Extraction only; no Blender, no GPU. Expect roughly
  1 s of added CPU per subject for the 26 sweeps.
- **Path:** For each of `chapel_arch`, `crucero`, `candelabra_shrine` run
  ```
  C:\tools\Hi3DGen\venv\Scripts\python.exe scripts\ai-pipeline\prop_extract.py ^
    target\prop-latents\<name> --out target\prop-solid-validation\<name>
  ```
  (fill on by default; writes `raw_solid.glb`, overwriting the previous solid
  run — that file is step 4's input). Save each stats line as
  `target/prop-solid-validation/<name>/extract_solid_v2.json`. Then assert, per
  subject, against the recorded hollow baseline:
  - **(a) face-count reduction ≥ 15% and trimesh volume ratio ≥ 1.2.** Expected
    values, pre-registered from this plan's field measurements: chapel_arch
    ~460k–560k faces (30–35% reduction) and volume ratio 2.2–3.2; crucero
    ~195k–245k faces (~36%) and ratio 1.9–2.4; candelabra_shrine ~230k–270k
    faces (20–30%) and ratio 1.4–1.8. Landing inside those bands is the success
    case. A reduction under 15% or a ratio under 1.2 on any subject → **park and
    report** with the numbers; do not adjust the direction set.
  - **(b) upper bound on over-fill, a hard invariant.** Exposure over 26
    directions is a subset of exposure over 6, so the filled interior cannot
    exceed the 6-direction count. Assert `volume` ≤ **0.0698** (chapel_arch),
    ≤ **0.0308** (crucero), ≤ **0.0264** (candelabra_shrine) — the 6-direction
    cell counts (758,977 / 288,055 / 172,745) plus the material, at
    `(1/256)³ = 5.9605e-8` per cell. A value above the bound means the sweep is
    filling exterior space → **park and report**; it is an implementation bug,
    not a calibration question.
  - **(c) `is_watertight == true` and `body_count` recorded.** Marching cubes on
    a modified scalar field is closed by construction, so a `false` here means
    the field was corrupted → **park and report**. `body_count` is recorded, not
    gated (hollow was 17 / 15 / 11).
  - **(d) `elapsed_s` recorded** and compared to the hollow CPU replay's; a rise
    above 10 s attributable to the sweep → record it and note it for rework 5's
    extraction-time gate, do not park.
- **Verify:** the three `extract_solid_v2.json` files exist and every assertion
  above is evaluated in the step's report, each with its measured number. Local
  gate: `python -m py_compile scripts/ai-pipeline/prop_extract.py` (unchanged
  file, sanity only); cargo workspace untouched; no commit to the vordar repo
  beyond the artifacts under `target/` (which is gitignored — the step commits
  nothing).

### 3. Measured 2026-07-29 — one subject parks

Artifacts: `target/prop-solid-validation/<name>/extract_solid_v2.json`.

| | faces (hollow) | reduction | band | vol ratio | band | volume | (b) bound |
|---|---|---|---|---|---|---|---|
| chapel_arch | 754,740 (773,566) | **2.43%** | 460–560k ✗ | **3.214** | 2.2–3.2 ✗ | 0.066115 | ≤0.0698 ✓ |
| crucero | 230,452 (341,878) | 32.59% | 195–245k ✓ | **2.582** | 1.9–2.4 ✗ | 0.030757 | ≤0.0308 ✓ |
| candelabra_shrine | 250,892 (334,938) | 25.09% | 230–270k ✓ | 1.671 | 1.4–1.8 ✓ | 0.025880 | ≤0.0264 ✓ |

**(a)** chapel_arch parks on the 15% face-reduction floor. Two of three subjects
land in band on faces; all three overshoot or top out on volume.

**(b)** never trips — the 26-direction fill stays under the 6-direction bound on
every subject, so there is no step-1 implementation bug of the kind (b) screens
for. Note what (b) cannot do: it bounds the 26-direction fill by the 6-direction
fill, and a sightline test too coarse to see out of a concavity is too coarse in
both sets. (b) passing is not evidence that the filled cells are interior.

**(c) the plan's premise is wrong, and it did not park on it.** "Marching cubes
on a modified scalar field is closed by construction, so a `false` here means the
field was corrupted" — chapel_arch and crucero both come back `is_watertight
false`, but both were **already** `false` on the hollow CPU baseline. The
solidification did not break watertightness; the inputs were never watertight.
Body counts 16 → 3,700 (chapel_arch), 15 → 24 (crucero), 11 → 7
(candelabra_shrine).

**(d)** the sweep costs far more than the ~1 s budgeted: +15.67 s chapel_arch
(38.50 vs 22.83), +7.05 s crucero, +6.96 s candelabra_shrine. Recorded for
rework 5's extraction-time gate per the step's own instruction.

**Open question this raises.** Volume climbing past its ceiling while face count
barely moves and components explode is not the signature of filling a hollow
core. If 26 directions cannot see out of a deep exterior concavity — under an
arch, inside a cross crook — those cells read as hidden and get welded solid, and
chapel_arch is both the most concave subject and the one that overshot. A
direction-count sensitivity probe (26 → 98 → 342, strictly more permissive, so
counts must fall monotonically) settles it: a stable count means the fill is
genuinely enclosed interior, a collapsing one means the direction count is doing
load-bearing work the design does not admit to.

### Direction-count sensitivity: the mechanism has no stable answer

Run 2026-07-29 to settle step 3's open question. Hidden-cell counts under
strictly-nested direction sets (each a superset of the one above, so the
monotone decrease is guaranteed by construction and was observed). The
26-direction column reproduces the shipped `solidify_hidden_interior`
cell-for-cell on all three subjects, and the generalized traversal reproduces
`_escapes_along` exactly on a random grid.

| directions | chapel_arch | crucero | candelabra_shrine |
|---|---|---|---|
| 26 (\|c\|≤1) | 690,882 — 1.000 | 283,407 — 1.000 | 162,787 — 1.000 |
| 98 (primitive \|c\|≤2) | 141,562 — 0.205 | 222,101 — 0.784 | 161,064 — 0.989 |
| 124 (all \|c\|≤2) | 94,857 — 0.137 | 198,370 — 0.700 | 160,564 — 0.986 |
| 316 (primitive \|c\|≤3) | 5,944 — 0.0086 | 16,827 — 0.059 | 134,610 — 0.827 |
| 342 (all \|c\|≤3) | 4,832 — 0.0070 | 14,627 — 0.052 | 127,707 — 0.785 |
| 728 (all \|c\|≤4) | 284 — 0.0004 | 676 — 0.0024 | 41,022 — 0.252 |
| 1330 (all \|c\|≤5) | **7** — 0.0000 | **6** — 0.0000 | 3,473 — 0.021 |

**It does not converge; it collapses.** The limit is zero, not a plateau. A true
straight-line visibility test fills essentially nothing on these props, because
none of them has an enclosed interior cavity of any consequence. The volume the
26-direction sweep claims is almost entirely **exterior concavity** that happens
to block all 26 coarse sightlines — arch undersides, cross crooks.

chapel_arch confirms the prediction exactly: the most concave subject, the one
that overshot its volume ceiling and exploded to 3,700 bodies, is also the most
direction-sensitive — 79.5% of its fill disappears at the first refinement.

**Consequence, and it is larger than this plan.** `solidify_hidden_interior` has
no principled direction count: any value is a tuning knob silently setting how
much exterior concavity gets welded shut, and the correct limit does nothing.
This is not a miscalibration to fix, and step 4 would only measure where one
arbitrary point on a collapsing curve happens to land.

It also generalizes past this mechanism. Rework 1's boundary-reachability flood
and this plan's exposure sweep are both members of one family — *find the
enclosed interior in the SDF grid and fill it* — and the family's premise is now
measured false on real prop fields. The network emits a genuine hollow shell
whose inner wall is real predicted surface, so there is no enclosed region for
any SDF-space criterion to find. Steps 4 and 5 are moot as written; rework 1's
steps 7-8 do not unblock by this route.

### 4. Paired `prop_cleanup.py` runs and the predicate re-evaluation

- **Evidence:** `scripts/ai-pipeline/prop_cleanup.py` strips camera-unreachable
  faces (`strip_interior_faces`, 64 hemisphere rays per face), decimates to
  `--tri-budget 15000`, unwraps with xatlas and prints one JSON stats line
  including `raw_tris`, `interior_tris_removed`, `fragments_removed`,
  `hires_tris`, `clean_tris`, and `geometry_health`'s `is_watertight`,
  `boundary_edge_count`, `euler_number`, `component_count`,
  `two_crossing_ray_fraction`. Recorded hollow-pair values
  (`target/prop-solid-validation/<name>/cleanup_hollow.json`): chapel_arch
  `interior_tris_removed` 263,759 / `raw_tris` 773,574 = 0.3409,
  `boundary_edge_count` 14,833, `component_count` 3,824, `euler_number` 864,
  `tcrf` 0.415; crucero 123,308 / 341,880 = 0.3607, 1,745, 208, 132, 0.41;
  candelabra_shrine 87,596 / 334,942 = 0.2615, 948, 22, −81, 0.28. Registry
  heights (`content/models/assets.json`): chapel_arch 5.497,
  candelabra_shrine 1.3, crucero 3.5. Invocation:
  ```
  & "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" --background ^
    --python scripts\ai-pipeline\prop_cleanup.py -- <in.glb> <out.glb> --height <h>
  ```
- **Ideal:** Six stats lines (the three step-3 `raw_solid.glb` plus the three
  `target/prop-latents/<name>/raw.glb` hollow baselines, re-run so the pair is
  measured by the same code on the same day) under
  `target/prop-solid-validation/<name>/cleanup_{solid,hollow}_v2.json`, plus a
  rewritten `target/prop-solid-validation/summary.json` carrying every predicate
  verdict. This is the artifact trail later docs cite.
- **Gap:** The campaign's primary success metric, `interior_tris_removed /
  raw_tris ≤ 0.02`, is only measurable through `prop_cleanup.py` on real props,
  and the structural damage the strip currently does (14,838 boundary edges,
  3,824 components on a 15,000-triangle mesh) has never been evaluated as a
  gate.
- **Suggestion:** Six Blender CPU runs. The solid runs should be materially
  cheaper than the hollow ones — the strip has ~34% fewer faces to test and far
  fewer of them to delete. Re-running the hollow side rather than reusing the
  recorded numbers costs three runs and removes any same-code doubt.
- **Path:** Run all six, saving each stats line. Then evaluate, per subject:
  - **(1) `interior_tris_removed / raw_tris` on the solid run.** ≤ 0.02 →
    **success**. 0.02–0.05 → **proceed and flag** in the report with the value.
    > 0.05 → inspect the residual before deciding: load the solid `raw_solid.glb`
    and the corresponding `clean_*_hires.glb` in a scratchpad trimesh script and
    report whether the removed faces form a few coherent patches (a real
    surviving inner wall → **park and report**) or are diffuse sub-voxel
    crevices spread over the surface (a roughness artefact → record the measured
    value as the new floor, **proceed**, and tell the predecessor plan's step 8
    to set its gate at `1.5 ×` the worst measured value). Do not change the
    fork's direction set to chase this number.
  - **(2) clean `boundary_edge_count`** — expect a ≥ 90% collapse from 14,833 /
    1,745 / 948. Record the values; a collapse under 50% on any subject is
    reported, not parked (decimation to 15k from ~500k tris can legitimately
    open pinholes).
  - **(3) clean `component_count`** — expect a large collapse from 3,824 / 208 /
    22. Record.
  - **(4) `euler_number`** — record. chapel_arch's through-opening surviving as
    a handle would show as `≤ 0`; 864 today reflects the shredding, so this
    number is only interpretable once (2) and (3) have collapsed. Record, do not
    gate.
  - **(5) `two_crossing_ray_fraction`** — **record only, do not gate.** It moved
    1.000× on candelabra_shrine in the hollow/solid pair where the fill
    genuinely worked, so it carries no signal on this subject class.
  - **(6) `fragments_removed`** — record; > 0 keeps `min_component_fraction` on
    the recalibration list without changing it here.
  - **(7) candelabra_shrine sanity:** if its clean `component_count` collapses
    to 1 while the concept image shows separated arms, note it in the report as
    a welding risk for the downstream turntable review. Do not park on it —
    it is a visual judgement, not a numeric one.
- **Verify:** the six `cleanup_*_v2.json` files exist, `summary.json` is
  rewritten with all seven verdicts per subject, and the step's report states
  each measured number next to its bar. Cargo workspace untouched; nothing under
  `content/` changes; the step commits nothing to the vordar repo (artifacts
  live under the gitignored `target/`).

### 5. Record the outcome and unblock the predecessor plan (docs-only)

- **Evidence:** `docs/reviews/hi3dgen/reworks-hi3dgen-2026-07-28.md`'s queue
  note (lines 49–67) records rework 1 as PARKED at step 6 of 8 with steps 7–8
  blocked on rework 13's predicates, and states direction (i) as "the only
  remaining path". `plan-rework1-solid-interior-2026-07-28.md` carries a design
  decision — "**Sealed-hollow inputs extract solid, accepted.** A cavity with no
  positive path to the grid boundary is indistinguishable from solid interior on
  the grid" — that steps 1–4 supersede, and its step 8's Suggestion still
  instructs a `two_crossing_ray_fraction` gate conditional on all three subjects
  reaching ≥ 0.3.
- **Ideal:** The campaign's living state matches what was measured: the queue
  note records rework 13's outcome with its numbers, rework 15 is struck as
  landed inside step 2, and the predecessor plan's superseded clause and its
  step-8 gate instruction are corrected so steps 7–8 can be executed by a worker
  who reads only that page.
- **Gap:** Steps 7–8 are executed as isolated runs against the predecessor
  document; if it still describes the reachability mechanism and the inert
  `tcrf` gate, the worker implements the wrong thing.
- **Suggestion:** Amend, do not rewrite history: the dated finding text in the
  reworks file's finding 13/15 bodies stays as the record, the queue note and
  the predecessor *plan* (a live instruction set) are updated.
- **Path:**
  1. In `reworks-hi3dgen-2026-07-28.md`'s queue note, replace the rework-1 PARK
     paragraph's last three sentences with the measured outcome of steps 1–4
     (mechanism swapped to directional exposure; the three subjects' face-count
     reductions, volume ratios and `interior_tris_removed` fractions; steps 7–8
     unblocked or still blocked, with the reason).
  2. Strike rework 13 and rework 15 in the queue note per the standing
     convention, naming the commits.
  3. In `plan-rework1-solid-interior-2026-07-28.md`: replace the
     "Sealed-hollow inputs extract solid, accepted" design decision with the
     exposure contract in one sentence and a pointer to this plan; in its
     step 8's Suggestion, delete the conditional `two_crossing_ray_fraction`
     gate and replace it with the step-4-measured `boundary_edge_count` and
     `component_count` gates (stating the measured values the thresholds come
     from); in its Ideal end state, replace the `tcrf ≥ 0.3` clause with the
     same. Leave step 7 (`BAKE_MAX_RAY_DISTANCE_M`) untouched — its derivation
     is independent of which mechanism produced the solid mesh, and it now has
     solid `clean_*_hires.glb` inputs to measure against.
  4. No source file changes, no test. **Verify:** `git diff --stat` touches only
     the two `docs/reviews/hi3dgen/` files; the cargo workspace and every
     `scripts/` file are untouched.
