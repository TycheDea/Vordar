# Expert Review: Real-Time Rendering & Visual Quality
**Reviewer persona:** Principal Real-Time Rendering Engineer
**Date:** 2026-07-27
**Scope:** engine-renderer, client presentation, visual pipeline

## Executive summary

Vordar’s renderer is already past “prototype Lambert slab.” The live path is a coherent HDR forward stack: MSAA 4× `Rgba16Float` scene color, Cook–Torrance GGX with full glTF material slots, split-sum IBL (same HDRI for sky + ambient), three concentric sun cascades with PCF, dual-filter Kawase bloom, ACES tonemap before egui, height-aware distance fog, SSAO on ambient only, textured soft particles, CPU dual-frustum culling, and skinned glTF with joint-palette instancing. The quality bar in `docs/visual-quality.md` is unusually precise (VQ-IDs, machine vs eyeball), and verification is real: analytic offscreen readbacks plus adapter-locked mean-FLIP goldens.

What keeps it from reading like a shipping AA dark-fantasy client is not missing checkboxes so much as **frame-graph waste, theme/lighting authority fights, content-level incompleteness of the three-beat VFX contract, and CPU-side skinning/upload cost that will not survive the enemy mesh wave under VQ-F1**. The depth prepass is paid twice (SSAO only; main pass re-clears depth). Zone dressing nails dusk, then `DayNightSystem` can overwrite it with noon-neutral sun against a dusk HDRI. Ability VFX RON files author cast only; weapons are untextured cuboids; telegraphs remain SDF discs. Portability is desktop-BC-or-bust.

Net: foundation is production-shaped and worth preserving. Priority work is (1) make lighting authority match VQ-A5, (2) stop double-drawing geometry / reclaim Early-Z or drop the prepass cost, (3) finish travel/impact + telegraph readability against the quality bar, (4) budget GPU time and skinning before enemy meshes land.

## Findings

### F1. [SEVERITY: High] Depth prepass is SSAO-only; main pass discards it and redraws all opaque geometry
- **Where:** `smirk/engine-renderer/src/frame.rs` (`record_depth_prepass` → `ssao_targets.prepass_depth_view`; `record_main_pass` clears `hdr.depth_view` to 1.0); `smirk/engine-renderer/src/ssao.rs`
- **What:** When `ssao_enabled`, every opaque SDF/mesh/skinned draw is recorded once into a single-sample prepass depth, SSAO runs, then the main MSAA pass **clears depth again** and redraws the same geometry with full PBR. There is no depth reuse, no `Equal`/`LessEqual` Early-Z main pass, and no Hi-Z hierarchical reject.
- **Why it matters:** At the VQ-F1 stress point (40 skinned + environment), geometry bandwidth and VS skinning cost are paid twice. SSAO is ambient-only (correct artistic choice) but its prepass is the most expensive way to get a depth buffer if main never inherits it. This is the largest structural GPU waste in the frame graph.
- **Recommendation:** Either (a) promote the prepass to a true Early-Z path (MSAA depth or resolve + main with depth test Equal and no redundant VS work where possible), or (b) generate SSAO from a cheaper half-res depth (pyramid / min-depth downsample of the main resolve) and delete the full-res geometry prepass. Measure `gpu shadow` / `gpu main` on F3 before/after; target reclaiming most of the prepass VS cost.

### F2. [SEVERITY: High] Zone dusk authority fights DayNight / noon-neutral sun (VQ-A5)
- **Where:** `client/vordar-client/src/presentation.rs` (`SUN_DIR` / `SUN_COLOR`, `set_light` on zone apply); `client/vordar-client/src/world_time.rs` (`DayNightSystem` → `day_night_light`); `game/vordar-game/src/world/mod.rs` (`day_night_light`: day color ≈ `(1.0, 0.95, 0.85)`, ambient up to 1.0)
- **What:** Zone dressing loads a Castilian plateau **dusk** HDRI and stamps a fixed low amber key. Once world time is synced, `DayNightSystem` overwrites direction/color/ambient every frame from a full day cycle, including bright noon against the same dusk cubemap. VQ-A5 explicitly marks bright noon-neutral scenes off-theme.
- **Why it matters:** IBL + sun coherence is what sells religious dark fantasy. A dusk irradiance with a high white sun reads wrong (double lighting, wrong shadow direction vs sky, washed stone). Sandbox offline may look “on theme” while networked play drifts off it.
- **Recommendation:** Make zone visuals the authority for *mood*: either lock zones to authored key+ambient (day/night only scales exposure/ambient within a dusk envelope), or author day/night as HDRI swaps + matched sun tables per zone. Do not let `day_night_light`’s noon white key ride a dusk environment. Add an offscreen/zone_review assertion: under default start-zone path, sun color stays inside VQ-A5 warm bias.

### F3. [SEVERITY: High] CPU skinning + full joint-palette upload every frame will miss VQ-F1 under enemy meshes
- **Where:** `smirk/engine-renderer/src/mesh/sync.rs` (`pose_player_into`, `pose_with_lod`, `MAX_SKINNED_INSTANCES = 256`); `smirk/engine-renderer/src/frame.rs` (`upload_gpu_buffers` writes entire skinned instance + joint buffers); `smirk/engine-renderer/src/skinned_pipeline.rs` (`MAX_JOINT_MATRICES = 256×64`)
- **What:** Every visible skinned entity samples clips, blends, builds globals/palette on CPU, then `queue.write_buffer`s the whole joint SSBO (up to 16 384 mat4s) and instance buffer each display frame. Far LOD halves pose rate past 40 m (good) but still re-uploads. Caps and 80% warnings exist; there is no GPU skinning, no persistent-mapped ring, no dirty joint ranges.
- **Why it matters:** VQ-F1 is 60 fps @ 1080p with 40 skinned + 2k particles. CPU pose + PCIe upload scales linearly with on-screen rigs; enemy introduction multiplies this path. This is the first hard ceiling after content leaves ShapeGroup blobs.
- **Recommendation:** Before enemy mesh enablement: (1) dirty-range or double-buffer joint uploads; (2) consider compute/GPU skinning or at least pose on a job pool; (3) add a criterion/GPU-timer stress bench that fails the gate when skinned pose+upload exceeds a fixed ms budget at 40 / 128 instances. Keep the 256 cap until that bench is green.

### F4. [SEVERITY: High] Desktop BC compression hard-required; no portable fallback path
- **Where:** `smirk/engine-renderer/src/state.rs` (`create_surface_and_device`: `required_features: TEXTURE_COMPRESSION_BC`, panic message “desktop GPU needed”); `smirk/engine-renderer/src/texture.rs` (BC7/BC5 DDS path); content ground/detail ships `*_2048.dds`
- **What:** Device creation demands BC. Offscreen tests skip without an adapter, but a non-BC adapter cannot run the client at all. VQ-C5 and the art pipeline assume BC7/BC5 sidecars; RGBA8 mipgen exists but is not a runtime fallback for missing BC.
- **Why it matters:** wgpu’s value is portability (DX12/Metal/Vulkan/GLES/Web). Forcing BC excludes many iGPUs, all WebGPU-without-BC targets, and CI fallback adapters for full client runs. It also couples shipping to the DDS bake step.
- **Recommendation:** Feature-detect BC; if absent, load PNG/RGBA8 (already in tree) with mipgen, or ship dual assets. Keep BC as the fast path. Document the matrix in visual-quality or a renderer README. Do not `expect` on BC in `request_device`.

### F5. [SEVERITY: Medium] MSAA sample count is a compile-time constant, not an adapter-negotiated fallback (VQ-D4)
- **Where:** `smirk/engine-renderer/src/post.rs` (`SCENE_SAMPLES: u32 = 4` — comment claims WebGPU guarantee); all scene pipelines hardcode `count: SCENE_SAMPLES`
- **What:** VQ-D4 requires MSAA 4× with a documented fallback to 1× when unsupported. Code always builds 4× HDR MSAA targets and pipelines. WebGPU guarantees 4× for this format pair on conforming implementations, but wgpu on real drivers can still fail limits/memory, and there is no runtime downgrade path on resize/OOM.
- **Why it matters:** A single unsupported sample count fails pipeline creation or exhausts tile memory on weaker GPUs with no graceful degrade — while VQ promises one.
- **Recommendation:** Query sample-count support / memory at init; rebuild `HdrTargets` + pipelines for 4 or 1. Surface the choice on the F3 overlay. Keep analytic MSAA edge test for the 4× path only.

### F6. [SEVERITY: Medium] Ability VFX RON is cast-only; three-beat contract is split and easy to ship incomplete (VQ-E1)
- **Where:** `content/vfx/*.ron` (only `cast: Some(...)`); `game/vordar-game/src/vfx.rs` (cast from RON; travel/impact via `VfxTrail` on prefabs); `client/vordar-client/src/vfx.rs`; `client/vordar-client/src/telegraph.rs` (scheduled impact burst hardcoded)
- **What:** Quality bar VQ-E1 requires cast / travel / impact per ability. Data model supports it, but authored ability files only define cast. Travel/impact depend on projectile prefab components and ad-hoc telegraph resolve bursts. Melee-style abilities (cleave, rend, onslaught) have cast colors that skew warm/ember — overlapping threat language (VQ-A4) rather than votive cool for all player VFX.
- **Why it matters:** Without content-lint tying every ability id to all three beats, the game will ship “muzzle flash only” abilities. Color language drift (warm player casts vs cool bolt) weakens telegraph-vs-self readability in chaotic AOE.
- **Recommendation:** Extend content-lint: every combat ability references cast + (travel|telegraph) + impact. Move telegraph resolve VFX into data. Re-tint martial casts into the votive cool band or explicitly document a “weapon ember” sub-role that still sits ≥10° from threat crimson.

### F7. [SEVERITY: Medium] Telegraphs are SDF discs, not grounded PBR decals — legibility depends on emissive alone (VQ-E4 / VQ-A2)
- **Where:** `client/vordar-client/src/telegraph.rs` (`RenderShape` + `TELEGRAPH_DIM`→`TELEGRAPH_BRIGHT` HDR lerp); SDF path in main pass
- **What:** Scheduled mechanics use scaled SDF shapes with a client-side fill from synced server time (excellent fairness model). Visually they are untextured emissive blobs, not projected decals with occlusion against the heightmapped ground, edge rings, or depth-fade.
- **Why it matters:** VQ-E4 demands clear contrast vs ground and ≥0.4 s lead. Emissive red-orange works at dusk but can wash out on bright stone, sink through hills at the play-edge ramp, or fail colorblind separation without a secondary channel (pulse, chevron, dark core).
- **Recommendation:** Replace or layer a ground-projected mesh/decal (or soft particle ring) with dark core + threat rim, depth-tested to terrain, optional stencil. Keep the time→fill pure function. Add a sandbox feel-check screenshot pair (mid-fill / resolve) to the appendix.

### F8. [SEVERITY: Medium] Full-frame GPU buffer uploads ignore dirty ranges for mesh/skinned/particles
- **Where:** `smirk/engine-renderer/src/frame.rs` `upload_gpu_buffers`; contrast SDF path using `dirty_ranges` / `InstancePool`
- **What:** Static mesh instances, skinned instances, joints, and particles are rewritten from offset 0 every frame whenever non-empty. SDF is the only pool with sparse dirty uploads.
- **Why it matters:** Under VQ-F3 (“never allocate unbounded per-entity GPU resources”) the caps are fine, but **bandwidth** still scales with live counts every frame. 16 k mesh instances × instance stride + joint SSBO is real PCIe pressure on integrated GPUs.
- **Recommendation:** Mirror the SDF dirty-range pattern (or persistent/coherent staging rings). At minimum, skip joint rewrite when LOD reuses palette and no near rig posed. Track upload bytes on the dev overlay next to `skinned` / `particles`.

### F9. [SEVERITY: Medium] Transparent path is correct-ish but kills instancing; OIT still approximate
- **Where:** `smirk/engine-renderer/src/frame.rs` `collect_transparent_draws` / transparent loop (`draw_indexed(..., instance..instance+1)`); visual-quality “Future work” OIT note
- **What:** Blend primitives are sorted back-to-front across static+skinned (good; unit-tested). Each transparent instance is a separate draw with pipeline flips when skinned bit changes. Intersecting transparents and particle-vs-glass order remain approximate (documented).
- **Why it matters:** Foliage, shrine glass, and VFX-adjacent alpha will thrash the API thread. Sorted alpha is acceptable for AA prototype density; it will not survive dense vegetation without batching or WBOIT/weighted blended.
- **Recommendation:** Keep sort for correctness now; when prop density rises, bucket by (pipeline, mesh, material) within depth bands or adopt weighted blended OIT for non-critical glass. Never sort particles with mesh glass in the same list without an explicit layer policy.

### F10. [SEVERITY: Medium] Shadow cascades are focus-centric concentric maps, not frustum-split CSM
- **Where:** `smirk/engine-renderer/src/shadow.rs` (`CASCADE_HALF_EXTENTS = [24, 60, 160]`, `fit_vp` around `camera.target`); `snippets/shadow_sample.wgsl` (tightest cascade first, outer fade)
- **What:** Three 2048² layers, texel-snapped, slope-scaled bias, PCF 3×3 — solid craft. Fitting is concentric around the orbit target, not practical-split along the view frustum. Looking away from the focus wastes near-cascade resolution; max orbit can still see past 160 m (handled by fade-to-lit).
- **Why it matters:** Contact shadows at feet (feel-checklist item 12) depend on the 24 m cascade covering the player. It usually does because the camera targets the player — until cinematics, large AOE cameras, or multi-focus spectate break that invariant.
- **Recommendation:** Assert camera target ≈ player for gameplay modes. If camera modes diversify, switch to frustum-clipped cascades or enlarge near extent with a player-centric anchor separate from look-at. Add offscreen test: character under camera offset still receives near-cascade contact darkening.

### F11. [SEVERITY: Medium] Skinned characters skip world-space detail overlay; weapons are flat placeholders
- **Where:** `mesh_shader.wgsl` detail path (`material.emissive.w` opt-in + group 3); `skinned_mesh_shader.wgsl` (no detail include); `client/vordar-client/src/weapons.rs` (solid-color cuboids, metallic/roughness factors only)
- **What:** Environment stone gets triplanar micro-detail (Mikkelsen gradients — high-end touch). Characters do not. Held weapons are procedural untextured boxes explicitly marked placeholder — violating VQ-A2 if treated as shipped silhouettes.
- **Why it matters:** Cohesion (VQ-A3): hero reads smoother/plasticky next to triplanar limestone ground and BC7 props. Weapons break the semi-realistic register in every close camera orbit.
- **Recommendation:** Short term: hide or theme-tint weapons; do not ship sandbox footage with grey boxes. Medium term: skinned-compatible detail or higher-res character maps; real grip meshes on sockets. Content-lint: ban default-white materials on non-dev prefabs.

### F12. [SEVERITY: Medium] Mesh streaming cap `MESH_UPLOADS_PER_FRAME = 1` stalls zone pop-in
- **Where:** `smirk/engine-renderer/src/mesh/store.rs` (`MESH_UPLOADS_PER_FRAME`); `mesh/sync.rs` `store.integrate(..., MESH_UPLOADS_PER_FRAME)`; zone props in `presentation.rs`
- **What:** At most one mesh GPU upload completes per frame after async load. A zone with many props/portals appears over dozens of frames; `streaming` counter is on the overlay but there is no prioritized hero/ground-first queue beyond request order.
- **Why it matters:** First impressions and zone transfers hitch visually (pop-in, missing collision meshes for eyes). Not a GPU frame-budget issue — a latency/presentation issue.
- **Recommendation:** Priority tiers (ground=0, player=0, portals=1, props=2) and a budget in ms/bytes rather than count=1. Pre-warm start zone at loading screen.

### F13. [SEVERITY: Medium] No automatic exposure coupling to day/night or HDRI intensity
- **Where:** `facade::set_exposure`; `DayNightSystem` only calls `set_light`; bloom prefilter uses exposure (`bloom.rs` / `tonemap.wgsl`)
- **What:** Exposure is a manual knob (default 1.0). Ambient scales IBL with day fraction, sun color lerps, but tonemap exposure and bloom threshold stay fixed. Zone HDRIs can be authored hot or cold without a calibrate step in-engine.
- **Why it matters:** Bloom either crushes magic emissives at dusk or halos everything at noon; VQ feel-checklist item 6 (soft bloom, no clipping halos) depends on exposure discipline that is not automated.
- **Recommendation:** Derive a target exposure from HDRI average luminance at bake time (store in `.manifest.json`) and lerp with day/night. Keep manual override for art. Offscreen monotonic emissive tests already exist — add a “dusk vs noon mid-grey” band check.

### F14. [SEVERITY: Low] Tonemap is ACES Narkowicz only; VQ-D1 allows AgX but it is not implemented
- **Where:** `smirk/engine-renderer/src/tonemap.wgsl` (`aces`); `docs/visual-quality.md` VQ-D1 “ACES/AgX”
- **What:** Fitted ACES is fine and tested. AgX is often preferred for punchy game emissives and skin. No runtime selector.
- **Why it matters:** Low urgency — ACES is a valid bar — but the quality doc over-promises operator choice.
- **Recommendation:** Either implement AgX as a second curve behind a debug mode or narrow VQ-D1 wording to ACES until AgX lands.

### F15. [SEVERITY: Low] GPU timer uses blocking readback on the overlay path
- **Where:** `smirk/engine-renderer/src/frame.rs` (`timer.read_blocking` when `sample_gpu`); `gpu_timer.rs`
- **What:** Every N frames while F3 is open, the CPU waits on timestamp queries after submit.
- **Why it matters:** Profiling hitch skews the numbers you came to measure and can drop a frame in the stress scene.
- **Recommendation:** Double-buffer queries; read N−1 results asynchronously. Never block the frame that emitted the timestamps.

### F16. [SEVERITY: Low] Soft particles sample MSAA depth sample 0 only
- **Where:** `particle_shader.wgsl` (`texture_depth_multisampled_2d`, `textureLoad(..., 0)`)
- **What:** Soft fade uses one MSAA sample. Cheap and usually fine; can shimmer on thin geometry edges.
- **Why it matters:** Minor VFX quality under 4× MSAA.
- **Recommendation:** When touching particles next, average two samples or resolve a single-sample depth for FX. Not a blocker.

### F17. [SEVERITY: Info] Goldens are adapter-locked FLIP thresholds — correct discipline, fragile CI story
- **Where:** `smirk/engine-renderer/tests/golden.rs`; `tests/goldens/*.png`; analytic `tests/offscreen.rs`
- **What:** Mean-FLIP vs checked-in PNGs with `UPDATE_GOLDENS=1` regeneration; offscreen tests use analytic assertions and skip without GPU. Excellent separation of concerns (VQ-G1).
- **Why it matters:** Cross-machine CI cannot treat goldens as absolute without hardware locks. That is intentional, not a bug.
- **Recommendation:** Keep analytic offscreen as the merge gate; run goldens on a labeled GPU runner only. Never auto-regenerate in agents (already documented).

### F18. [SEVERITY: Info] Pass ordering is coherent and worth freezing as a contract
- **Where:** `frame.rs` module docs and `run`: env poll → uploads → shadow cascades → (depth prepass+SSAO) → main (SDF → opaque mesh → opaque skinned → sky → sorted transparent) → particles (MSAA resolve) → bloom → tonemap → egui
- **What:** Resolve is deliberately deferred to the particle pass (`Load` + `resolve_target` + `StoreOp::Discard` on MSAA). Sky is drawn after opaque with far-plane pin. UI never enters HDR. Point lights capped to 16 nearest focus with flicker.
- **Why it matters:** This is a clean mental model for contributors. The particle-pass resolve means “skip particles pass” would black-frame the game — a footgun worth a comment/assert (empty particle pass still resolves today — good).
- **Recommendation:** Codify the frame graph in a short `engine-renderer` diagram; add a debug assert that tonemap’s HDR view is the resolve target written this frame.

## Strengths worth preserving

1. **PBR core is real, not fake gloss** — shared `pbr_common.wgsl` Cook–Torrance, specular AA (Karis/Tokuyoshi), glTF MASK via alpha-to-coverage + `fwidth`, premultiplied BLEND, normal z-reconstruction for BC5.
2. **IBL + sky unity** — one HDRI → cubemap, irradiance, prefilter mips, BRDF LUT; sky samples the same env; async decode so zone crosses do not hitch the frame thread.
3. **Shadow craft** — cascade array, texel snap, slope bias, PCF, edge margin + outer fade; dual frustum classification packs cam/shadow instance ranges without double transforming.
4. **Post stack matches the bar** — HDR MSAA → resolve → Kawase bloom (display-referred threshold) → ACES → sRGB swapchain; egui composited after tonemap.
5. **Presentation systems are gameplay-literate** — telegraph fill is a pure function of server time; day/night seam exists (even if authority is wrong); sockets drive VFX and weapons; particle sim is CPU-tested (caps, additive/alpha partition, far-first alpha).
6. **Art pipeline direction** — BC7/BC5 DDS sidecars, mipgen, aniso 8×, detail triplanar with distance fade, procedural ground with flat play radius + hill skirt (gameplay plane preserved).
7. **Verification culture** — snippet preprocessing tests, transparent collector unit tests, offscreen analytic suite (~30 tests), FLIP goldens, content_lint hooks for rigs — rare at this project size and aligned with VQ-G1.
8. **Dev overlay metering** — skinned/particles 80% warnings, GPU pass timers, texture MB, streaming pending — the right knobs for VQ-F2/F3.

## Suggested priority order

1. **F2 — Lighting authority / VQ-A5** — cheapest high visual impact; stop noon-vs-dusk fights before more art lands.
2. **F1 — Depth prepass / Early-Z** — largest GPU structural win before content density rises.
3. **F3 + F8 — Skinning & upload bandwidth** — prerequisite to enemy meshes and VQ-F1.
4. **F6 + F7 — VFX three-beat + telegraph decals** — combat readability is the game’s fantasy; color language enforcement.
5. **F11 — Weapons / character micro-detail** — close-camera cohesion.
6. **F4 + F5 — Portability & MSAA fallback** — before any non-desktop target or broader CI.
7. **F12 + F13 — Streaming priority & exposure calibration** — zone transfer and bloom stability.
8. **F9 + F10 — Transparency batching & cascade policy** — when prop/camera scope expands.
9. **F14–F16 — Tonemap choice, timer hitch, soft-particle polish** — quality-of-life.
10. **F17–F18 — Process** — freeze frame-graph contract; keep analytic vs golden split.

---

*Evidence base: `docs/visual-quality.md`, `tasks/aa-visual-upgrade-plan.md`, `docs/benchmarks/{BASELINE,WEAKPOINTS}.md` (sim-side; GPU benches still manual), `smirk/engine-renderer/src/{frame,state,post,bloom,shadow,ssao,ibl,mesh/*,*pipeline*,snippets/*,particle_*,tonemap,texture,culling,camera,light_sync}.rs`, client `presentation`, `vfx`, `telegraph`, `ground`, `weapons`, `world_time`, `content/{vfx,textures,models}`, `tests/{offscreen,golden}.rs`. No files under `docs/reviews/**` were read.*
