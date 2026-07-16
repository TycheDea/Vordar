# Plan: Sorted transparency pass — glTF BLEND is silently punched to cutout — 2026-07-16

Source: docs/reviews/rendering/reworks-rendering-2026-07-16.md finding 3.

## Ideal end state

A glTF material authored `"alphaMode": "BLEND"` survives import as a distinct
mode instead of being punched to MASK 0.5, and its primitives render through a
transparent variant of the mesh/skinned pipelines: drawn inside the main pass
after opaque + sky, back-to-front by per-primitive centroid distance from the
camera, depth-test-on / depth-write-off, premultiplied alpha, fogged by the
same `apply_fog` as everything else. Red glass over a white cube reads pink.
Opaque and MASK content renders bit-identically to today. Transparent
primitives do not cast shadows; intersecting transparents remain approximate
(no OIT — accepted at AA scale); particles keep their own pass and always
composite after mesh transparency.

## Design decisions

**`AlphaMode` enum replaces the `alpha_cutoff: f32` field — no parallel
field.** `MaterialData.alpha_cutoff` (gltf_import.rs:34) currently encodes
three modes in one float and is where the data loss happens. It becomes
`alpha_mode: AlphaMode` with `Opaque`, `Mask(f32)`, `Blend`. Every external
construction site (client/vordar-client/src/ground.rs:142, weapons.rs:95)
uses `..Default::default()` and compiles unchanged; only
tests/offscreen.rs:380 names the field. *Rejected:* keeping the float and
adding a bool (two fields encoding one fact drift), or a magic sentinel value
(the current bug generalized).

**GPU encoding rides the existing `MaterialUniform.mr` vec4 — no layout
change, no new binding.** `mr.z` stays the mask cutoff (0 = no cutout);
`mr.w`, currently a zero pad (store.rs:115), becomes the blend flag
(1.0 = BLEND). Blend materials only ever draw through the transparent
pipeline and opaque/mask ones only through the opaque pipeline, so one shader
module serves both pipelines with a uniform branch. *Rejected:* a second
shader entry point or pipeline-overridable constant (build.rs preprocessing
and both pipeline factories would fork for a one-line branch).

**Premultiplied alpha, fog before premultiply.** The transparent blend state
is `One / OneMinusSrcAlpha` on color and alpha — byte-identical to the
particle premultiplied variant (particle_pipeline.rs:171-182), so the two
transparent systems compose under one convention. The fragment computes
`rgb = apply_fog(shade_pbr(...))` then multiplies rgb by alpha: each blended
term is individually fogged, which is exactly "fogged consistently".
Depth write off, depth compare Less (hidden behind opaque), MSAA sample count
unchanged, **alpha-to-coverage off** on the transparent variants (a2c would
dither the fractional alpha that blending already handles).

**Routing is decided per primitive at record time — draw lists and sync are
untouched.** `GpuPrimitive` gains `blend: bool` and `centroid: Vec3` (AABB
center of its vertices, computed once in `upload_mesh`). `MeshDrawList` /
`SkinnedDrawList` and `MeshRenderSyncSystem` keep their exact shapes: instance
buffers are already uploaded whole, so the transparent phase draws
`instance..instance+1` sub-ranges of the same buffers. The opaque main-pass
and shadow-pass loops skip `blend` primitives; a collector builds the sorted
transparent list from (ranges × blend-prims × instances). *Rejected:*
separate transparent draw lists built in sync.rs — duplicates range plumbing
across three resources and moves camera knowledge into sync for no gain at
current transparent counts.

**Sort granularity: primitive × instance, key = squared distance from camera
eye to `model × centroid`, descending.** Per the finding, per-primitive is
enough at current mesh counts; squared distance is order-equivalent to
distance and cheaper; `f32::total_cmp` keeps the sort NaN-safe. Pipeline
switches between skinned/static items follow sort order (correctness first;
switch cost is irrelevant at these counts).

**Transparents draw inside the existing main pass, after the sky.** The main
pass (frame.rs:442-537) already owns the MSAA HDR color + depth attachments
and ends with the sky; appending the sorted transparent draws there keeps one
render pass, keeps MSAA, and leaves the resolve where it is (end of the
particle pass). *Rejected:* a dedicated transparent pass (attachment
juggling for nothing).

**Skinned transparents are fully supported, same mechanism.** The skinned
pipeline gets the same transparent variant; the sort key uses the bind-pose
centroid through the instance model matrix (posing moves vertices by at most
a rig's extent — irrelevant at sort granularity). Ghosts/spirits are exactly
the skinned case the art direction wants; the marginal cost over static-only
is one pipeline field and one loop arm.

**Transparents do not cast shadows.** The shadow pipelines bind no material
(frame.rs:397-437), so a blend primitive would cast a fully solid shadow —
worse than none for glass/spirits. Blend prims are skipped in both shadow
loops. (MASK cutouts already cast solid shadows today; unchanged.)

**Particles and mesh transparency do not share a sort.** Particles stay in
their own later pass (soft-fade needs the resolved opaque depth as a sampled
texture). Consequence: particles always composite over transparent meshes
regardless of depth order, and glass never occludes a particle. Accepted:
particles are emissive/media effects; this is the same class of approximation
as no-OIT.

**Content check done during planning:** no live asset ships BLEND today —
`statue_vroid.glb` (BLEND, static, referenced only by benches/tests) and
`human.glb` (MASK only) are the only alpha-mode users. Intermediate steps
therefore change no in-game image; the offscreen tests carry all proof.

## Findings (execution order)

### 1. Import and upload carry the glTF alpha mode faithfully

- **Evidence:** `smirk/engine-renderer/src/mesh/gltf_import.rs:27-44` —
  `MaterialData.alpha_cutoff: f32` with the struct comment "BLEND is
  approximated as MASK at 0.5 — there is no sorted transparency pass";
  `gltf_import.rs:238-242` maps `AlphaMode::Blend => 0.5`;
  `smirk/engine-renderer/src/mesh/store.rs:107-121` encodes it into
  `MaterialUniform.mr` as `[metallic, roughness, alpha_cutoff, 0.0]`;
  `store.rs:12-20` `GpuPrimitive` carries no material metadata beyond the
  bind group.
- **Ideal:** the CPU data model distinguishes the three glTF alpha modes, and
  each uploaded `GpuPrimitive` knows whether it blends and where its centroid
  is — with zero change to rendered output this step (BLEND still *encodes*
  to the GPU as mask 0.5 until step 2 flips it; the plan is the record of
  that staging — add no TEMP comment).
- **Gap:** the mode collapses to one float at import; nothing downstream can
  ever route BLEND primitives differently.
- **Suggestion:** in `gltf_import.rs` add
  `#[derive(Clone, Copy, PartialEq, Debug)] pub enum AlphaMode { Opaque, Mask(f32), Blend }`
  and replace `MaterialData.alpha_cutoff: f32` with
  `pub alpha_mode: AlphaMode` (Default → `Opaque`; delete the "BLEND is
  approximated" sentence from the field comment — keep the sentence
  explaining MASK's cutoff/discard meaning). `read_material` maps
  `Opaque → AlphaMode::Opaque`, `Mask → AlphaMode::Mask(cutoff or 0.5)`,
  `Blend → AlphaMode::Blend`. Export `AlphaMode` from
  `smirk/engine-renderer/src/mesh/mod.rs:24`'s `pub use` list. In
  `store.rs::upload_mesh` compute
  `let cutoff = match m.alpha_mode { Opaque => 0.0, Mask(c) => c, Blend => 0.5 };`
  for `mr[2]` (mr[3] stays 0.0 this step), and give `GpuPrimitive` two new
  fields: `pub(crate) blend: bool` (`alpha_mode == AlphaMode::Blend`) and
  `pub(crate) centroid: glam::Vec3` (AABB center: component-wise
  `(min + max) / 2` over `p.vertices` positions; `Vec3::ZERO` for an empty
  vertex list). Update the one field-naming call site outside the crate
  sources: `smirk/engine-renderer/tests/offscreen.rs:380`
  `alpha_cutoff: 0.5,` → `alpha_mode: AlphaMode::Mask(0.5),` (add `AlphaMode`
  to the line-9 import). Grep the workspace for `alpha_cutoff` afterwards —
  zero hits outside gltf-crate API calls (`mat.alpha_cutoff()` at
  gltf_import.rs:240 stays).
- **Path:**
  1. Add a `write_blend_glb` builder to
     `smirk/engine-renderer/src/mesh/test_glb.rs` — copy `write_test_glb`
     verbatim, change the materials entry to
     `"alphaMode": "BLEND"` (no `alphaCutoff` key) and the baseColorFactor to
     `[1.0, 0.0, 0.0, 0.6]`.
  2. Fail-first unit test in `gltf_import.rs` tests mod:
     `blend_alpha_mode_survives_import` — write the blend GLB to a temp path,
     `load_gltf_data`, assert
     `p.material.alpha_mode == AlphaMode::Blend` (fails to compile/assert
     against today's float field). Update the existing assertion at
     gltf_import.rs:320 to `AlphaMode::Mask(0.35)` and keep its comment
     about OPAQUE reading differently (now `AlphaMode::Opaque`).
  3. Implement the enum + `read_material` mapping + `MaterialData` field +
     export.
  4. Implement the `upload_mesh` changes (`cutoff` match, `blend`,
     `centroid`).
  5. Add a store test (in `store.rs`'s existing
     `#[cfg(all(test, feature = "offscreen"))]` mod, HeadlessGpu skip
     pattern): `upload_records_blend_flag_and_centroid` — build a `MeshData`
     with two primitives via the `triangle_mesh_data` helper pattern, one
     Default material and one `alpha_mode: AlphaMode::Blend`, vertices
     (0,0,0)/(1,0,0)/(0,1,0); `upload_mesh`; assert prim0
     `blend == false`, prim1 `blend == true`, and both centroids equal
     `(0.5, 0.5, 0.0)`.
  6. Full crate gate: `cargo test -p engine-renderer` plus
     `cargo test -p vordar-client` (its ground/weapons builders compile
     against the new field via `..Default::default()`). Zero new warnings.

### 2. Transparent mesh pipeline + shader blend path, proven offscreen

- **Evidence:** `smirk/engine-renderer/src/mesh_pipeline.rs:140-174` — the
  only mesh pipeline blends `REPLACE` with `alpha_to_coverage_enabled: true`
  and depth write on; `mesh_shader.wgsl:92-103` and
  `skinned_mesh_shader.wgsl:103-116` — identical fragment alpha blocks that
  only know opaque/mask; `store.rs:115` — `mr[3]` uploads 0.0 for every
  material after step 1; `offscreen.rs:400-472` — `compose` draws
  shadow → scene closure → optional sky, with no stage after the sky;
  `offscreen.rs:358-398` — `render_mesh` draws every primitive through the
  one opaque pipeline in both closures.
- **Ideal:** a Blend material uploads `mr = [metallic, roughness, 0.0, 1.0]`;
  both WGSL fragment shaders branch on `material.mr.w` to output real alpha
  with premultiplied rgb; a transparent variant of the static mesh pipeline
  exists (premultiplied blend, depth-write-off, a2c-off); and
  `OffscreenRenderer::render_mesh` routes blend primitives through it after
  the sky, back-to-front — so an analytic readback proves red glass over a
  white surface reads pink and stacked glasses composite in depth order.
- **Gap:** no transparent pipeline exists anywhere; the shaders can only
  cut out; the offscreen harness has no stage after the sky.
- **Suggestion:** four seams, one step:
  (a) **store.rs**: `AlphaMode::Blend` now encodes `mr[2] = 0.0`,
  `mr[3] = 1.0` (Opaque/Mask keep `mr[3] = 0.0`); update the
  `MaterialUniform` field comment in `mesh_pipeline.rs:42` (z = mask cutoff,
  w = 1 for BLEND).
  (b) **both WGSL shaders** (`mesh_shader.wgsl`, `skinned_mesh_shader.wgsl` —
  keep the two files' fragment tails textually parallel): keep every
  `textureSample` above the branch (uniformity requirement — do not move
  samples into control flow), then replace the alpha block with
  ```wgsl
  let alpha_test = albedo_s.a * material.base_color.a;
  var out_alpha = 1.0;
  if material.mr.w > 0.5 {
      // BLEND: real coverage; rgb premultiplied at return.
      out_alpha = alpha_test;
  } else if material.mr.z > 0.0 {
      /* existing mask/a2c body unchanged, including the fwidth comment */
  }
  ```
  and change the return to
  ```wgsl
  var rgb = apply_fog(color, in.world_pos);
  if material.mr.w > 0.5 { rgb = rgb * out_alpha; }
  return vec4<f32>(rgb, out_alpha);
  ```
  (c) **mesh_pipeline.rs**: `create_mesh_pipeline` gains a
  `transparent: bool` parameter. When true: label
  `"Mesh Pipeline (transparent)"`, `depth_write_enabled: Some(false)`,
  `alpha_to_coverage_enabled: false`, and blend state
  `One/OneMinusSrcAlpha` on both color and alpha (copy the `premultiplied`
  BlendState from particle_pipeline.rs:171-182). Callers updated:
  `state.rs:334` passes `false` (frame routing lands in step 3);
  `offscreen.rs:196` passes `false` and a new
  `mesh_transparent_pipeline` field is built with `true`.
  (d) **offscreen.rs**: add `camera_eye: Vec3` to `OffscreenRenderer`
  (init: `Camera::new(aspect).eye()`; `set_camera_level` updates it from its
  leveled camera — `Camera::eye()` is pub(crate) at camera.rs:128). `compose`
  gains a third closure `transparent_draw`, invoked after the sky block;
  `render_sdf` passes a no-op closure. `render_mesh` partitions
  `gpu_mesh.primitives` indices into opaque and blend; opaque indices draw in
  the shadow and main closures exactly as today (blend prims skipped in
  both — transparents don't cast); blend indices sort by descending
  `camera_eye.distance_squared(prim.centroid)` (identity model) and draw in
  the transparent closure: `mesh_transparent_pipeline`, camera bind group 0,
  per-prim material group 1, environment group 2, instance buffer slot 1,
  `draw_indexed(0..index_count, 0, 0..1)`.
- **Path:**
  1. Fail-first integration test in
     `smirk/engine-renderer/tests/offscreen.rs`:
     `blend_material_blends_instead_of_cutout`. Add a helper
     `view_quad(dist_frac: f32, material: MaterialData) -> PrimitiveData` —
     generalize `camera_filling_quad` (line 327): same eye/right/up math
     (radius 34, angle π/4, pitch 0.8), quad centered at
     `eye + forward * (radius * dist_frac)`, half-extent
     `radius * dist_frac * tan(fovy/2) * 1.02` so it fills the frame at any
     fraction. Scene: `set_uniform_environment([1,1,1])`,
     `set_light(direction Y, color ZERO, ambient 1.0)`; one MeshData with
     prim0 = `view_quad(1.0, white opaque: base_color [1,1,1,1], roughness 1,
     metallic 0)` and prim1 = `view_quad(0.5, red blend: base_color
     [1,0,0,0.6], alpha_mode Blend, roughness 1, metallic 0)`. Assert
     analytically on the full-frame channel means: `g_mean > 25` (the white
     layer shows through — today's cutout punch renders opaque red, g≈0) and
     `r_mean > g_mean * 1.3` (the glass tints red). Run it, watch it fail on
     current code.
  2. Second test, sort order: `stacked_glass_composites_back_to_front`.
     Same lighting; prim0 = white opaque `view_quad(1.0, …)`, then — in
     deliberately adversarial vec order — prim1 = near **blue** glass
     `view_quad(0.5, base_color [0,0,1,0.5], Blend)` and prim2 = far **red**
     glass `view_quad(0.75, base_color [1,0,0,0.5], Blend)`. Correct
     back-to-front gives layer weights near 0.5 blue / 0.25 red / 0.25
     white → assert `b_mean > r_mean`; drawing in primitive order (unsorted)
     gives 0.5 red / 0.25 blue → `r_mean > b_mean`, so the assertion is a
     real sort probe.
  3. Implement (a)–(d). The lib.rs `generated_shader_tests` parse test
     validates both edited WGSL files automatically.
  4. Confirm both new tests pass and the pre-existing offscreen suite is
     untouched (in particular `masked_cutout_edge_has_intermediate_resolved_pixels`
     and the two bloom tests — they exercise the mask path and REPLACE
     pipeline, which this step must not perturb).
  5. Full gate: `cargo test -p engine-renderer`. If test 1's `g_mean`
     margin proves flaky across adapters, lower the floor to 15 but keep the
     `r_mean > g_mean * 1.3` shape — never assert exact values.

### 3. The frame graph routes transparents: skip in opaque/shadow, sorted draws after the sky

- **Evidence:** `smirk/engine-renderer/src/frame.rs:405-437` (shadow mesh +
  skinned loops), `frame.rs:490-529` (main mesh + skinned loops) — all four
  iterate every `gpu_mesh.primitives` entry unconditionally;
  `frame.rs:532-536` — the sky draw ends the main pass with nothing after
  it; `state.rs:39-45` — `RendererState` has one `mesh_pipeline` and one
  `skinned_pipeline`; `skinned_pipeline.rs:65-147` — the skinned factory has
  no transparent variant; `state.rs` `create_scene_pipelines`/
  `create_skinned_pipeline_resources` build one of each.
- **Ideal:** in the real frame, blend primitives are skipped by all four
  opaque/shadow loops and drawn once, after the sky, in a single sorted
  back-to-front sequence spanning static and skinned instances, through
  `mesh_transparent_pipeline` / `skinned_transparent_pipeline`, reusing the
  already-uploaded instance buffers via `instance..instance+1` draws.
- **Gap:** step 2 made blend materials encode `mr.w = 1.0`, but the frame
  still pushes them through the opaque pipelines (premultiplied output under
  REPLACE) and the shadow pass; nothing sorts or draws them correctly on
  screen. (No shipped asset triggers this today — statue_vroid is
  bench-only — but the seam must close.)
- **Suggestion:**
  (a) `skinned_pipeline.rs::create_skinned_pipeline` gains the same
  `transparent: bool` parameter as the mesh factory (same three descriptor
  changes, label `"Skinned Pipeline (transparent)"`).
  (b) `RendererState` gains `mesh_transparent_pipeline` and
  `skinned_transparent_pipeline` fields; `create_scene_pipelines` and
  `create_skinned_pipeline_resources` return both variants (opaque `false`,
  transparent `true`).
  (c) `frame.rs`: all four prim loops gain `if prim.blend { continue; }`.
  (d) `frame.rs`: a record-time collector,
  ```rust
  pub(crate) struct TransparentDraw {
      skinned:  bool,
      mesh_idx: usize,
      prim_idx: usize,
      instance: u32,
      depth_sq: f32,
  }
  fn collect_transparent_draws(
      store: &MeshStore,
      mesh_list: Option<&MeshDrawList>,
      skinned_list: Option<&SkinnedDrawList>,
      eye: glam::Vec3,
      out: &mut Vec<TransparentDraw>,
  )
  ```
  — clears `out`; for each mesh range applies the same
  `first >= MAX_MESH_INSTANCES` break / count clamp as the opaque loop
  (frame.rs:496-497), then for each `blend` primitive and each instance
  computes `model.transform_point3(prim.centroid)` from the CPU-side
  `list.instances[i].model` (`Mat4::from_cols_array_2d`) and
  `depth_sq = eye.distance_squared(world_centroid)`; skinned ranges the same
  from `skinned_list` (no extra cap — sync already enforces
  `MAX_SKINNED_INSTANCES`). Sort
  `out.sort_by(|a, b| b.depth_sq.total_cmp(&a.depth_sq))`.
  `RenderSystem` gains a reusable `transparent_draws: Vec<TransparentDraw>`
  scratch field (the `gpu_buf` pattern, frame.rs:22); `run` fills it via the
  collector (eye from `state.camera.eye()`) before `record_main_pass`, which
  takes `&[TransparentDraw]` and, after the sky draw, replays it: on each
  `skinned` flip set pipeline + bind groups + instance buffer (static:
  camera→0, env→2, `mesh_instance_buffer`; skinned: camera→0, joints→2,
  env→3, `skinned_instance_buffer` — group indices per the opaque loops at
  frame.rs:492-518), then per item bind `material_bind_group` at 1, set the
  prim's vertex/index buffers, `draw_indexed(0..index_count, 0,
  instance..instance+1)`.
  (e) Update frame.rs's module header (line 1-2) to
  "shadow → main (SDF/mesh/skinned/sky/transparent) → particles → …".
- **Path:**
  1. Fail-first unit test for the collector in a new
     `#[cfg(all(test, feature = "offscreen"))] mod tests` in `frame.rs`
     (HeadlessGpu skip pattern from store.rs:333). Build a `MeshStore` via
     `register` with: mesh "glass+solid" = prim0 opaque + prim1 Blend
     (triangle vertices (0,0,0)/(1,0,0)/(0,1,0) → centroid (0.5,0.5,0));
     and a second registered mesh "glass2" = one Blend prim, same vertices.
     For the skinned arm, `register` a `MeshData` whose `skeleton` is a
     1-joint stub (the `stub_skin` construction pattern at sync.rs:315-331,
     inlined) and whose single primitive is Blend. Hand-build a
     `MeshDrawList` with instances at translations (0,0,0) and (0,0,-10)
     ranged over the two meshes, a `SkinnedDrawList` with one instance at
     (0,0,-5) (`joint_base` 0, joints vec irrelevant to the collector), and
     `eye = (0,0,10)`. Assert: the opaque prim never appears; the result
     interleaves static and skinned strictly by descending `depth_sq`
     (expected instance order: z=-10, z=-5 skinned, z=0); each item carries
     the right `(mesh_idx, prim_idx, instance)`. Also assert a mesh range
     starting at `first >= MAX_MESH_INSTANCES as u32` contributes nothing.
     Written against the not-yet-existing function first — compile failure
     is the fail-first signal here.
  2. Implement (a)–(e).
  3. Re-run the collector test to green; run the full offscreen integration
     suite (step 2's tests prove the shader/pipeline half this step reuses).
  4. Full workspace gate (`cargo test --workspace` per the repo's loop-final
     convention), zero new warnings — the client crate links `RenderPlugin`
     and must compile against the new `RendererState` fields untouched.

### 4. Close-out: future-work note and queue strike (docs-only)

- **Evidence:** `docs/visual-quality.md:133-136` — the future-work list
  (CSM, SSAO, TAA, GPU particles, LOD, frustum culling, KTX2/Basis,
  creature pipeline) says nothing about transparency;
  `docs/reviews/rendering/reworks-rendering-2026-07-16.md:19-32` — the
  cross-type queue note still lists rework 3 as open.
- **Ideal:** the accepted approximations are recorded where future audits
  look, and the queue reflects reality (the reworks-queue-mark-done
  convention: strike the finished rework unprompted).
- **Gap:** none in code — bookkeeping only.
- **Suggestion:** one docs commit, no source files.
- **Path:**
  1. In `docs/visual-quality.md`'s future-work list append
     "order-independent transparency (sorted per-primitive blending shipped;
     intersecting transparents and particle-vs-glass ordering remain
     approximate)".
  2. In `docs/reviews/rendering/reworks-rendering-2026-07-16.md`'s queue
     note, extend the strikethrough to cover "rework 3" and append a done
     line naming the date, `plan-rendering-rework-3-2026-07-16.md`, and the
     step count (4), matching the format used for reworks 1–2.
  3. No test — verify by reading the rendered markdown.
