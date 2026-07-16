# Plan: Image-quality ladder — CSM, SSAO, specular/temporal AA, richer fog — 2026-07-16

Source: docs/reviews/rendering/reworks-rendering-2026-07-16.md finding 6.

## Ideal end state

The dusk image gains its last polish tier over the now-complete PBR + accent-light
+ culling foundation: **height fog** layered on the existing distance term so low
ground reads hazy while rooftops stay clear; **geometric specular anti-aliasing**
in the shared PBR core so normal-mapped surfaces stop shimmering in motion;
**cascaded shadow maps** — three concentric, texel-snapped cascades around the
camera focus — so contact shadows at the player's feet are crisp while distant
geometry still casts; and a **half-resolution SSAO term** multiplying only the IBL
ambient so props and characters ground into corners. Every item lands as its own
fix-sized diff with a headless analytic test, leaves the workspace green, and
reproduces today's image exactly when its parameters are at their neutral
defaults. TAA is deliberately **not** built (see Design decisions).

## Design decisions

- **This rework is four independent sub-ladders, sequenced — not one monolith.**
  The finding says "ordered last; plan the items individually." Fog, specular AA,
  CSM, and SSAO share only the shared shading/ambient/shadow code; none depends on
  another. They are ordered here cheapest-and-least-invasive first (fog →
  specular AA → CSM → SSAO → docs), each a standalone green landing. The user may
  stop after any step; adopting a subset is coherent. Rework 1 (accent lights) and
  rework 5 (culling) — the layers the finding wanted below this one — are already
  done, so this can proceed.

- **TAA is deferred; specular AA is done in the shader (geometric specular AA),
  not Toksvig-at-import.** The finding's suggested path — "Toksvig (roughness
  filtering) at import" — *fights this codebase*, which is information, not a
  detail to shim around. Rework 4 already transcodes every shipped material map to
  BC7/BC5 with **baked mips committed to disk as DDS sidecars**; a BC5 normal map
  cannot be re-mipped at runtime, and the generic `mipgen.rs` blit has no access to
  the sibling normal map while generating a roughness map's mips. Toksvig-at-import
  would therefore have to live in the content pipeline (`bake_textures.mjs`,
  another domain) *and* still leave the RGBA8 fallback path inconsistent. The
  in-domain, simpler, universal answer is **geometric specular AA** (Tokuyoshi–
  Kaplanyan / Karis): estimate the shading normal's screen-space variance with
  `dpdx/dpdy(N)` in the fragment and clamp roughness up before the specular BRDF.
  It lives in one place (`snippets/pbr_common.wgsl`), works for every material
  regardless of compression, needs no import step or new pass, and is exactly zero
  on flat surfaces (regression-safe). TAA is skipped entirely: MSAA 4× already
  resolves geometry edges, TAA needs per-pixel motion vectors the forward pipeline
  does not produce and conflicts with the MSAA investment (the finding itself
  says "TAA only if still needed"). Rejected: Toksvig-at-import (cross-domain, BC
  wall), TAA (motion-vector infrastructure for a shimmer GSAA already fixes).

- **CSM uses three concentric texel-snapped cascades around the camera focus,
  selected by projection containment — not PSSM view-frustum splits.** The camera
  is a bounded orbit (radius 16–55) over compact zones, already fitted around
  `camera.target` (shadow.rs:49-75). Concentric ortho boxes at half-extents
  ≈ [24, 48, 80] centered on the focus give a tight near cascade (~2.3 cm/texel at
  2048²) for crisp contact shadows and a loose far cascade (~7.8 cm/texel, today's
  density) for coverage. The receiver picks the smallest cascade whose projected
  UV lands inside `[margin, 1-margin]` — no split-distance tuning, robust in every
  projection mode. Rejected: PSSM/view-frustum splits (more machinery, and the
  orbit frustum is already small; concentric-around-focus matches how the single
  cascade already behaves).

- **The shadow array stays inside scene bind group 0 (bindings 2–4).** The skinned
  pipeline already uses the default max of 4 bind groups (camera.rs:228-230
  comment), so CSM must not add a group. It doesn't: binding 2 becomes
  `array<mat4x4, CASCADE_COUNT>` (receiver light-VPs), binding 3 becomes
  `texture_depth_2d_array`, binding 4 (comparison sampler) is unchanged. Casting
  reads its per-cascade matrix through a **separate** 256-byte-stride uniform with
  a dynamic offset (uniform array stride can't be both tight-64 for the receiver
  and 256-aligned for a dynamic offset in one buffer). `CASCADE_COUNT` is injected
  into WGSL by build.rs from `shadow.rs` via the existing u32-const machinery
  (identical treatment to `MAX_POINT_LIGHTS`), so the two sides can never drift.

- **SSAO is forward-friendly: a full-res depth prepass feeds a half-res AO pass
  whose result multiplies only the ambient term in `shade_pbr`.** Forward shading
  computes ambient inline, so AO must exist *before* the color pass — hence a
  camera-space depth-only prepass (new depth-prepass pipelines, mirroring the
  shadow pipelines but bound to the camera group). The SSAO pass reconstructs
  view position and normal *from depth alone* (no normal G-buffer), samples a fixed
  hemisphere kernel, and a box blur denoises it. The scene shaders sample the AO
  texture by screen UV and multiply `ambient` only (never direct light — that would
  be wrong). AO defaults to a 1×1 white texture (AO = 1 → no change), so the
  feature is neutral-by-default and every existing test stays green; the offscreen
  harness opts in per-test via `set_ssao(true)` exactly as it opts into `draw_sky`.
  Rejected: MRT ambient-separation (bigger pipeline change), post-hoc AO on final
  color (darkens direct light too).

- **New passes carry no GPU-timer brackets.** `gpu_timer.rs` has a fixed 6-pass
  timestamp set; the depth-prepass, SSAO, blur, and extra shadow-cascade passes
  record with `timestamp_writes: None`. Dev-overlay timing (shadow/main/particles/
  bloom+tonemap/egui) is unchanged; per-pass timing of the new passes is out of
  scope (dev-only, addable later without touching these diffs).

- **Neutral-default parameters reproduce today's image bit-for-bit.** Height fog
  falloff defaults 0 (→ pure distance fog), GSAA is 0 on flat normals, CSM at
  `CASCADE_COUNT = 1` is the current single fitted cascade, SSAO defaults to white
  AO. Each step's regression proof is that an existing offscreen test still passes
  unchanged.

- **Open product/prioritization note (recommendation, not a blocker):** whether to
  ship all four items or a subset is a prioritization call. Recommendation: land
  them in order and stop wherever the visible payoff plateaus for the current dusk
  zones — height fog and CSM give the largest dark-fantasy read; SSAO is the
  subtlest and costs the most machinery. The plan sequences all four; no step
  depends on a later one.

## Findings (execution order)

### 1. Height fog in a shared fog snippet

- **Evidence:**
  - `apply_fog` is duplicated verbatim in three shaders —
    `smirk/engine-renderer/src/mesh_shader.wgsl:126-131`,
    `smirk/engine-renderer/src/shader.wgsl:116-121`,
    `smirk/engine-renderer/src/skinned_mesh_shader.wgsl:138-143` — each computing
    `t = 1 - exp(-light.fog_density * dist)` and `mix(color, light.fog_color, t)`.
    This is the exact "three divergent copies" the finding-1 ordering warned about;
    it was never consolidated.
  - `LightUniform` (Rust) `smirk/engine-renderer/src/camera.rs:192-205`: after
    `fog_color[3]`/`fog_density` comes `point_count: u32, _pad2: [u32; 3]` (16-byte
    block at offset 48), then `points` at offset 64. The WGSL mirror is
    `smirk/engine-renderer/src/snippets/scene_uniforms.wgsl:21-33`
    (`point_count, _pad0, _pad1, _pad2`).
  - Public setter `facade::set_fog(color, density, resources)`
    (`smirk/engine-renderer/src/facade.rs:199-204`) mutates
    `state.light_state.fog_color/fog_density` and uploads the whole struct.
  - Offscreen setter `OffscreenRenderer::set_fog(color, density)`
    (`smirk/engine-renderer/src/offscreen.rs:295-299`) mutates `light_state` and
    uploads; the sky-fog test `sky_fog_blends_toward_horizon_and_stays_bit_stable_at_zero_density`
    (tests/offscreen.rs:882-922) is the existing fog regression.
  - build.rs `SNIPPETS` array (build.rs:17) lists the three shared snippets and
    emits `rerun-if-changed` for each; the geometry shaders `//#include` them.
- **Ideal:** one shared `snippets/fog.wgsl` holds `apply_fog`; all three geometry
  shaders include it (local copies gone). `apply_fog` adds a height term: fog
  density is attenuated above a configurable `fog_height` by
  `exp(-fog_height_falloff * max(y - fog_height, 0))`, so low fragments fog more
  than high ones. `fog_height_falloff = 0` reproduces pure distance fog
  bit-for-bit. A new `set_fog_height` setter (facade + offscreen) drives it.
- **Gap:** fog is distance-only and triplicated; nothing modulates fog by world
  height.
- **Suggestion:** extend `LightUniform` in place (reuse the existing padding
  words, no size change), consolidate `apply_fog` into a snippet, add the height
  term and the setters.
- **Path:**
  1. **Fail-first test** in tests/offscreen.rs (new fog section): render one
     `ground_quad`-style flat quad through `render_mesh` at the origin under
     `set_fog(fog_color, density=0.05)`, twice — once with
     `set_fog_height(height=100.0, falloff=0.15)` (quad far *below* fog height →
     max fog) and once with `set_fog_height(height=-100.0, falloff=0.15)` (quad far
     *above* → height_atten → little fog). Same geometry, same eye distance, so the
     only variable is height. Assert the "below" render's mean channel value toward
     `fog_color` (e.g. `channel_mean` of the dominant fog channel) is clearly
     higher than the "above" render's. Also assert `set_fog(color, 0.0)` +
     `set_fog_height(0.0, 0.0)` reproduces the no-fog render exactly
     (`assert_eq!` on pixels, mirroring the sky test's density-0 bit-stability).
     Fails to compile until `set_fog_height` exists, then fails on the height
     assertion until the shader term lands.
  2. `camera.rs`: replace `_pad2: [u32; 3]` with `fog_height: f32,
     fog_height_falloff: f32, _pad2: u32` (still a 16-byte block; `point_count`
     stays at offset 48, `points` stays at 64 — no layout churn, no build.rs
     change). `default_sun()` sets both new fields to `0.0`.
  3. `snippets/scene_uniforms.wgsl`: mirror the tail —
     `point_count: u32, fog_height: f32, fog_height_falloff: f32, _pad1: u32,`.
  4. New `snippets/fog.wgsl` containing:
     ```
     /// Exponential distance fog with a height falloff; density 0 disables.
     fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
         let dist = length(camera.eye.xyz - world_pos);
         let h = max(world_pos.y - light.fog_height, 0.0);
         let height_atten = exp(-light.fog_height_falloff * h);
         let t = 1.0 - exp(-light.fog_density * dist * height_atten);
         return mix(color, light.fog_color, t);
     }
     ```
     Add `"fog.wgsl"` to build.rs `SNIPPETS` (its `rerun-if-changed` loop covers
     it). Delete the three local `apply_fog` defs and add
     `//#include "snippets/fog.wgsl"` after the `scene_uniforms.wgsl` include in
     each of mesh_shader.wgsl / shader.wgsl / skinned_mesh_shader.wgsl (the snippet
     needs `camera`/`light` in scope, which scene_uniforms provides).
  5. `facade.rs`: add
     `pub fn set_fog_height(height: f32, falloff: f32, resources: &mut Resources)`
     mutating `state.light_state.fog_height/fog_height_falloff` and uploading (same
     shape as `set_fog`). `offscreen.rs`: add
     `pub fn set_fog_height(&mut self, height: f32, falloff: f32)` mutating
     `light_state` and uploading.
  6. Green gate: `cargo test -p engine-renderer --features offscreen` — the new
     height test passes, `generated_shader_tests` (lib.rs) still parses all three
     shaders with the new include, and the existing sky-fog + all other offscreen
     tests are unchanged (falloff 0 default = today's distance fog). Then the full
     workspace suite (`set_fog` callers in the client are source-compatible; the
     new setter is additive).

### 2. Geometric specular anti-aliasing in the shared PBR core

- **Evidence:**
  - `smirk/engine-renderer/src/snippets/pbr_common.wgsl:50-82` — `shade_pbr(P, N,
    V, albedo, metallic, roughness, ao, emissive, shadow)` clamps roughness once
    (`let rough = clamp(roughness, 0.045, 1.0)`) and uses `rough` for the sun
    `direct_brdf`, the point-light loop, and the IBL prefilter lookup
    (`rough * PREFILTER_MAX_MIP`). It is the single core all three geometry
    fragment shaders include and call.
  - The shading normal `N` is computed per-fragment (normal-mapped) at
    mesh_shader.wgsl:109-117 and skinned_mesh_shader.wgsl:121-129, then passed into
    `shade_pbr`; `shader.wgsl:104` passes the geometric normal. All call sites are
    fragment shaders, so `dpdx/dpdy` are valid.
  - Existing specular-shape test `smooth_surface_has_tighter_brighter_specular_peak_than_rough`
    (tests/offscreen.rs:169-205) is the style to mirror; `tilted_normal_map_changes_luminance_vs_flat`
    (tests/offscreen.rs:259-283) shows the normal-map-fixture harness
    (`TextureSource::Rgba8(solid_normal_texture(...))`).
- **Ideal:** `shade_pbr` filters roughness upward by the shading normal's
  screen-space variance before any specular evaluation, so a low-roughness surface
  whose normal varies fast per pixel behaves like a rougher one — killing specular
  shimmer. Flat/constant-normal surfaces are unaffected (derivatives zero →
  identical to today's image).
- **Gap:** nothing addresses specular aliasing on normal-mapped surfaces in motion;
  a shiny normal-mapped surface sparkles.
- **Suggestion:** add a small `specular_aa_roughness(N, rough)` helper in
  pbr_common.wgsl and apply it once at the top of `shade_pbr`.
- **Path:**
  1. **Fail-first test** in tests/offscreen.rs (new specular-AA section): a smooth
     dielectric quad (`quad_with_material`, roughness ≈ 0.15, metallic 0) with a
     **tiled high-frequency normal map** rendered under the default sun only
     (`set_light(TestLight{ direction: (-1,2,-1), color: (1,0.95,0.85),
     ambient: 0.0 })`). Render it at two normal-map tiling densities (encode the
     tiling by scaling the fixture's UVs or repeating the normal texture so the
     on-screen normal frequency differs; the denser tiling → higher per-pixel
     `dpdx(N)`). Assert peak luminance (the `peak` helper) **decreases
     monotonically** as tiling density increases — GSAA raises effective roughness
     with normal gradient, softening the highlight. Record the measured peaks in a
     test comment.
     - **Worker decision rule (measured value):** if the peak drop between sparse
       and dense tiling is under ~5% at 256², first lower base roughness to 0.08
       and raise the normal-map tilt contrast; if still indistinct, render at
       512². If it is *still* under 5%, fall back to the weaker but robust
       assertion that the densest-tiling peak is strictly below the **flat**
       normal-map (128,128,255) peak of the same material — GSAA must soften
       high-frequency detail relative to no detail — and record all measured peaks.
       Do not add any runtime on/off knob; GSAA is always-on production behavior.
  2. `snippets/pbr_common.wgsl`: add
     ```
     // Karis/Tokuyoshi geometric specular AA: fold the shading normal's
     // screen-space variance into roughness so sub-pixel normal detail does not
     // alias the specular lobe. Zero on flat surfaces (derivatives vanish).
     fn specular_aa_roughness(N: vec3<f32>, rough: f32) -> f32 {
         let dndu = dpdx(N);
         let dndv = dpdy(N);
         let variance = 0.25 * (dot(dndu, dndu) + dot(dndv, dndv));
         let kernel   = min(2.0 * variance, 0.18);
         let a2       = rough * rough;
         return sqrt(clamp(a2 + kernel, 0.0, 1.0));
     }
     ```
     In `shade_pbr`, change `let rough = clamp(roughness, 0.045, 1.0);` to clamp
     first then `let rough = specular_aa_roughness(N, clamp(roughness, 0.045,
     1.0));`. No call-site changes (signature unchanged).
  3. Green gate: `cargo test -p engine-renderer --features offscreen` — the new
     test passes, `smooth_surface_has_tighter_brighter_specular_peak_than_rough`
     and every flat-surface PBR/IBL test are unchanged (flat quads have constant N
     → GSAA is identity), `generated_shader_tests` parses. Then full workspace
     suite.

### 3. Cascaded shadow plumbing — depth array + light-VP array at CASCADE_COUNT = 1

- **Evidence:**
  - `smirk/engine-renderer/src/shadow.rs:11-31` — `SHADOW_SIZE = 2048`, single
    `create_shadow_texture` (`depth_or_array_layers: 1`, `Depth32Float`);
    `fit_light_vp(target, light_dir)` returns one texel-snapped ortho VP
    (shadow.rs:49-75) with pure tests (shadow.rs:244-273).
  - `smirk/engine-renderer/src/snippets/shadow_sample.wgsl:1-25` — binding 2
    `light_vp: mat4x4<f32>`, binding 3 `t_shadow: texture_depth_2d`, binding 4
    `s_shadow: sampler_comparison`; `shadow_factor` does PCF 3×3 on the single map.
  - `smirk/engine-renderer/src/camera.rs:230-302` — `create_gpu_resources` builds
    `light_vp_buffer` (64B identity) and the scene BGL: binding 2 uniform, binding
    3 `Texture{ Depth, D2 }`, binding 4 `Sampler(Comparison)`.
  - Casting: `ShadowPipelines` (shadow.rs:77-238) with cast BGL binding 0 =
    light_vp uniform (shadow.wgsl reads a single mat4); `shadow_bind_group`
    (state.rs:397-405, offscreen.rs:185-192) binds `light_vp_buffer`.
  - Render: `record_shadow_pass` (frame.rs:385-465) writes `fit_light_vp(...)` and
    renders one depth pass into `state.shadow_view`; the offscreen mirror is
    `compose`'s shadow pass (offscreen.rs:464-487).
  - build.rs already injects u32 consts (`MAX_POINT_LIGHTS`) via
    `extract_u32_const` + the u32 branch of `resolve` (build.rs:25-33,85-94).
  - Regression: `floating_cube_casts_shadow_band_on_ground` (tests/offscreen.rs:789-855).
- **Ideal:** the shadow map is a `Depth32Float` **array** with `CASCADE_COUNT`
  layers (injected from `shadow.rs` into WGSL by build.rs), the receiver light-VP
  is `array<mat4x4, CASCADE_COUNT>`, and `shadow_factor` loops the cascades. At
  `CASCADE_COUNT = 1` the image is byte-identical to today — this step is pure
  plumbing that de-risks the multi-cascade change.
- **Gap:** every shadow resource is scalar; there is no array to grow into.
- **Suggestion:** convert texture/uniform/bindings/render loop to arrays sized by a
  new `CASCADE_COUNT` const set to 1; keep the single-cascade math.
- **Path:**
  1. `shadow.rs`: add `pub(crate) const CASCADE_COUNT: u32 = 1;`. Change
     `create_shadow_texture` to `depth_or_array_layers: CASCADE_COUNT` and return
     the texture plus a `Vec<TextureView>` of per-layer views (each
     `base_array_layer: i, array_layer_count: Some(1)`) **and** an array view
     (`D2Array`) for sampling. Add
     `pub(crate) fn fit_cascades(target, light_dir) -> [Mat4; CASCADE_COUNT as
     usize]` returning `[fit_light_vp(target, light_dir)]` for now (keep
     `fit_light_vp` as the per-cascade helper). Add a **256-byte-stride** cast
     uniform buffer helper (N × 256) for dynamic-offset casting.
  2. `camera.rs::create_gpu_resources`: `light_vp_buffer` becomes
     `CASCADE_COUNT × 64` identity matrices; scene BGL binding 2 stays a uniform
     (now holding the tight `array<mat4x4,N>`), binding 3 becomes
     `Texture{ Depth, D2Array }` bound to the array view. Add the cast BGL's
     dynamic-offset flag (`has_dynamic_offset: true`) on the shadow cast layout in
     shadow.rs so casting can index a cascade by 256-aligned offset.
  3. `snippets/shadow_sample.wgsl`: add `//#const CASCADE_COUNT`; binding 2 →
     `array<mat4x4<f32>, CASCADE_COUNT>`, binding 3 → `texture_depth_2d_array`.
     `shadow_factor` loops `c in 0..CASCADE_COUNT`, projects with `light_vp[c]`, and
     on the first cascade whose UV is in-bounds does the PCF 3×3 with
     `textureSampleCompareLevel(t_shadow, s_shadow, uv, c, ndc.z)`; outside all →
     return 1.0. (With N=1 this is exactly today's logic.)
     `build.rs`: extract `CASCADE_COUNT` from `src/shadow.rs` and inject it (u32
     branch), add `rerun-if-changed=src/shadow.rs` already present.
  4. `shadow.wgsl` (cast): read a single `light_vp` mat4 at cast binding 0 (a
     dynamic-offset slice) — unchanged shader, the offset selects the cascade.
     `frame.rs::record_shadow_pass` and `offscreen.rs::compose`: write
     `fit_cascades(...)` into both the tight receiver buffer (N×64) and the
     256-stride cast buffer; render a per-cascade loop (N=1 → one iteration) each
     into its layer view, binding the cast group at the cascade's dynamic offset.
     State plumbing: `state.rs`/`offscreen.rs` store the per-layer views + array
     view + cast buffer.
  5. Green gate: `floating_cube_casts_shadow_band_on_ground` and every other
     offscreen test pass **unchanged** (N=1 is byte-identical); the existing
     `shadow.rs` pure fit tests pass; `generated_shader_tests` parses the array
     bindings. Then full workspace suite.

### 4. Three concentric texel-snapped cascades with containment selection

- **Evidence:**
  - After step 3: `shadow.rs` exposes `CASCADE_COUNT`, `fit_cascades`, an array
    shadow texture + per-layer views, and the 256-stride cast buffer; the receiver
    samples `light_vp[c]` / layer `c` and picks the first in-bounds cascade
    (shadow_sample.wgsl). Both `frame.rs` and `offscreen.rs` render a per-cascade
    loop.
  - `fit_light_vp` (shadow.rs:49-75) is pure and unit-tested (texel-snap +
    containment) — the pattern the cascade fit test mirrors.
  - `HALF_EXTENT = 80.0`, `SHADOW_SIZE = 2048` → today's ~7.8 cm/texel; a 24-unit
    near cascade gives ~2.3 cm/texel.
- **Ideal:** `CASCADE_COUNT = 3`; `fit_cascades` returns three concentric,
  independently texel-snapped ortho VPs at half-extents ≈ [24, 48, 80] around the
  focus; the receiver already selects the tightest containing cascade, so contact
  shadows near the player are crisp while the far cascade preserves today's
  coverage. Existing shadow behavior (a caster at the focus still casts) is
  preserved.
- **Gap:** only one cascade exists; near-camera contact shadows are as coarse as
  the 80-unit box allows.
- **Suggestion:** bump `CASCADE_COUNT` to 3, generalize `fit_cascades` to per-
  cascade half-extents with per-cascade texel snapping, add pure fit tests; the
  render loops and receiver from step 3 already handle N cascades.
- **Path:**
  1. **Fail-first pure tests** in `shadow.rs` `#[cfg(test)]` (mirroring the
     existing `fit_light_vp` tests, no GPU needed): with `CASCADE_COUNT = 3`,
     `fit_cascades(target, light_dir)` must satisfy — (a) cascade 0's world-space
     texel size < cascade 2's (near is denser): recover each cascade's half-extent
     from its ortho VP or assert via projecting a fixed world offset and comparing
     NDC magnitudes; (b) each cascade texel-snaps (a sub-texel target move at that
     cascade's texel size leaves its matrix identical — the existing snap test,
     per cascade); (c) a point 20u from the target projects inside cascade 0's NDC
     `[-1,1]`, and a point 78u out projects inside cascade 2 but outside cascade 0.
     Fails while `fit_cascades` returns a single 80-unit box for all layers.
  2. `shadow.rs`: set `CASCADE_COUNT = 3`; replace the single `HALF_EXTENT` const
     with `const CASCADE_HALF_EXTENTS: [f32; CASCADE_COUNT as usize] =
     [24.0, 48.0, 80.0];`. Generalize `fit_cascades` to build each cascade with its
     own half-extent and its own texel size `(2*extent)/SHADOW_SIZE` for the snap,
     reusing the existing view/snap/ortho math from `fit_light_vp` per cascade.
     Add a small margin (e.g. 0.02 in NDC) to the receiver's containment test in
     shadow_sample.wgsl so a fragment near a tighter cascade's edge falls through
     to the next cascade rather than sampling off-map (avoids seams). Keep the
     depth-bias tuning (shadow.rs:141-142) as-is; note in a comment it is shared
     across cascades.
  3. No change needed to `frame.rs`/`offscreen.rs`/`camera.rs`/build.rs — the N
     buffers, N layer views, N-iteration render loops, and the injected
     `CASCADE_COUNT` from step 3 already scale to 3 (this is why step 3 landed
     first). Verify the cast buffer is sized `CASCADE_COUNT × 256` and the receiver
     buffer `CASCADE_COUNT × 64` from step 3's constants.
  4. Green gate: the new pure fit tests pass; `floating_cube_casts_shadow_band_on_ground`
     still passes (the focus caster is covered by the near cascade, which is a
     superset-in-density of the old fit); `generated_shader_tests` parses. Then
     full workspace suite. **Worker decision rule:** if `floating_cube...` regresses
     because the caster (6 units above a 60-unit ground) exceeds the near cascade's
     depth range, widen `CASCADE_HALF_EXTENTS[0]` to the smallest value that
     re-passes it and record the value in a `shadow.rs` comment — do not weaken the
     test.

### 5. Depth prepass + half-res SSAO texture (produced, not yet consumed)

- **Evidence:**
  - Forward MSAA pipeline: main pass writes MSAA HDR color + MSAA `Depth32Float`
    (`post.rs:8-11,37-48`; `frame.rs:479-504`); the particle pass samples that MSAA
    depth read-only (frame.rs:663-695). There is no single-sample depth and no
    normal buffer anywhere.
  - `CameraUniform` carries `inv_view_proj` (camera.rs:154-176) — the SSAO pass
    reconstructs view/world position from depth without a G-buffer.
  - Depth-only pipeline pattern to mirror: `ShadowPipelines` (shadow.rs:77-238)
    builds sdf/mesh/skinned depth-only variants (fragment `None`) with vertex
    layouts already worked out; the difference for a prepass is the layout binds
    the **camera** group (view_proj) instead of light_vp.
  - `HeadlessGpu`/`read_texture_mip` (offscreen.rs:35-53,546-597) can read back an
    Rgba8/`R8` texture; `OffscreenRenderer::compose` (offscreen.rs:451-522) is the
    single place both `render_sdf`/`render_mesh` route through, and `draw_sky` is
    the precedent for an opt-in harness flag.
- **Ideal:** a full-res single-sample `Depth32Float` prepass renders opaque
  geometry in camera space; a half-res SSAO pass reconstructs position+normal from
  that depth, samples a fixed hemisphere kernel to an `R8Unorm` AO target, and a box
  blur denoises it. The AO texture is observable and correct — darker in occluded
  creases — before anything consumes it.
- **Gap:** no depth prepass, no SSAO pass; nothing produces an AO signal.
- **Suggestion:** add depth-prepass pipelines + target, an SSAO pass (WGSL, half
  res), and a blur, all wired into `state.rs`/`frame.rs` and mirrored in
  `offscreen.rs`; expose an AO readback for the test.
- **Path:**
  1. New `smirk/engine-renderer/src/ssao.rs` (module header: intent + "reconstructs
     view position/normal from the depth prepass; AO multiplies IBL ambient only").
     Contents: `DepthPrepassPipelines` (sdf/mesh/skinned depth-only, camera-group
     layout, new `depth_prepass.wgsl` whose vertex entries mirror `shadow.wgsl` but
     `clip_pos = camera.view_proj * world`); `SsaoTargets` (full-res prepass depth
     + two half-res `R8Unorm` targets: raw AO and blurred AO); an `SsaoPass`
     (fullscreen pipeline sampling prepass depth, reconstructing position/normal,
     hemisphere kernel of ~16 fixed offsets with a per-pixel hash rotation, radius
     ≈ 0.5 m, range check) and a `BlurPass` (fullscreen box blur). `ssao.wgsl`
     holds both the SSAO and blur fragment entries; a small params uniform carries
     screen size, radius, and bias.
  2. `state.rs`: build `DepthPrepassPipelines`, `SsaoTargets`, `SsaoPass`,
     `BlurPass` in `init` (recreate `SsaoTargets` in `resize`). `frame.rs`: before
     `record_main_pass`, add `record_depth_prepass` (opaque SDF/mesh/skinned into
     the full-res prepass depth, camera group) and `record_ssao` (SSAO pass →
     blur), both with `timestamp_writes: None`. Guard by `state.ssao_enabled`
     (default true in production).
  3. `offscreen.rs`: add `pub draw_ssao: bool` (default **false**), an
     `SsaoTargets`/pipelines set built in `new`, and — when `draw_ssao` — run the
     depth prepass + SSAO + blur inside `compose` before the main pass, into
     targets sized to the `SceneTarget`. Add `pub fn ao_readback(&self, target)`
     returning the blurred half-res AO as bytes via `read_texture_mip` (R8 → widen
     to the reader's 4-byte assumption, or add an R8 read path).
  4. **Behavioral test** in tests/offscreen.rs (new SSAO section): a box (SDF cube)
     sitting on a large ground quad, sun off, uniform env ambient 1.0,
     `draw_ssao = true`. Read the AO texture via `ao_readback`; assert the mean AO
     in the tile covering the box-ground contact crease is clearly **lower** (more
     occluded) than the mean AO in an open-ground tile far from the box. Analytic,
     no exact values. This exercises prepass + SSAO + blur end to end through the
     real pipelines.
  5. Green gate: the AO test passes; every existing offscreen test is unchanged
     (`draw_ssao` defaults false → no new passes run); `generated_shader_tests`
     parses `depth_prepass.wgsl`/`ssao.wgsl` if they are added to the parse list
     (add them). Then full workspace suite. **Worker decision rule:** if
     depth-from-`inv_view_proj` normal reconstruction is too noisy for a stable
     crease-vs-open separation at 256², increase the render size to 512² for this
     test and/or widen the kernel radius; record the working radius in `ssao.rs`.
     If the separation still does not clear ~10%, park and report — do not ship an
     AO signal that cannot be shown to occlude.

### 6. Consume SSAO — multiply the IBL ambient in `shade_pbr`

- **Evidence:**
  - After step 5: `state`/`offscreen` own a blurred AO texture; production runs the
    prepass+SSAO passes each frame, the offscreen harness runs them under
    `draw_ssao`.
  - `shade_pbr` (pbr_common.wgsl:79) computes `let ambient = light.ambient *
    (diffuse + spec_ibl) * ao;` where `ao` is the **material** AO texture value —
    the exact term SSAO must additionally scale. The three call sites pass
    `in.world_pos` and have `@builtin(position)` available for screen UV.
  - Scene BGL (camera.rs:262-287) has bindings 0–4 used; 5+ are free within group 0
    (the 4-*group* budget is untouched by adding *bindings*).
  - Regression: every ambient-lit offscreen test (`uniform_white_environment_lights_surfaces_uniformly`
    tests/offscreen.rs:657-689, the point-light and IBL tests) uses flat/open
    geometry where SSAO ≈ 1.
- **Ideal:** the scene group carries the AO texture (binding 5) + a filtering
  sampler (binding 6); `shade_pbr` samples it by screen UV and multiplies the
  **ambient** term only (never direct or emissive). A 1×1 white AO fallback is
  bound whenever SSAO is off, so the change is neutral by default. A behavioral
  test shows a crease darkening in the final tonemapped image with SSAO on vs off.
- **Gap:** the AO signal exists but nothing reads it; corners don't ground into
  ambient.
- **Suggestion:** add the two scene-group bindings, thread a screen UV into
  `shade_pbr`, multiply ambient by the sampled AO, bind white-when-disabled, and
  add the crease test with the offscreen `set_ssao` knob.
- **Path:**
  1. **Fail-first test** in tests/offscreen.rs (SSAO section): the same box-on-
     ground scene, sun off, uniform env ambient 1.0, rendered through the real
     tonemapped output twice — `set_ssao(false)` then `set_ssao(true)`. Assert the
     mean luminance in the contact-crease tile drops by a clear margin with SSAO on,
     while an open-ground tile far from the box stays within noise. Fails until
     `shade_pbr` consumes AO. (Requires `OffscreenRenderer::set_ssao(bool)` — add
     it, gating both the passes from step 5 and the white-vs-real AO bind.)
  2. `camera.rs::create_gpu_resources`: add scene BGL binding 5
     `Texture{ Float filterable, D2 }` and binding 6 `Sampler(Filtering)`; the
     scene bind group binds the current AO view (real when enabled, a shared 1×1
     white texture when not). `state.rs`/`offscreen.rs`: rebuild the scene bind
     group when the AO view changes (init, resize, and `set_ssao` toggle);
     production always binds the real blurred AO.
  3. `snippets/scene_uniforms.wgsl` (or shadow_sample.wgsl, wherever group-0
     texture bindings live): declare `@group(0) @binding(5) var t_ssao:
     texture_2d<f32>;` and `@binding(6) var s_ssao: sampler;`.
     `snippets/pbr_common.wgsl`: `shade_pbr` gains a `screen_uv: vec2<f32>`
     parameter; compute `let ssao = textureSample(t_ssao, s_ssao, screen_uv).r;`
     and change the ambient line to `let ambient = light.ambient * (diffuse +
     spec_ibl) * ao * ssao;`. The three call sites pass
     `in.clip_pos.xy / vec2<f32>(<screen size>)` — screen size from a uniform (reuse
     the SSAO params or add a `viewport` field to `CameraUniform`); prefer adding a
     `viewport: vec2` to the camera uniform since `@builtin(position)` is already in
     framebuffer space (`in.clip_pos.xy` in the fragment). Confirm each frag main
     forwards the screen UV.
  4. Green gate: the crease test passes; the white-AO fallback keeps every existing
     ambient test unchanged (flat geometry → AO ≈ 1, and disabled → exactly 1);
     `generated_shader_tests` parses. Then full workspace suite. **Worker decision
     rule:** if binding a screen-space AO texture in an MSAA fragment shader trips a
     validation/filtering issue, sample the AO with `textureSampleLevel(..., 0.0)`
     (AO is full/half-res single-level) rather than adding a mip chain; record the
     choice in a comment.

### 7. Feel-checklist entries + strike the shipped items from future-work (docs-only)

- **Evidence:** `docs/visual-quality.md:133-138` — the "Future work (out of scope,
  tracked)" paragraph lists "Cascaded shadow maps, SSAO, TAA, GPU particles,
  mesh-geometry LOD, KTX2/Basis transcoding, …" ; the manual feel-checklist
  appendix (docs/visual-quality.md:140-154) is the sanctioned home for GUI checks
  (per MEMORY: no GUI run checks — the user eyeballs at phase boundaries), items
  numbered and citing VQ clauses.
- **Ideal:** the future-work list no longer names the four items this rework
  shipped (CSM, SSAO, height fog via the fog note, specular AA), TAA is explicitly
  recorded as deliberately deferred (not pending), and the feel-checklist carries
  items for the new visible payoffs.
- **Gap:** the docs still list CSM/SSAO as future and offer no feel-check for the
  new fog/shadow/AO/specular behavior.
- **Suggestion:** edit the future-work paragraph and append checklist items; touch
  nothing else.
- **Path:**
  1. `docs/visual-quality.md`: in the future-work paragraph, remove "Cascaded
     shadow maps, SSAO" and change the TAA mention to note it is deliberately
     deferred in favor of geometric specular AA (not pending work). Leave GPU
     particles / mesh-geometry LOD / creature pipeline as-is.
  2. Append feel-checklist items to the appendix (continue the numbering):
     - **Height fog** (VQ-A1/A5): at dusk, low ground/hollows read hazier than
       rooftops and ridgelines; density 0 zones look exactly as before.
     - **Contact shadows** (VQ future-work → shipped): stand a character on flat
       ground — the shadow at the feet is crisp, not the soft blur the old single
       cascade gave; distant props still cast.
     - **Grounding AO** (VQ-A2): props and characters darken into corners and floor
       contact instead of floating on flat ambient.
     - **Specular calm** (VQ-A2): pan the camera across a shiny normal-mapped
       surface — highlights stay stable, no crawling sparkle.
  3. No code, no test. (docs-only)
