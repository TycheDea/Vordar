# Plan: Asset streaming — first-sight loads and zone-change IBL bakes stall the frame — 2026-07-16

Source: docs/reviews/rendering/reworks-rendering-2026-07-16.md finding 2.

## Ideal end state

No frame ever pays for asset decode. First sight of a glTF asset marks it
pending, decodes it (disk IO, parse, PNG/JPG decode, tangent generation) on a
background thread, and integrates the finished `MeshData` onto the GPU under a
per-frame budget — the entity simply renders nothing until then. A zone
crossing decodes its HDRI and its ground texture set on background threads the
same way, and its IBL bake reuses pipelines compiled once at renderer init and
lands in a single queue submit, so the swap-on-arrival costs milliseconds, not
tens of them. The measured first-sight and zone-crossing hitches are recorded
before and after in `docs/benchmarks/BASELINE.md`.

## Design decisions

**CPU work moves to detached threads; GPU work stays on the main thread,
budgeted.** The stall has two natures: CPU decode (`load_gltf_data` — gltf
parse + embedded PNG decode + tangent gen; `load_ground_material` — 3× 2k JPG
decode; `image::open` on a 5 MB Radiance HDR + 8M f32→f16 conversions) and GPU
submission (`upload_mesh`, the 42-submit IBL bake). All decode results are
plain `Send` data (`MeshData`, an f16 pixel buffer), so each miss spawns a
detached `std::thread` that sends its result over an `mpsc` channel; the main
thread drains completions with a budget. *Rejected:* a long-lived worker pool
(shutdown protocol and lifetime plumbing for events that happen a handful of
times per session — thread spawn cost is irrelevant at this frequency);
tokio/rayon (new dependency, wrong shape for fire-and-forget jobs);
`wgpu::Device` sharing across threads for off-thread upload (wgpu allows it,
but budgeted main-thread upload is simpler and the upload is the small half
once decode is off-frame).

**Rework 7 (per-environment bake-pipeline recompilation) is absorbed as steps
2–3.** Its own Suggestion says "alongside or as a prerequisite step of rework
2", and its measurement shows the zone-crossing bake cost (~24 ms) is
dominated by `Baker::new` pipeline recompilation (~9–10 ms) plus 42 individual
encoder/submit round-trips (~14 ms) — chunking or pre-baking that work without
first deleting the waste would be machinery wrapped around waste. `Baker`
compiles once at `RendererState`/`OffscreenRenderer` init (the `bake_brdf_lut`
treatment), and the whole bake records into one command encoder with one
submit. At close-out both rework 2 and rework 7 are struck from the queue.

**Environment: async decode + swap-on-arrival, not chunked bakes, not
pre-baked disk artifacts.** After compile-once pipelines and a single submit,
the main-thread residual is one 16 MB `write_texture` plus recording 42 tiny
render passes — predicted low single-digit ms. Chunking the bake across frames
adds partial-environment state for a cost that no longer warrants it;
pre-baked IBL artifacts add a content-pipeline step that belongs with rework
4's KTX2/BC direction if the residual ever measures large (steps record the
number; nothing is built on speculation). Decode (HDR load + f16 conversion)
runs on a thread; the old environment stays visible until the new one is
ready; a request for the already-applied or already-pending path is a no-op —
both shipped zones use the same HDRI, so the start↔east crossing stops paying
for IBL entirely. Last write wins: a new request replaces a pending one.

**Missing mesh renders nothing — no placeholder asset.** `MeshRenderSyncSystem`
already skips entities whose asset resolves to `None`; streaming reuses that
path. Pop-in of props/characters for a few frames is accepted at the
pre-content stage (the finding's Ideal allows it); a placeholder policy would
touch presentation for no current need.

**`MeshStore` keeps its index-stable `Vec`; `by_path` becomes a three-state
enum** (`Loaded(usize) | Pending | Failed`). `get_or_load` becomes
`get_or_request` — pure bookkeeping, no device/queue parameters — and all GPU
upload centralizes in a new `integrate(budget)` called once per frame by
`MeshRenderSyncSystem`. `Failed` stays sticky (today's "a bad path logs once"
contract). Budget = 1 asset integration per frame; the environment swap is
polled independently in `RenderSystem` (both are rare; coupling them buys
nothing).

**The zone ground streams through the same seam as glTF assets.** `MeshStore`
gains `request_job(key, FnOnce() -> Result<MeshData, String> + Send)`; the
client passes a closure doing `load_ground_material` + `generate_ground`
(pure CPU, already in the client crate), keyed `zone-ground:{zone}`. Since
ground data is deterministic per key, a key that is already known (any state)
is a no-op — zone re-entry stops re-decoding 12 MB of JPGs. *Consequence,
deliberate:* the dev-slab fallback narrows to zones with no authored ground
(and headless runs, where no `MeshStore` exists); an authored-but-broken
ground dir now logs an error and renders nothing instead of falling back to
the slab — the slab is a dev fixture for unauthored zones, not an error
handler. `register_procedural_mesh` stays synchronous for the weapon meshes
(trivial procedural data, no IO) and for tests.

**Measurement is part of the plan, not an afterthought.** Finding 2 (per-pass
GPU timing) instruments GPU passes, not CPU hitches, so step 1 adds an
`asset_load` criterion bench for the decode costs and each subsequent step
records its own before/after wall-clock in `docs/benchmarks/BASELINE.md`,
following rework 7's precedent of temporary instrumentation removed after
recording.

## Findings (execution order)

### 1. Baseline: an `asset_load` bench pins the decode costs streaming will move off-frame

- **Evidence:** `smirk/engine-renderer/src/mesh/store.rs:184-208` —
  `MeshStore::get_or_load` runs `load_gltf_data` (disk read, glTF parse,
  embedded PNG decode, tangent generation) synchronously inside
  `MeshRenderSyncSystem`'s entity loop (`smirk/engine-renderer/src/mesh/sync.rs:186-189`).
  `client/vordar-client/src/presentation.rs:86-99` runs
  `load_ground_material` (3× 2k JPG decode: `content/textures/ground/mud_leaves/`,
  12.6 MB of JPGs) + `generate_ground` synchronously on every zone change.
  No bench measures either; `benchmarks/benches/render_cpu.rs` covers only
  posing and particle fill. `docs/benchmarks/BASELINE.md` has no asset
  section.
- **Ideal:** a criterion bench `asset_load` records the CPU cost of (a)
  `load_gltf_data` on the heaviest shipped assets and (b) the zone-ground
  decode+generate, and `BASELINE.md` carries the numbers as the "before" of
  this rework.
- **Gap:** the rework has no instrument to prove its win.
- **Suggestion:** new `benchmarks/benches/asset_load.rs` + a `[[bench]]`
  entry in `benchmarks/Cargo.toml` (`name = "asset_load"`, `harness = false`,
  matching the nine existing entries). Benches, in one group with
  `.sample_size(10)` (iterations are 50–500 ms; accept criterion's small-N
  warning):
  - `first_sight/statue_vroid`: `engine_renderer::mesh::load_gltf_data`
    on `concat!(env!("CARGO_MANIFEST_DIR"), "/../content/models/statue_vroid.glb")`
    (11 MB, embedded textures).
  - `first_sight/human`: same fn on `../content/models/human.glb` (9 MB,
    skinned + clip library).
  - `zone_ground/decode_and_generate`:
    `vordar_client::ground::load_ground_material("<manifest>/../content/textures/ground/mud_leaves")`
    then `vordar_client::ground::generate_ground(size, tile, material)` with
    the same `size`/`tile` `ZoneDressingSystem` passes (`tile: 7.0` from
    `content/zones/zones.ron:21`; `size` is the `GroundDef` serde default —
    read it from `game/vordar-game/src/zones.rs` and use that value).
  Both crates are already dependencies of `vordar-benches`
  (`benchmarks/Cargo.toml:23-24`); `load_gltf_data` and the `ground` module
  are already `pub`. Panic with a clear message if a content path is missing
  (content ships in this repo).
- **Path:** add the bench → `cargo bench -p vordar-benches --bench asset_load`
  → append a "### Asset streaming — `asset_load` (rendering rework 2)"
  section to `docs/benchmarks/BASELINE.md` with a Before column, and note the
  environment-load baseline measured by rework 7 (~24 ms per
  `set_uniform_environment`, 241 ms/10 loads — cite
  `reworks-rendering-2026-07-16.md` finding 7; steps 2–3 re-measure it).
  Verify: the bench runs green; no production code changes in this step.

### 2. IBL bake pipelines compile once at init (absorbs rework 7)

- **Evidence:** `smirk/engine-renderer/src/ibl.rs:131` — `Baker::new(device)`
  runs inside `Environment::from_equirect_pixels` on every environment load,
  recompiling the `ibl.wgsl` shader module and all four bake pipelines
  (`ibl.rs:303-419`), including a `brdf_pipeline` that is never invoked there
  since the LUT hoist (audit finding 7). `bake_brdf_lut` (`ibl.rs:226-242`)
  constructs a second throwaway `Baker` at init. Rework 7 measured
  `Baker::new` at ~9–10 ms of the ~24 ms per-environment cost. Constructors
  that would own it: `create_hdr_and_ibl_resources`
  (`smirk/engine-renderer/src/state.rs:268-287`) and `OffscreenRenderer::new`
  (`smirk/engine-renderer/src/offscreen.rs:186-189`). The only external
  caller of `from_hdr` is `facade::set_environment`
  (`smirk/engine-renderer/src/facade.rs:120-138`).
- **Ideal:** the shader module and all four pipelines compile exactly once
  per device — at `RendererState::init` / `OffscreenRenderer::new` — and
  every environment load takes `&Baker` and reuses them (the exact treatment
  `bake_brdf_lut`'s LUT already got).
- **Gap:** ~9–10 ms of redundant pipeline compilation per zone crossing, plus
  a wasted brdf pipeline compile.
- **Suggestion:** make `Baker` `pub(crate)` (definition stays in `ibl.rs`,
  fields private). Change signatures: `bake_brdf_lut(device, queue, baker: &Baker)`,
  `Environment::from_hdr(..., baker: &Baker, ...)`,
  `Environment::from_equirect_pixels(..., baker: &Baker, ...)`,
  `Environment::default_gray(..., baker: &Baker, ...)` — delete the
  `Baker::new` call inside `from_equirect_pixels`. Store `baker: ibl::Baker`
  on `RendererState` (created in `create_hdr_and_ibl_resources` before the
  LUT bake) and on `OffscreenRenderer` (before `bake_brdf_lut` at
  `offscreen.rs:188`). `facade::set_environment` passes `&state.baker`
  (same borrow as `&state.device` — one `&RendererState` covers both).
  Add a `#[cfg(feature = "offscreen")]` thread-local `BAKER_COUNT`
  incremented in `Baker::new`, exposed as
  `OffscreenRenderer::baker_construction_count()` — mirror the existing
  `BAKE_COUNT`/`brdf_bake_count` pattern (`ibl.rs:244-259`,
  `offscreen.rs:248-253`).
- **Path:** write the fail-first test in
  `smirk/engine-renderer/tests/offscreen.rs` (next to the brdf-count test at
  ~line 750): construct `OffscreenRenderer`, snapshot
  `baker_construction_count()`, call `set_uniform_environment` three times,
  assert the count is unchanged — confirm it fails today (grows by 1 per
  call) → land the hoist → full offscreen suite green (white furnace,
  prefiltered-reflection, sky/fog tests all bake environments) → measure:
  temporary `Instant` timing around a 10× `set_uniform_environment` loop in a
  scratch offscreen test, record before/after in the BASELINE.md section from
  step 1 (expect ~24 ms → ~14 ms per load), then remove the instrumentation.
  If the drop is much smaller than ~9 ms, record the actual number and
  proceed — the hoist is correct regardless; note the surprise for the
  close-out step.

### 3. The environment bake lands in one queue submit instead of 42

- **Evidence:** `smirk/engine-renderer/src/ibl.rs:491-524` — `Baker::run`
  creates a fresh `CommandEncoder` and calls `queue.submit` per bake pass;
  `from_equirect_pixels` (`ibl.rs:134-165`) drives it 42 times per
  environment (6 equirect + 6 irradiance + 30 prefilter faces). Rework 7
  measured the three bake stages together at ~14 ms wall clock — dominated by
  per-submit overhead, not GPU work.
- **Ideal:** one encoder records all 42 render passes; one `queue.submit` per
  environment load.
- **Gap:** ~40 redundant encoder/submit round-trips per zone crossing.
- **Suggestion:** split `Baker::run` into a `record` that takes
  `encoder: &mut wgpu::CommandEncoder` (pass creation, pipeline/bind-group
  set, draw) — `bake_face` gains the same `encoder` parameter and loses
  `queue`. `from_equirect_pixels` creates one encoder before stage 1, records
  all faces of all three stages into it, submits once at the end.
  Within-encoder ordering is safe: wgpu inserts usage transitions between
  render passes, so the cubemap written by the equirect passes is legal to
  sample in the irradiance/prefilter passes of the same encoder. `bake_2d`
  (LUT, runs once at init) keeps its own internal encoder+submit. Per-face
  bind groups / params buffers stay as they are.
- **Path:** land the encoder threading → full offscreen suite green (the
  furnace/reflection/sky tests prove bake output is unchanged) → if wgpu
  validation rejects the same-encoder write-then-sample (not expected), fall
  back to exactly two submits — one after the equirect faces, one for
  irradiance+prefilter — and record which variant landed → re-measure the
  10× `set_uniform_environment` loop (temporary instrumentation, as step 2),
  record the number in BASELINE.md (expect low single-digit ms per load),
  remove instrumentation.

### 4. Zone-change HDRI decodes on a worker thread; the swap happens on arrival

- **Evidence:** `smirk/engine-renderer/src/facade.rs:120-138` —
  `set_environment` runs `Environment::from_hdr` synchronously:
  `image::open` on `content/textures/env/evening_road_01_puresky_2k.hdr`
  (5 MB Radiance HDR), `into_rgb32f`, then `from_equirect_pixels` converts
  8.4M floats to f16 on the main thread (`ibl.rs:79-104`) before baking.
  Called from `ZoneDressingSystem` (`client/vordar-client/src/presentation.rs:64-70`)
  on every zone change — both zones name the same HDRI (the default,
  `zones.ron` authors none), so even the same-file crossing pays full price.
  `RenderSystem::run` (`smirk/engine-renderer/src/frame.rs:142-178`) is the
  natural per-frame poll point.
- **Ideal:** `set_environment` returns immediately; decode + f16 conversion
  happen on a detached thread; the bake + swap run on the main thread the
  frame the pixels arrive; requesting the already-applied or already-pending
  path is a no-op; the previous environment stays visible until then.
- **Gap:** HDR decode + conversion (tens of ms) plus the bake all land in the
  frame that crosses a zone.
- **Suggestion:** in `ibl.rs`:
  - `pub(crate) struct EquirectImage { pub width: u32, pub height: u32, pub pixels: Vec<f16> }`
    with `fn decode_hdr(path: &str) -> Result<Self, String>` (moves
    `from_hdr`'s `image::open` + `into_rgb32f` + f16 conversion — the
    conversion now happens once, off-thread) and
    `fn from_rgba_f32(width, height, &[f32]) -> Self` (for `default_gray`
    and the offscreen `set_uniform_environment`, which stay synchronous for
    init/test determinism). `from_equirect_pixels` takes `&EquirectImage`
    and writes `pixels` directly; `from_hdr` dissolves into
    `decode_hdr` + `from_equirect_pixels`.
  - `pub(crate) struct PendingEnvironment { path: String, rx: mpsc::Receiver<Result<EquirectImage, String>> }`
    with `fn spawn(path: &str) -> Self` (detached `std::thread`,
    `let _ = tx.send(EquirectImage::decode_hdr(&path));`) and
    `fn try_take(&self) -> Option<Result<EquirectImage, String>>`
    (`TryRecvError::Empty` → `None`; `Disconnected` → `Some(Err(...))` so a
    panicked decode thread clears the pending slot instead of wedging it).
  In `state.rs`: `RendererState` gains `pending_env: Option<PendingEnvironment>`
  and `current_env_path: Option<String>`, plus
  `fn poll_pending_environment(&mut self)`: on `Some(Ok(img))` → bake via
  `from_equirect_pixels` (with `&self.baker`), swap `self.environment`, set
  `current_env_path`, log the bake wall time (the seam the old
  `facade.rs:127` log covered); on `Some(Err(e))` → `log::error!`, keep the
  old environment; either way clear `pending_env`. In `facade.rs`:
  `set_environment` returns `false` headless (unchanged contract); returns
  `true` without spawning when `path` equals `current_env_path` or the
  pending path; otherwise replaces `pending_env` (last write wins — the stale
  thread's send lands in a dropped receiver) and returns `true`. In
  `frame.rs`: call `poll_pending_environment` at the top of
  `RenderSystem::run` via `resources.get_mut::<RendererState>()`.
  `default_gray` at init leaves `current_env_path = None` so the first zone
  always requests. Update `set_environment`'s doc comment: the swap happens
  when the background decode completes; failure keeps the previous
  environment.
- **Path:** before rewiring, add a content-gated offscreen test-scratch
  timing (`Environment::from_hdr` on the real HDR, skip-if-absent) and record
  the full synchronous decode+bake cost in BASELINE.md → land the design →
  behavioral test in `ibl.rs` (no GPU needed): encode a 4×2 uniform-color HDR
  with `image::codecs::hdr::HdrEncoder` (the `image` crate's `hdr` feature is
  already enabled, `smirk/engine-renderer/Cargo.toml:33`) into a temp file;
  `PendingEnvironment::spawn(path)`; poll `try_take` in a bounded loop
  (10 ms sleeps, ≤5 s, panic on timeout); assert `Ok` with the right
  dimensions and pixel values (f16 of the authored color). If `HdrEncoder`'s
  0.25 API fights (encode-side missing), fall back to decoding the real
  content HDR skip-if-absent style (assert 2048×1024, `pixels.len() == w*h*4`,
  all finite) and say so in the close-out notes → full offscreen suite green
  (`set_uniform_environment` now goes through `from_rgba_f32`, still
  synchronous) → client tests green (`set_environment` headless still returns
  `false`) → record the new split in BASELINE.md: decode cost (worker) vs
  bake-on-arrival cost (main thread, from the new log line exercised via the
  timing scratch) → remove temporary instrumentation. If the bake-on-arrival
  residual exceeds ~8 ms, still land — record the number and flag pre-baked
  IBL artifacts (rework 4 dovetail) as the identified follow-up; do not build
  chunking.

### 5. `MeshStore` streams glTF assets: request on miss, budgeted integrate per frame

- **Evidence:** `smirk/engine-renderer/src/mesh/store.rs:184-208` —
  `get_or_load` performs the full `load_gltf_data` + `upload_mesh`
  synchronously on first miss, inside `MeshRenderSyncSystem`'s entity loop
  (`smirk/engine-renderer/src/mesh/sync.rs:181-189`, which already `continue`s
  on `None`). `by_path: HashMap<String, Option<usize>>` caches
  loaded/failed only (`store.rs:152-156`). The module header
  (`smirk/engine-renderer/src/mesh/mod.rs:5-9`) documents the synchronous
  contract. `MeshStore::default()` is inserted at renderer init
  (`smirk/engine-renderer/src/state.rs:484`) and `mem::take`n each frame by
  both `sync.rs:160` and `frame.rs:177` (the channel travels with the taken
  value; the temporary `Default` is untouched).
- **Ideal:** a miss marks the path `Pending` and spawns a detached decode
  thread; the entity renders nothing meanwhile; completed `MeshData` is
  uploaded by an explicit per-frame `integrate` call bounded by a budget;
  failures are cached sticky and logged once; the dev overlay shows the
  pending count.
- **Gap:** every first-sighted asset (props, characters, weapons on other
  players) freezes the frame for its full decode (statue_vroid: 11 MB glb).
- **Suggestion:** in `store.rs`:
  - `enum MeshEntry { Loaded(usize), Pending, Failed }`;
    `by_path: HashMap<String, MeshEntry>`.
  - `MeshStore` gains `results_tx: mpsc::Sender<(String, Result<MeshData, String>)>`
    and `results_rx: mpsc::Receiver<...>`; hand-write `Default` to create the
    channel (drop `#[derive(Default)]`).
  - `get_or_load` → `pub(crate) fn get_or_request(&mut self, path: &str) -> Option<usize>`
    — no device/queue/layout/mipgen parameters. `Loaded(i)` → `Some(i)`;
    `Pending`/`Failed` → `None`; vacant → insert `Pending`, clone `results_tx`,
    spawn a detached thread sending `(path, load_gltf_data(&path))`
    (`let _ =` on the send — app shutdown may have dropped the receiver),
    return `None`.
  - `pub(crate) fn integrate(&mut self, device, queue, layout, mipgen, budget: usize) -> usize`:
    up to `budget` `try_recv`s; `Ok(data)` with entry still `Pending` →
    `upload_mesh`, push, `Loaded(idx)`; `Err(e)` with entry `Pending` →
    `log::error!("mesh load failed: {e}")`, `Failed`; a result whose entry is
    no longer `Pending` (a `register` raced it) is dropped. Returns results
    drained. `pub(crate) const MESH_UPLOADS_PER_FRAME: usize = 1;`
  - `register` keeps its synchronous replace-in-place contract
    (`store.rs:164-182`): `Loaded(idx)` → replace `meshes[idx]`;
    `Pending`/`Failed`/vacant → push and mark `Loaded`.
  - `pub(crate) fn pending_count(&self) -> usize` (count `Pending` values —
    the map holds tens of entries).
  In `sync.rs` (`MeshRenderSyncSystem::run`): right after taking `store` and
  binding `state`, call
  `store.integrate(&state.device, &state.queue, &state.material_bgl, &state.mipgen, MESH_UPLOADS_PER_FRAME)`;
  the loop body becomes `let Some(idx) = store.get_or_request(&mesh.asset) else { continue };`.
  Next to the existing `"skinned"` meter (`sync.rs:275-277`), add
  `stats.set("streaming", format!("{} pending", store.pending_count()));`.
  Update the `mod.rs:5-9` header ("GPU upload" stage now reads: get_or_request
  enqueues a background decode; integrate uploads completed loads under a
  per-frame budget). The `needs_player`/`AnimationPlayer` attach flow is
  untouched — delayed attach just moves from a 1-frame to an N-frame gap,
  which every consumer (locomotion, sockets, weapons) already tolerates via
  the missing-entry fallbacks.
- **Path:** tests in `store.rs` under `#[cfg(all(test, feature = "offscreen"))]`
  using `HeadlessGpu` (skip cleanly without an adapter, matching
  `register_same_key_replaces_in_place`):
  1. `first_sight_streams_in_background`: `test_glb::write_test_glb` to a
     temp path; `get_or_request` returns `None`; pump
     `{ integrate(..., 1); sleep 10ms }` bounded ≤5 s until it returns
     `Some(idx)`; assert `meshes.len() == 1` and a second `get_or_request`
     is `Some(idx)` immediately.
  2. `failed_load_is_cached_not_retried`: `get_or_request("does/not/exist.glb")`
     → `None`; pump until `integrate` has drained 1 result; assert
     `get_or_request` still `None` and the entry is `Failed` (add a
     test-gated accessor for the entry state — state inspection, no logic
     re-implementation).
  3. `budget_bounds_integrations_per_call`: request two distinct temp glbs
     (`write_test_glb`, `write_skinned_glb`); pump with budget 1 asserting
     every `integrate` return is ≤ 1 until `meshes.len() == 2`.
  4. `statue_streams_and_uploads_within_budget` (content-gated,
     skip-if-absent like the human.glb tests): request
     `<manifest>/../../content/models/statue_vroid.glb`; pump to `Loaded`;
     time the single `integrate` call that performs the upload and print it.
  Run the full engine + client suites green → record in BASELINE.md: the
  statue's main-thread residual upload cost from test 4. If it exceeds
  ~10 ms, still land — note in BASELINE that rework 4's BC-compressed
  textures (4–6× smaller uploads) are the identified reducer; do not build
  per-texture upload chunking.

### 6. The zone ground streams through the same seam

- **Evidence:** `client/vordar-client/src/presentation.rs:85-103` —
  `ZoneDressingSystem` (Phase::Update) runs `load_ground_material` (3× 2k JPG
  decode from `content/textures/ground/mud_leaves/`, 12.6 MB —
  `client/vordar-client/src/ground.rs:119-150`) + `generate_ground` +
  `register_procedural_mesh` synchronously on every zone change, keyed
  `zone-ground:{zone}`; failure or headless falls back to the dev slab
  (`presentation.rs:104-118`). `facade::register_procedural_mesh`
  (`smirk/engine-renderer/src/facade.rs:165-178`) uploads immediately and is
  also used by `WeaponAttachSystem` for trivial procedural meshes
  (`client/vordar-client/src/weapons.rs:149-154`).
- **Ideal:** the ground's decode+generate runs on a background thread through
  the same `MeshStore` machinery as glTF assets; the ground entity spawns
  immediately and renders when the mesh integrates; a key already known (any
  state) is a no-op since ground data is deterministic per key — zone
  re-entry costs nothing.
- **Gap:** the largest single CPU chunk of the zone-crossing hitch
  (multi-2k-JPG decode) still lands in one frame after steps 2–4.
- **Suggestion:** in `store.rs` add
  `pub(crate) fn request_job(&mut self, key: &str, job: impl FnOnce() -> Result<MeshData, String> + Send + 'static)`:
  entry exists in any state → no-op; vacant → insert `Pending`, spawn a
  detached thread sending `(key, job())` on `results_tx` (integration,
  budget, failure logging all ride step 5's `integrate`). In `facade.rs` add
  `pub fn request_procedural_mesh(key: &str, job: impl FnOnce() -> Result<MeshData, String> + Send + 'static, resources: &mut Resources) -> bool`
  — `false` when `MeshStore` is absent (headless: renderer init never ran),
  else `store.request_job(...)` via `get_mut` (no take-dance needed — no
  device access), `true`. Export it from `lib.rs:51` alongside
  `register_procedural_mesh` (which stays synchronous for weapons and
  tests). In `presentation.rs`, replace the ground block: when
  `visuals.ground` is authored, clone `texture_dir`, copy `size`/`tile`,
  build `move || Ok(crate::ground::generate_ground(size, tile, crate::ground::load_ground_material(&dir)?))`,
  call `request_procedural_mesh(&key, job, resources)`; on `true`, spawn the
  ground entity (`RenderMesh { asset: key, .. }` + `ZoneDressing` +
  `HudHidden`) and set `mesh_ground = true`. The dev slab now covers only
  unauthored ground and headless (`request` returned `false`). Behavior
  change, deliberate (see Design decisions): an authored-but-broken ground
  dir logs `mesh load failed: ...` once and renders nothing instead of
  falling back to the slab.
- **Path:** land store + facade + presentation changes → test in `store.rs`
  (offscreen-gated, `HeadlessGpu`): `request_job_runs_once_per_key` — a
  closure incrementing an `AtomicUsize` and returning a minimal one-triangle
  `MeshData` (reuse the existing `triangle_mesh_data` fixture); call
  `request_job` twice with the same key; pump `integrate` to `Loaded`; assert
  the counter is 1 and `meshes.len() == 1`, and `get_or_request(key)` returns
  the index → client suite green, in particular
  `client/vordar-client/tests/presentation_plugin.rs` (headless: request
  returns `false`, slab path preserved) → confirm the `GroundDef` field names
  (`size`, `tile`, `texture_dir`) against `game/vordar-game/src/zones.rs`
  before writing the closure.

### 7. Close-out: after-numbers recorded, reworks 2 and 7 struck (docs-only)

- **Evidence:** `docs/benchmarks/BASELINE.md` gains its asset-streaming
  section across steps 1–5; `docs/reviews/rendering/reworks-rendering-2026-07-16.md:17-40`
  and `docs/reviews/rendering/audit-rendering-2026-07-16.md:23-45` carry the
  mirrored cross-type queue note listing reworks 2–7 as open; rework 7's
  ideal (`reworks-rendering-2026-07-16.md:234-259` — all four bake pipelines
  and the shader module compile once at init, every environment load reuses
  them) is fully delivered by steps 2–3 of this plan.
- **Ideal:** BASELINE.md presents the rework's before/after story in one
  table (decode costs and where they now run; env load 10× before/after;
  bake-on-arrival and statue-upload residuals); both queue notes strike
  rework 2 and rework 7 with a pointer to this plan and its step count; no
  temporary instrumentation from steps 2–4 survives in source.
- **Gap:** without the strike-through, the queue shows work as open that is
  done; scattered per-step numbers don't tell the story.
- **Suggestion:** consolidate the BASELINE.md section (keep each measured
  number, one row per seam, Before/After columns, a short paragraph naming
  what moved off-thread vs what remains on-frame and why the remainder is
  accepted — the rework 4 dovetail for texture size, pre-baked IBL artifacts
  only if the recorded residual ever grows). Strike rework 2 and rework 7 in
  the reworks file's queue note and mirror the identical edit in the audit
  file's queue note, following the existing strike style (rework 1's entry).
  Note in the queue text that rework 7 was absorbed by this plan's steps 2–3.
- **Path:** grep the renderer crate for leftover `Instant::now` scratch
  timing from steps 2–4 (remove any) → edit BASELINE.md → strike both queue
  entries in both files → re-read the finding 2 and finding 7 sections
  against the landed state and confirm every Ideal clause is either delivered
  or explicitly recorded as an accepted residual with its number.
