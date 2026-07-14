# Plan: Decompose engine-renderer lib.rs (1,526) and mesh.rs (1,396) — 2026-07-14

Source: `docs/reviews/hygiene/reworks-hygiene-2026-07-14.md` finding 3.

## Ideal end state

`smirk/engine-renderer/src/lib.rs` shrinks to module decls + re-exports +
`RenderPlugin` (~80 lines); its tenants move to files whose names predict them:
`state.rs` (RendererState + init/resize + the window-ready/resize wiring),
`facade.rs` (the 19 public free functions + `TextureHandle` + `CameraConfig`),
`frame.rs` (RenderSystem — the frame graph), `menu_actions.rs` (the deferred
menu-action applier), `instance_sync.rs` (SaveTransformSystem, RenderSyncSystem,
slot attach/despawn); `CycleCameraSystem` joins `camera.rs`; `ParticleDrawList`
joins its siblings in `particle_pipeline.rs`. `mesh.rs` becomes a `mesh/` family:
`mesh/gltf_import.rs` (CPU parse), `mesh/store.rs` (GPU upload + MeshStore),
`mesh/sync.rs` (per-frame MeshRenderSyncSystem + draw lists + sockets),
`mesh/test_glb.rs` (`#[cfg(test)]` GLB writers), `mesh/mod.rs` (decls +
re-exports). Behavior is bit-identical: `RenderPlugin::build`'s registration
sequence and `RenderSystem::run`'s pass order are preserved verbatim, every
external path (`engine_renderer::mesh::load_gltf_data`,
`engine_renderer::{RenderSyncSystem, MeshRenderSyncSystem, ParticleDrawList,
CameraConfig, UiLayers, SocketTransforms, SocketConfig, MeshVertex, set_light,
screen_to_ground, …}`) still resolves via re-exports so `client/`, `game/`,
`benchmarks/`, and both integration-test binaries compile untouched, and every
step leaves `cargo nextest run -p engine-renderer` green at unchanged test
counts (31 unit + 14 integration today; 12 of the unit tests move homes) with
the full workspace gate green at the end.

## Design decisions

- **`mesh.rs` becomes a `mesh/` dir family, not flat siblings.** The finding's
  Ideal names flat files (`gltf_import.rs`, `mesh_store.rs`, `mesh_sync.rs`), but
  external callers import module-qualified paths — `engine_renderer::mesh::{load_gltf_data,
  MeshData, MaterialData, PrimitiveData, ImageData, load_image_rgba}` at
  `client/vordar-client/src/weapons.rs:18`, `ground.rs:9`,
  `game/vordar-game/tests/content_lint.rs:11`, `tests/offscreen.rs:9` — so flat
  files would force either caller edits or a `mesh.rs` reduced to a pure re-export
  shim (a file whose name no longer predicts its contents). A `mesh/` directory
  keeps `engine_renderer::mesh::X` valid with zero caller churn and mirrors the
  `net/` families from hygiene reworks 1-2. Inside the dir the names drop the
  stutter: `mesh/store.rs`, `mesh/sync.rs` (`mesh::mesh_store` was rejected).
- **lib.rs decomposes into flat top-level siblings** — the crate root cannot
  itself become a directory, and the five tenants are peers of the existing
  `camera.rs`/`menu.rs`/`pipeline.rs` files, so `state.rs`, `facade.rs`,
  `frame.rs`, `menu_actions.rs`, `instance_sync.rs` join that flat family.
  `instance_sync.rs` (a home the finding's list omits, decided here) takes the
  four systems that keep the SDF `InstancePool` in sync with the world:
  `SaveTransformSystem` (the PreviousTransform snapshot both sync systems lerp
  against), `RenderSyncSystem` + `shape_to_gpu`, `RenderSlotAttachSystem`,
  `RenderSlotDespawnSystem`. A separate `interp.rs` for the 8-line
  SaveTransformSystem was rejected as a one-item file.
- **`CycleCameraSystem` moves into `camera.rs`**, following the crate's existing
  pattern of a domain file owning its input system (`menu.rs` holds `MenuSystem`,
  `dev_overlay.rs` holds `DevOverlaySystem`). A standalone 30-line file was the
  rejected alternative. This adds a `crate::state::RendererState` import to
  camera.rs — an ordinary within-crate sibling dependency.
- **The stray mid-file types get homes by ownership:** `TextureHandle` and
  `CameraConfig` go to `facade.rs` (they exist only as facade-function return/
  input types — `camera.rs` was rejected for CameraConfig because camera.rs is
  `pub(crate)` math and CameraConfig is a game-inserted tuning resource read
  only by `zoom_camera`); `ParticleDrawList` goes to `particle_pipeline.rs`
  beside `ParticleInstance` and `MAX_PARTICLES`, which its one external consumer
  (`client/vordar-client/src/vfx.rs:16`) already imports from the same root
  re-export line.
- **The menu-action applier becomes a free function.** Today it is
  `impl RenderSystem { fn apply_pending_menu_actions(&mut self, …) }` touching
  only `self.pending_menu`. Rust privacy is module-scoped, so an `impl` block in
  `menu_actions.rs` could not see `frame.rs`'s private field without widening it.
  Instead `menu_actions.rs` exports
  `pub(crate) fn apply_menu_actions(actions: Vec<MenuAction>, resources: &mut Resources)`
  and `RenderSystem::run` starts with
  `menu_actions::apply_menu_actions(std::mem::take(&mut self.pending_menu), resources);`
  — `mem::take` is semantically identical to the current `drain(..)` loop
  (empties the Vec, processes every action). Widening the field to `pub(crate)`
  to keep the method shape was rejected: it leaks a scratch field crate-wide to
  preserve a syntax detail.
- **The `pub` surface shrinks to actual external users** (the finding's
  Suggestion asks for exactly this decision; same philosophy as reworks 1-2).
  Workspace grep confirms zero external references to `init`, `on_resize`,
  `RenderSystem`, `SaveTransformSystem`, `CycleCameraSystem`,
  `RenderSlotDespawnSystem`, `RenderSlotAttachSystem` — all become `pub(crate)`
  in their new homes (RenderPlugin, same crate, still registers them).
  Everything else stays `pub` and re-exported at the root: the facade functions
  are the engine's designed game-facing API (e.g. `alloc_render_slot`'s doc says
  "Call from SpawnQueue callbacks") and are kept public even where currently
  uncalled; `RenderSyncSystem` and `MeshRenderSyncSystem` are load-bearing as
  `SystemOrder::before/after` type anchors in `client/vordar-client/src/lib.rs:235-244`
  and `src/net/mod.rs:95-105` (re-exports preserve the TypeId — same type, same
  ordering).
- **Moved code keeps its comments verbatim** (the queue runs this rework before
  the comment-cleanup findings so cleaning happens once, post-split), with
  exactly three exceptions decided here: (1) the stale six-line TODO at
  `lib.rs:747-752` is deleted during the `frame.rs` move — it describes the
  dirty-range scratch design the two fields below it already implement, and
  audit finding 7 explicitly defers it to this rework ("subsumed by rework 3");
  (2) the dead doc line `/// Register via register_render_cleanup(app).` on
  `RenderSlotDespawnSystem` (lib.rs:1400-1401) — that function exists nowhere in
  the workspace — is rewritten to state the real registration; (3) comments in
  OTHER files that cite the dead file layout are retargeted to items, not files,
  in the step that breaks them (`anim.rs:2-3,72` cite "mesh.rs";
  `dev_overlay.rs:5` cites "lib.rs"; `camera.rs:74` cites "zoom_camera in
  lib.rs"). The TEMP anim-feel-check log in MeshRenderSyncSystem
  (`log_accum` + two log blocks) moves VERBATIM — deleting it is audit finding
  7's job, later in the queue.
- **GLB writers become a shared test helper** at `mesh/test_glb.rs`, declared
  `#[cfg(test)] mod test_glb;` in `mesh/mod.rs` with `pub(crate)` writer fns —
  the finding's Ideal asks for "the GLB writers as a test-support helper".
  Today both writers serve only gltf_import tests, but the cfg(test) module
  costs nothing in shipping builds and any future store/sync test can reuse it.
- **Test relocation:** the 9 CPU-parse tests (`loads_triangle_glb…`,
  `missing_file_is_an_error`, `loads_skinned_animated_glb`, the four
  real-asset `*_if_present` probes, `human_clips_stay_above_the_floor`,
  `human_skinned_vertices_stand_on_the_floor`,
  `human_hand_socket_exists_and_moves_during_swing`) move beside
  `load_gltf_data` into `mesh/gltf_import.rs`;
  `human_locomotion_clips_actually_animate` (drives `pose_player`) moves beside
  `pose_player` into `mesh/sync.rs`; lib.rs's two `unproject_to_ground` tests
  move beside the function into `facade.rs`. 12 unit tests total — homes change,
  the count must not.
- **Extraction order: mesh family first, then lib.rs from the inside out**
  (state → facade → frame → sync/plugin). mesh/ first because it is fully
  decoupled from the lib.rs split (`mesh` code references `crate::RendererState`,
  a path that stays valid throughout via a root re-export). Within lib.rs,
  `state.rs` goes first because every later file imports RendererState; `frame.rs`
  (the largest, riskiest move) goes after facade so its diff is pure.
- **Verification per step** (the finding's constraints): (1)
  `cargo nextest run -p engine-renderer` green with pass/skip counts identical
  to a baseline recorded before the step (offscreen/egui GPU tests skip cleanly
  on adapterless machines — compare total counts, pass+skip); (2)
  `cargo check --workspace --all-targets` with zero new warnings — this is also
  the public-API proof: `client/`, `game/`, `benchmarks/` compile with zero
  source changes (`git status` shows no modifications outside
  `smirk/engine-renderer/` and this plan's docs); (3) at the frame.rs step,
  `RenderSystem::run`'s body diffs against git HEAD only in the first
  (menu-apply) line and the deleted TODO — the shadow → main → sky → particle →
  bloom/tonemap → egui pass sequence byte-identical; (4)
  `RenderPlugin::build`'s registration sequence byte-identical apart from `use`
  path spelling; (5) full `cargo nextest run --workspace` at the final code step.
  The behavioral gate for the frame graph is `smirk/engine-renderer/tests/offscreen.rs`
  (13 analytic pixel tests through the real pipeline factories) plus
  `client/vordar-client/tests/ground_render.rs`.
- **No follow-on plan invalidation.** No pending plan in `docs/reviews/` cites
  engine-renderer line numbers (checked: only the hygiene audit/reworks pair
  does, and audit finding 7's `mesh.rs:776` / `lib.rs:747-752` citations are
  handled — the TODO is deleted here; the TEMP-log worker locates by content).
  Historical documents keep their citations.

## Findings (execution order)

### 1. Convert mesh.rs to `mesh/`; extract `mesh/gltf_import.rs` + `mesh/test_glb.rs`

- **Evidence:** `smirk/engine-renderer/src/mesh.rs` (1,396 lines) holds the CPU
  glTF stage at L30-470: `ImageData` (L33), `VertexSkin` (L42), `MaterialData` +
  Default (L51-86), `PrimitiveData` (L88), `MeshData` (L98), `load_gltf_data`
  (L110), `extract_skeleton` (L141), `extract_clips` (L230), `keyframe_values`
  (L297), `visit_node` (L305), `read_material` (L408), `to_rgba8` (L454), plus
  the pure image decoder `load_image_rgba` (L615-622, currently stranded in the
  GPU section). Its `#[cfg(test)] mod tests` (L946-1396) holds two hand-rolled
  GLB writers — `write_test_glb` (L954-1008), `write_skinned_glb` (L1046-1130) —
  and 10 tests, 9 of which exercise only this CPU stage. External importers of
  these items (must keep compiling untouched): `client/vordar-client/src/weapons.rs:18`,
  `ground.rs:9`, `game/vordar-game/tests/content_lint.rs:11`,
  `smirk/engine-renderer/tests/offscreen.rs:9` — all via `engine_renderer::mesh::…`.
  In-crate comments that cite "mesh.rs" and break with this move:
  `smirk/engine-renderer/src/anim.rs:2-3` ("same discipline as mesh.rs's CPU
  stage… mesh.rs constructs `Skeleton` + `AnimationClip`") and `anim.rs:72`
  ("see mesh.rs").
- **Ideal:** `git mv smirk/engine-renderer/src/mesh.rs smirk/engine-renderer/src/mesh/mod.rs`
  (module path `engine_renderer::mesh` unchanged — `lib.rs:8`'s `pub mod mesh;`
  needs no edit). Then `mesh/gltf_import.rs` (declared `mod gltf_import;` in
  mod.rs — private decl, the re-export below carries the visibility, so no new
  `mesh::gltf_import::…` path is added to the API) receives the 13 CPU-stage
  items above verbatim, all currently-`pub`
  types/fns staying `pub`, the private helpers (`extract_skeleton`,
  `extract_clips`, `keyframe_values`, `visit_node`, `read_material`, `to_rgba8`)
  staying private, plus a `#[cfg(test)] mod tests` holding the 9 CPU-parse tests
  (`loads_triangle_glb_with_baked_node_transform`, `missing_file_is_an_error`,
  `loads_skinned_animated_glb`, `loads_real_textured_asset_if_present`,
  `loads_skinned_fox_asset_if_present`, `loads_human_character_asset_if_present`,
  `human_clips_stay_above_the_floor`, `human_skinned_vertices_stand_on_the_floor`,
  `human_hand_socket_exists_and_moves_during_swing`). The two GLB writers move to
  `mesh/test_glb.rs` (declared `#[cfg(test)] mod test_glb;` in mod.rs), fns
  `pub(crate)`, with a 2-line header ("Hand-rolled GLB builders for CPU-stage
  tests — no asset files, no GPU"). mod.rs re-exports so every existing path
  survives: `pub use gltf_import::{load_gltf_data, load_image_rgba, ImageData,
  MaterialData, MeshData, PrimitiveData, VertexSkin};`. The GPU stage, sync
  system, and the 1 remaining test (`human_locomotion_clips_actually_animate`)
  stay in mod.rs for now.
- **Gap:** the CPU parse — the only part testable without a GPU, and the seam
  three other crates import — is fused to GPU upload and a per-frame system in
  one 1,396-line file.
- **Suggestion:** pure moves. gltf_import.rs imports (take exactly what the
  compiler demands): `crate::anim::{AnimationClip, Interp, Joint, JointTracks,
  LocalTransform, Skeleton, Track}`, `crate::mesh_pipeline::MeshVertex`,
  `crate::tangent::generate_tangents`, `glam::{Mat3, Mat4, Quat, Vec3}`,
  `std::collections::HashMap`; its test module needs `super::*` plus
  `super::super::test_glb` (spell it `use crate::mesh::test_glb;`) and
  `crate::anim::{joint_matrices, sample_pose}` where tests use them. test_glb.rs
  needs only `std` (writers take `&std::path::Path`). Fix the two anim.rs
  comment sites to cite items, not files: anim.rs:2-3 → "same discipline as the
  glTF import stage (mesh::gltf_import)… `load_gltf_data` constructs `Skeleton`
  + `AnimationClip` from a glTF file"; anim.rs:72 → "(see `load_gltf_data`)".
  Touch nothing else in mod.rs besides deleting the moved code and adding the
  two `mod` decls + the re-export line; `use crate::RendererState;` and the rest
  of the header stay.
- **Path:** (1) Baseline: run `cargo nextest run -p engine-renderer` and record
  the final counts line (31 unit + 14 integration today; GPU-dependent ones may
  skip). (2) `git mv` to `mesh/mod.rs`; create `mesh/gltf_import.rs` and
  `mesh/test_glb.rs`; move the items + 9 tests + 2 writers; add decls +
  re-export; fix the two anim.rs comments. (3) Verify:
  `cargo nextest run -p engine-renderer` green at identical counts — the 9 moved
  tests drive `load_gltf_data` end-to-end on synthetic GLBs and real assets;
  `cargo check --workspace --all-targets` zero new warnings proves
  weapons.rs/ground.rs/content_lint.rs/offscreen.rs compile untouched
  (`git status` confirms no files changed outside `smirk/engine-renderer/`).

### 2. Extract `mesh/store.rs` and `mesh/sync.rs`; finalize `mesh/mod.rs`

- **Evidence:** after step 1, `smirk/engine-renderer/src/mesh/mod.rs` still holds
  (pre-split mesh.rs line numbers): the GPU stage — `GpuPrimitive` (L474),
  `CpuSkin` (L486), `GpuMesh` (L491), `slot_texture` (L500), `upload_mesh`
  (L516-611), `MeshStore` + `register` + `get_or_load` (L624-676) — and the
  per-frame side — `pose_player` (L684-728), `MeshDrawList` (L735),
  `SkinnedDrawList` (L743), `SocketConfig` + Default (L751-759),
  `SocketTransforms` (L766), `MeshRenderSyncSystem` + `new` + `run` (L772-944)
  — plus the test `human_locomotion_clips_actually_animate` (L1273-1287), which
  constructs a `CpuSkin` and calls `pose_player`. In-crate users outside the
  mesh tree: `frame.rs`-to-be (currently lib.rs L888-890, L1036-1147) takes
  `MeshDrawList`/`SkinnedDrawList`/`MeshStore` out of Resources and reads
  `store.meshes`, `gpu_mesh.primitives`, `prim.vertex_buffer`/`index_buffer`/
  `index_count`/`material_bind_group`, `list.instances`, `list.ranges` — all
  already `pub(crate)`; `src/offscreen.rs:15+287` calls
  `mesh::upload_mesh` (`pub(crate)`); lib.rs `init()` (L640-644) inserts
  `MeshStore::default()` + the draw lists + `SocketConfig`/`SocketTransforms`;
  facade-to-be `register_procedural_mesh` (lib.rs L523-536) calls
  `store.register`. External users of the root re-exports (lib.rs:23):
  `SocketTransforms` (`client/…/weapons.rs:20`, `vfx.rs:252`), `SocketConfig`
  (`game/…/tests/content_lint.rs:12`), `MeshRenderSyncSystem` (SystemOrder
  anchor, `client/…/lib.rs:240-244`, `net/mod.rs:101-105`).
- **Ideal:** `mesh/store.rs` (`mod store;` — private decl, re-exports carry
  visibility): `GpuPrimitive`, `CpuSkin`,
  `GpuMesh`, `slot_texture` (private), `upload_mesh`, `MeshStore` — all moved
  verbatim, visibilities unchanged (`pub(crate)` structs/fns stay `pub(crate)`
  because frame.rs and offscreen.rs live outside the mesh tree; `MeshStore`
  stays `pub`). `mesh/sync.rs` (`mod sync;` — private decl): `pose_player` (private),
  `MeshDrawList`, `SkinnedDrawList`, `SocketConfig`, `SocketTransforms`,
  `MeshRenderSyncSystem`, and a `#[cfg(test)] mod tests` with the moved
  locomotion test. `mesh/mod.rs` reduces to: a short module-map header (CPU
  parse = gltf_import, GPU upload = store, per-frame = sync — derived from the
  current L1-11 comment), the four `mod` decls, and re-exports preserving every
  existing path: `pub use gltf_import::{…};` (from step 1),
  `pub use store::MeshStore;`, `pub(crate) use store::upload_mesh;` (keeps
  `src/offscreen.rs:287`'s `mesh::upload_mesh` call compiling untouched),
  `pub use sync::{MeshDrawList, MeshRenderSyncSystem, SkinnedDrawList,
  SocketConfig, SocketTransforms};`.
- **Gap:** GPU upload and the per-frame system still share one file with the
  module's front door; `pose_player` and its test sit 550 lines apart.
- **Suggestion:** pure moves. store.rs imports: `super::gltf_import::{ImageData,
  MeshData, VertexSkin}` (or via `crate::mesh::…`), `crate::anim::{AnimationClip,
  Skeleton}`, `crate::mesh_pipeline::MaterialUniform`, `crate::mipgen::MipGenerator`,
  `crate::skinned_pipeline::{SkinnedVertex}`, `crate::texture::{self, ColorTexture}`,
  `crate::mesh_pipeline::MeshVertex` if named, wgpu types + `wgpu::util::…`,
  `std::collections::HashMap`. sync.rs imports: `super::store::{CpuSkin, MeshStore}`,
  `crate::mesh_pipeline::MeshInstance`,
  `crate::skinned_pipeline::{SkinnedMeshInstance, MAX_JOINT_MATRICES,
  MAX_SKINNED_INSTANCES}`, `crate::RendererState`, `crate::anim` (sample_pose/
  blend_poses/global_transforms via `crate::anim::` paths as today),
  `engine_app::scheduler::{InterpolationAlpha, System}`,
  `engine_core::components::{AnimationPlayer, PreviousTransform, RenderMesh,
  Transform}`, `engine_core::traits::Resources`, `engine_core::World`,
  `glam::Mat4`, `hecs::Entity`, `std::collections::HashMap`. The TEMP
  `log_accum` field and its two log blocks move VERBATIM (audit finding 7
  deletes them later — do not clean here). Let the compiler prune mod.rs's
  leftover imports (zero-warning gate arbitrates).
- **Path:** (1) Baseline: record `cargo nextest run -p engine-renderer` counts.
  (2) Create the two files, move the code + 1 test, write mod.rs's final header
  + decls + re-exports. (3) Verify: `cargo nextest run -p engine-renderer` green
  at identical counts — behavioral gates: the moved locomotion test drives
  `pose_player` on the real human.glb; `tests/offscreen.rs`'s mesh tests
  (`upload_mesh` through real pipelines) and `client/vordar-client/tests/ground_render.rs`
  exercise the moved store. `cargo check --workspace --all-targets` zero new
  warnings with no source changes outside `smirk/engine-renderer/`
  (proves SocketTransforms/SocketConfig/MeshRenderSyncSystem re-export paths).
  Structure check: `grep -c "impl System" smirk/engine-renderer/src/mesh/mod.rs`
  returns 0.

### 3. Extract `state.rs` (RendererState + init/resize + window wiring)

- **Evidence:** in `smirk/engine-renderer/src/lib.rs`: consts `MAX_INSTANCES` +
  `MAX_MESH_INSTANCES` (L67-68), `RendererState` struct (L70-129, all fields
  already `pub(crate)`), `RendererState::init` (L131-352), `resize` (L354-375),
  `create_particle_fx_bind_group` (L377-394), and the window-lifecycle wiring
  fns `init(window, resources)` (L623-646, inserts WinitEventProcessor,
  MenuState, RendererState, InstancePool, MeshStore, draw lists, SocketConfig/
  Transforms, ParticleDrawList) and `on_resize` (L659-663). Users:
  `RenderPlugin::build` (L1459-1460) registers `init`/`on_resize` as callbacks;
  `RendererState` is read via `resources.get::<RendererState>()` by the facade
  fns (L416-621), RenderSystem (L913), CycleCameraSystem (L1389), and
  `mesh/sync.rs` (`use crate::RendererState;`). Workspace grep confirms zero
  references to `engine_renderer::init` / `engine_renderer::on_resize` /
  `RendererState` outside the crate. `MAX_MESH_INSTANCES` is used by
  RenderSystem's shadow + mesh passes (L1041-1042, L1114-1115, L924);
  `MAX_INSTANCES` only inside init (L241, L349).
- **Ideal:** `smirk/engine-renderer/src/state.rs` (declared `mod state;`):
  everything above moved verbatim — `RendererState` + both impls,
  `create_particle_fx_bind_group` (private), `MAX_INSTANCES` (private — single
  user is in this file), `MAX_MESH_INSTANCES` (`pub(crate)` — frame.rs needs
  it), and `init`/`on_resize` demoted from `pub` to `pub(crate)` (zero external
  users; RenderPlugin registers them from lib.rs). lib.rs adds
  `pub(crate) use state::{init, on_resize, RendererState, MAX_MESH_INSTANCES};`
  so `RenderPlugin::build`'s body and every `crate::RendererState` path in
  mesh/sync.rs stay byte-identical.
- **Gap:** the GPU-resource owner — the struct every system borrows — is fused
  to the crate root alongside 19 API functions and five systems.
- **Suggestion:** pure move + the two visibility demotions. state.rs imports
  (compiler arbitrates the final list): `crate::bloom`, `crate::camera::{self,
  Camera, CameraUniform, LightUniform}`, `crate::ibl`, `crate::instance::{InstancePool,
  SdfInstance}`, `crate::menu::MenuState`, `crate::mesh::{MeshDrawList, MeshStore,
  SkinnedDrawList, SocketConfig, SocketTransforms}`, `crate::mesh_pipeline`,
  `crate::mipgen`, `crate::particle_pipeline`, `crate::pipeline`, `crate::post`,
  `crate::shadow`, `crate::skinned_pipeline`, `crate::texture::{self, ColorTexture}`,
  `crate::ParticleDrawList` (still at root until step 4), `crate::ui_layers`? —
  no: UiLayers is not touched by init; `engine_app::config::WindowConfig`,
  `engine_app::winit_processor::WinitEventProcessor`, `engine_core::traits::Resources`,
  `glam::Vec3 as GlamVec3`, `std::mem::size_of`, `std::sync::{Arc, Mutex}`,
  `winit::window::Window`. Keep the struct's section comments (── meshes ──,
  ── egui ──, …) verbatim.
- **Path:** (1) Baseline: record `cargo nextest run -p engine-renderer` counts.
  (2) Create state.rs, move the items, demote `init`/`on_resize` to
  `pub(crate)`, add the lib.rs re-export line, prune lib.rs imports the compiler
  flags. (3) Verify: `cargo nextest run -p engine-renderer` green at identical
  counts; `cargo check --workspace --all-targets` zero new warnings and no
  source changes outside `smirk/engine-renderer/` (proves nothing external
  named `init`/`on_resize`); `RenderPlugin::build` body diffs against HEAD
  empty. Behavioral gate: `tests/offscreen.rs` + `tests/egui_probe.rs` still
  pass/skip identically (egui_probe drives the real window init path headlessly
  where supported).

### 4. Extract `facade.rs`; move `ParticleDrawList` to `particle_pipeline.rs`

- **Evidence:** in `smirk/engine-renderer/src/lib.rs`: `TextureHandle` (L50-52),
  `CameraConfig` + Default (L54-65), and the 19 public free functions L396-621:
  `alloc_render_slot`, `alloc_shape_group_slots`, `free_render_slot`,
  `update_camera`, `set_camera_target`, `zoom_camera`, `screen_to_ground`,
  `unproject_to_ground`, `set_environment`, `set_exposure`, `set_light`,
  `register_procedural_mesh`, `set_fog`, `create_checker_texture`,
  `load_texture`, `set_texture`, `clear_texture`, `camera_yaw`,
  `camera_movement_axes` — plus the crate's 2 unit tests
  (`unproject_topdown_ortho_hits_expected_ground_point`,
  `unproject_horizontal_ray_misses_ground`, L1491-1526) pinning
  `unproject_to_ground`. `ParticleDrawList` sits mid-file at L648-657. External
  callers (all must compile untouched): `set_light`
  (`client/…/world_time.rs:53`), `set_environment`/`set_fog`/
  `register_procedural_mesh`/`camera_yaw`/`screen_to_ground`
  (`presentation.rs:70-270`), `screen_to_ground` (`cast.rs:109`),
  `register_procedural_mesh` (`weapons.rs:152-153`),
  `camera_movement_axes`/`zoom_camera`/`update_camera` (`client/…/lib.rs:124-157`),
  `CameraConfig` + `UiLayers` (`ui/mod.rs:9`), `ParticleDrawList`/
  `ParticleInstance`/`MAX_PARTICLES` (`vfx.rs:16`). In-crate users:
  `RenderSlotAttachSystem` calls `alloc_render_slot`/`alloc_shape_group_slots`
  (L1444-1448); `state.rs::init` inserts `ParticleDrawList::default()`;
  RenderSystem consumes `ParticleDrawList` (L891-951). `camera.rs:74` has a doc
  comment citing "zoom_camera in lib.rs".
- **Ideal:** `smirk/engine-renderer/src/facade.rs` (declared `mod facade;`):
  module header "The game-facing API: free functions over Resources — the only
  supported way for game/client code to poke the renderer." All 19 fns +
  `TextureHandle` + `CameraConfig` moved verbatim, all staying `pub`, plus a
  `#[cfg(test)] mod tests` with the 2 moved unproject tests. lib.rs adds
  `pub use facade::{alloc_render_slot, alloc_shape_group_slots, camera_movement_axes,
  camera_yaw, clear_texture, create_checker_texture, free_render_slot,
  load_texture, register_procedural_mesh, screen_to_ground, set_camera_target,
  set_environment, set_exposure, set_fog, set_light, set_texture,
  unproject_to_ground, update_camera, zoom_camera, CameraConfig, TextureHandle};`.
  `ParticleDrawList` (struct + doc comment verbatim) moves to
  `particle_pipeline.rs` beside `ParticleInstance`/`MAX_PARTICLES`; lib.rs:25's
  re-export line becomes `pub use particle_pipeline::{ParticleDrawList,
  ParticleInstance, ATLAS_GRID, MAX_PARTICLES};`. camera.rs:74's comment becomes
  "(see `zoom_camera`)".
- **Gap:** the public API is interleaved with systems and state in the crate
  root; two of its types sit mid-file between unrelated systems.
- **Suggestion:** pure moves. facade.rs imports: `crate::camera::{CameraUniform,
  ProjectionMode}`, `crate::ibl`, `crate::instance::{InstancePool, InstanceSlot,
  ShapeGroupSlots}`, `crate::mesh::{self, MeshStore}`, `crate::state::RendererState`
  (via `crate::RendererState` re-export is also fine — match the crate's
  existing spelling), `crate::texture`, `engine_core::traits::Resources`,
  `glam::Mat4`, `glam::Vec3 as GlamVec3`; tests add `glam::Vec2`.
  particle_pipeline.rs gains no new imports (`ParticleInstance` and `Vec` are
  already in scope). Do not reorder or reword any fn docs.
- **Path:** (1) Baseline: record `cargo nextest run -p engine-renderer` counts.
  (2) Create facade.rs, move the 21 items + 2 tests; move ParticleDrawList;
  update the two lib.rs re-export lines; fix camera.rs:74; prune lib.rs imports.
  (3) Verify: `cargo nextest run -p engine-renderer` green at identical counts
  (the 2 moved tests exercise the real unprojection math);
  `cargo check --workspace --all-targets` zero new warnings with no source
  changes outside `smirk/engine-renderer/` — this compiles all ten external
  facade call sites and vfx.rs's ParticleDrawList path through the re-exports.

### 5. Extract `frame.rs` (RenderSystem) + `menu_actions.rs`

- **Evidence:** in `smirk/engine-renderer/src/lib.rs`: `RenderSystem` struct
  (L746-762) opens with the stale six-line TODO (L747-752) that describes the
  dirty-range scratch design its own `gpu_buf`/`dirty_ranges` fields already
  implement (audit-hygiene finding 7: "subsumed by rework 3");
  `GPU_TIMING_INTERVAL` (L764-766); `RenderSystem::new` (L768-779); the
  ~470-line `System::run` (L781-1255) — menu-apply, dirty-range collection,
  egui frame, resource take-out, particle cap guardrail, GPU uploads, shadow
  pass, main pass (SDF → mesh → skinned → sky), particle pass, bloom/tonemap,
  egui pass, present, restore; `restore_mesh_resources` (L1257-1270); and the
  menu applier `impl RenderSystem { fn apply_pending_menu_actions }`
  (L1272-1369), which touches only `self.pending_menu` and Resources.
  `dev_overlay.rs:5` cites "(see lib.rs) because the egui context lives in
  RendererState". Workspace grep: zero external references to `RenderSystem`.
  The behavioral gate for all of this is `tests/offscreen.rs` (13 tests through
  the same pipeline factories) — the finding's constraint (2) forbids any
  pass-order change inside `run`.
- **Ideal:** `smirk/engine-renderer/src/menu_actions.rs` (declared
  `mod menu_actions;`): header "Applies menu actions deferred from the egui
  frame — window mode/resolution/vsync, menu navigation, quit." One function,
  `pub(crate) fn apply_menu_actions(actions: Vec<MenuAction>, resources: &mut Resources)`,
  whose body is the current method's body with `self.pending_menu.drain(..)`
  replaced by `actions` (an owned Vec iterates directly; every other line
  verbatim). `smirk/engine-renderer/src/frame.rs` (declared `mod frame;`):
  header "RenderSystem — the per-frame graph: shadow → main (SDF/mesh/skinned/
  sky) → particles → bloom/tonemap → egui → present." Contents:
  `RenderSystem` demoted to `pub(crate)` (zero external users), the TODO
  deleted (fields + their existing inline comments stay), `GPU_TIMING_INTERVAL`,
  `new`, the `System` impl with `run` verbatim except line 1:
  `self.apply_pending_menu_actions(resources);` becomes
  `crate::menu_actions::apply_menu_actions(std::mem::take(&mut self.pending_menu), resources);`,
  and `restore_mesh_resources` (private). lib.rs keeps
  `use frame::RenderSystem;` for the plugin. dev_overlay.rs:5 cites
  "RenderSystem's egui pass" instead of lib.rs.
- **Gap:** the frame graph — the file a rendering contributor reads first — is
  buried between the facade and four smaller systems in the crate root; a
  settings-application concern hides inside a render system's impl.
- **Suggestion:** frame.rs imports (compiler arbitrates): `crate::dev_overlay`,
  `crate::instance::{InstancePool, SdfInstance, INSTANCE_SIZE}`,
  `crate::menu::{draw_menu, MenuAction, MenuState}`, `crate::mesh::{MeshDrawList,
  MeshStore, SkinnedDrawList}`, `crate::particle_pipeline`,
  `crate::pipeline::INDICES`, `crate::shadow`,
  `crate::state::{RendererState, MAX_MESH_INSTANCES}`, `crate::ui_layers::UiLayers`
  (or the root re-export), `crate::ParticleDrawList`, `std::sync::Arc`,
  `winit::window::Window`. menu_actions.rs imports: `crate::menu::{MenuAction,
  MenuScreen, MenuState, SettingsDraft}`, `crate::state::RendererState`,
  `engine_app::config::{Resolution, WindowConfig, WindowMode}`,
  `engine_core::traits::Resources`, `std::sync::Arc`, `winit::window::Window`
  (the method's local `use` lines at L1274-1276 dissolve into the file header).
- **Path:** (1) Baseline: record `cargo nextest run -p engine-renderer` counts.
  (2) Create menu_actions.rs (free fn), then frame.rs; delete the TODO; wire
  `use frame::RenderSystem;` in lib.rs; prune lib.rs imports. (3) Verify:
  diff `RenderSystem::run` against git HEAD — the ONLY changes are the first
  line's call spelling; the shadow/main/particle/bloom/tonemap/egui sequence,
  every `begin_render_pass` descriptor, and every draw call byte-identical.
  `cargo nextest run -p engine-renderer` green at identical counts — behavioral
  gate: `tests/offscreen.rs`'s 13 analytic frame tests plus
  `client/vordar-client/tests/ground_render.rs` render through the moved graph
  (they use the same pipeline factories; a pass-order slip shows up as coverage/
  luminance failures). `cargo check --workspace --all-targets` zero new
  warnings, no source changes outside `smirk/engine-renderer/`.

### 6. Extract `instance_sync.rs`; move `CycleCameraSystem` into `camera.rs`; final lib.rs shape

- **Evidence:** remaining in `smirk/engine-renderer/src/lib.rs` after step 5
  (pre-split line numbers): `SaveTransformSystem` (L667-678), `RenderSyncSystem`
  (L680-744), `shape_to_gpu` (L1479-1489), `RenderSlotDespawnSystem`
  (L1400-1424) — whose doc (L1400-1401) cites `register_render_cleanup(app)`, a
  function that exists nowhere in the workspace — `RenderSlotAttachSystem`
  (L1426-1452), `CycleCameraSystem` (L1371-1398), `RenderPlugin` (L1454-1477),
  and the crate-root decls/re-exports (L1-48). External type-anchor users that
  pin the re-export list: `SystemOrder::before::<engine_renderer::RenderSyncSystem>()`
  at `client/vordar-client/src/lib.rs:235` and `src/net/mod.rs:95`;
  `MeshRenderSyncSystem` anchors at `client/…/lib.rs:240-244`,
  `net/mod.rs:101-105`. Workspace grep: zero external references to
  `SaveTransformSystem`, `CycleCameraSystem`, `RenderSlotDespawnSystem`,
  `RenderSlotAttachSystem`.
- **Ideal:** `smirk/engine-renderer/src/instance_sync.rs` (declared
  `mod instance_sync;`): header "Keeps the SDF InstancePool in sync with the
  world — fixed-step position snapshot, dirty-slot writes, slot attach/free."
  Contents moved verbatim: `SaveTransformSystem` (`pub(crate)`),
  `RenderSyncSystem` (stays `pub` — external SystemOrder anchor),
  `shape_to_gpu` (private), `RenderSlotAttachSystem` + `RenderSlotDespawnSystem`
  (`pub(crate)`), with the despawn system's dead doc line rewritten to
  "Registered by RenderPlugin in Phase::DespawnFlush, First — must run before
  DespawnFlushSystem." `CycleCameraSystem` + `new` (`pub(crate)`) move to the
  end of `camera.rs` (the crate's pattern: menu.rs owns MenuSystem,
  dev_overlay.rs owns DevOverlaySystem), keeping its doc verbatim. lib.rs ends
  at its final shape, top to bottom: module decls (existing ones plus `mod facade;
  mod frame; mod instance_sync; mod menu_actions; mod state;`), the re-export
  block (existing lines plus `pub use facade::{…};` from step 4,
  `pub use instance_sync::RenderSyncSystem;`,
  `pub(crate) use state::{init, on_resize, RendererState, MAX_MESH_INSTANCES};`),
  and `RenderPlugin` + its `Plugin` impl with the `build` body byte-identical
  to HEAD apart from `use`-path spelling (registration order, phases,
  `SystemOrder` arguments untouched) — roughly 80 lines, zero `impl System`
  blocks.
- **Gap:** four pool-sync systems and a camera input toggle still share the
  crate root with the plugin; one doc comment references a function that never
  existed.
- **Suggestion:** instance_sync.rs imports: `crate::facade::{alloc_render_slot,
  alloc_shape_group_slots}` (or the root re-exports), `crate::instance::{InstancePool,
  InstanceSlot, SdfInstance, ShapeGroupSlots}`,
  `engine_app::scheduler::{InterpolationAlpha, System}`,
  `engine_core::components::{PreviousTransform, RenderShape, RenderShapeType,
  ShapeGroup, Transform}`, `engine_core::traits::{DespawnQueue, Resources}`,
  `engine_core::World`, `glam::Mat4`, `hecs`. camera.rs additions for
  CycleCameraSystem: `crate::state::RendererState` (CameraUniform is local),
  `engine_app::input::KeyboardState`, `engine_app::scheduler::System`,
  `engine_core::traits::Resources`, `engine_core::World`,
  `winit::keyboard::KeyCode`. After the moves, sweep lib.rs's `use` block down
  to what the plugin + re-exports need and update dev_overlay.rs:5 if step 5
  did not already.
- **Path:** (1) Baseline: record `cargo nextest run -p engine-renderer` counts
  AND run `cargo nextest run --workspace` once, recording its counts.
  (2) Create instance_sync.rs; append CycleCameraSystem to camera.rs; rewrite
  the one dead doc line; write lib.rs's final decl/re-export/plugin shape.
  (3) Verify: `RenderPlugin::build` diff against HEAD = `use` spelling only;
  `cargo nextest run -p engine-renderer` green at identical counts;
  full `cargo nextest run --workspace` green at the recorded counts —
  behavioral gates: client e2e/render tests register systems ordered
  `before::<RenderSyncSystem>` / `after::<MeshRenderSyncSystem>` through the
  re-exports (a TypeId break fails scheduling immediately), and
  `tests/offscreen.rs` + `ground_render.rs` exercise the moved sync path.
  `cargo check --workspace --all-targets` zero new warnings; `git status`
  confirms zero source changes outside `smirk/engine-renderer/`.
  (4) Structure checks: `smirk/engine-renderer/src/mesh.rs` does not exist;
  `grep -c "impl System" smirk/engine-renderer/src/lib.rs` returns 0;
  `wc -l smirk/engine-renderer/src/lib.rs` ≤ ~120 and no new file except
  frame.rs and mesh/gltf_import.rs exceeds ~600 lines (frame.rs lands near 550
  — the frame graph moves whole by design; gltf_import.rs carries its 9 tests).
  If a file exceeds the estimate, record the number in the final report — do
  NOT split a system or the parse loop to chase a line count.

### 7. Close this rework's queue entry (docs-only)

- **Evidence:** the cross-type queue note listing "… ~~rework 1~~ → ~~rework 2~~
  → rework 3 → finding 2 → …" exists in two places that must stay mirrored
  verbatim: `docs/reviews/hygiene/reworks-hygiene-2026-07-14.md` (L17-28) and
  `docs/reviews/hygiene/audit-hygiene-2026-07-14.md` (L19-30). Audit finding 7's
  Evidence (audit file L166-181) cites `smirk/engine-renderer/src/lib.rs:747-752`
  for the TODO this rework deleted in step 5, and `mesh.rs:776` / `L796-803` /
  `L869-874` for the TEMP anim log that now lives in
  `smirk/engine-renderer/src/mesh/sync.rs`. No pending plan in `docs/reviews/`
  cites engine-renderer line numbers (verified by grep — only the hygiene
  audit/reworks pair mentions these files).
- **Ideal:** the hygiene queue shows rework 3 done in both mirrored notes, and
  the finding-7 worker is not sent chasing a TODO that no longer exists.
- **Gap:** the queue notes still list rework 3 as pending; audit finding 7's
  Evidence points at a deleted comment and a moved file.
- **Suggestion:** strike `rework 3` (`~~rework 3~~`) in the cross-type queue
  note in BOTH files. In `docs/reviews/hygiene/audit-hygiene-2026-07-14.md`
  finding 7's Evidence, update the two renderer citations: replace
  "`smirk/engine-renderer/src/lib.rs:747-752` a multi-line TODO describing a
  future refactor (subsumed by rework 3)" with "(the lib.rs TODO was deleted by
  rework 3)" and retarget the mesh.rs citation to
  "`smirk/engine-renderer/src/mesh/sync.rs` — the `log_accum` field and its two
  log blocks in `MeshRenderSyncSystem` ('Remove once the character animates on
  screen')" without line numbers.
- **Path:** (1) Make the strike edits in both files and the two finding-7
  citation updates; (2) verification: the queue blockquotes in the two hygiene
  files remain byte-identical to each other (diff them). No code, no test —
  docs-only.
