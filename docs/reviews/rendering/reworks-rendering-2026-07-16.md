# Rendering & Graphics Audit (Reworks) — 2026-07-16

Rework-scale companion to `audit-rendering-2026-07-16.md`: findings that need
a design pass before anyone writes code. Consumed by /plan-rework.

## Ideal end state

The renderer closes the distance between "one sun + IBL over uncompressed
textures, everything drawn every frame" and the AA dark-fantasy image: accent
lights that actually light, materials that can be transparent, assets that
stream without hitches in compressed formats, and a frame whose cost scales
with what is visible rather than what exists — with the already-tracked
CSM/SSAO/TAA ladder landing on top of that foundation.

## Findings (implementation order)

Cross-type queue (mirrored verbatim from `audit-rendering-2026-07-16.md`):

> **Cross-type queue**: **~~finding 1 → finding 2 → finding 3 → finding 4 →
> finding 5 → finding 6 → finding 7 → rework 1 → finding 8 → finding 9 →
> finding 10 → rework 2 → rework 3 → rework 4 → rework 5 → rework 6~~.**
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

### 1. Punctual lights — torches, embers, and magic must light the world

- **Evidence:** the entire lighting model is one directional sun + IBL
  ambient: `LightUniform` carries a single direction/color
  (`camera.rs:174-198`), `shade_pbr` evaluates exactly one light
  (`mesh_shader.wgsl:144-178`), and no light component exists anywhere in the
  workspace (zero grep hits for point/spot lights). The art bar demands "warm
  accent light (fire/ember/magic)" (VQ-A1) and "emissive accents (portals,
  magic)" that sell dusk scenes (VQ-A5) — today an emissive portal blooms on
  screen but casts nothing on the ground, characters, or walls around it,
  which is the most immediately visible difference between this image and
  the Diablo IV / Lost Ark register the project locked.
- **Ideal:** N point lights (spot later) with physically-plausible falloff,
  driven by an ECS component (portals, projectiles, VFX bursts, future
  braziers emit light), evaluated in the shared PBR core for all three
  geometry passes, day/night aware, no per-light shadows initially.
- **Gap:** accent light exists only as bloom; nothing in the world responds
  to it.
- **Tradeoffs:** *Wins:* the single largest step toward the locked art
  direction; makes VFX/ability beats (VQ-E1) land physically. *Losses:* a
  uniform/storage light array + selection policy adds real complexity; naive
  N×M cost needs a cap; light leakage without per-light shadows is accepted
  and must be art-directed (small radii).
- **Suggestion:** /plan-rework after audit finding 1 (the loop lands once in
  the shared PBR include). The plan must decide: forward loop over a small
  capped array (≤16 lights, distance-prioritized — right size for compact
  zones) vs clustered forward (future-proof, much more machinery); where the
  light list is extracted (a RenderSync system mirroring the draw-list
  builders); and the component/content schema (radius, color, intensity,
  flicker seam for fire).
- **Path:** plan → component + extraction system → shared-shader loop +
  cap/selection → offscreen analytic test (a point light brightens the near
  face of a cube and falls off with distance) → content hookup (portal
  prefab emits) → feel-check entry.

### 2. Asset streaming — first-sight loads and zone-change IBL bakes stall the frame

- **Evidence:** `MeshStore::get_or_load` runs inside
  `MeshRenderSyncSystem`'s entity loop (`sync.rs:150-158`): the first frame
  an entity references an unloaded asset performs disk read + full glTF
  parse + PNG decode + tangent generation + mipgen + GPU upload synchronously
  (`store.rs:177-201`, `gltf_import.rs:86-111`) — `statue_vroid.glb` is
  11 MB with embedded textures. Zone changes additionally run
  `set_environment` (`presentation.rs:64`), a synchronous 43-submit IBL bake
  (`ibl.rs:126-174`: 6 equirect faces + 6 irradiance faces at 512 samples
  each + 30 prefilter faces at 256 samples + LUT until audit finding 7
  removes it). Every one of these lands inside `Phase::RenderSync`/`Render`
  of one display frame.
- **Ideal:** IO, decode, tangent/mip prep happen on worker threads; GPU
  uploads are budgeted per frame; an unloaded mesh renders nothing (or a
  placeholder) for the frames it needs; the environment bake is either
  chunked across frames or pre-baked to disk artifacts at content time.
- **Gap:** every new asset and every zone crossing is a visible hitch — the
  exact "frame never allocates unbounded work" spirit of VQ-F3.
- **Tradeoffs:** *Wins:* smooth zone crossings and mid-fight spawns
  (enemies!). *Losses:* async introduces load states the sync systems must
  tolerate; a placeholder policy touches presentation; pre-baked IBL
  artifacts add a content-pipeline step.
- **Suggestion:** /plan-rework after audit findings 6+7 (eviction + LUT
  hoist shrink the moving parts) and finding 2 (per-pass timing to measure
  the win). The plan should weigh worker-thread decode + budgeted upload vs
  full pre-baked artifacts (meshes are already preprocessed by the content
  pipeline — baking tangents/mips/BC there dovetails with rework 4).
- **Path:** plan → measure current hitch (wall-clock around get_or_load /
  set_environment) → land the chosen design in fix-sized steps → hitch
  re-measured under the same scenario.

### 3. Sorted transparency pass — glTF BLEND is silently punched to cutout

- **Evidence:** `gltf_import.rs:31-34`: "BLEND is approximated as MASK at
  0.5 — there is no sorted transparency pass"; both mesh pipelines blend
  `REPLACE` (`mesh_pipeline.rs:160`, `skinned_pipeline.rs:133`). Any asset
  with real alpha — glass, ghosts, banners, water planes, wisp effects —
  loses its blend mode at import with only a struct comment as witness.
- **Ideal:** BLEND primitives route to a transparent draw list rendered
  after opaque+sky in back-to-front order (per-primitive centroid sort),
  depth-test-on/write-off, premultiplied alpha, fogged consistently.
- **Gap:** a whole material class the dark-fantasy art direction will want
  (spirits, fog volumes, stained glass) cannot ship.
- **Tradeoffs:** *Wins:* unlocks the material class; removes a silent data
  loss. *Losses:* sorting is per-frame CPU; intersecting transparents remain
  wrong without OIT (accepted at AA scale); a second pipeline pair + list
  plumbing.
- **Suggestion:** /plan-rework. The plan decides list granularity (per
  primitive is enough at current mesh counts), how skinned transparents are
  handled, and whether particles and mesh transparency share the sort.
- **Path:** plan → import keeps BLEND distinct → transparent list + pipeline
  variants → offscreen analytic test (a red glass quad over a white cube
  reads pink, not cutout).

### 4. Compressed GPU textures for the material path + a texture-memory meter

- **Evidence:** every glTF/procedural material texture is decoded to RGBA8
  and uploaded uncompressed with runtime mipgen (`store.rs:37-51`,
  `texture.rs:98-143`). The mud_leaves 2k ground set alone is ~65 MB decoded
  (3 maps × 2048² × 4 B × 1.33) vs ~11 MB as BC7; character glbs embed PNGs
  that take the same path. The device already requires
  `TEXTURE_COMPRESSION_BC` (`state.rs:213`) and a BC7 DDS loader exists
  (`texture.rs:40-94`) — but nothing on the material path uses it. VQ-C5's
  ≤1 GB budget has no meter anywhere: no dev-overlay line, no content-lint
  size check on decoded footprint.
- **Ideal:** content-pipeline preprocessing transcodes material maps to BC7
  (color, sRGB) / BC5 (normals) with baked mips (KTX2 container or DDS
  side-files); the importer prefers them and falls back to RGBA8; the dev
  overlay meters resident texture memory; content-lint enforces VQ-C5.
- **Gap:** 4-6× the necessary VRAM and upload time per material, and the
  stated budget is unenforced.
- **Tradeoffs:** *Wins:* VRAM, bandwidth, load time; dovetails with rework
  2's pre-bake direction. *Losses:* a content-pipeline step (transcoder
  dependency) and dual-path import; BC5 normals need a shader z-reconstruct.
- **Suggestion:** /plan-rework, coordinated with the content-pipeline domain
  (the preprocessing scripts live there). The meter + lint can be the plan's
  first fix-sized step regardless of the transcode design.
- **Path:** plan → meter + lint → transcode step in the asset pipeline →
  importer dual path → measure resident memory before/after on the current
  zone set.

### 5. Scene scalability — nothing is culled, dead SDF slots still cost vertices

- **Evidence:** every frame draws every allocated instance of everything:
  the main pass iterates all mesh ranges (`frame.rs:471-510`), the shadow
  pass re-draws the same full lists into the sun's 160 m box
  (`frame.rs:392-424`), and the SDF passes draw `0..slot_count` where
  `slot_count` is the pool's historical high-water mark — freed slots are
  zeroed but never skipped or compacted (`frame.rs:389`,
  `instance.rs:55-78`), so 36 indices × every-slot-ever-allocated go through
  vertex fetch forever. No frustum test exists anywhere.
  `docs/visual-quality.md:135` already tracks frustum culling and LOD as
  future work.
- **Ideal:** camera-frustum culling on mesh/skinned ranges and SDF slots,
  light-volume culling for the shadow pass, and distance LOD for skinned
  rigs (pose at reduced rate or fewer joints beyond N meters) — frame cost
  tracks the visible set, satisfying VQ-F1 at chapter-20 populations.
- **Gap:** cost scales with world population, not visibility; harmless at
  today's counts, structural before enemies land.
- **Tradeoffs:** *Wins:* the stress-scene budget survives content growth.
  *Losses:* culling needs bounds data (mesh AABBs exist implicitly, must be
  computed at import); wrong bounds = popping; LOD posing adds state.
- **Suggestion:** /plan-rework once a measured driver exists (the
  pre-content stage means current scenes are far under budget — but the bar
  is the ideal, so it is ordered, not parked). First plan step should be the
  cheap independent win: skip zeroed/freed SDF slots (or compact the pool).
- **Path:** plan (gate its scope on finding 2's per-pass numbers under a
  synthetic 40-rig + full-camp scene) → import-time AABBs → frustum cull
  lists → shadow-volume cull → LOD policy.

### 6. Image-quality ladder — CSM, SSAO, specular/temporal AA, richer fog

- **Evidence:** one 2048² ortho cascade over a fixed 160 m box
  (`shadow.rs:11-16`) yields ~7.8 cm/texel — soft contact around feet blurs
  at PCF 3×3 (`mesh_shader.wgsl:31-49`); ambient occlusion is texture-AO
  only (`mesh_shader.wgsl:175` — no screen-space term, so props and
  characters don't ground into ambient-lit corners); MSAA 4× resolves
  geometry edges but nothing addresses specular shimmer on normal-mapped
  surfaces in motion; fog is exponential-distance only (no height fog /
  aerial perspective gradient). All four are the named residents of
  `docs/visual-quality.md:135`'s future-work list.
- **Ideal:** 2-3 shadow cascades with stable splits; a half-res SSAO term
  multiplying the IBL ambient; a specular-AA measure (Toksvig/roughness
  filtering first, TAA only if still needed against MSAA's cost); height
  fog layered on the distance term.
- **Gap:** the last mile between "correct PBR image" and "AA image" — each
  item is visible in the dusk register the project targets.
- **Tradeoffs:** *Wins:* the polish tier. *Losses:* every item is a
  subsystem with tuning surface; TAA in particular conflicts with the MSAA
  investment and needs motion vectors the pipeline doesn't produce.
- **Suggestion:** /plan-rework per item, after rework 1 (accent lights
  change what AO/shadows must play against). Roughness-filtering
  (Toksvig at import) may be a fix-sized extraction the plan should check
  first.
- **Path:** ordered last; plan the items individually when their layer
  below (lights, culling) is in place.

### 7. Per-environment IBL pipeline recompilation dominates the zone-crossing cost the LUT hoist targeted

- **Origin:** measured while implementing audit finding 7 (BRDF LUT
  re-baked per environment). The fix landed as specified — `bake_brdf_lut`
  runs once at `RendererState`/`OffscreenRenderer` init and every
  `Environment` shares the view (`ibl.rs`) — but the wall-clock win the
  finding expected from removing the redundant bake did not show up.
- **Evidence:** instrumented timing (removed after measurement) over 10
  offscreen `set_uniform_environment` calls showed ~24ms/environment both
  before and after the fix (241ms vs ~240ms for 10 loads). Breaking a single
  load down: `Baker::new(device)` (recompiles the shader module and all 4
  bake pipelines — equirect, irradiance, prefilter, brdf — from scratch)
  costs ~9-10ms; the equirect+irradiance+prefilter bake passes together cost
  ~14ms. `Baker::new` runs unconditionally inside
  `Environment::from_equirect_pixels` on every zone crossing
  (`ibl.rs:from_equirect_pixels`), including compiling a `brdf_pipeline` that
  is now never invoked there (the shared LUT already exists) — pure waste
  post-fix, on top of the pre-existing waste of recompiling the other three
  pipelines identically every load.
- **Ideal:** none of the four bake pipelines' compiled state depends on the
  equirect pixel data — only the per-face bind groups and the source texture
  do. All four pipelines (and the shader module) should compile once at
  init, like the LUT itself, and every environment load should reuse them.
- **Gap:** the measured per-zone-crossing cost is dominated by pipeline
  recompilation, not by any of the bake passes — the actual target of
  finding 7's fix is a small fraction of the total, so hoisting the LUT
  alone doesn't move the number that matters (zone-crossing wall time).
- **Tradeoffs:** *Wins:* likely the largest remaining win for zone-crossing
  latency, directly feeds rework 2 (asset streaming) which already cites
  finding 7 as shrinking "the moving parts" — this shrinks the same moving
  part further. *Losses:* `Baker` currently borrows `device` for its
  lifetime and is constructed fresh per bake; hoisting it means either
  storing it on `RendererState`/`OffscreenRenderer` (more fields) or
  restructuring bake calls to take pipelines by reference instead of owning
  a `Baker`.
- **Suggestion:** /plan-rework alongside or as a prerequisite step of rework
  2. The plan should hoist `Baker`'s shader module + 4 pipelines to init
  (same treatment as `bake_brdf_lut`), leaving only the per-environment
  texture/bind-group creation and the actual bake draw calls in
  `from_equirect_pixels`.
- **Path:** plan → hoist `Baker` construction to
  `RendererState::init`/`OffscreenRenderer::new` → thread it (or its
  pipelines) into `from_equirect_pixels` → re-measure the same
  zone-crossing timing this finding used, confirm the ~9-10ms/load drops
  out.

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run.
