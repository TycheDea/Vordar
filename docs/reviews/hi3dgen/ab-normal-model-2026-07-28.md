# A/B: normal predictor — turbo vs the full two-stage StableNormal (finding 13, A/B half)

Date: 2026-07-28. Repo at `271020c` (post finding 17). Seed 20260728 everywhere,
`--normal-resolution 1024` (today's default) unless a cell says otherwise.
Same two subjects as the conditioning A/B, so the two studies line up:

- **candelabra** — `target/prop-batch/rebuild/candelabra_shrine/cand_4/concept.png`
- **crucero** — `target/prop-batch/rebuild/crucero/cand_21/concept.png`

Artifacts: `target/ab-normal-model/` (normals, `grid.json`, `grid_probe.json`,
`metrics.json`, `compare_*.png`, `zoom_*.png`) and `target/ab-normal-geo/`
(six end-to-end runs, `mesh_stats.json`).

The fetch/pin/plumb half of finding 13 was already landed: `Stable-X/stable-normal-v0-1`
is in `fork:weights/`, pinned in `models.sha256`, and `--normal-model turbo|full`
+ `--normal-steps` exist. This document is the gated A/B only. No default was
changed — that call is the user's.

## Instrument

Two cross-checks establish that this study is on the same scale as
`ab-conditioning-2026-07-28.md`:

- The turbo cell's `normal.png` is **byte-identical** to that study's
  `candelabra__capped__r1024` (`69076427b0fe…`). Same pipeline, same seeding.
- The angular-difference metric reproduces its published numbers exactly:
  turbo r768 vs r1024 measures 2.52 deg mean / 7.82 p95 (candelabra) and
  2.82 / 6.22 (crucero) against the published 2.5/7.8 and 2.8/6.2.

The detail metrics here are re-implementations (the earlier script was not kept),
so absolute values differ from that report's table; every cell below — including
its r768 reference cells — is measured by this one instrument. Definitions:
grayscale of the normal map, object mask from the conditioning alpha, interior =
6-px erosion, silhouette band = 3-px inner rim, detail pixels = top decile of
conditioning luminance gradient inside the interior; `speckle` = fraction of
interior pixels whose decoded normal sits >20 deg from its own 5x5 median
(a 1–2 px ornament ridge survives a median filter, salt-and-pepper does not).

One caveat on the radial-spectrum ring: measured here, the r768 reference carries
*more* top-octave energy than r1024 on both subjects, the opposite of the
conditioning report's ordering. Both arms denoise at 768 internally (see the
reworks entry below), so that ring is dominated by resample ringing rather than
by signal, and it is not used as evidence in this document.

## Normal maps

| subject | cell | lap energy | grad interior | grad detail px | coarse grad | speckle | normal s |
|---|---|---|---|---|---|---|---|
| candelabra | turbo *(default)* | 0.000137 | 0.01181 | 0.02095 | 0.00875 | 0.0000 | 0.61 |
| candelabra | full, 2 steps | 0.000470 | 0.01207 | 0.02167 | 0.00873 | 0.0003 | 1.35 |
| candelabra | full, 5 steps | 0.000629 | 0.01305 | 0.02273 | 0.00899 | 0.0008 | 2.19 |
| candelabra | full, 10 steps | 0.000634 | 0.01318 | 0.02223 | 0.00908 | 0.0006 | 1.86 |
| candelabra | full, 10 steps @768 | 0.000841 | 0.01427 | 0.02564 | 0.00920 | 0.0016 | 1.82 |
| crucero | turbo *(default)* | 0.000076 | 0.00997 | 0.01702 | 0.00564 | 0.0000 | 0.40 |
| crucero | full, 2 steps | 0.000110 | **0.00554** | **0.00733** | 0.00331 | 0.0000 | 0.89 |
| crucero | full, 5 steps | 0.000519 | 0.00852 | 0.01154 | 0.00426 | 0.0007 | 1.04 |
| crucero | full, 10 steps | 0.001462 | 0.01378 | 0.01734 | 0.00522 | **0.0095** | 1.37 |
| crucero | full, 10 steps @768 | 0.002742 | 0.01738 | 0.02298 | 0.00540 | **0.0229** | 1.36 |

Angular difference from the turbo default, over the object:

| subject | cell | mean deg | p95 deg | mean deg on detail px |
|---|---|---|---|---|
| candelabra | turbo r768 *(prior study's loser)* | 2.52 | 7.82 | 2.31 |
| candelabra | full, 2 / 5 / 10 steps | 11.04 / 14.98 / 15.98 | 31.1 / 42.0 / 43.1 | 19.1 / 25.4 / 26.2 |
| crucero | turbo r768 *(prior study's loser)* | 2.82 | 6.22 | 3.19 |
| crucero | full, 2 / 5 / 10 steps | 10.71 / 12.68 / 17.75 | 23.4 / 33.2 / 51.9 | 9.7 / 12.3 / 18.3 |

The full predictor is not a refinement of the turbo estimate — it is a different
normal field. 11–18 deg mean is 4–7x the 2.5–2.8 deg shift that the conditioning
A/B found sufficient to move crucero's face count by 21.8%.

The step count is the dominant knob, and neither end of it is good:

- **2 steps** (the pipeline's own `default_denoising_steps`) *smooths*. On
  crucero it cuts interior gradient 44% and detail-pixel gradient 57% below
  turbo; `zoom_crucero.png` panel 3 shows the star ornament and aggregate-chip
  field washed out to a near-flat plate.
- **10 steps** buys the energy back as **speckle**, not detail. Crucero's
  isolated-outlier fraction goes 0.0000 (turbo) -> 0.0095, and 0.0229 at r768;
  `zoom_crucero.png` panel 4 and `compare_crucero.png` panel 4 show scattered
  red/green pixel dust and blocky patches over the shaft. The 19x Laplacian
  energy at that cell is that dust.
- **Candelabra is the favourable case.** Speckle stays low (≤0.0016) and the
  gain is real: +11.6% interior gradient, +6% detail-pixel gradient, and in
  `zoom_candelabra.png` the base scroll wires resolve into thin ridges with
  distinct cores where turbo renders them as broad troughs. This is the one
  place the full predictor looks like what finding 13 hoped for.

## Geometry — six end-to-end `prop_hi3dgen.py` runs

Turbo reproduces the conditioning study's r1024 meshes to within the noise floor
(candelabra 124390 verts here vs 124394 published; crucero 103731 vs 103629),
so the geometry stage is unchanged by finding 17 and these rows are comparable.
Reference noise floor: ~0.01% of vertex count.

| subject | cell | verts | faces | area | area/bbox | comps (>200 f) | largest comp | total s | peak VRAM reserved |
|---|---|---|---|---|---|---|---|---|---|
| candelabra | turbo | 124390 | 248828 | 1.3872 | 0.358 | 10 (7) | 170436 | 42.1 | 6.79 |
| candelabra | full s2 | 129579 | 259130 | 1.5108 | 0.389 | 18 (10) | 170698 | 51.7 | **11.73** |
| candelabra | full s10 | 138959 | 277858 | 1.5984 | 0.411 | 18 (18) | 151084 | 55.7 | **11.75** |
| crucero | turbo | 103731 | 207394 | 1.1865 | 0.828 | 28 (2) | 153750 | 40.8 | 6.78 |
| crucero | full s2 | 73239 | 146470 | 0.9695 | 0.739 | 3 (1) | 146454 | 52.2 | **11.75** |
| crucero | full s10 | 56439 | 112850 | 0.6546 | 0.497 | 14 (3) | 111222 | 57.3 | **11.75** |

- **Crucero collapses.** −29.4% faces at 2 steps, **−45.6% at 10 steps**;
  surface area −18% / −45%; rugosity (area/bbox) 0.828 -> 0.739 -> 0.497. The
  arm with the largest angular deviation produced the worst mesh. Four thousand
  times the noise floor — this is not drift.
- **Candelabra gains volume but fragments.** Faces +4.1% / +11.7% and area
  +8.9% / +15.2% read as a win until the component breakdown: significant
  components (>200 faces) go 7 -> 10 -> 18 while the **largest component shrinks
  from 170436 to 151084 faces**. The extra triangles are the body coming apart
  into shells, not ornament resolving on one body.
- No cell gives a mesh that is better on both counts than turbo's.

The likely mechanism, stated as hypothesis: `trellis-normal-v0-1` is conditioned
on the normal distribution the turbo predictor emits, and a 15–18 deg systematic
shift is out-of-distribution input for it. The data is consistent — the ranking
of mesh damage matches the ranking of angular deviation on both subjects — but
nothing here proves it.

## Cost

| | turbo | full (2 steps) | full (10 steps) |
|---|---|---|---|
| model load, per process | 24.9–25.9 s | 32.4–33.3 s | 35.5–37.6 s |
| normal stage | 0.7 s | 1.5 s | 2.1 s |
| total per candidate | 40.8–42.1 s | 51.7–52.2 s | 55.7–57.3 s |
| predictor weights resident | 2.48 GiB | 6.86 GiB | 6.86 GiB |
| process peak reserved | 6.78–6.79 GiB | 11.73–11.75 GiB | 11.75 GiB |
| spill warning | no | **fires** | **fires** |

The full path costs +24% to +40% wall time per candidate — but the headline cost
is VRAM. It puts the process peak at **97.9% of the 12 GiB card**, tripping
`prop_hi3dgen.py`'s spill warning on every run and re-creating exactly the
driver-fallback condition finding 17 removed (which had cost 2445 s on a single
run). The +10 s of load time and part of the +1.4 s normal stage are already
inflated by that spilling; on a card with less headroom, or with ComfyUI
resident, the full path would be far worse than these numbers.

Contributing detail, if the full path is ever revisited: `prop_hi3dgen.py` loads
the normal predictor *before* the BiRefNet matte, so 6.86 GiB of predictor
weights are resident while BiRefNet runs. Measured in isolation, the full
predictor's own normal stage peaks at 9.54 GiB reserved — still under the 90%
warning line. Reordering the load to after `del hi3dgen_pipeline.birefnet_model`
would recover roughly 2 GiB of the peak.

## Recommendation (the call is the user's)

**Keep `--normal-model turbo` as the default.** This is not a
quality-versus-speed tradeoff where the slower option is the better result: on
the subject that carries the most ornament (crucero) the full predictor destroys
44.8% of the surface area, and on the subject where its normal map genuinely
looks sharper (candelabra) the extra geometry arrives as shell fragmentation
rather than as ornament. It is simultaneously 24–40% slower and pushed to 98% of
the card.

**Do not promote it to an opt-in quality flag either — for props, today.** An
opt-in is worth documenting when the slower path is better and the user chooses
when to pay; here the slower path is worse on the measured geometry, so
advertising it would just be a trap. Keep the flag (it costs nothing, the
weights are pinned, and this table is the record of why it is off).

**Two things would make it worth re-measuring**, and both belong to other work
rather than to this A/B: the internal 768 processing-resolution cap (reworks
finding 7 — the full pipeline's refinement never sees more than 768 px, which is
a plausible cause of the speckle at higher step counts), and any change to the
geometry checkpoint's conditioning distribution (rework 1). If the character
pipeline ever gets its own normal bridge, re-run this grid there: cloth folds
are the case finding 13 argued from, and no character concept exists in the repo
to test it on today.
