# Plan: Scene scalability — nothing is culled, dead SDF slots still cost vertices — 2026-07-16

Source: docs/reviews/rendering/reworks-rendering-2026-07-16.md finding 5.

## Ideal end state

Frame cost tracks the visible set instead of the allocated set. The SDF passes
draw only live pool slots (freed slots cost nothing, forever). The static and
skinned mesh draw lists are culled per volume: the main pass draws only
instances whose world-space bounds intersect the camera frustum, the shadow
pass draws only instances inside the sun's fitted ortho volume — with zero
image change, because both tests are exactly the volumes the GPU already clips
against. Skinned rigs beyond 40 units from the camera target pose at half rate
from a cached palette, and rigs visible in neither volume are not posed at
all. Bounds come for free out of the existing per-vertex upload loop; a
criterion bench pins the added CPU cull cost in BASELINE.md, and the dev
overlay shows drawn-vs-total so the scaling property is observable live.

## Design decisions

**Cull where the draw lists are built (sync.rs), using one `Frustum` type for
both volumes.** `MeshRenderSyncSystem` already borrows `RendererState`
(sync.rs:181), which owns the camera and `light_dir`; the camera frustum comes
from `state.camera.build_view_projection_matrix()` (camera.rs:59-78) and the
shadow volume from `shadow::fit_light_vp(state.camera.target, state.light_dir)`
(shadow.rs:49-75) — the *identical* matrices the render pass uploads later the
same frame, so culling is image-preserving by construction (anything culled
was going to be clipped). Gribb–Hartmann plane extraction works unmodified for
perspective, orthographic, and the sun box (wgpu clip z ∈ [0,1] variant).
*Rejected:* culling at record time in frame.rs (would re-derive world AABBs
per pass from instance matrices — same work twice, and the instance upload
would still carry dead weight). *Accepted skew:* a window resize applied via
menu actions changes the aspect inside `RenderSystem::run`, one phase after
sync classified — a single conservative frame during resize, invisible.

**One instance buffer, two range lists — no second GPU buffer.** Draw ranges
are already `Vec<(mesh_idx, first, count)>` with nothing assuming one range
per mesh (frame.rs:423, 512, 537). Per mesh, visible instances are packed in
category order `[Both][CamOnly][ShadowOnly]`; the camera view is then one
contiguous range (`Both`+`CamOnly`) and the shadow view is at most two
(`Both`, then `ShadowOnly`) — both lists index the same uploaded instance
array. Instances visible in neither volume are dropped before packing, so
upload size shrinks too, and cap pressure (`MAX_MESH_INSTANCES` = 16 384,
state.rs:27; `MAX_SKINNED_INSTANCES` = 256) eases rather than grows.
*Rejected:* dedicated shadow instance buffers (new state.rs buffers, double
upload, for byte-identical data); emitting visible "runs" over the uncalled
full list (visibility is spatially random within a mesh group — fragmentation
degenerates to one draw call per instance).

**Bounds are computed at upload in `upload_mesh`, not at glTF import.**
store.rs:117-128 already loops every vertex computing a min/max just to
collapse it into `centroid` — the AABB is that loop's intermediate. Keeping it
covers both asset paths through one seam: streamed glTF (`integrate`) and
procedural ground (`register`). `GpuPrimitive.centroid` is replaced by
`aabb: Aabb` with a `centroid()` method (one source of truth; the transparent
sort at frame.rs:623/637 and offscreen.rs:400-402 switch to the method).
`GpuMesh` gains `local_aabb` = union over primitives, inflated ×2.0 about its
center on each half-extent when the mesh is skinned — a conservative constant
covering animation excursions from bind pose (locomotion and attack clips stay
well inside double the bind volume; popping-safe, still culls fully off-screen
rigs). *Rejected:* per-primitive culling (nothing needs it — instances are the
unit of visibility), animation-driven dynamic bounds (per-frame state for a
problem the inflation constant solves).

**SDF slots get dead-slot skipping, not frustum culling — a deliberate
divergence from the finding's Ideal.** The pool's contract is stable slot
indices + dirty-range delta upload (instance.rs:1-4, frame.rs:55-78);
per-frame visibility repacking would destroy exactly that design. And the SDF
content is combat-local by nature (telegraphs, hit reactions, class body
accents — client/vordar-client/src/{telegraph,hit_react,body}.rs), i.e.
almost always in view, at 36 indices per live slot. The structural leak the
finding names is the *dead* slots — the high-water-mark draw `0..slot_count`
(frame.rs:416, 502) — and that is fixed exactly: the pool tracks `in_use` and
the passes draw contiguous used runs, so freed slots cost zero vertex fetches
and trailing frees shrink the draw entirely. Revisit trigger: if the pool ever
holds hundreds of world-scattered instances, add frustum classification to
the run builder.

**LOD = half-rate posing beyond 40 units from the camera target; no joint
reduction, no geometry LOD.** 40 matches the AOI radius (a rig farther than
40 from the player is fringe even at max zoom-out); parity on
`(frame + entity.id())` spreads the posing across frames. Skipped frames
replay a cached palette+globals (copied through the existing `PoseScratch` so
socket publication and the joint upload path are untouched) and accumulate the
skipped `dt`, so animation wall-clock speed is exact. Rigs visible in
*neither* volume are not posed at all — the real CPU win — with the already
documented `SocketTransforms` miss contract (sync.rs:119-124) covering their
consumers. *Rejected:* fewer-joints LOD (per-rig skeleton surgery for a cost
half-rate already halves), camera-only pose skipping (the shadow pass needs
posed palettes for off-screen in-volume casters).

**The finding's measurement gate is resolved here, at plan time.** The Path
said to gate scope on per-pass GPU numbers under a synthetic 40-rig +
full-camp scene. No headless skinned-scene harness exists (`OffscreenRenderer`
drives static meshes only, offscreen.rs:373-374), and building one solely to
measure would be new machinery larger than the rework itself; GPU per-pass
numbers remain the dev overlay's job (finding 2's instrument) during the
user's manual feel-check. The scope question the gate would answer is already
answered by the finding ("ordered, not parked — the bar is the ideal"), so
every item lands in its minimal form, and the plan's regression instruments
are: pure-count assertions on the pack/classify seam, a criterion bench for
the added per-frame CPU cull cost (recorded in BASELINE.md), and
drawn-vs-total dev-overlay meters.

No product decision is open; everything above is engineering-derivable.

## Findings (execution order)

### 1. Frustum/AABB math module + mesh bounds captured at upload

- **Evidence:** no culling math exists anywhere in
  `smirk/engine-renderer/src/` (lib.rs:1-28 lists every module). The only
  bounds-adjacent code is store.rs:117-128, where `upload_mesh` loops every
  vertex of each primitive to compute a min/max and then keeps only
  `(min + max) / 2.0` as `GpuPrimitive.centroid` (store.rs:12-22), consumed by
  the transparent sort (frame.rs:623, 637) and by `OffscreenRenderer::render_mesh`
  (offscreen.rs:400-402) and asserted in `upload_records_blend_flag_and_centroid`
  (store.rs:524-570).
- **Ideal:** a `culling` module with `Aabb` and `Frustum` types plus a
  `classify` function, and every `GpuMesh` carrying a `local_aabb` (skinned
  meshes inflated), computed inside the existing vertex loop — zero rendering
  behavior change this step.
- **Gap:** neither the math nor the stored bounds exist.
- **Suggestion:** new file `smirk/engine-renderer/src/culling.rs`, registered
  `pub mod culling;` in lib.rs (public: step 6's bench in the `benchmarks`
  crate uses it). Contents:
  - `pub struct Aabb { pub min: Vec3, pub max: Vec3 }` with `pub fn union(self, other) -> Aabb`,
    `pub fn center(&self) -> Vec3`, `pub fn inflated(&self, factor: f32) -> Aabb`
    (half-extents × factor about the center), and
    `pub fn transformed(&self, m: &Mat4) -> Aabb` using the abs-matrix (Arvo)
    method — new center = `m.transform_point3(center)`, new half-extents =
    component-wise `abs(m.x_axis.xyz()) * he.x + abs(m.y_axis.xyz()) * he.y + abs(m.z_axis.xyz()) * he.z`.
  - `pub struct Frustum { planes: [Vec4; 6] }` with
    `pub fn from_view_proj(m: Mat4) -> Self` — Gribb–Hartmann for wgpu clip
    z ∈ [0,1]: with `r_i = m.row(i)`, planes are `r3+r0`, `r3-r0`, `r3+r1`,
    `r3-r1`, `r2` (near), `r3-r2` (far); no normalization needed for a
    sign-only test — and `pub fn intersects(&self, aabb: &Aabb) -> bool`
    using the p-vertex test: for each plane, pick per-axis `max` where the
    plane's component ≥ 0 else `min`; if `dot(plane.xyz, p) + plane.w < 0`
    return false; else true.
  - `pub enum Visibility { Both, CamOnly, ShadowOnly }` (derive
    `Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug`; declaration order
    exactly this — step 3's packing sorts on it) and
    `pub fn classify(world_aabb: &Aabb, camera: &Frustum, shadow: &Frustum) -> Option<Visibility>`
    returning `None` when the AABB intersects neither.

  In store.rs: `GpuPrimitive.centroid: Vec3` becomes `aabb: Aabb` plus a
  `pub(crate) fn centroid(&self) -> Vec3 { self.aabb.center() }`; the
  store.rs:117-128 loop keeps min/max instead of collapsing (empty primitive →
  `Aabb { min: Vec3::ZERO, max: Vec3::ZERO }`, matching today's fallback).
  `GpuMesh` (store.rs:31-36) gains `pub(crate) local_aabb: Aabb` = union of
  primitive aabbs, then `.inflated(2.0)` iff `skin.is_some()`; add
  `const SKINNED_AABB_INFLATE: f32 = 2.0;` in store.rs with a constraint
  comment (animation must stay within double the bind-pose half-extents — the
  convention culling relies on). Update the three centroid consumers
  (frame.rs:623, 637 → `prim.centroid()`; offscreen.rs:400-402 → same) and the
  existing centroid assertions in `upload_records_blend_flag_and_centroid`.
- **Path:**
  1. Write `culling.rs` unit tests first (pure, no GPU): a perspective frustum
     from `Camera::new(16.0/9.0)`'s VP (camera.rs:59 — `Camera` and
     `build_view_projection_matrix` are `pub(crate)`, same crate) must
     intersect a unit cube at the camera target, reject one 30 units behind
     the eye, reject one 500 units past the target (zfar = 200), and accept
     one straddling a side plane; an ortho frustum from
     `shadow::fit_light_vp(Vec3::ZERO, Vec3::new(-1.0, 2.0, -1.0))` must
     accept a cube at origin and reject one at `(300, 0, 0)` (half-extent 80).
     `transformed`: a unit AABB under a translation+90°-rotation+scale matrix
     yields the expected world box. `classify`: cases for all three variants
     and `None`.
  2. Implement culling.rs to green.
  3. Make the store.rs changes; extend the store test module with
     `upload_records_mesh_aabb`: the existing triangle fixture (verts (0,0,0),
     (1,0,0), (0,1,0)) yields `local_aabb` min (0,0,0) / max (1,1,0), and a
     skinned `MeshData` (reuse the skeleton-stub pattern from
     frame.rs:785-795) yields the ×2-inflated box. Fix the centroid consumers
     and existing assertions.
  4. Full workspace gate: `cargo test` green, zero new warnings; rendering
     output untouched (no draw-path change this step).

### 2. SDF passes draw contiguous used runs — freed slots cost zero vertices

- **Evidence:** `InstancePool` (instance.rs:39-79) zeroes freed slots and
  pushes them to a free list, but `slots.len()` never shrinks; both SDF draws
  step the whole high-water mark: `record_shadow_pass` does
  `pass.draw_indexed(0..INDICES.len() as u32, 0, 0..slot_count as u32)`
  (frame.rs:416) and `record_main_pass` the same (frame.rs:502), where
  `slot_count` is `pool.slots.len()` from `collect_dirty_ranges`
  (frame.rs:55-78, returned at 71). Every slot ever allocated costs 36 index
  fetches per pass, forever.
- **Ideal:** both SDF draws iterate contiguous runs of in-use slots; freed
  slots — interior and trailing — are never drawn. Delta upload (dirty
  ranges) is untouched.
- **Gap:** the pool doesn't know which slots are live in a scan-friendly form
  (only a free-index Vec), and the draws take a single count.
- **Suggestion:** in instance.rs, add `in_use: Vec<bool>` to `InstancePool`
  (alloc reuse path sets `self.in_use[idx] = true`, grow path pushes `true`,
  `free` sets `false`; initialize with capacity in `new`). Add
  `pub(crate) fn used_runs(&self, out: &mut Vec<(u32, u32)>)` — clear `out`,
  scan `in_use`, push `(first, count)` per maximal true-run. In frame.rs,
  give `RenderSystem` a reused `sdf_runs: Vec<(u32, u32)>` scratch field,
  fill it right after `collect_dirty_ranges` (one more short `InstancePool`
  borrow), and change `record_shadow_pass` / `record_main_pass` to take
  `sdf_runs: &[(u32, u32)]` instead of `slot_count: usize`, drawing
  `for &(first, count) in sdf_runs { pass.draw_indexed(0..INDICES.len() as u32, 0, first..first + count); }`.
  An empty run list draws nothing (previously `0..0`).
- **Path:**
  1. Fail-first unit tests in instance.rs: alloc 5 slots → runs `[(0,5)]`;
     free indices 1 and 3 → `[(0,1),(2,1),(4,1)]`; additionally free 4 →
     `[(0,1),(2,1)]` (trailing free excluded); re-alloc (pops the free list)
     → the returned index turns live again in the runs and is marked dirty;
     invariant in every case: `sum(count) == pool.used()`.
  2. Implement the pool changes, then the frame.rs rewiring.
  3. Gate: workspace tests green (offscreen SDF tests at
     `tests/offscreen.rs` — `cube_renders_with_coverage_and_color`,
     `nearer_cube_occludes_farther_cube` — go through `render_sdf`'s own
     buffer, not the pool, and must stay green untouched); zero new warnings.

### 3. Static mesh draw list culled per camera and light volume

- **Evidence:** `MeshRenderSyncSystem` pushes every `(Transform, RenderMesh)`
  entity unconditionally (sync.rs:184-243) and packs by mesh index into
  `MeshDrawList { instances, ranges }` (sync.rs:92-96, 251-259). The main
  pass draws `list.ranges` (frame.rs:507-525), the shadow pass draws the
  *same* `list.ranges` (frame.rs:419-435), and `collect_transparent_draws`
  walks `list.ranges` too (frame.rs:614-628). Nothing is tested against any
  volume. Step 1 landed `culling::{Aabb, Frustum, Visibility, classify}` and
  `GpuMesh.local_aabb`; step 2 is independent of this one.
- **Ideal:** the main pass draws only camera-frustum-intersecting static
  instances, the shadow pass only sun-volume-intersecting ones, from one
  shared instance buffer; instances in neither volume never reach the GPU.
  Image identical (both volumes are exactly what the GPU clips against). A
  dev-overlay line shows drawn/total.
- **Gap:** no classification, no per-volume ranges, shadow and main share one
  list.
- **Suggestion:**
  - `MeshDrawList` (sync.rs:92-96) gains
    `pub(crate) shadow_ranges: Vec<(usize, u32, u32)>`, cleared alongside
    `ranges` (sync.rs:169-170).
  - In `MeshRenderSyncSystem::run`, while the `RendererState` borrow is live
    (sync.rs:181), build once:
    `let cam = Frustum::from_view_proj(state.camera.build_view_projection_matrix());`
    `let sun = Frustum::from_view_proj(crate::shadow::fit_light_vp(state.camera.target, state.light_dir));`
    (all `pub(crate)`, same crate). Keep `get_or_request` first in the entity
    loop (streaming prefetch must not depend on visibility); after `model` is
    computed, classify:
    `let world = store.meshes[idx].local_aabb.transformed(&model);`
    `let Some(vis) = classify(&world, &cam, &sun) else { continue };` and push
    `(idx, vis, MeshInstance { .. })` — `items` becomes
    `Vec<(usize, Visibility, MeshInstance)>`.
  - Replace the pack loop (sync.rs:251-259) with a generic pure function in
    sync.rs, shared with step 4:
    ```rust
    pub(crate) fn pack_visible<T: Copy>(
        items: &mut Vec<(usize, Visibility, T)>,
        instances: &mut Vec<T>,
        ranges: &mut Vec<(usize, u32, u32)>,        // camera view: Both + CamOnly
        shadow_ranges: &mut Vec<(usize, u32, u32)>, // sun view: Both, then ShadowOnly
    )
    ```
    Sort `items` by `(mesh_idx, visibility)` (the `Ord` derive from step 1
    gives `Both < CamOnly < ShadowOnly`), then per mesh group append all
    instances in that order and emit: one camera range covering the
    `Both`+`CamOnly` prefix (skip if zero), one shadow range covering `Both`
    (skip if zero), one shadow range covering the `ShadowOnly` suffix (skip
    if zero). Both output lists stay ascending by `first`, so the existing
    `first >= MAX_MESH_INSTANCES { break }` clamps (frame.rs:424, 513, 616)
    keep working.
  - frame.rs: `record_shadow_pass`'s static-mesh loop (frame.rs:423) iterates
    `list.shadow_ranges` instead of `list.ranges`. Main pass and
    `collect_transparent_draws` keep `list.ranges` (now the camera view —
    transparent culling falls out for free).
  - Dev overlay: in sync's stats block (sync.rs:273-277), add
    `stats.set("statics drawn", format!("{}/{}", camera-visible instance count, total entity count))`.
- **Path:**
  1. Fail-first unit tests for `pack_visible` in sync.rs's test module with
     `T = u32` sentinel payloads: two meshes, instances covering every
     `Visibility` variant → assert exact `instances` order
     (`[Both..][CamOnly..][ShadowOnly..]` per mesh), exact `ranges` and
     `shadow_ranges` tuples, and that a mesh with only `ShadowOnly` instances
     appears in `shadow_ranges` alone. Plus a classification integration
     test: with the real `Camera::new` VP and
     `fit_light_vp(camera.target, default sun dir)`, a unit AABB 30 units
     behind the eye but within 80 units of the target classifies
     `ShadowOnly`, and one 300 units off classifies `None`.
  2. Implement: draw-list field, frusta, classify-in-loop, `pack_visible`,
     frame.rs shadow switch, dev-overlay line.
  3. Gate: workspace green, zero new warnings. Offscreen tests are unaffected
     (they bypass sync); the shadow test
     `floating_cube_casts_shadow_band_on_ground` (tests/offscreen.rs:790)
     stays green untouched.

### 4. Skinned draw list culled the same way; invisible rigs are never posed

- **Evidence:** the skinned arm of `MeshRenderSyncSystem` poses every rig
  every frame — `pose_player_into` at sync.rs:219 runs before any visibility
  notion exists — and packs into `SkinnedDrawList { instances, joints, ranges }`
  (sync.rs:100-105, 260-268). The shadow pass re-draws the same skinned
  `ranges` (frame.rs:443), the main pass draws them at frame.rs:537. Step 3
  landed `pack_visible` + `Visibility` + the per-frame `cam`/`sun` frusta in
  this same function, and `GpuMesh.local_aabb` is already ×2-inflated for
  skinned meshes (store.rs, step 1).
- **Ideal:** skinned instances are classified before posing: in neither
  volume → skipped entirely (no pose, no joints, no instance); otherwise
  packed by `pack_visible` with camera ranges for the main pass and
  `shadow_ranges` for the shadow pass. Posing cost now scales with the
  in-volume set.
- **Gap:** skinned path has no classification, one shared range list, and
  poses unconditionally.
- **Suggestion:**
  - `SkinnedDrawList` gains `pub(crate) shadow_ranges: Vec<(usize, u32, u32)>`,
    cleared with the others (sync.rs:171-173).
  - In the skinned match arm (sync.rs:206-241), immediately after resolving
    `cpu_skin` and before the cap checks and `pose_player_into`:
    `let world = store.meshes[idx].local_aabb.transformed(&model);`
    `let Some(vis) = classify(&world, &cam, &sun) else { continue };`
    Rigs skipped here publish no `SocketTransforms` entry — that is the
    documented miss contract (sync.rs:119-124: "consumers treat a missing
    entry as 'no socket this frame' and fall back to an entity-relative
    offset"), and any attachment is inside the rig's inflated bounds, so it
    culls with its owner.
    `skinned_items` becomes `Vec<(usize, Visibility, SkinnedMeshInstance)>`;
    the pack loop (sync.rs:260-268) becomes a `pack_visible` call.
  - frame.rs: the skinned shadow loop (frame.rs:443) iterates
    `list.shadow_ranges`.
  - Dev overlay: extend the existing block (sync.rs:273-277) with
    `stats.set("skinned drawn", format!("{}/{}", camera-visible skinned instance count, posed-entity total))`.
    The existing `"skinned"` cap meter stays as is (it guards
    `MAX_SKINNED_INSTANCES`).
- **Path:**
  1. Fail-first test: extend the step-3 `pack_visible` suite with a
     `SkinnedMeshInstance`-shaped case (payload carrying `joint_base`) proving
     packing preserves each instance's `joint_base` while reordering — the
     seam that would corrupt rigs if packing and joint upload desynced (joints
     are appended at pose time, before packing, so `joint_base` must travel
     with its instance).
  2. Implement the classification, list field, pack switch, frame.rs shadow
     switch, meter.
  3. Note for the worker: the joint-budget and instance-cap checks
     (sync.rs:213-217) stay where they are but now run only for in-volume
     rigs — that is the intended improvement, no compensating change needed.
  4. Gate: workspace green, zero new warnings.
     `human_locomotion_clips_actually_animate` and
     `pose_player_into_stops_growing_scratch_buffers_after_warmup`
     (sync.rs:300, 338) are pure posing tests and must stay green untouched.

### 5. Distance LOD: rigs beyond 40 units pose at half rate from a cached palette

- **Evidence:** every in-volume skinned rig is posed every display frame
  (sync.rs:219, `pose_player_into` — sample + blend + globals + palette), the
  cost that quadruples when enemies land
  (docs/benchmarks/BASELINE.md "Render CPU": `joint_palette_40x64` ≈ 105 µs
  at the 40-rig stress figure). No distance policy exists. Step 4 gave the
  skinned arm a classification gate; `state.camera.target` is available where
  the frusta are built.
- **Ideal:** rigs farther than 40 units from the camera target pose every
  other frame; skipped frames replay the last palette/globals and bank the
  skipped `dt`, so animation speed stays wall-clock exact; near rigs are
  untouched; despawned rigs leak no cache.
- **Gap:** no pose cache, no frame parity, no distance test.
- **Suggestion:** in sync.rs:
  - `pub(crate) const LOD_POSE_DISTANCE: f32 = 40.0;` with a constraint
    comment tying it to the AOI radius (rigs past the AOI edge are fringe at
    max zoom).
  - ```rust
    pub(crate) struct LodEntry {
        palette:    Vec<Mat4>,
        globals:    Vec<Mat4>,
        pending_dt: f32,
        last_seen:  u64,
    }
    ```
    `MeshRenderSyncSystem` gains `lod: HashMap<Entity, LodEntry>` and
    `frame: u64` (incremented once per `run`).
  - A testable free function next to `pose_player_into`:
    ```rust
    pub(crate) fn pose_with_lod(
        lod:     &mut HashMap<Entity, LodEntry>,
        entity:  Entity,
        frame:   u64,
        far:     bool,
        player:  &mut AnimationPlayer,
        skin:    &CpuSkin,
        delta:   f32,
        scratch: &mut PoseScratch,
    )
    ```
    Behavior: `!far` → `lod.remove(&entity)` then
    `pose_player_into(player, skin, delta, scratch)`. `far` → look up the
    entry; pose when the entry is absent or
    `(frame + entity.id() as u64) % 2 == 0` (if `hecs::Entity::id()` is
    unavailable in the pinned hecs version, use `entity.to_bits().get()` —
    do not explore further, either is just a parity source), calling
    `pose_player_into` with `delta + entry.pending_dt`, resetting
    `pending_dt`, and refreshing the entry's `palette`/`globals` from scratch
    via `clear()` + `extend_from_slice` (warm capacity → zero steady-state
    allocation); otherwise `pending_dt += delta` and copy the cached
    `palette`/`globals` back into `scratch.palette`/`scratch.globals` (same
    clear+extend) so every caller downstream — joint append at sync.rs:220,
    socket publication at sync.rs:222-234 — reads scratch uniformly. Always
    stamp `last_seen = frame`.
  - Call site: replace the sync.rs:219 `pose_player_into` call with
    `pose_with_lod(...)`, where
    `far = transform.position.distance_squared(camera_target) > LOD_POSE_DISTANCE * LOD_POSE_DISTANCE`
    (`camera_target` captured alongside the frusta). After the entity loop:
    `self.lod.retain(|_, e| e.last_seen == self.frame);`.
- **Path:**
  1. Fail-first unit tests in sync.rs's test module using the existing
     `stub_skin` helper (sync.rs:316-332) and entities minted from a scratch
     `hecs::World`:
     - far rig across 4 consecutive frames at `delta = 0.016`: exactly 2
       poses occur; after frame 4, `player.time` equals `4 × 0.016` within
       1e-6 (banked dt replayed); on each skipped frame `scratch.palette`
       is bit-equal to the previous posed frame's palette and `player.time`
       is unchanged;
     - near rig (`far = false`): posed all 4 frames, no `lod` entry remains;
     - eviction: a far rig posed at frame N, absent at frame N+1 → after the
       retain, the map no longer contains it.
  2. Implement `pose_with_lod`, the call-site switch, the retain.
  3. Gate: workspace green, zero new warnings; the two existing posing tests
     (sync.rs:300, 338) stay green untouched.

### 6. Cull-cost bench in render_cpu; BASELINE row

- **Evidence:** `benchmarks/benches/render_cpu.rs` holds the render-side CPU
  baselines (`joint_palette_40x64`, `particle_fill_4096`) that guard the
  enemy-influx figures; docs/benchmarks/BASELINE.md has a "Render CPU —
  `render_cpu`" table (around line 157). Steps 1-5 added a per-instance
  per-frame cost — `Aabb::transformed` + `classify` against two frusta — that
  has no baseline. `engine_renderer::culling` is a public module (step 1).
- **Ideal:** a criterion bench pins the classification cost at the stress
  population (40 rigs + 512 statics = 552 instances), recorded in
  BASELINE.md, so future audits see when cull cost itself becomes material.
- **Gap:** no bench, no number.
- **Suggestion:** add to render_cpu.rs (imports:
  `engine_renderer::culling::{Aabb, Frustum, classify}`):
  `fn frustum_classify(c: &mut Criterion)` benching `frustum_classify_552` —
  setup builds a perspective frustum
  (`Mat4::perspective_rh(45f32.to_radians(), 16.0/9.0, 0.1, 200.0) * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y)`
  with eye ≈ `(24.0, 22.7, 24.0)`, the default orbit geometry) and an ortho
  frustum
  (`Mat4::orthographic_rh(-80.0, 80.0, -80.0, 80.0, 0.0, 400.0) * Mat4::look_at_rh(sun_dir * 200.0, Vec3::ZERO, Vec3::Y)`
  with `sun_dir = Vec3::new(-1.0, 2.0, -1.0).normalize()` — the
  `fit_light_vp` shape without depending on the `pub(crate)` function), plus
  552 unit `Aabb`s and model matrices scattered deterministically over a
  ±80-unit square (index-hash positions, no RNG dependency); per iteration:
  transform each local AABB and `classify` it against both frusta,
  `black_box`-accumulating the variant counts. Register in the
  `criterion_group!`.
- **Path:**
  1. Add the bench; `cargo bench -p vordar-benches --bench render_cpu -- frustum_classify_552`.
  2. Record the measured time as a new row in BASELINE.md's Render CPU table
     with the note "per-frame cull cost at 40 rigs + 512 statics (rendering
     rework 5)". Expected order: single-digit microseconds. If it measures
     above 100 µs, do not investigate — record the number, and check only
     that `Aabb::transformed` uses the abs-matrix form (not an 8-corner
     transform) and that `Frustum::intersects` does no per-plane
     normalization; if it still exceeds 100 µs after that bounded check,
     record it with a "surprisingly high — flagged for next audit" note and
     proceed. Only a number above 1 ms is a stop-and-report.
  3. Gate: workspace green (benches compile under the workspace check), zero
     new warnings.

### 7. Close-out: future-work list and reworks queue reflect what landed (docs-only)

- **Evidence:** `docs/visual-quality.md:135` still lists "LOD, frustum
  culling" among future work ("Cascaded shadow maps, SSAO, TAA, GPU
  particles, LOD, frustum culling, KTX2/Basis transcoding, ..."); the
  cross-type queue note in
  `docs/reviews/rendering/reworks-rendering-2026-07-16.md` (lines 19-42)
  shows "rework 5 → rework 6" outside the struck-through span and says
  "Reworks 5–6 remain open, none planned yet".
- **Ideal:** the docs state what is now true: frustum culling and shadow-volume
  culling shipped, skinned pose-rate LOD shipped, geometry LOD remains
  future work; the queue note strikes rework 5 with its plan reference and
  step count, following the exact pattern the note already uses for reworks
  1-4.
- **Gap:** both documents still describe the pre-rework state.
- **Suggestion:** two edits, no source files:
  - visual-quality.md:135: replace "LOD, frustum culling" with
    "mesh-geometry LOD" (frustum + shadow-volume culling landed; pose-rate
    LOD landed; only geometry LOD levels remain future work). Leave every
    other list item untouched.
  - reworks-rendering-2026-07-16.md queue note: extend the strikethrough to
    cover "rework 5" and append a done-sentence in the established style:
    "Rework 5 done 2026-07-16 (plan-rendering-rework-5-2026-07-16.md,
    7 steps; SDF used-run draws, upload-time AABBs, camera + sun-volume
    culled draw lists over one instance buffer, half-rate pose LOD beyond
    40 u, `frustum_classify_552` baseline — loop-final gate N/N)" — fill N/N
    with the actual final test count from the last implementation step's gate.
    Also update "Reworks 5–6 remain open" to "Rework 6 remains open, not
    planned yet".
- **Path:**
  1. Make both edits.
  2. Verify: re-read both changed hunks; confirm no other doc references
     rework 5 as open (`grep -rn "rework 5" docs/reviews/rendering/`).
