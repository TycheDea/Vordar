# Rendering & Graphics Audit — 2026-07-16

First audit of this domain. Scope: `smirk/engine-renderer` (all pipelines,
WGSL, glTF import, egui), animation/skinning, and the game/client rendering
surface, judged against the locked AA semi-realistic dark-fantasy bar
(`docs/visual-quality.md`).

## Ideal end state

One frame is: culled draw lists → shadow cascade(s) → opaque PBR lit by sun +
IBL + local accent lights → sorted transparency → particles → bloom/tonemap →
UI, every pass metered, no per-frame allocation or synchronous IO anywhere on
the hot path. Every shader constant has exactly one definition; every texture
reaches the GPU in the right color space and a compressed format; a zone
change or a first-sighted asset never stalls a frame. The image reads
Diablo-IV-register at dusk: torch/ember/magic light actually falls on
surroundings, fog owns the horizon including the sky, and masked cutouts,
smoke overlaps, and shadow edges are free of the artifacts that scream
"engine demo".

## Findings (implementation order)

> **Cross-type queue** (mirrored in `reworks-rendering-2026-07-16.md`):
> **~~finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 → rework 1 → finding 8 → finding 9 → finding 10 → rework 2 →
> rework 3 → rework 4 → rework 5 → rework 6~~.**
> Findings 1–10 all done 2026-07-16 (one commit each, loop-final gate
> 340/340; finding 7's worker filed rework 7 — per-environment bake-pipeline
> recompilation — during its bounded probe, absorbed by rework 2 steps 2–3).
> Rework 1 done 2026-07-16 (plan-rendering-rework-1-2026-07-16.md, 4 steps; a
> capped 16-entry point-light array rides the existing LightUniform, `shade_pbr`
> loops it with windowed inverse-square falloff, the `PointLight` component +
> `PointLightSyncSystem` extract nearest-16 with flicker, and the portal
> prefab emits — loop-final gate 347/347). Rework 2 done 2026-07-16
> (plan-rendering-rework-2-2026-07-16.md, 7 steps; asset streaming and
> environment-load optimization; rework 7 absorbed). Rework 3 done 2026-07-16
> (plan-rendering-rework-3-2026-07-16.md, 4 steps; sorted per-primitive
> blending for order-independent transparency — intersecting transparents and
> particle-vs-glass ordering remain approximate; loop-final gate 360/360).
> Rework 4 done 2026-07-16 (plan-rendering-rework-4-2026-07-16.md, 9 steps;
> a texture-memory meter and VQ-C5 content-lint land first, `bake_textures.mjs`
> transcodes shipped material maps to committed BC7/BC5 DDS sidecars,
> `TextureSource` carries compressed images through the importer/ground-loader/
> store with a shared shader z-reconstruct for BC5 normals, and close-out adds
> sidecar freshness lint plus a compressed-aware budget assert (138.0 MB
> measured on current content, was ≈300 MB RGBA8-estimated) — loop-final gate
> 377/377). Rework 5 done 2026-07-16 (plan-rendering-rework-5-2026-07-16.md,
> 7 steps; SDF used-run draws, upload-time AABBs, camera + sun-volume culled
> draw lists over one instance buffer, half-rate pose LOD beyond 40 u,
> `frustum_classify_552` baseline — loop-final gate 393/393). Rework 6 done
> 2026-07-17 (plan-rendering-rework-6-2026-07-16.md, 7 steps; height fog in a
> shared fog snippet, geometric specular AA in the PBR core, three concentric
> texel-snapped shadow cascades, depth-prepass SSAO multiplying the IBL
> ambient — TAA deliberately deferred in favor of GSAA; loop-final gate
> 400/400). Every rework in this report is closed.
> Finding 1 first: findings 3, 4, 9 and rework 1 all edit shader lighting
> code, and landing them against three divergent copies multiplies every
> diff by three. Finding 2 second: it is the measurement instrument findings
> 7/9 and rework 2 use to prove their wins. Rework 1 sits after finding 7 by
> dependency (the lighting loop should land once, in finding 1's shared PBR
> source) and before findings 8–10 by impact — accent lighting is the single
> largest visible gap to the art-direction bar. Rework 2 after findings 6+7:
> mesh-store eviction and the once-only BRDF bake shrink exactly the work
> streaming has to move off-frame. Numbers are stable — findings are never
> renumbered.

### 1. One source of truth for the shared WGSL (PBR core, shadow sampling, uniform structs)

- **Evidence:** the Cook-Torrance block (`d_ggx`/`g_smith`/`f_schlick`/
  `shade_pbr`) is pasted three times: `shader.wgsl:166-222`,
  `mesh_shader.wgsl:119-178`, `skinned_mesh_shader.wgsl:130-186`.
  `shadow_factor` is pasted three times (`shader.wgsl:29-45`,
  `mesh_shader.wgsl:31-49`, `skinned_mesh_shader.wgsl:32-48`), each with a
  hardcoded texel `1.0 / 2048.0` that silently desyncs from
  `shadow::SHADOW_SIZE` (`shadow.rs:11`). `PREFILTER_MAX_MIP: f32 = 4.0` is a
  literal in all three shaders, tied by comment only to
  `ibl::PREFILTER_MIPS = 5` (`ibl.rs:16`). The `Camera`/`LightUniform` struct
  declarations exist four times. Drift is not hypothetical: `shader.wgsl`'s
  `shade_pbr` already lost the `emissive` parameter the other two carry.
- **Ideal:** each function and constant exists once; the three geometry
  shaders assemble from shared snippets at build time; renderer-side constants
  (`SHADOW_SIZE`, `PREFILTER_MIPS`) are injected, not repeated.
- **Gap:** every lighting change is a three-file edit with no guard against
  missing one; two cross-language constants can rot silently.
- **Suggestion:** a ~40-line `build.rs` preprocessor: resolve
  `//#include "snippets/pbr_common.wgsl"` lines and `//#const NAME` markers
  into `OUT_DIR` copies; load with
  `include_str!(concat!(env!("OUT_DIR"), ...))` instead of `include_wgsl!`.
  No new dependency (skip naga_oil — includes + const injection is all that
  is needed).
- **Path:**
  1. Add the build.rs preprocessor + `src/snippets/` with `pbr_common.wgsl`,
     `shadow_sample.wgsl`, `scene_uniforms.wgsl`; port the three geometry
     shaders (restoring the emissive parameter uniformly — `shader.wgsl`'s
     SDF pass passes `vec3(0.0)`, keeping its over-1 instance-color emissive
     handling outside `shade_pbr` as today).
  2. Inject `SHADOW_SIZE` and `PREFILTER_MAX_MIP` from the Rust constants.
  3. Test: a unit test that parses each generated shader with `naga`'s WGSL
     front end (already in the tree via wgpu) and asserts the generated text
     contains no `2048` literal; the offscreen suite must stay green
     unchanged (same image, same analytic assertions).

### 2. Per-pass GPU timing in the dev overlay

- **Evidence:** `gpu_timer.rs:20-24` allocates exactly 2 timestamps;
  `frame.rs:243-266` brackets shadow-begin → tonemap-end, so the overlay
  shows one aggregate "gpu: N ms" line. Shadow, main, particles, bloom chain,
  tonemap, and egui are indistinguishable.
- **Ideal:** the overlay lists per-pass milliseconds (shadow / main /
  particles / bloom+tonemap / egui) at the same sparse cadence
  (`GPU_TIMING_INTERVAL`, `frame.rs:34`), still `None`-safe without
  `TIMESTAMP_QUERY`.
- **Gap:** VQ-F1 regressions can't be attributed; findings 7/9 and rework 2
  have no instrument to prove their wins.
- **Suggestion:** widen the query set to 6 pairs, give each recorded pass its
  `timestamp_writes`, resolve once, publish one overlay line per pass.
- **Path:**
  1. Generalize `GpuTimer` to N labeled pairs (begin/end per pass).
  2. Wire each pass in `frame.rs`; keep the single blocking read per interval.
  3. Verify: existing offscreen suite green; manual check that the overlay
     lists per-pass lines (headless can only prove compilation + no panic —
     say so in the report).

### 3. Fog never touches the sky — hard horizon seam

- **Evidence:** the three geometry shaders end with exponential distance fog
  (`mesh_shader.wgsl:213-218`); the sky pass returns the raw cubemap
  (`sky.wgsl:34-41`). With `fog_density > 0` (zones opt in via `set_fog`,
  `presentation.rs:71`) distant ground converges to `fog_color` while sky
  pixels one row above stay unfogged — a hard silhouette seam exactly where
  VQ-A5's "depth fog" is supposed to sell distance.
- **Ideal:** the sky blends toward the fog color near the horizon with an
  elevation-based factor, converging to the ground's far-distance fade, and
  stays clear at the zenith.
- **Gap:** the seam is fully visible in every fogged zone.
- **Suggestion:** the sky pipeline already binds `camera_bgl` (group 0), which
  carries the light/fog uniform at binding 1 (`camera.rs:237-262`) — read it
  in `sky.wgsl` and mix:
  `mix(sky, fog_color, saturate(exp(-max(dir.y, 0.0) * H)) * density_on)`
  with `H` tuned so the blend dies out by ~15° elevation and `density_on =
  saturate(fog_density * K)` so density 0 keeps today's image bit-stable.
- **Path:**
  1. After finding 1, add the fog read + horizon blend to `sky.wgsl`.
  2. Offscreen analytic test: render sky-only with high density — bottom rows
     within a small distance of fog color, zenith rows unchanged vs a
     density-0 render; density 0 identical to today.
  3. Add "horizon seam gone in fogged zones" to the manual feel-checklist.

### 4. Masked cutouts alias despite 4× MSAA — no alpha-to-coverage

- **Evidence:** glTF MASK (and BLEND, approximated as MASK —
  `gltf_import.rs:31-34`) resolves via `discard`
  (`mesh_shader.wgsl:192-196`, `skinned_mesh_shader.wgsl:199-204`), and both
  mesh pipelines leave `MultisampleState` at default
  `alpha_to_coverage_enabled: false` (`mesh_pipeline.rs:154`,
  `skinned_pipeline.rs:126`). `discard` kills the whole fragment, so cutout
  edges (foliage, hair cards, fences) stay hard-stepped no matter the sample
  count — the exact artifact VQ-D4's MSAA exists to remove.
- **Ideal:** masked materials write a sharpened alpha and let per-sample
  coverage anti-alias the cutout edge.
- **Gap:** the one tool MSAA gives cutouts is switched off.
- **Suggestion:** enable `alpha_to_coverage_enabled: true` on the mesh and
  skinned pipelines; replace the hard discard with
  `alpha = saturate((a - cutoff) / max(fwidth(a), 1e-4) + 0.5)` written to
  the output's alpha channel for `cutoff > 0` materials (keep a discard at
  `alpha <= 0` to preserve fully-transparent texels), and write alpha 1.0 on
  the opaque path so nothing else changes. SDF and particle pipelines
  untouched.
- **Path:**
  1. After finding 1, land the shader + pipeline flags.
  2. Offscreen analytic test in the VQ-D4 pattern: a diagonal masked edge must
     produce intermediate resolved pixels (fail-first against today's
     hard-discard edge).

### 5. Alpha-blended particles composite in pool order, not depth order

- **Evidence:** `fill_draw_list` (`client/vordar-client/src/vfx.rs:193-218`)
  emits the additive partition then the alpha partition in raw pool order;
  the alpha pipeline blends premultiplied `One / OneMinusSrcAlpha`
  (`particle_pipeline.rs:171-182`). Additive is order-independent, but two
  overlapping smoke puffs (`ParticleBlend::Alpha`, VQ-E3's media class)
  composite differently depending on spawn order and pop when pool order
  changes (swap-remove on expiry).
- **Ideal:** the alpha partition draws back-to-front by view depth every
  frame.
- **Gap:** wrong compositing plus temporal popping on exactly the smoke/dust
  effects the alpha variant exists for.
- **Suggestion:** add `pub fn camera_eye(resources) -> Vec3` to the facade
  (sibling of `camera_yaw`, `facade.rs:233-238`); extend `fill_draw_list` to
  take the eye and sort the alpha slice by descending
  `distance_squared(eye)` before pushing.
- **Path:**
  1. Facade getter + `fill_draw_list(particles, eye, out)`.
  2. Unit test: two alpha particles at different depths land far-first
     regardless of pool order (fail-first with today's insertion order).
  3. Re-run the `particle_fill` bench (`benchmarks/benches/render_cpu.rs`) —
     a 4096-element partial sort must stay in budget; record the delta.

### 6. `MeshStore::register` leaks the mesh it replaces

- **Evidence:** `store.rs:160-175` — re-registering a key pushes a new
  `GpuMesh` and repoints `by_path`, but the old entry stays in `meshes`
  forever (indices must remain stable, so nothing can ever be removed).
  `ZoneDressingSystem` re-registers `zone-ground:{zone}` on every zone
  *change* (`presentation.rs:47-99`), so walking start → east → start leaks a
  full ground set — vertex+index buffers plus five mipped textures; the
  mud_leaves 2k set alone decodes to ~65 MB of GPU memory. VQ-F3 forbids
  exactly this shape of unbounded per-entity GPU growth.
- **Ideal:** re-registering a key replaces the `GpuMesh` in place; wgpu frees
  the old buffers/textures on drop; indices stay stable by construction.
- **Gap:** every zone crossing after the first burns a ground set of VRAM.
- **Suggestion:** in `register`, when `by_path` already maps the key to
  `Some(idx)`, do `self.meshes[idx] = upload_mesh(...)` and return `idx`.
- **Path:**
  1. Land the in-place replace.
  2. Offscreen-gated test (HeadlessGpu, skips without adapter): register the
     same key twice, assert `meshes.len()` did not grow and the returned index
     is stable (fail-first: today it grows).

### 7. The BRDF LUT is re-baked for every environment, including the throwaway startup one

- **Evidence:** `Environment::from_equirect_pixels` bakes the split-sum BRDF
  LUT (`ibl.rs:163-174`) on every call — but the LUT is a pure function of
  (NdotV, roughness) (`ibl.wgsl:155-183`); no environment data enters it.
  It is baked at startup for `default_gray` (`state.rs:273`) and again on
  every `set_environment` zone change (`facade.rs:120-133`,
  `presentation.rs:64`): 512×512 pixels × 512 importance samples of redundant
  GPU work per zone crossing, inside the synchronous bake that already stalls
  the frame.
- **Ideal:** the LUT bakes once at renderer init and is shared by every
  `Environment`.
- **Gap:** the most expensive single bake pass runs N times for one result.
- **Suggestion:** hoist the LUT (texture + view) out of `Environment` into
  `RendererState` (bake once in `RendererState::init`); `Environment` binds
  the shared view. Note for rework 2's plan: the prefilter bake samples the
  base cube at level 0 only (`ibl.wgsl:137`) — rough mips of a 512² base with
  256 samples will firefly on high-frequency HDRIs; fixing that belongs to
  the async-bake design pass, not here.
- **Path:**
  1. Move the LUT bake to init; thread the shared view into
     `Environment::from_equirect_pixels` (offscreen harness constructs
     environments too — update `offscreen.rs`).
  2. Verify: offscreen white-furnace/sky tests stay green; measure the
     `set_environment` wall time before/after (log line) and record it.

### 8. `load_dds` ignores color space — BC7 albedo decodes as linear

- **Evidence:** `texture.rs:56` always creates `Bc7RgbaUnorm`; the shipped
  `content/textures/ground/floor_tile/*.dds` are color (albedo) data, which
  is sRGB-encoded by every BC7 authoring tool's default. Sampled as Unorm,
  sRGB bytes enter the linear HDR pipeline undecoded → washed-out midtones,
  then the swapchain re-encodes. This is a direct VQ-C2 violation on the DDS
  path. Mitigating: `facade::load_texture` (`facade.rs:202-216`) currently
  has no game/client caller, so the defect is latent, not visible.
- **Ideal:** the loader picks `Bc7RgbaUnormSrgb` vs `Bc7RgbaUnorm` per the
  slot's color-vs-data nature, like `create_rgba_texture` already does with
  its `srgb` flag (`texture.rs:148-156`).
- **Gap:** the first real use of the DDS path ships wrong colors with no
  error.
- **Suggestion:** add an `srgb: bool` parameter to `load_dds` and
  `load_texture`; extract the format pick into a pure
  `fn dds_format(srgb: bool) -> TextureFormat`.
- **Path:**
  1. Land the parameter (color = true default at the facade).
  2. Unit test on `dds_format` (both arms); grep-level check that no caller
     passes untyped `false` for color content.

### 9. Bloom thresholds in pre-exposure space — the day/night exposure seam breaks "what glows"

- **Evidence:** the bloom prefilter thresholds raw scene values at
  `THRESHOLD = 1.0` (`bloom.rs:11`, bound at `bloom.rs:158`), but exposure is
  applied later, in the tonemap (`tonemap.wgsl:45`:
  `aces((hdr + bloom) * post.exposure)`). `set_exposure`
  (`facade.rs:135-140`) is the documented day/night seam (VQ-D5): at exposure
  0.5, a 1.5-raw emissive displays at 0.75 — dimmer than white — yet still
  blooms; at exposure 2.0, a 0.8-raw ember that displays at 1.6 doesn't
  bloom at all. VQ-C3's "HDR emissive > 1.0 blooms" silently becomes ">1.0
  before exposure", which no artist authors against.
- **Ideal:** the threshold is display-referred: prefilter on
  `hdr * exposure`, tonemap composites `aces(hdr * exposure + bloom)`.
- **Gap:** latent today (no caller drives exposure ≠ 1 yet) but it corrupts
  the exact seam day/night is designed to use.
- **Suggestion:** make the prefilter params buffer `COPY_DST`, write exposure
  into its spare slot from `TonemapPass::set_exposure` (or a small shared
  params buffer), multiply in `prefilter_frag`, and drop the exposure
  multiply on the bloom term in `tonemap.wgsl`.
- **Path:**
  1. After finding 1, land the plumbing + shader change.
  2. Offscreen analytic test (fail-first): at exposure 0.5, a 1.5-raw
     emissive quad produces no bloom halo; at exposure 1.0 behavior is
     unchanged vs today's captures.

### 10. `pose_player` allocates five buffers per skinned entity per display frame

- **Evidence:** `sync.rs:17-61` — every frame, per skinned instance:
  `sample_pose` collects a fresh `Vec<LocalTransform>` (`anim.rs:148-169`),
  a blend collects another (`anim.rs:174-183`), `global_transforms` allocates
  two more (`anim.rs:191-199`), the palette collects a fifth, and an active
  crossfade clones the previous clip's name `String` (`sync.rs:35-38`).
  At the VQ-F1 stress figure (40 rigs × 64 joints) that is ~200 heap
  allocations per display frame from one system — churn the
  `joint_palette_40x64` bench (`benchmarks/benches/render_cpu.rs:57`)
  measures directly.
- **Ideal:** steady-state posing reuses scratch buffers; zero allocations per
  frame once warmed.
- **Gap:** allocation churn on the hottest per-entity CPU path the enemy
  influx will multiply.
- **Suggestion:** out-parameter variants (`sample_pose_into`,
  `global_transforms_into`, palette-into) fed by a scratch struct owned by
  `MeshRenderSyncSystem`; resolve the prev-clip by reference instead of
  cloning the name (the borrow conflict is local — restructure to look up
  the clip index first).
- **Path:**
  1. Add `_into` variants (keep the allocating wrappers delegating to them so
     existing tests/benches stay valid).
  2. Rewire `pose_player`; kill the `String` clone.
  3. Bench before/after (`joint_palette_40x64`); record the delta in
     `docs/benchmarks/BASELINE.md`.

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run.
