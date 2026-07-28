# A/B: normal-predictor resolution and crop source (finding 12, A/B half)

Date: 2026-07-28. Fork at `vordar-fixes` `957f7a3`. Seed 20260728 everywhere.
Subjects (both props; the repo holds no character concept):

- **candelabra** — `target/prop-batch/rebuild/candelabra_shrine/cand_4/concept.png`
  (scrolled arms, free-standing candles, cast-ornament base)
- **crucero** — `target/prop-batch/rebuild/crucero/cand_21/concept.png`
  (incised vine ornament over an aggregate-stone field)

Artifacts: `target/ab-conditioning/` (normals, `grid.json`, `metrics.json`,
`bands.json`, `compare_*.png`, `zoom_*.png`) and
`target/ab-conditioning-geo/` (full geometry runs).

## What changed in the pipeline script

`scripts/ai-pipeline/prop_hi3dgen.py` gained two knobs, both defaulting to
today's behaviour:

- `--normal-resolution` (default 768, the value the fork's Gradio demo passes;
  the predictor's own signature default is 1024).
- `--crop-from-original` (default off) plus `full_res_conditioning_source()`:
  the matte's alpha resampled back onto the untouched full-resolution RGB, so
  `preprocess_image()`'s bbox crop comes from original pixels.

Both are recorded in `hi3dgen_manifest.json` (`normal_resolution`,
`crop_from_original`).

## Axis 2 — crop-from-original: a no-op at today's concept resolution

`matte_concept()` caps the longest side at 1024 before BiRefNet. Every concept
image the pipeline produces is 1024x1024 — `workflows/prop_concept.json` fixes
`EmptySD3LatentImage` at 1024x1024, and all 42 `concept.png` under
`target/prop-batch/` are 1024x1024. At that size `scale = min(1, 1024/1024) = 1`
and the cap never fires, so the crop is already taken from original pixels.

Measured, not just argued — all four cells of subject x crop mode:

| subject | crop mode | cond.png sha256 (12) | normal.png sha256 (12) |
|---|---|---|---|
| candelabra | capped | `dd639d4545a7` | `ea6d612bc797` |
| candelabra | original | `dd639d4545a7` | `ea6d612bc797` |
| crucero | capped | `8c35869296bf` | `23305b052d2b` |
| crucero | original | `8c35869296bf` | `23305b052d2b` |

Byte-identical conditioning images and byte-identical normal maps. A full
geometry run with the flag on (`crucero_r768_croporig`) confirms it end to end:
same `normal_sha256`, mesh differing by 0.06% of faces (170074 vs 170178) —
which is the geometry sampler's own run-to-run noise, since the seed was
identical. That noise floor is worth keeping in mind for the numbers below.

**The axis has no effect until concept generation exceeds 1024 on its longest
side.** The flag is implemented and correct, and becomes live the moment the
concept stage is raised; it is not a knob to spend a decision on today.

## Axis 1 — normal resolution 768 vs 1024

At `resolution=768` a 1024x1024 conditioning image is downsampled to 768,
denoised, then LANCZOS-upsampled back to 1024. At `resolution=1024` the stage
is resample-free (1024 is already a multiple of 64). Mean angular difference
between the two normals over the object: **2.5 deg (candelabra, p95 7.8) /
2.8 deg (crucero, p95 6.2)** — a small but real difference, not noise.

### Naive detail metrics favour 768 — and are misleading

| subject | res | Laplacian energy (interior) | grad @ silhouette band | grad @ detail pixels |
|---|---|---|---|---|
| candelabra | 768 | 0.00756 | 0.1846 | 0.1449 |
| candelabra | 1024 | 0.00517 | 0.1716 | 0.1333 |
| crucero | 768 | 0.00422 | 0.1620 | 0.0985 |
| crucero | 1024 | 0.00265 | 0.1495 | 0.0875 |

("detail pixels" = the top decile of conditioning-image luminance gradient
inside the object — folds, ornament, thin edges.)

### Radial spectrum — invalid as evidence (correction, see below)

Fraction of spectral energy per ring (fraction of Nyquist), Hann-windowed:

| subject | res | 0–.25 | .25–.5 | .5–.75 | **.75–1.0** |
|---|---|---|---|---|---|
| candelabra | 768 | 0.97121 | 0.02104 | 0.00612 | **0.00154** |
| candelabra | 1024 | 0.96187 | 0.02236 | 0.00819 | **0.00605** |
| crucero | 768 | 0.99260 | 0.00560 | 0.00145 | **0.00032** |
| crucero | 1024 | 0.99065 | 0.00563 | 0.00200 | **0.00146** |

**Correction (finding 7, hi3dgen reworks queue):** this section originally
called the top-octave gap "the decisive evidence" for 1024, reasoning that a
768-processed map cannot legitimately occupy that band. That premise is
false — both arms denoise at 768 (`--normal-resolution` selects only the
resample chain wrapped around the same 768 YOSO pass; `processing_resolution`
never reaches the pipeline). So neither arm carries legitimate signal above
0.75 of the 1024-frame Nyquist; the entire top-octave gap above is resample
artifact — LANCZOS overshoot and mask-edge ringing from the 768 arm's two
extra 8-bit PIL resamples, not denoised detail unique to 1024. The numbers in
the table are real measurements and are left as recorded, but they support no
conclusion about which arm resolves more detail. The 0.5–0.75 ring and the
high-detail-ROI numbers below are the same artifact, not corroboration.
(candelabra 0.00643 vs 0.00176; crucero 0.00197 vs 0.00056 — unchanged
figures, same caveat.)

Meanwhile coarse relief is flat: gradient magnitude after a sigma=3 blur is
0.0545 (768) vs 0.0541 (1024) on candelabra and 0.0334 vs 0.0324 on crucero —
under 3% apart. So 768 is not producing *more* relief overall; it is producing
the *same* relief rendered as broader, higher-amplitude troughs, which is what
inflates the Laplacian and gradient numbers above. Softened features with
exaggerated amplitude read as "more detail" to an edge-magnitude metric and as
less detail to a spectrum.

### Visual check on the vocabulary detail

`target/ab-conditioning/zoom_candelabra.png` and `zoom_crucero.png`
(conditioning | 768 | 1024, 3x nearest):

- Candelabra base ornament: at 1024 the scroll wire is a thin ridge with a
  distinct core and the rivet bosses are separate bumps; at 768 the same wires
  are broad soft troughs and the bosses smear into them.
- Crucero: at 1024 the aggregate stone chips resolve as individual bumps and
  the incised vine lines are one-to-two pixels narrower; at 768 the chip field
  is a mottled wash.

### Cost

No measurable difference. Normal-stage wall time over the 8 grid cells:
768 mean **4.02 s** (4.49 / 4.04 / 3.84 / 3.69), 1024 mean **4.14 s**
(4.58 / 3.69 / 4.52 / 3.77) — 3%, inside a ±0.5 s spread that includes the
first-cell warmup. The 1-step YOSO-turbo model is dominated by fixed overhead,
not pixel count, so the 1.78x pixel increase does not show up in the clock.
Peak VRAM was identical across every full run (10.60 GiB allocated /
11.79–11.81 GiB reserved).

## Geometry, winner vs current default

Full `prop_hi3dgen.py` runs, seed 20260728, `--normal-resolution` 768 vs 1024:

| subject | res | verts | faces | surface area | area / bbox area | total s |
|---|---|---|---|---|---|---|
| candelabra | 768 | 126099 | 252242 | 1.4173 | 0.366 | 64.4 |
| candelabra | 1024 | 124394 | 248832 | 1.3871 | 0.358 | 58.4 |
| crucero | 768 | 85103 | 170178 | 1.0396 | 0.771 | 56.5 |
| crucero | 1024 | 103629 | 207186 | 1.1862 | 0.828 | 42.6 |

- **crucero: 1024 is a clear win.** +21.8% faces and +14.1% surface area at the
  same marching-cubes resolution, +7.4% rugosity — the aggregate chips became
  geometry instead of being smoothed away. This is 350x the 0.06% sampler noise
  floor measured above. Component count rises 8 -> 27, but the 19 extra pieces
  total ~200 faces of 8–16-face dust; the two real components both grew
  (120214 -> 153618 and 49916 -> 53360).
- **candelabra: neutral.** −1.4% faces, −2.1% area — within the range the
  sampler's own nondeterminism plus a 2.5 deg normal shift can produce. Slight
  structural edge to 1024: 7 components over 200 faces instead of 9, main body
  167970 -> 170416, i.e. marginally less fragmentation of a shape whose parts
  do touch.
- Geometry wall time was *lower* for 1024 in both subjects (58.4 vs 64.4;
  42.6 vs 56.5), which is sampler-side variance, not an effect of the normal
  stage.

## Recommendation (the call is the user's)

**Set `--normal-resolution` default to 1024.** The predictor still denoises
at 768 in both arms (see the radial-spectrum correction above), so the
top-octave energy gap is not the reason. The reason that holds: 1024 is the
strictly cleaner resample chain around the same 768 denoise — one float,
pre-quantization resample each way instead of 768's two lossy 8-bit LANCZOS
passes (including an upsample of an already-quantized, hard-masked map) — at
identical cost. That chain difference is what the zoom panels and the
angular-difference numbers in Axis 1 actually show: it visibly resolves the
thin ornament and stone-chip detail the dark-fantasy vocabulary is built on,
it produces materially more geometric relief on the ornament-heavy subject
and no regression on the other, and it costs nothing measurable in time or
VRAM. For future normal-map comparisons, use the angular-domain suite (mean/
p95 angular difference, detail-pixel angular gradient, speckle fraction) as
the instrument, not the radial spectrum — it is invalid whenever the two
arms being compared share a denoising resolution but differ in resample
chain, which is the situation here.

**Leave `--crop-from-original` off.** It is provably a no-op while concepts are
generated at 1024x1024. Revisit it in the same breath as any decision to raise
concept resolution — at which point it stops being free and starts being
required.
