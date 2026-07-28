# Hi3DGen SLAT sampler A/B — `slat_cfg` x `slat_steps`

Grid run 2026-07-28 against `scripts/ai-pipeline/prop_hi3dgen.py` at HEAD `7c83852`
(the flags/manifest half of finding 11). Twelve runs: `slat_cfg` in {3.0, 5.0} x
`slat_steps` in {6, 12, 25}, on two subjects, fixed seed 1 for every run.
`--ss-cfg` and `--ss-steps` left at their defaults (5.0 / 50) throughout, so the
sparse-structure stage is a constant and every difference below is SLAT-only.

Artifacts live under `target/ab-sampler/` (gitignored): one directory per run
holding `raw.glb`, `normal.png`, `concept_rgba.png`, `hi3dgen_manifest.json`, and
a 4-angle turntable under `tt/`.

## Subjects

| id | concept image | why |
|---|---|---|
| `column` | `target/prop-batch/timed/cand_0/concept.png` | bulky, near-convex, closed silhouette — the easy case |
| `candelabra` | `target/prop-batch/rebuild/candelabra_shrine/cand_4/concept.png` | thin scrolled arms, seven free-standing candles, high genus — the hard case |

**Deviation from the brief.** The brief asked for one prop and one *character*
concept image "already present in the repo's target/ candidate dirs". There is
none. A repo-wide sweep for `concept*.png` returns only prop candidates under
`target/prop-batch/` and `target/prop-latents/`; `target/base-bakeoff/*/view_*.png`
is the image-model bake-off and its subject is the candelabra, not a figure; the
only character asset ever built (`target/char-mpfb/`) came from the MPFB
parametric path (`--mpfb`), which by construction has no concept image, and
`content/models/assets.json` registers no character subject. Producing one would
have meant inventing a prompt and running `workflows/char_concept.json` — new
content, not a measurement. Instead the second subject is the candelabra, the
closest present analogue to a character's failure mode: thin, branching,
free-standing limbs at the resolution limit. **If the character axis matters, it
needs a character concept generated first; this grid does not cover it.**

## Results

Counts and `degenerate_face_count` are read from each run's
`hi3dgen_manifest.json`; watertightness, components, Euler number and
boundary-edge count are measured with trimesh directly on the exported
`raw.glb` (`process=False`, so the loaded counts match the manifest exactly —
they did, in all twelve runs).

### column (broken stone column)

| cfg | steps | verts | faces | degen | watertight | comps | euler | boundary edges | geometry_s |
|---|---|---|---|---|---|---|---|---|---|
| 3.0 | 6  | 176194 | 352384 | 0 | no | 4 | -15 | 34 | 15.72 |
| 3.0 | 12 | 176391 | 352784 | 0 | no | 2 | -22 | 42 | 57.43 |
| 3.0 | 25 | 175811 | 351620 | 0 | no | 2 | -25 | 52 | 36.62 |
| 5.0 | 6  | 175255 | 350502 | 0 | no | 5 | -18 | 44 | 15.12 |
| 5.0 | 12 | 175166 | 350308 | 0 | no | 6 | -9  | 42 | 47.40 |
| 5.0 | 25 | 174704 | 349378 | 0 | no | 3 | -14 | 58 | 28.35 |

### candelabra (wrought-iron candelabra shrine)

| cfg | steps | verts | faces | degen | watertight | comps | euler | boundary edges | geometry_s |
|---|---|---|---|---|---|---|---|---|---|
| 3.0 | 6  | 131800 | 263492 | 0 | no | 20 | -37 | 182 | 31.94 |
| 3.0 | 12 | 131725 | 263342 | 0 | no | 20 | -63 | 234 | 35.57 |
| 3.0 | 25 | 130871 | 261604 | 0 | no | 20 | -43 | 224 | 66.54 |
| 5.0 | 6  | 131992 | 263862 | 0 | no | 21 | -32 | 186 | 15.67 |
| 5.0 | 12 | 132217 | 264298 | 0 | no | 25 | -42 | 220 | 56.07 |
| 5.0 | 25 | 131852 | 263580 | 0 | no | 22 | -58 | 240 | 74.39 |

Winding is consistent in all twelve meshes. Peak allocated VRAM is 10.60 GiB in
all twelve, and every run tripped the script's own spill warning
(11.79 GiB reserved of 11.99 GiB).

Per subject, `normal.png` is byte-identical across all six configs
(`2a943a99e43e...` for column, `ea6d612bc797...` for candelabra), which is the
control this grid needed: the normal bridge is fully seeded and constant, so the
table above isolates the SLAT sampler.

## Renders

Headless turntables via the repo's existing `engine-renderer` `turntable` bin
(`--features offscreen`), run directly on the untextured `raw.glb` — it loads a
material-less, UV-less trimesh export without complaint. Four angles at 512x512
per run:

- per-run frames and contact sheet: `target/ab-sampler/<subject>/slat<cfg>_s<steps>/tt/`
- side-by-side comparison sheets:
  - `target/ab-sampler/grid2_column_frame_00.png`
  - `target/ab-sampler/grid2_candelabra_frame_00.png`
  - `target/ab-sampler/grid_column.png`, `target/ab-sampler/grid_candelabra.png` (front + side, all six configs in one row)

## Reading the results

**1. `slat_steps` above 6 buys nothing and costs 2-5x.** Visually the six
configs are indistinguishable on both subjects: identical silhouette, identical
fluting on the column, the same seven candles and the same scrolled arms on the
candelabra, down to the broken-capital chipping. No config recovered a detail
another lost. Objectively, vert/face counts move by under 1% across the whole
grid and no config produced a single degenerate face.

**2. Where the metrics do move, more steps is mildly *worse*.** Boundary-edge
count rises with steps on both subjects and at both cfg values: column 34 -> 42 -> 52
at cfg 3.0 and 44 -> 42 -> 58 at cfg 5.0; candelabra 182 -> 234 -> 224 at cfg 3.0 and
186 -> 220 -> 240 at cfg 5.0. Nothing is watertight at any setting, so this is a
question of how much open boundary the cleanup stage inherits, and the 6-step
runs inherit the least. Euler number wanders in both directions and carries no
signal at one seed.

**3. `slat_cfg` 3.0 vs 5.0 is a wash, with a faint tilt to 3.0.** At the
6-step column that matters, cfg 3.0 gives 4 components / 34 boundary edges
against cfg 5.0's 5 / 44, and on the candelabra 20 / 182 against 21 / 186 —
3.0 is at least as good on both axes on both subjects. But one component and
four boundary edges at a single seed is inside the noise; treat this as "no
reason to move" rather than "3.0 is better".

**4. `elapsed_s.geometry` is not a clean cost signal here.** Every run sat at
11.79 GiB reserved of a 12 GiB card, so the driver was spilling throughout and
the timings carry that noise: cfg 3.0 / 25 steps (36.6 s) came in *faster* than
cfg 3.0 / 12 steps (57.4 s) on the column. What survives the noise is the floor:
every 6-step run finished its geometry stage in 15.1-31.9 s, and no 12- or
25-step run beat 28 s. Six steps is unambiguously the cheapest tier; the ordering
within the more expensive tiers is unmeasurable at this VRAM pressure.

**5. The quality lever is not in this stage.** With the normal map pinned and the
sparse-structure stage pinned, the SLAT sampler's two knobs move the output by
less than a percent of its geometry and not at all in silhouette. Any further
Hi3DGen quality work should aim at the sparse-structure stage's resolution/steps
or at the normal bridge, not here.

## Recommendation

Keep the shipped defaults: `slat_cfg 5.0`, `slat_steps 6`. Nothing in the grid
justifies paying 2-5x the geometry time for 12 or 25 steps, and the cfg move to
3.0 is not separable from single-seed noise. The value delivered by finding 11's
code half is the *record* — every manifest now states the effective post-merge
sampler params, so this grid is reproducible and the next one can vary the SS
stage instead.

If a second opinion is wanted before locking this in, the cheapest next
experiment is not more of this grid: it is the same six configs at a second seed
per subject, to size the noise floor that currently swallows the cfg comparison.

**The winner is the user's call. `prop_hi3dgen.py`'s defaults were not changed by
this run.**
