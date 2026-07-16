# Plan: Punctual lights — torches, embers, and magic must light the world — 2026-07-16

Source: docs/reviews/rendering/reworks-rendering-2026-07-16.md finding 1.

## Ideal end state

Entities carry a `PointLight` ECS component (color, intensity, radius, offset,
flicker) that RON prefabs can author. Every display frame a render-sync system
collects the lit entities, keeps the 16 nearest the camera focus, and uploads
them into the shared scene light uniform. `shade_pbr` — the single Cook-Torrance
core all three geometry passes (SDF primitives, static meshes, skinned meshes)
include — evaluates the point-light array with windowed inverse-square falloff,
so a portal's cyan gate actually spills light onto the ground, pillars, and any
character standing near it, in every pass, with no per-light shadows. The
portal prefab emits; an offscreen analytic test proves brightening and
monotonic falloff through the real pipelines.

## Design decisions

- **Forward loop over a capped uniform array (16 lights), not clustered
  forward.** `MAX_POINT_LIGHTS = 16`, one fixed-size array appended to the
  existing `LightUniform` (576 bytes total — trivially within uniform limits).
  Zones are compact and the scene population is pre-content; clustered forward
  adds bins, per-tile lists, and a compute pass for a problem that does not
  exist yet. Scene scalability is rework 5's territory; if light counts ever
  exceed the cap in practice, the selection policy degrades gracefully
  (nearest-first) rather than breaking. Rejected: storage-buffer unbounded
  array (needs a new binding + layout churn in every pipeline for no present
  benefit).
- **The array lives inside the existing `LightUniform` buffer (group 0,
  binding 1), not a new binding.** `camera.rs::create_gpu_resources` is the
  single factory both `RendererState` and `OffscreenRenderer` use, and the
  buffer is created from struct contents, so it resizes automatically; no bind
  group layout changes, no pipeline changes, and the skinned pipeline's
  4-bind-group budget (camera.rs:206-208 comment) is untouched. The CPU-side
  `light_state` copy pattern (state.rs:70-72) already exists for exactly this
  kind of partial update — `set_light`/`set_fog`/the new extraction each own
  their fields and upload the whole 576-byte struct (negligible).
- **The loop lands once, in `snippets/pbr_common.wgsl`.** `shade_pbr` gains a
  world-position parameter `P` and iterates `light.points[0..point_count]`.
  The sun's Cook-Torrance body is extracted into a `direct_brdf(N, V, L,
  albedo, metallic, rough)` helper shared by the sun term and the loop — no
  duplicated BRDF math. All three call sites already have `in.world_pos` in
  hand. Attenuation is Karis windowed inverse-square:
  `att = saturate(1 - (d/r)^4)^2 / (d^2 + 0.01)` — physically plausible near
  the source, exactly zero at the radius (the art-directed leakage control the
  finding accepts in lieu of per-light shadows).
- **`MAX_POINT_LIGHTS` is injected into WGSL by build.rs, matching the
  existing no-drift machinery.** build.rs already extracts `u32` consts from
  Rust source and injects them (as f32) via `//#const`. It gains a u32-typed
  injection for `MAX_POINT_LIGHTS` read from camera.rs, so the WGSL array size
  can never drift from the Rust struct.
- **Component schema:** `PointLight { color: Vec3 (linear), intensity: f32
  (radiance at ~1 m, same unitless scale as the sun's ~1.0 color), radius:
  f32 (meters, hard cutoff), offset: Vec3 (entity-local, rotated by the
  entity's rotation — a torch flame sits above its base), flicker: f32 (0..1
  fraction of intensity, default 0 — the fire seam) }`. Lives in
  `engine_core::components` beside `RenderShape`/`RenderMesh` (same
  what-to-draw vs renderer-bookkeeping split), registered in
  `register_core_components` so any prefab can author it. Rejected: baking
  intensity into color (kills flicker/day-tuning ergonomics), physical lumen
  units (the whole pipeline is unitless-relative), spot parameters now
  (schema stays extendable; "spot later" per the finding).
- **Extraction is a dedicated `PointLightSyncSystem` in engine-renderer,
  Phase::RenderSync,** mirroring the draw-list builders: query `(Transform,
  Option<PreviousTransform>, &PointLight)`, lerp by `InterpolationAlpha`
  (consistent with `RenderSyncSystem`/`MeshRenderSyncSystem`), select the 16
  nearest `state.camera.target` (the player-centered focus), apply the
  flicker factor CPU-side (deterministic sin-noise, per-entity phase from the
  entity id), write `light_state.points/point_count`, upload. Selection and
  flicker are pure functions with unit tests. Order vs the client's
  `DayNightSystem` (RenderSync/First, writes sun fields via
  `facade::set_light`) is irrelevant for correctness because both routes
  mutate the persistent `light_state` and upload the full struct.
- **Day/night awareness needs no coupling code.** Point lights are additive
  HDR direct light, deliberately independent of `light.ambient` (the IBL
  scale the day/night cycle drives). At dusk/night the sun color and ambient
  drop (`day_night_light`), so accents dominate naturally; at noon they wash
  out — that is the physically correct behavior and the art direction's
  intent. Rejected: scaling point lights by ambient (backwards — would dim
  torches at night).
- **No protocol/replication impact.** Portals are client-local zone dressing
  (`presentation.rs` spawns them; "Never spawned by the server" per
  portal.ron's header), and `PointLight` rides the existing prefab component
  path — nothing crosses the wire.
- **No per-light shadows** (finding's explicit scope): leakage is accepted
  and art-directed via small radii.

## Findings (execution order)

### 1. Point-light array in the shared light uniform + Cook-Torrance loop in `shade_pbr`

- **Evidence:**
  - `smirk/engine-renderer/src/camera.rs:179-203` — `LightUniform` (Rust,
    `#[repr(C)]`, bytemuck Pod): `direction/_pad/color/ambient/fog_color/
    fog_density`, 48 bytes; `default_sun()` constructs it.
  - `smirk/engine-renderer/src/snippets/scene_uniforms.wgsl:13-22` — the WGSL
    mirror, bound at group 0 binding 1 in every geometry pass.
  - `smirk/engine-renderer/src/snippets/pbr_common.wgsl:26-60` — `shade_pbr`
    evaluates exactly one directional light (`let L = light.direction;`) plus
    IBL ambient; no world position parameter.
  - Call sites (all already have `in.world_pos`):
    `mesh_shader.wgsl:117`, `skinned_mesh_shader.wgsl:129`, `shader.wgsl:113`.
  - `smirk/engine-renderer/build.rs` — preprocessor: `//#include` splices
    snippets; `//#const NAME` emits `const NAME: f32 = …;` from u32 consts
    extracted out of shadow.rs/ibl.rs (`extract_u32_const`). No u32-typed
    emission exists yet.
  - `smirk/engine-renderer/src/camera.rs:210-282` — `create_gpu_resources`
    builds the light buffer from `LightUniform::default_sun()` contents
    (`min_binding_size: None`), used by both `RendererState`
    (state.rs:97-98 via `create_camera_and_shadow_view`) and
    `OffscreenRenderer` (offscreen.rs:156-157).
  - `smirk/engine-renderer/src/offscreen.rs:254-273` — `set_fog` and
    `set_light` construct `LightUniform` struct literals (the only literal
    constructors besides `default_sun`); no persistent CPU copy exists in
    the offscreen renderer (unlike `RendererState.light_state`,
    state.rs:70-72).
  - `smirk/engine-renderer/src/lib.rs:33-47` — `generated_shader_tests`
    parses all three generated geometry shaders with naga.
  - Test conventions: `smirk/engine-renderer/tests/offscreen.rs:1-66`
    (skip-if-no-GPU via `renderer_or_skip`, `channel_mean`, `luminance`,
    analytic assertions only); sun-off pattern at line 424:
    `r.set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 0.0 })`.
- **Ideal:** the light uniform carries `point_count` + a 16-entry point-light
  array in both Rust and WGSL (sizes injected from one Rust const);
  `shade_pbr(P, N, V, …)` adds each in-range light's Cook-Torrance
  contribution with windowed inverse-square falloff; all three passes pass
  `in.world_pos`; the offscreen harness can set test point lights; an
  analytic test proves a point light brightens an otherwise-unlit surface
  and falls off monotonically with distance. Zero lights = today's image.
- **Gap:** no point-light data reaches the GPU anywhere; `shade_pbr` has no
  world position and no loop; the offscreen harness has no point-light
  setter.
- **Suggestion:** extend the schema in `camera.rs`, the WGSL snippet, and
  build.rs; refactor `shade_pbr`'s direct term into a shared `direct_brdf`
  helper and add the loop; thread `in.world_pos` through the three call
  sites; give `OffscreenRenderer` a persistent `light_state` CPU copy (the
  `RendererState` pattern) plus `set_point_lights`.
- **Path:**
  1. **Fail-first test** in `smirk/engine-renderer/tests/offscreen.rs`
     (new section, reusing `renderer_or_skip`, `ground_quad`,
     `channel_mean`): with
     `set_light(TestLight { direction: Vec3::Y, color: Vec3::ZERO, ambient: 0.0 })`
     and one `TestPointLight { position: Vec3::new(0.0, 2.0, 0.0), color:
     Vec3::new(0.2, 0.8, 1.0), intensity: 30.0, radius: 30.0 }` over
     `ground_quad(20.0, 0.9, 0.0)` rendered via `render_mesh` on a black
     clear, assert: (a) mean luminance with the light ≥ 4× the unlit render's
     mean; (b) moving the light to heights 2.0 / 6.0 / 12.0 gives strictly
     decreasing mean luminance; (c) with `radius: 1.0` (light at height 2.0,
     outside its own radius) the image matches the unlit render's mean within
     noise (< 1.0 mean-luminance difference); (d) the cyan light's blue
     channel mean exceeds its red channel mean (color is carried). If ACES
     tonemapping compresses (a) below 4×, lower the ratio to 2× and record
     the measured ratio in the test comment — do not raise intensity above
     100. Write the `set_point_lights` harness API first so the test
     compiles, loop absent → test fails on (a).
  2. `camera.rs`: add `pub(crate) const MAX_POINT_LIGHTS: u32 = 16;`, a
     `#[repr(C)]` Pod `GpuPointLight { position: [f32; 3], radius: f32,
     color: [f32; 3], intensity: f32 }` (32 bytes), and extend
     `LightUniform` with `point_count: u32, _pad2: [u32; 3],
     points: [GpuPointLight; MAX_POINT_LIGHTS as usize]` (offsets: count at
     48, array at 64, total 576 — matches WGSL uniform layout rules; array
     stride 32 is a 16 multiple). `default_sun()` zero-fills them
     (`GpuPointLight` derives Zeroable).
  3. `build.rs`: extract `MAX_POINT_LIGHTS` from `src/camera.rs` with the
     existing `extract_u32_const`; extend the injection so this const emits
     `const MAX_POINT_LIGHTS: u32 = 16u;` (a second, u32-typed consts map
     checked before the f32 map in `resolve`) and add
     `cargo:rerun-if-changed=src/camera.rs`.
  4. `snippets/scene_uniforms.wgsl`: add `//#const MAX_POINT_LIGHTS` (before
     the struct), `struct PointLight { position: vec3<f32>, radius: f32,
     color: vec3<f32>, intensity: f32 }`, and the new `LightUniform` tail:
     `point_count: u32, _pad0: u32, _pad1: u32, _pad2: u32,
     points: array<PointLight, MAX_POINT_LIGHTS>`.
  5. `snippets/pbr_common.wgsl`: extract the direct term into
     `fn direct_brdf(N, V, L, albedo, metallic, rough) -> vec3<f32>`
     returning `(kd * albedo + specular) * NdotL` (f0 computed inside);
     `shade_pbr` gains a first parameter `P: vec3<f32>` and computes
     `direct = direct_brdf(N, V, light.direction, …) * light.color * shadow`
     plus a loop: for each `i < light.point_count`, `to_l = points[i].position - P`,
     `d = length(to_l)`; skip when `d >= radius`; else
     `att = saturate(1.0 - pow(d / radius, 4.0))`, `att = att * att / (d * d + 0.01)`,
     accumulate `direct_brdf(N, V, to_l / max(d, 1e-4), …) * points[i].color
     * points[i].intensity * att`. Clamp roughness once before both uses.
  6. Update the three call sites to `shade_pbr(in.world_pos, N, V, …)`
     (mesh_shader.wgsl:117, skinned_mesh_shader.wgsl:129, shader.wgsl:113).
  7. `offscreen.rs`: add `pub struct TestPointLight { position: Vec3, color:
     Vec3, intensity: f32, radius: f32 }`; give `OffscreenRenderer` a
     `light_state: LightUniform` field initialized to `default_sun()`;
     refactor `set_light`/`set_fog` to mutate only their fields of
     `light_state` and upload it (update `set_fog`'s doc comment — it no
     longer resets sun fields; the fog tests at tests/offscreen.rs:707-748
     never call `set_light` first, so they stay green); add
     `pub fn set_point_lights(&mut self, lights: &[TestPointLight])` filling
     `points`/`point_count` (truncate at 16) and uploading.
  8. Green gate: `cargo test -p engine-renderer --features offscreen` — the
     new analytic test passes, `generated_shader_tests` still parses all
     three shaders, and every pre-existing offscreen test (sun-only PBR,
     fog, IBL) is unchanged because `point_count = 0` reproduces today's
     image. Then the full workspace test suite.

### 2. `PointLight` ECS component + render-sync extraction with distance-prioritized cap and flicker

- **Evidence:**
  - `smirk/engine-core/src/components.rs:103-158` — `RenderShape` /
    `RenderMesh` show the component pattern: plain data +
    `serde::Deserialize` (glam `Vec3` deserializes — `RenderMesh.tint`
    proves the serde feature is on), renderer bookkeeping kept out.
  - `smirk/engine-core/src/prefab.rs:253-281` — `register_core_components`
    registers every core component by name (`reg.register::<RenderMesh>("RenderMesh")`);
    `smirk/engine-app/src/prefab_plugin.rs` calls it at startup.
  - `smirk/engine-renderer/src/instance_sync.rs:28-92` — `RenderSyncSystem`:
    the extraction pattern (query + `InterpolationAlpha` lerp against
    `PreviousTransform`).
  - `smirk/engine-renderer/src/mesh/sync.rs:150-156` — headless guard
    (`resources.get::<RendererState>().is_none() → return`) and
    DevStats metering pattern (sync.rs:274-277:
    `stats.set("skinned", format!("{n}/{cap}"))`).
  - `smirk/engine-renderer/src/lib.rs:69-89` — `RenderPlugin::build`
    registers sync systems at `Phase::RenderSync` (`RenderSlotAttachSystem`
    First, the two sync systems Default).
  - `smirk/engine-renderer/src/facade.rs:151-160` — `set_light` mutates
    `state.light_state` + uploads: the persistent-CPU-copy contract the new
    system must follow (fields are owned per-writer; every writer uploads the
    whole struct).
  - `smirk/engine-renderer/src/state.rs:54-72` — `RendererState` has
    `camera` (with `camera.target` — the focus point), `queue`,
    `light_buffer`, `light_state`.
  - `client/vordar-client/src/net/mod.rs:92` — `DayNightSystem` runs at
    `Phase::RenderSync, SystemOrder::First` and calls `set_light` every
    frame; because both it and the new system write through `light_state`,
    relative order does not affect correctness.
- **Ideal:** `PointLight { color: Vec3, intensity: f32, radius: f32, offset:
  Vec3 (default ZERO), flicker: f32 (default 0.0) }` is a core component any
  prefab can author; `PointLightSyncSystem` fills
  `light_state.points/point_count` each display frame from the world —
  positions lerped, offset rotated by the entity rotation, the 16 nearest
  `camera.target` kept, flicker applied as a deterministic time modulation —
  and uploads; the dev overlay shows `lights: n/16`.
- **Gap:** no component, no extraction, nothing writes the (now existing)
  array.
- **Suggestion:** component + registration in engine-core; a new
  `smirk/engine-renderer/src/light_sync.rs` module holding the system and
  two pure, unit-tested functions (selection, flicker).
- **Path:**
  1. `engine-core/src/components.rs`: add `PointLight` as above,
     `#[derive(Clone, serde::Deserialize)]` with `#[serde(default)]` on
     `offset` and `flicker` (mirror `RenderMesh`'s default pattern). Doc
     comment states the contract: color linear RGB, intensity ≈ radiance at
     1 m on the engine's unitless scale, radius = hard cutoff in meters,
     additive HDR (independent of the day/night ambient scale).
  2. `engine-core/src/prefab.rs`: `reg.register::<PointLight>("PointLight");`
     in `register_core_components`. **Fail-first test** in a `#[cfg(test)]`
     mod in prefab.rs (create one; none exists): build a
     `ComponentRegistry`, run `register_core_components`, and
     `registry.compile("PointLight", RawValue)` from the RON text
     `(color: (1.0, 0.6, 0.2), intensity: 12.0, radius: 6.0)` — asserts the
     name is registered and `offset`/`flicker` default. (Fails before the
     register line lands.)
  3. New `smirk/engine-renderer/src/light_sync.rs` (module header comment:
     intent + "writes the point-light slice of `light_state` and uploads;
     field ownership shared with facade::set_light/set_fog"). Contents:
     - `pub(crate) struct LightCandidate { pub position: glam::Vec3, pub color: glam::Vec3, pub intensity: f32, pub radius: f32 }`
     - `pub(crate) fn flicker_factor(time: f32, phase: f32, amount: f32) -> f32`
       = `1.0 - amount * n` with
       `n = 0.5 + 0.3 * (13.0 * time + phase).sin() + 0.2 * (7.3 * time + 1.7 * phase).sin()`
       (n ∈ [0,1] ⇒ factor ∈ [1-amount, 1]).
     - `pub(crate) fn select_point_lights(candidates: &mut Vec<LightCandidate>, focus: glam::Vec3) -> ([crate::camera::GpuPointLight; 16], u32)`:
       sort by `distance_squared(focus)`, take `MAX_POINT_LIGHTS`, map into
       the GPU structs, zero-fill the tail.
     - `pub struct PointLightSyncSystem { time: f32, candidates: Vec<LightCandidate> }`
       (`Default`); `run`: headless guard (return if no `RendererState`);
       `self.time += delta`; read `InterpolationAlpha`; query
       `(hecs::Entity, &Transform, Option<&PreviousTransform>, &PointLight)`;
       per entity: lerped position + `transform.rotation * pl.offset`,
       intensity × `flicker_factor(self.time, entity.id() as f32 * 2.399963, pl.flicker)`;
       then `select_point_lights` with `state.camera.target` as focus, write
       `state.light_state.points/point_count`,
       `queue.write_buffer(&state.light_buffer, …)`, and
       `stats.set("lights", format!("{n}/16"))` on `DevStats` (mirror
       sync.rs:275-277).
  4. `lib.rs`: `pub(crate) mod light_sync;`, re-export
     `light_sync::PointLightSyncSystem`, and register it in
     `RenderPlugin::build` at `Phase::RenderSync, SystemOrder::Default`
     (alongside the draw-list builders).
  5. **Tests** (in light_sync.rs, behavioral through the real functions):
     (a) 20 candidates at strictly increasing distance from the focus →
     `select_point_lights` returns count 16 and every kept light is nearer
     than every dropped one (assert by radius marker or position);
     (b) 3 candidates → count 3, tail entries zeroed;
     (c) `flicker_factor(t, p, 0.0) == 1.0` for several t;
     (d) `flicker_factor` with amount 0.6 stays within [0.4, 1.0] over 100
     sampled times and is non-constant (min < max).
  6. Green gate: full workspace suite. If any system-name-list test fails
     (the client asserts registered system name lists in
     `client/vordar-client/tests/presentation_plugin.rs`; an engine-side
     analog may assert RenderPlugin's), add `"PointLightSyncSystem"` to the
     expected list — that is the only sanctioned test edit.

### 3. Portal prefab emits light (content hookup + typed spawn test)

- **Evidence:**
  - `content/prefabs/portal.ron` — Transform + ShapeGroup; the gate diamond
    at offset `(0.0, 0.8, 0.0)` with HDR emissive color `(0.9, 2.85, 3.0)`
    blooms but lights nothing (the finding's headline symptom).
  - `client/vordar-client/src/presentation.rs:134-147` —
    `ZoneDressingSystem` spawns `"portal"` via `spawn_prefab` at each zone
    exit; client-local only.
  - `smirk/engine-core/src/prefab.rs:197-229` — `spawn_prefab` compiles
    components by name through `ComponentRegistry`; unknown component ⇒
    spawn error, so the RON edit is only safe after step 2's registration.
  - `game/vordar-game/tests/content_lint.rs:120-127` — the existing home for
    content-shape assertions; its `prefab_dirs` array shows the path
    convention this test file already resolves (reuse it verbatim).
- **Ideal:** the portal prefab carries
  `"PointLight": (color: (0.30, 0.95, 1.05), intensity: 8.0, radius: 9.0, offset: (0.0, 0.8, 0.0), flicker: 0.15)`
  — cyan matched to the gate's emissive hue, centered on the diamond, radius
  covering the pillars and a character standing in the gate, gentle hum
  flicker — and a typed test proves the RON round-trips into a live entity
  component through the real registry + spawn machinery.
- **Gap:** no content references `PointLight`; nothing proves the
  data-driven path end to end.
- **Suggestion:** edit portal.ron; add a spawn-through-registry test beside
  the existing content lints.
- **Path:**
  1. **Fail-first test** in `game/vordar-game/tests/content_lint.rs`: build
     `ComponentRegistry::new()` + `register_core_components`, a
     `PrefabLibrary` loading the same `content/prefabs` dir the file's
     `prefab_dirs` already names (reuse its path handling exactly), a fresh
     `hecs::World` + `Resources` holding both, then
     `spawn_prefab("portal", Vec3::ZERO, &mut SpawnContext { world, resources })`
     and assert the entity has a `PointLight` with `radius > 0.0` and a
     cool hue (`color.z > color.x` — it must read as the gate's cyan, not a
     default). Fails while portal.ron lacks the component.
  2. Add the `"PointLight"` entry to `content/prefabs/portal.ron` with the
     values above (tuning is feel-checked later; the test asserts shape, not
     the exact numbers).
  3. Green gate: full workspace suite — in particular the client e2e helpers
     that load `content/prefabs` (client/vordar-client/src/net/e2e.rs:48)
     and the prefab_spawn bench crate must still compile and pass, which
     they do because step 2 registered the loader in
     `register_core_components` (the one registry everyone builds from).

### 4. Feel-checklist entry for accent lighting (docs-only)

- **Evidence:** `docs/visual-quality.md:138-154` — the "Appendix — manual
  feel-checklist (sandbox)" is the sanctioned home for GUI checks (per
  MEMORY: no GUI run checks; the user eyeballs). Items are numbered and cite
  VQ clauses (e.g. item 1 "Theme read (VQ-A1/A5)"). Item 9 is currently the
  last (docs/visual-quality.md:154).
- **Ideal:** the checklist carries an accent-light item so the user's next
  phase-boundary pass verifies the rework's visible payoff.
- **Gap:** no checklist item exercises punctual lighting.
- **Suggestion:** append one item; touch nothing else in the doc.
- **Path:** add item 10 to the appendix:
  "**Accent light** (VQ-A1/A5): stand a character beside a portal at dusk —
  the gate's cyan light spills onto the ground, both pillars, and the
  character, fading smoothly with distance (no hard edge at the light
  radius); the gate itself still blooms." No code, no test. (docs-only)
