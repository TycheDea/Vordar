# Plan: Compressed GPU textures for the material path + a texture-memory meter — 2026-07-16

Source: docs/reviews/rendering/reworks-rendering-2026-07-16.md finding 4.

## Ideal end state

Every material map of shipped content reaches the GPU as a block-compressed
texture with baked mips: BC7 (sRGB) for albedo/emissive, BC7 (linear) for
metallic-roughness/AO, BC5 for normals — transcoded once at content time by an
asset-pipeline script into committed DDS sidecar files, preferred by the
importer, with today's RGBA8+runtime-mipgen path surviving as the fallback for
un-baked assets. The F3 dev overlay shows resident material-texture memory,
`content_lint.rs` enforces VQ-C5 (map dimensions, sidecar presence/freshness,
total budget ≤ 1 GB), and `docs/benchmarks/BASELINE.md` records the resident
memory and load-cost before/after. Expected on the current zone set: ~300 MB
RGBA8-decoded resident drops to ~60–70 MB, and integrate-upload cost per asset
drops with it (no runtime mipgen, ¼ the bytes).

## Design decisions

**Container: DDS sidecars, not KTX2, not repacked glbs.** The engine already
has a working BC7 DDS parser+uploader (`texture.rs:41-95` `load_dds`) and
`ddsfile 0.5.2` as a dependency; the device already requires
`TEXTURE_COMPRESSION_BC` (`state.rs:255-259`). KTX2 would add a new crate, a
new parser, and a BasisU transcode stage for zero benefit on a BC-only desktop
target. Repacking glbs with compressed payloads (KHR_texture_basisu) would
mutate source assets and break every existing import test. Sidecar files next
to the source asset keep sources pristine and make the fallback trivial:
sidecar absent → exact current behavior.

**Encoder: `texconv.exe` (DirectXTex) driven by a dependency-free node script,
outputs committed to git.** `smirk/texconv.exe` is already fetched and
gitignored (`.gitignore:36`) — the sanctioned local tool (the existing
`content/textures/ground/floor_tile/*.dds` are its output). The bake script
follows the `scripts/asset-pipeline/*.mjs` conventions (plain node, no npm
deps — `fix_glb_materials.mjs` already parses GLB containers manually).
Committing the ~50–70 MB of derived DDS means no other machine ever needs the
encoder — consistent with committing the preprocessed `content/models/*.glb`,
which are themselves derived artifacts. *Rejected:* a Rust encoder crate
(`intel_tex_2` is an unmaintained native-bindings build; `texpresso` has no
BC7); bake-on-clone (requires texconv everywhere); runtime transcode (BC7
encode of a 2k map is seconds of CPU — content-time work).

**Sidecars are self-describing; the manifest exists only for lint freshness.**
The bake script always writes DX10-header DDS (`-dx10`), so the file itself
carries BC7_UNorm / BC7_UNorm_sRGB / BC5_UNorm — the runtime never consults a
manifest or guesses color space (it trusts `Dds::get_dxgi_format()`, ddsfile
`format/dxgi.rs:115,130,131`). A `manifest.json` per sidecar dir records the
source file's SHA-256 and the transcoded image indices so `content_lint.rs`
can fail on stale sidecars after a source re-export; the runtime ignores it.
Convention: glTF assets get `<asset stem>.textures/img<N>.dds` (N = glTF image
index) beside the asset; ground sets get `<map stem>.dds` beside each source
map plus a composed `mr_2k.dds`.

**One new type at the material seam: `MaterialData` slots become
`Option<TextureSource>`, an enum of `Rgba8(ImageData)` |
`Compressed(CompressedImage)`.** `CompressedImage { width, height, mip_count,
format: wgpu::TextureFormat, data }` is plain `Send` CPU data, so it flows
through rework 2's worker-thread decode → `MeshStore::integrate` upload
pipeline unchanged: DDS read+parse happens on the background thread inside
`load_gltf_data`/`load_ground_material`, only `queue.write_texture` runs on
the main thread. `load_dds` splits into pure `parse_dds` (CPU) +
`create_bc_texture` (GPU) to serve both this and the existing facade path.
*Rejected:* adding format/mips fields onto `ImageData` (every RGBA8 consumer
would carry dead fields and the invalid states they imply); a parallel set of
`*_compressed` slots (doubles the material surface).

**Normals: BC5 with unconditional shader z-reconstruction.** BC5 stores RG
only; sampling `.z` returns 0. Instead of dual shader variants or a per-
material flag, both mesh shaders reconstruct `z = sqrt(1 - x² - y²)` from the
sampled `.rg` always — for unit-length tangent-space normals (z > 0 by
construction) this is mathematically identical to reading the stored z, so
RGBA8 maps and the 1×1 neutral default render the same and one shader path
serves both texture formats. This lands *before* any BC5 texture is loaded so
no intermediate step ever renders normals wrong.

**Meter: exact bytes from wgpu's own format introspection, summed over live
`ColorTexture`s in the `MeshStore`.** `ColorTexture` gains a `bytes` field
computed at creation from `TextureFormat::block_dimensions()` +
`block_copy_size()` over the mip chain (wgpu-types 29 `format.rs:694,1211`) —
no per-format tables to maintain, automatically correct for BC. The overlay
line sums the store per frame (tens of meshes × 5 slots — trivial), published
from `MeshRenderSyncSystem` next to the existing "streaming" line. Render
targets, shadow map, IBL cubemaps, and egui atlases are deliberately excluded:
they are fixed-size infrastructure, not content-scaled memory; the line is
labeled "tex mem (assets)" accordingly. `RendererState::texture_store` holds
nothing in production (facade `load_texture` has no callers) and is likewise
excluded.

**Lint enforces VQ-C5 in three clauses** (docs/visual-quality.md:72-74):
per-map dimension caps (≤ 2k character maps, ≤ 4k tiling environment sets),
sidecar presence + SHA-256 freshness for every material image of shipped
content, and a total resident-estimate ≤ 1 GB computed with the same
preference rule as the runtime (DDS byte size when a sidecar exists, RGBA8 ×
4/3 otherwise). Measured content today: all character maps are ≤ 2048
(verified: statue 15 images max 2048², human 12 images max 2048², others 1×
1024²), mud_leaves is 2048 ≤ 4096, and the RGBA8 total estimate is ~300 MB —
every clause passes from day one.

**Offscreen tests get optional BC support.** `HeadlessGpu::new`
(`offscreen.rs:34-49`) requests `TEXTURE_COMPRESSION_BC` only when the adapter
offers it and exposes the fact; BC-dependent tests skip cleanly on fallback
adapters, mirroring the existing "no adapter → skip" pattern. Tiny BC fixtures
(8×8 DDS, a few hundred bytes each) are generated once by the bake script's
fixture mode and committed — BC cannot be encoded at test runtime, and
hand-written bitstreams are not maintainable.

**The redundant embedded decode is eliminated last.** `gltf::import` eagerly
decodes every embedded PNG even when sidecars will win; switching to
`gltf::Gltf::open` + `gltf::import_buffers` (gltf-1.4.1 `import.rs:118`) with
per-slot lazy decode completes the load-time win (statue: ~15 PNG decodes
skipped) but is the only step that changes import structure, so it lands after
the sidecar preference is proven. Side effect, accepted and documented: a
corrupt source image becomes a per-slot warn+`None` instead of failing the
whole asset — matching `fetch`'s existing unsupported-format behavior.

No product questions — all choices here are engineering-forced.

## Findings (execution order)

### 1. Texture-memory meter — `ColorTexture` knows its size, the dev overlay shows the sum

- **Evidence:** `smirk/engine-renderer/src/texture.rs:15-19` — `ColorTexture
  { texture, view, sampler }` carries no size information; nothing in the
  workspace can report resident texture memory (grep for any byte-accounting
  of textures: zero hits). The dev overlay publishes lines via
  `DevStats::set(key, impl Display)`
  (`smirk/engine-app/src/dev_stats.rs:60-66`); `MeshRenderSyncSystem` already
  publishes the "streaming" line from the `MeshStore` at
  `smirk/engine-renderer/src/mesh/sync.rs:273-275`. VQ-C5's ≤ 1 GB budget
  (docs/visual-quality.md:72-74) has no meter.
- **Ideal:** every `ColorTexture` records its GPU byte size at creation; the
  `MeshStore` sums its live material textures; the F3 overlay shows
  "tex mem (assets)" in MB; `docs/benchmarks/BASELINE.md` records the current
  (all-RGBA8) resident number for the heaviest shipped assets as this
  rework's Before.
- **Gap:** the budget is unenforceable and the rework has no instrument to
  prove its VRAM win.
- **Suggestion:** in `smirk/engine-renderer/src/texture.rs`: add
  `pub fn gpu_texture_bytes(format: wgpu::TextureFormat, width: u32, height:
  u32, mip_count: u32) -> u64` — for each mip level `l`, dims
  `(width>>l).max(1) × (height>>l).max(1)`, blocks =
  `dim.div_ceil(block_dimensions())` per axis, bytes = blocks_x × blocks_y ×
  `block_copy_size(None).unwrap()`; sum the levels. Add `pub bytes: u64` to
  `ColorTexture` and set it at every construction site (all five live in
  `texture.rs`: `load_dds`, `create_rgba_texture_mipped`,
  `create_rgba_texture`, `create_checker_texture`, `create_default_white` —
  each already knows format/dims/mips). In
  `smirk/engine-renderer/src/mesh/store.rs`: add
  `pub(crate) fn texture_memory_bytes(&self) -> u64` summing
  `p._textures.iter().map(|t| t.bytes)` over `self.meshes` /
  `primitives`. In `sync.rs` next to the existing `stats.set("streaming",
  ...)` line: `stats.set("tex mem (assets)", format!("{} MB",
  store.texture_memory_bytes() / (1024 * 1024)))`.
- **Path:**
  1. Unit test (no GPU) in `texture.rs` for `gpu_texture_bytes`: RGBA8
     8×8 with 4 mips = (64+16+4+1)×4 = 340 bytes; Bc7RgbaUnormSrgb 8×8 with
     4 mips = (4+1+1+1) blocks × 16 = 112 bytes; Bc5RgUnorm same block math.
     Write it first — it fails to compile until the helper exists.
  2. Implement helper + `bytes` field + `texture_memory_bytes()` + the
     overlay line.
  3. Offscreen test (`#[cfg(all(test, feature = "offscreen"))]` in
     `store.rs`, follow `register_same_key_replaces_in_place`'s
     `HeadlessGpu` skip pattern): register `triangle_mesh_data()` whose
     material has an 8×8 base-color `ImageData` (extend the fixture builder
     locally); assert `texture_memory_bytes()` = 340 (mipped 8×8 albedo) +
     4×4 (the four 1×1 neutral defaults) = 356. Derivation as a test comment.
  4. Content-gated measurement test in `store.rs`'s test module (skip when
     content absent, like `statue_streams_and_uploads_within_budget` at
     `store.rs:565` — note `MeshStore`'s streaming API is `pub(crate)`, so
     this must live inside the engine crate): stream
     `content/models/statue_vroid.glb` and `content/models/human.glb`
     through `get_or_request`/`integrate`, then `println!`
     `texture_memory_bytes()`.
  5. Append a "### Texture memory — rework 4" section to
     `docs/benchmarks/BASELINE.md` with the measured Before for the
     statue+human pair (expect very roughly 87 + 66 ≈ 150 MB; record the
     actual and state it covers those two assets — the ground set's ~67 MB
     is tracked by the zone_ground bench instead). Workspace green:
     `cargo nextest run --workspace`.

### 2. VQ-C5 content-lint — map-dimension caps and the total-budget estimate

- **Evidence:** `game/vordar-game/tests/content_lint.rs` covers VQ-B1–B4 and
  VQ-E1 (`race_models_within_budgets` checks joints and *disk* bytes at
  :86-104) but nothing checks texture dimensions or GPU footprint.
  docs/visual-quality.md:72-74 (VQ-C5): "≤ 2k per character map, ≤ 4k per
  tiling environment set; total texture memory ≤ 1 GB" — "*Test:*
  asset-pipeline verify step + content-lint size checks" is aspirational
  today. Measured now: all character maps ≤ 2048², mud_leaves maps are
  2048², total RGBA8 estimate ≈ 300 MB — the lint passes on current content.
- **Ideal:** `content_lint.rs` fails when a character material map exceeds
  2048 on either axis, when a zone ground map exceeds 4096, or when the
  summed GPU-byte estimate of all shipped material textures exceeds 1 GB.
- **Gap:** VQ-C5 is stated but unenforced; a future 8k asset drop would ship
  silently.
- **Suggestion:** add to `game/vordar-game/tests/content_lint.rs` (reuse its
  `race_models()` and `repo_root()` helpers, and `zone_visual_refs_load`'s
  zones.ron walk):
  - `character_maps_within_dimension_cap`: for each race model's
    `MeshData`, walk every primitive's `MaterialData` image slots (today all
    `Option<ImageData>`); assert `width <= 2048 && height <= 2048` with the
    race id and slot name in the message.
  - `ground_sets_within_dimension_cap`: for each zone with a ground def,
    for each of the `diff`/`nor_gl`/`rough` maps, decode via
    `engine_renderer::mesh::load_image_rgba` (already exported;
    full decode of 3 jpgs in a test is acceptable) and assert dims ≤ 4096.
  - `total_texture_memory_within_budget`: sum over (a) every race model's
    image slots, (b) every zone prop's image slots (parse via
    `load_gltf_data`, path walk as in `zone_visual_refs_load`), (c) every
    zone ground set's three maps: estimate `w × h × 4 × 4 / 3` bytes per
    image (RGBA8 + runtime mip chain — what the runtime allocates today).
    Assert `total <= 1_073_741_824` and print the total in MB. Note for step
    9: this estimator later learns the compressed variant.
- **Path:** write the three tests → run
  `cargo nextest run -p vordar-game --test content_lint` — all must pass on
  current content (the measured numbers above say they will). If
  `total_texture_memory_within_budget` unexpectedly exceeds 1 GB, do not
  raise the cap: leave the two dimension tests in, move the total assert to
  this plan's step 9 (where compressed sizes count), and report the measured
  number in the final message. Workspace green.

### 3. Bake script — texconv transcodes shipped material maps to committed DDS sidecars

- **Evidence:** no transcode step exists: `scripts/asset-pipeline/` holds
  `fetch_polyhaven.mjs`, `fix_glb_materials.mjs` (parses GLB containers
  manually, dependency-free — the precedent), `glb_to_fbx.py`,
  `mixamo_to_glb.py`, `vrm_to_glb.mjs`. `smirk/texconv.exe` exists
  (`.gitignore:36` ignores it) and produced the only DDS in the repo
  (`content/textures/ground/floor_tile/*.dds`, currently unreferenced by
  code). Shipped material sources: 5 character glbs with embedded PNGs
  (statue 15 images / human 12 / elf, dwarf, valkyrie 1 each — all dims
  multiples of 4, max 2048), 4 prop gltfs with external `textures/*.jpg`,
  and `content/textures/ground/mud_leaves/{diff,nor_gl,rough}_2k.jpg`.
- **Ideal:** one script bakes every material map of shipped content into
  self-describing DX10 DDS sidecars with full mip chains, writes freshness
  manifests, and can regenerate the engine's tiny BC test fixtures; its
  outputs are committed.
- **Gap:** nothing produces compressed textures; every map decodes to RGBA8
  at load.
- **Suggestion:** new `scripts/asset-pipeline/bake_textures.mjs`, plain node
  (no npm deps — `zlib`/`crypto`/`child_process` are built-in), header
  comment documenting usage and the sidecar convention. Locate texconv via
  `process.env.TEXCONV ?? "smirk/texconv.exe"`; if missing, exit with a
  message pointing at https://github.com/microsoft/DirectXTex/releases.
  Modes:
  - **glTF mode** (`node bake_textures.mjs gltf <asset.glb|asset.gltf> ...`):
    parse the JSON (GLB: chunk layout as in `fix_glb_materials.mjs:14-18`;
    .gltf: plain JSON). Classify each *material-referenced* image index by
    slot: `pbrMetallicRoughness.baseColorTexture` → base,
    `.metallicRoughnessTexture` → mr, `normalTexture` → normal,
    `emissiveTexture` → emissive, `occlusionTexture` → ao. Skip images
    referenced by two conflicting color-space classes (warn; none exist in
    shipped content) and images with any dimension not a multiple of 4
    (warn; none exist). Extract bytes (GLB bufferView slice, or URI file
    read relative to the .gltf) to a temp file with the right extension,
    then run texconv into `<asset dir>/<asset stem>.textures/`:
    - base/emissive: `-f BC7_UNORM_SRGB -srgb -m 0 -dx10 -y`
    - mr/ao: `-f BC7_UNORM -m 0 -dx10 -y`
    - normal: `-f BC5_UNORM -m 0 -dx10 -y`
    Rename outputs to `img<N>.dds`. Write `manifest.json`:
    `{ "source": "<basename>", "sha256": "<hex of source asset file>",
    "images": [{ "index": N, "slot": "base", "file": "img<N>.dds" }, ...] }`.
  - **ground mode** (`node bake_textures.mjs ground <set dir> ...`): find the
    `diff`/`nor_gl`/`rough` maps by filename tag (non-.dds); bake
    `diff*.jpg → <stem>.dds` (BC7 sRGB flags above), `nor_gl*.jpg → <stem>.dds`
    (BC5), and compose the MR map from rough via texconv swizzle:
    `-f BC7_UNORM -m 0 -dx10 -y -swizzle 0r01 -sx _mr` (glTF MR convention:
    R unused, G = roughness, B = metallic = 0; output name will be
    `rough_2k_mr.dds` — any name containing `mr` and ending `.dds` satisfies
    step 6's lookup). Probe first: run that command once and inspect the
    output with `texconv`'s log / `texdiag info`. If the local texconv build
    rejects `0`/`1` swizzle constants (older releases lack them), park the
    composed-MR sidecar entirely — bake only diff + nor_gl, omit the `mr`
    entry from the manifest, and note it in the commit message; ground MR
    then keeps the CPU composition fallback at load (step 6 already handles
    the absent file), which stays correct, merely uncompressed. Do NOT
    approximate with a different swizzle — a wrong channel order corrupts
    roughness. Manifest as above with the three source hashes.
  - **fixture mode** (`node bake_textures.mjs fixtures`): write three 8×8
    BMPs (54-byte BITMAPINFOHEADER + raw BGR rows — hand-assembled buffer):
    solid red (255,0,0), solid mid-gray (128,128,128), and a uniform tilted
    normal (r=200, g=128, b=235); texconv them to
    `smirk/engine-renderer/tests/data/red8x8_bc7_srgb.dds` (BC7 sRGB),
    `gray8x8_bc7_linear.dds` (BC7 linear), `tilt8x8_bc5.dds` (BC5).
- **Path:** write the script → run fixture mode → run glTF mode on all five
  `content/models/*.glb` and the four `content/models/props/*/ *.gltf` →
  run ground mode on `content/textures/ground/mud_leaves` → sanity: every
  sidecar dir has `manifest.json` + one `img<N>.dds` per referenced image;
  DDS files are 1/4–1/6 the decoded size (spot-check `du`); texconv exit
  codes 0. Commit script + fixtures + sidecars + manifests. No Rust code
  changes — workspace trivially green. (Verification of DDS *content*
  happens in step 4's parse/render tests; if step 4 later finds a bad bake,
  fix the script here and re-run.)

### 4. Engine carries compressed images — `TextureSource` at the material seam, DDS parse/upload split, BC in the offscreen harness

- **Evidence:** `MaterialData`'s five image slots are `Option<ImageData>`
  (RGBA8-only, `gltf_import.rs:54-58`); `slot_texture`
  (`store.rs:40-54`) uploads RGBA8 + runtime mipgen only. `load_dds`
  (`texture.rs:41-95`) fuses file IO, parse, and GPU upload in one function,
  blindly assumes BC7, and ignores the DDS's own DXGI format.
  `HeadlessGpu::new` (`offscreen.rs:44-46`) requests default features — BC
  uploads would fail validation in tests. Construction sites that must wrap:
  `gltf_import.rs:260-274` (`read_material`), `ground.rs:143-145`; consumers
  that must match: `store.rs:103-107`, tests at `offscreen.rs:264-267,394`,
  `gltf_import.rs:333,408`.
- **Ideal:** a `CompressedImage` flows from a worker-thread DDS parse through
  `MeshData` into a direct `write_texture` upload of all baked mips; RGBA8
  keeps its exact current path; offscreen tests can exercise BC when the
  adapter supports it.
- **Gap:** the engine has no way to represent, transport, or upload a
  compressed material map.
- **Suggestion:**
  - `texture.rs`: `pub struct CompressedImage { pub width: u32, pub height:
    u32, pub mip_count: u32, pub format: wgpu::TextureFormat, pub data:
    Vec<u8> }` (mips contiguous, DDS layout). `pub fn parse_dds(bytes: &[u8])
    -> Result<CompressedImage, String>`: `ddsfile::Dds::read`, map
    `get_dxgi_format()` — `BC7_UNorm → Bc7RgbaUnorm`, `BC7_UNorm_sRGB →
    Bc7RgbaUnormSrgb`, `BC5_UNorm → Bc5RgUnorm`, legacy fourCC `ATI2 →
    Bc5RgUnorm` via `get_d3d_format()`, anything else → `Err` naming the
    format. `pub fn load_dds_image(path: &str) -> Result<CompressedImage,
    String>` = `fs::read` + `parse_dds` (the worker-thread entry; export via
    `mesh/mod.rs` alongside `load_image_rgba`). `pub(crate) fn
    create_bc_texture(device, queue, img: &CompressedImage) -> ColorTexture`:
    generalize `load_dds`'s existing mip-upload loop to use
    `img.format.block_dimensions()`/`block_copy_size()` instead of
    hard-coded 4/16; set `bytes` via step 1's helper. Reimplement `load_dds`
    as read → `parse_dds` → honor its `srgb` hint *only when* the DDS has no
    DXGI format (legacy header; preserves `facade::load_texture` behavior
    for the old floor_tile files) → `create_bc_texture`.
  - `gltf_import.rs`: `pub enum TextureSource { Rgba8(ImageData),
    Compressed(crate::texture::CompressedImage) }`; the five `MaterialData`
    slots become `Option<TextureSource>`; `read_material` wraps its fetches
    in `TextureSource::Rgba8` (sidecar preference is step 7). Re-export
    `TextureSource` from `mesh/mod.rs`.
  - `store.rs` `slot_texture`: match — `Rgba8` → existing
    mipped/plain path (`srgb` param applies here only), `Compressed` →
    `create_bc_texture` (format is self-describing).
  - `ground.rs:130-149`: wrap the three loaded maps in
    `TextureSource::Rgba8` (DDS preference is step 6).
  - `offscreen.rs` `HeadlessGpu::new`: request `adapter.features() &
    wgpu::Features::TEXTURE_COMPRESSION_BC` in the device descriptor; update
    the module-header comment (line 9-10) — BC-dependent tests now skip on
    `!device.features().contains(...)` instead of being impossible.
  - Fix the four listed test sites (`is_some()` stays; `offscreen.rs:394`
    wraps in `Rgba8`; `gltf_import.rs:408` matches the `Rgba8` variant).
- **Path:**
  1. Unit tests for `parse_dds` on step 3's committed fixtures
     (`include_bytes!("../tests/data/red8x8_bc7_srgb.dds")` etc.): assert
     8×8, mip_count 4, expected `TextureFormat` per fixture; garbage bytes →
     `Err`. Write fail-first (no `parse_dds` yet).
  2. Implement the types and the split; mechanical fixes across the listed
     sites until the workspace compiles.
  3. Offscreen analytic test (in `tests/offscreen.rs`, skip without adapter
     or without BC support): build a `MeshData` quad whose material
     base-color is `TextureSource::Compressed(parse_dds(red fixture))`,
     `render_mesh`, read back — center pixels read red-dominant
     (`r > 2*g && r > 2*b`), proving BC7 decode + sRGB handling through the
     real pipeline. Also assert the RGBA8 path still renders (existing
     textured-mesh tests cover it).
  4. `cargo nextest run --workspace` green — nothing constructs `Compressed`
     in production yet, so behavior is unchanged outside the new tests.

### 5. Normal sampling reconstructs z — one shader path for RGBA8 and BC5 normals

- **Evidence:** both mesh shaders read three normal channels:
  `mesh_shader.wgsl:114` and `skinned_mesh_shader.wgsl:126` — `let nm =
  textureSample(t_normal, s_mat, in.uv).xyz * 2.0 - 1.0;`. A BC5 texture has
  no B channel — sampling `.z` returns 0, which would break lighting the
  moment step 6/7 bind a BC5 normal map. Tangent-space normal maps encode
  unit vectors with z > 0, so z is redundant: `z = sqrt(1 - x² - y²)`.
- **Ideal:** both shaders derive z from the sampled RG; RGBA8 maps, BC5
  maps, and the 1×1 neutral default `[128,128,255,255]` (xy ≈ 0 → z ≈ 1) all
  shade identically through one path.
- **Gap:** the shaders hard-require a stored z, blocking BC5.
- **Suggestion:** in both files replace the two lines inside the tangent
  branch with:
  ```wgsl
  let nm_xy = textureSample(t_normal, s_mat, in.uv).xy * 2.0 - 1.0;
  let nm_z  = sqrt(max(1.0 - dot(nm_xy, nm_xy), 0.0));
  N = normalize(T * nm_xy.x + B * nm_xy.y + Nv * nm_z);
  ```
  Update the `t_normal` binding comments (`mesh_shader.wgsl:13`,
  `skinned_mesh_shader.wgsl:12`) to state the constraint: "tangent-space,
  z reconstructed from xy — BC5 (RG-only) and RGBA8 sample identically".
- **Path:** offscreen analytic test first (skip without adapter; RGBA8 only,
  so no BC feature needed): render two quads through `render_mesh` under the
  default sun — one with a flat RGBA8 normal map (all `[128,128,255,255]`),
  one whose normal map uniformly tilts (e.g. `[200,128,235,255]`) — and
  assert their mean luminance differs by a clear margin (tilt changes N·L),
  proving the TBN path still perturbs normals after the rewrite. Run the
  full offscreen suite: every existing lighting/normal test must stay green
  (z-reconstruct is exact for unit normals — if a test moves beyond
  tolerance, the fixture's normal map wasn't unit-length; investigate the
  fixture, do not loosen the tolerance). Workspace green.

### 6. Ground sets prefer DDS sidecars — mud_leaves loads BC7/BC5/composed-MR

- **Evidence:** `client/vordar-client/src/ground.rs:119-150` —
  `load_ground_material` finds maps by filename tag (`find("diff")` etc. at
  :120-128; after step 3 the tag `diff` matches *both* `diff_2k.jpg` and
  `diff_2k.dds`, making the current find ambiguous), decodes 3× 2k JPGs to
  RGBA8 (~67 MB GPU after mipgen), and composes the MR map on the CPU
  (:135-140). Step 3 committed `diff_2k.dds` (BC7 sRGB), `nor_gl_2k.dds`
  (BC5), and a composed `*mr*.dds` (BC7 linear) into
  `content/textures/ground/mud_leaves/`. The load runs on a background
  thread via the `request_procedural_mesh` job (`presentation.rs:86-101`);
  `benchmarks/benches/asset_load.rs:43-55` measures the decode.
- **Ideal:** each map slot prefers its `.dds` sidecar
  (`engine_renderer::mesh::load_dds_image`, pure CPU, worker-thread safe)
  and falls back to the JPG path when absent; the composed-MR sidecar
  replaces the CPU composition; the zone-ground bench and BASELINE record
  the drop.
- **Gap:** the largest single texture set still decodes JPGs and uploads
  4/3× uncompressed.
- **Suggestion:** rework the finder in `load_ground_material`: `find_dds(tag)`
  = directory entry containing `tag` *and* ending `.dds`; `find_src(tag)` =
  containing `tag`, not ending `.dds`. Per slot: `diff` → dds ⇒
  `TextureSource::Compressed(load_dds_image(...)?)` else Rgba8 JPG path;
  same for `nor_gl`; MR → `find_dds("mr")` ⇒ Compressed, else the existing
  rough-JPG composition wrapped in `Rgba8`. A parse failure of an existing
  sidecar is an `Err` (bake output is committed content — fail loud, the
  job's error path logs it), not a silent fallback. Note: if step 3 parked
  the composed-MR sidecar (texconv swizzle unsupported), the MR arm simply
  keeps the composition fallback — the code is identical, only the file is
  absent.
- **Path:**
  1. Unit-level test in `ground.rs`'s test module won't work (needs content)
     — instead extend `client/vordar-client/tests/ground_render.rs`: after
     the existing render assertions, assert the loaded `MaterialData` slots
     are `Compressed` when the corresponding `.dds` exists in the mud_leaves
     dir (fail-first: they are `Rgba8` before the finder change). Gate the
     *render* part additionally on BC support (step 4's harness flag); the
     slot-variant assertions need no GPU.
  2. Implement the finder + preference.
  3. Re-run `cargo bench -p vordar-benches --bench asset_load` —
     `zone_ground/decode_and_generate` should drop hard (3 JPG decodes →
     3 file reads + header parses). Update the zone_ground row in
     `docs/benchmarks/BASELINE.md` (the table at the "Asset streaming" section,
     rows ~181-183) with the new number and a "BC sidecars (rework 4)" note.
  4. Workspace green.

### 7. glTF importer prefers `img<N>.dds` sidecars for material images

- **Evidence:** `gltf_import.rs:239-246` — `read_material`'s `fetch` closure
  converts the eagerly-decoded `images[index]` to RGBA8 unconditionally.
  Step 3 committed sidecar dirs for all five character glbs and four prop
  gltfs: `<asset dir>/<asset stem>.textures/img<N>.dds` (N = glTF image
  index), each self-describing DX10 DDS. `read_material` already receives
  the asset `path`.
- **Ideal:** for each material slot, the importer first checks the sidecar
  path and parses it into `TextureSource::Compressed`; only on absence does
  it fall back to the decoded RGBA8 image. Character and prop textures stop
  costing 4–6× their necessary VRAM.
- **Gap:** the sidecars exist on disk but nothing reads them.
- **Suggestion:** in `read_material` (which gets `path: &str`), compute the
  sidecar dir once: asset path with its extension stripped, `+ ".textures"`
  (`Path::with_extension("textures")`). Extend `fetch` to: build
  `sidecar_dir/img{index}.dds`; if it exists →
  `crate::texture::parse_dds(&fs::read(...))` → `Compressed`; on parse/read
  error log a warning naming the file and fall through to the RGBA8 path
  (unlike step 6's authored ground set, per-slot fallback is correct here —
  the RGBA8 source is still in the asset). This step deliberately leaves
  `gltf::import`'s eager decode in place (removed in step 8) — the sidecar
  just wins the slot.
- **Path:**
  1. Synthetic fail-first test in `gltf_import.rs`'s test module: write
     `test_glb::write_test_glb`-style… the existing synthetic triangle glb
     has *no* texture — instead copy the real seam: write a temp glb **with
     an embedded image** (extend `test_glb.rs` with a variant embedding the
     8×8 PNG bytes of any small image — simplest is to reuse
     `write_test_glb` plus a new `write_textured_glb` that embeds a tiny
     PNG; the `image` crate can encode one in-memory) into
     `std::env::temp_dir()`, create `<stem>.textures/img0.dds` beside it
     from `include_bytes!("../../tests/data/red8x8_bc7_srgb.dds")`, load,
     and assert `base_color_image` is `Some(TextureSource::Compressed(c))`
     with `c.format == Bc7RgbaUnormSrgb`. Before the implementation this
     asserts `Rgba8` — fail-first.
  2. Implement the sidecar preference.
  3. Content-gated assertion: extend the existing
     `loads_human_character_asset_if_present` (or a sibling test) — when
     `content/models/human.textures/` exists, every primitive's base-color
     slot is `Compressed`.
  4. Workspace green (the avocado/fox tests have no sidecars and exercise
     the fallback).

### 8. Skip the eager embedded decode when sidecars win — lazy per-slot image decode

- **Evidence:** `gltf_import.rs:101-127` — `load_gltf_data` uses
  `gltf::import`, which decodes *every* embedded PNG before `read_material`
  runs; after step 7 those decodes are thrown away whenever a sidecar wins
  (statue: 15 PNG decodes, the bulk of its 122 ms first-sight baseline —
  `docs/benchmarks/BASELINE.md:181`). gltf-1.4.1 exposes
  `Gltf::open(path)` (`lib.rs:278`) and `import_buffers(&Document,
  Option<&Path>, Option<Vec<u8>>)` (`import.rs:118`) — buffers without
  images.
- **Ideal:** image bytes are decoded only for slots the sidecar does not
  cover; a fully-sidecar'd asset performs zero image decodes.
- **Gap:** worker threads burn hundreds of ms decoding pixels that are
  discarded.
- **Suggestion:** in `load_gltf_data`: replace `gltf::import(path)` with
  `let mut gltf = Gltf::open(path)?; let blob = gltf.blob.take(); let
  buffers = gltf::import_buffers(&gltf.document, path.parent(), blob)?;`.
  Thread `&gltf.document` + `&buffers` (instead of `&images`) down to
  `read_material`. `fetch` becomes: sidecar hit → `Compressed` (step 7);
  miss → locate `doc.images().nth(index).source()`:
  `Source::View { view, .. }` → slice `buffers[view.buffer().index()]` at
  `view.offset()..offset+length`; `Source::Uri { uri, .. }` → if it starts
  with `data:` log warn + `None` (no shipped content uses data URIs), else
  `fs::read` relative to the asset dir (percent-decode is unnecessary for
  shipped filenames; if `fs::read` fails, warn + `None`); then
  `image::load_from_memory(bytes)` → `.into_rgba8()` → `ImageData`. Delete
  `to_rgba8` (`gltf_import.rs:280-296`) — `image`'s converter replaces the
  manual format match (it handles R8/RG8/RGB8 sources natively). Documented
  behavior change: a corrupt embedded image was previously a whole-asset
  `Err` from `gltf::import`; it is now a per-slot warn + `None`, matching
  `fetch`'s existing unsupported-format contract.
- **Path:**
  1. Fail-first test: extend step 7's `write_textured_glb` seam with a
     variant whose embedded "PNG" bytes are garbage; with an `img0.dds`
     sidecar present the asset must load successfully (today `gltf::import`
     decodes eagerly and errors). Second assertion: same garbage glb
     *without* a sidecar loads with `base_color_image == None` (warn path),
     not `Err`.
  2. Implement; keep `loads_real_textured_asset_if_present` (avocado — no
     sidecars, external decode fallback) and `loads_skinned_fox_asset_if_present`
     green.
  3. Re-run `cargo bench -p vordar-benches --bench asset_load`:
     `first_sight/statue_vroid` and `first_sight/human` should drop
     substantially (PNG decode gone; parse + tangents remain). Update their
     BASELINE.md rows with a "sidecar decode skip (rework 4)" note.
  4. Workspace green.

### 9. Close-out — sidecar presence/freshness lint, compressed-aware budget, BASELINE after-numbers, queue strike

- **Evidence:** after steps 1–8 the sidecars are load-bearing but
  unguarded: re-exporting `human.glb` (e.g. a future Mixamo clip merge via
  `mixamo_to_glb.py`) would silently shift image indices and either bind
  wrong textures or fall back to RGBA8 with no signal. Step 2's budget
  estimator still prices everything as RGBA8. `docs/benchmarks/BASELINE.md`
  carries step 1's Before but no After. The reworks queue note
  (`docs/reviews/rendering/reworks-rendering-2026-07-16.md:19-35`) still
  lists rework 4 as open.
- **Ideal:** content-lint fails on missing or stale sidecars for every
  shipped material image; the ≤ 1 GB assert measures what the runtime
  actually residents; BASELINE records the after-numbers; the queue note
  strikes rework 4.
- **Gap:** the compressed path can silently rot; the rework's win is
  unrecorded.
- **Suggestion & Path (all in one bounded diff):**
  1. `game/vordar-game/tests/content_lint.rs` —
     `material_textures_have_fresh_sidecars`: for each race model asset and
     each zone prop model (paths via the existing helpers), require
     `<stem>.textures/manifest.json` (parse with `serde_json` — add as
     dev-dependency of vordar-game if absent, `serde` is already in the
     tree), recompute the asset file's SHA-256 (add `sha2` as
     dev-dependency) and assert it equals `manifest.sha256`; assert every
     `images[].file` exists in the sidecar dir. For each zone ground set:
     require `manifest.json`, hash-match each source map, and require the
     `diff`/`nor_gl` `.dds` files (plus the `mr` `.dds` only if the manifest
     lists it — step 3 may have parked it). Failure message must name the
     regen command: `node scripts/asset-pipeline/bake_textures.mjs ...`.
  2. Upgrade step 2's `total_texture_memory_within_budget`: since the
     importer now returns `TextureSource`, match per slot — `Compressed(c)`
     → `c.data.len()`, `Rgba8(i)` → `i.width * i.height * 4 * 4 / 3`.
     Assert ≤ 1 GB (and if step 2 had to park this assert, land it now) and
     print the new total.
  3. Re-run step 1's content-gated measurement test(s); update the
     "Texture memory — rework 4" BASELINE section with the After column
     (expect roughly 4–6× down on the measured trio; record actuals).
  4. Strike rework 4 in the reworks file's queue note (the
     `> Reworks 4–6 remain open` line), following the established strike
     format: done date, plan filename, step count.
  5. Full gate: `cargo nextest run --workspace` green; report final test
     count.
