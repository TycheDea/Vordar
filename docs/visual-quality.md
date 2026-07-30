# Visual Quality Bar — Semi-Realistic Religious Dark Fantasy

The game's look is **Castilian/Andalusian religious dark fantasy** rendered
semi-realistically (Diablo IV / Path of Exile 2 register): realistic proportions,
PBR materials, sun-bleached and soot-darkened stone, warm candle and gilt accent light.
This document is the quality bar every visual asset and renderer feature must meet.

Rules have stable IDs (`VQ-xx`). **Machine-checked** rules name the test that
enforces them; **eyeball** rules have a written check procedure the user performs
in the sandbox. Enemy-specific clauses are written now but *deferred until real
enemies land* — enemies stay as placeholder ShapeGroup blobs until then.

Scope of the current push: **player character + environment**. Enemies/NPCs are
explicitly out of scope.

---

## A. Art direction (eyeball)

- **VQ-A1** — Semi-realistic religious dark fantasy: realistic proportions, PBR
  materials, sun-bleached/soot-darkened desaturated stone palette, warm
  candle/gilt accent light (votive flame, altarpiece gold, shrine glow). No
  flat-shaded or low-poly-stylized assets; no toon outlines.
  *Check:* sandbox screenshot review at each phase boundary.
- **VQ-A2** — Every shipped surface is PBR-textured (albedo + normal + roughness
  minimum). Untextured flat color is placeholder-only, never shipped.
  *Check:* eyeball in sandbox; machine-checked per-map presence arrives with the
  Phase 1 material importer (content-lint extension).
- **VQ-A3** — New asset packs require a side-by-side cohesion screenshot in the
  sandbox before adoption: texel density and realism level must match what is
  already in the scene.
  *Check:* manual, at pack-adoption time.
- **VQ-A4** — Reserved color language (HSV, S/V are floats 0–1):
  | Role | Hue | Saturation | Value |
  |---|---|---|---|
  | Player VFX (votive) | 190°–230° (pale blue-white "cold flame") | 0.15–0.45 | 0.85–1.0 (HDR boost via emissive) |
  | Environmental emissive (candle-gold) | 35°–50° (candle flame, gilt glint, shrine glow) | 0.55–0.85 | 0.75–1.0 (flames HDR via emissive) |
  | Threat / telegraph | 350°–25° (crimson→red-orange, wraps 0°) | 0.7–1.0 | 0.8–1.0 |
  | Ambient world | any; warm stone bias 20°–50° when chromatic | ≤ 0.35 | ≤ 0.6 |
  Roles keep the legibility split: player = cool, threat = warm; candle-gold sits
  ≥ 10° above the threat band and never telegraphs danger.
  *Check:* eyeball; VFX RON defs cite the role they use. The numeric ranges are a
  proposal — the user tunes them at the B1 phase-gate text review.
- **VQ-A5** — Lighting sells the theme: a neutral grey overcast key (sun
  intensity 0.25, elevation 60°, no warm tint), cool grey-blue fog sampled
  from the sky's upper-elevation mean (0.327, 0.352, 0.409), exposure 0.576
  (G1 gate, `docs/reviews/town/g1-lighting-2026-07-30.md`). Materials must
  read their true PBR color under this near-zero-chroma field — no hue cast
  from the key or the fog. Votive-candle/shrine emissive accents (portals,
  altars, magic) are the zone's one deliberate warm chroma, standing out
  precisely because the field around them carries none.
  *Check:* eyeball per zone.

## B. Characters (machine-checked; enemy clauses deferred)

- **VQ-B1** — Every combat-relevant entity renders as a rigged glTF mesh with the
  minimum clip set idle/walk/run/attack/hit/death. *(Players: enforced now;
  enemies: deferred.)* SDF ShapeGroup is a dev fallback only — banned from
  shipped prefabs by content-lint, not by code deletion.
  *Test:* `game/vordar-game/tests/content_lint.rs` (`race_clips_exist_in_gltf`).
- **VQ-B2** — Rigged assets: ≤ 64 joints (engine palette cap), height-normalized
  feet-on-ground, `forward_offset` documented in the race RON, ≤ 16 MB on disk.
  *Test:* `content_lint.rs` (`race_models_within_budgets`).
- **VQ-B3** — Character rigs expose sockets (right hand, left hand, head) via the
  per-race socket-bone mapping; every socket bone named must exist in the glb.
  *Test:* `content_lint.rs` (`race_models_expose_sockets`).
- **VQ-B4** — Every clip named in a race `.ron` exists in the referenced `.glb`.
  *Test:* `content_lint.rs` (`race_clips_exist_in_gltf`).
- **VQ-B5** — Generated props: ≤ 32 MB on disk.
  *Test:* `content_lint.rs` (`prop_models_within_byte_budget`).

## C. Materials & textures (machine-checked)

- **VQ-C1** — Every filtered texture has a full mip chain; surface samplers use
  anisotropic filtering ≥ 8×.
  *Test:* Phase 1 — mipgen unit test + texture-load assertions.
- **VQ-C2** — sRGB correctness: albedo/emissive sRGB; normal/metallic-roughness/AO
  linear.
  *Test:* Phase 1 — importer unit test on format selection per slot.
- **VQ-C3** — Anything "magical" glows via HDR emissive (> 1.0) so bloom picks it
  up — no fake glow via bright albedo.
  *Test:* Phase 4 — content-lint on emissive factors of magical materials/VFX.
- **VQ-C4** — Normal maps present on all environment surfaces and characters;
  tangents present in the asset or generated at import.
  *Test:* Phase 1 — tangent-gen unit tests; content-lint map presence.
- **VQ-C5** — Texture budgets: ≤ 2k per character map, ≤ 4k per tiling environment
  set; total texture memory ≤ 1 GB.
  *Test:* asset-pipeline verify step + content-lint size checks.

## D. Lighting & framebuffer (machine-checked where possible)

- **VQ-D1** — Scene renders HDR (`Rgba16Float`), tonemapped (ACES/AgX) before UI
  composite.
  *Test:* Phase 2 — `smirk/engine-renderer/tests/offscreen.rs` tonemap monotonicity.
- **VQ-D2** — Image-based lighting from the zone HDRI (diffuse irradiance +
  specular prefilter + BRDF LUT); the same HDRI is the visible sky.
  *Test:* Phase 2 — offscreen white-furnace sanity + sky-pixel checks.
- **VQ-D3** — Directional sun with real shadows (PCF); every grounded entity casts
  and receives.
  *Test:* Phase 3 — offscreen shadow-band assertion.
- **VQ-D4** — MSAA 4× on the scene pass; documented fallback to 1× when the
  adapter lacks support.
  *Test:* Phase 2 — offscreen diagonal-edge intermediate-pixel check.
- **VQ-D5** — Day/night flows through sun + IBL exposure via the existing
  `DayNightSystem` seam, never per-material hacks.
  *Check:* code review rule; no per-material time-of-day uniforms.

## E. VFX & feel (machine-checked count, eyeball quality)

- **VQ-E1** — Every ability has three VFX beats: cast (hand socket), travel
  (trail/beam), impact (scaled by outcome).
  *Test:* Phase 7 — content-lint checks `classes/*.ron` abilities against the VFX
  registry.
- **VQ-E2** — Every death has an effect; every hit has flinch + impact particles.
  *Test:* Phase 7 content-lint (players); enemies deferred.
- **VQ-E3** — Particles are textured (atlas), soft (depth-fade), support additive
  and alpha blending.
  *Test:* Phase 7 — atlas-cell lint + `ParticleSim` unit tests; softness eyeball.
- **VQ-E4** — Telegraphs legible: clear contrast against the ground, ≥ 0.4 s lead
  time.
  *Test:* lead time machine-checked in ability defs; contrast eyeball.

## F. Performance budgets (machine-checked via benchmarks)

- **VQ-F1** — 60 fps @ 1080p in a stress scene (40 skinned characters, 2k
  particles) on the dev GPU.
  *Check:* manual stress-scene run at phase boundaries; benchmark baselines in
  `benchmarks/`.
- **VQ-F2** — ≤ 256 skinned instances (engine cap) until raised deliberately;
  ≤ 4096 live particles.
  *Test:* existing engine caps + Phase 8 warnings at 80%.
- **VQ-F3** — A frame never allocates unbounded per-entity GPU resources.
  *Check:* code review rule; Phase 8 dev-overlay instrumentation.

## G. Verification policy

- **VQ-G1** — Every renderer feature lands with a headless test: a pure-CPU unit
  test (pattern: `anim.rs`, `load_gltf_data`) or an offscreen-render readback with
  **analytic** assertions (darker-than, coverage %, monotonic — never exact pixel
  values). Offscreen tests skip cleanly when no GPU adapter exists and use RGBA8
  assets (fallback adapters lack BC7). GUI checks are manual-only, listed in the
  feel-checklist appendix.
  *Harness:* `engine_renderer::offscreen` + `smirk/engine-renderer/tests/offscreen.rs`.

---

## Future work (out of scope, tracked)

TAA (deliberately deferred in favor of geometric specular AA), GPU particles,
mesh-geometry LOD, KTX2/Basis transcoding, enemy/NPC creature pipeline,
order-independent transparency (sorted per-primitive blending shipped;
intersecting transparents and particle-vs-glass ordering remain approximate).

## Appendix — manual feel-checklist (sandbox)

Run at phase boundaries; the user eyeballs, never automated:

1. **Theme read** (VQ-A1/A5): does the start zone read religious dark fantasy at
   a glance — neutral grey overcast, sun-bleached/soot-darkened stone,
   candle-gold accents?
2. **Material read** (VQ-A2): walk close to ground/props — surfaces show normal
   detail and roughness variation, no flat plastic.
3. **Cohesion** (VQ-A3): no asset looks like it came from a different game.
4. **Color language** (VQ-A4): player VFX reads votive cool blue-white;
   telegraphs read threat red-orange→crimson instantly.
5. **Grounding** (VQ-D3): characters/props sit in the world (contact shadows), no
   floating.
6. **Glow payoff** (VQ-C3/Phase 4): emissives bloom softly at dusk, no clipping halos.
7. **Feel** (VQ-E*): cast→travel→impact beats land; hits flinch; deaths pop.
8. **Performance** (VQ-F1): fps overlay ≥ 60 in the stress scene.
9. **Horizon fog** (VQ-A5): in a fogged zone, look toward the horizon — sky and
   ground converge smoothly; horizon seam gone.
10. **Accent light** (VQ-A1/A5): stand a character beside a portal at dusk —
   the gate's candle-gold light spills onto the ground, both pillars, and the
   character, fading smoothly with distance (no hard edge at the light
   radius); the gate itself still blooms.
11. **Height fog** (VQ-A1/A5): at dusk, low ground/hollows read hazier than
   rooftops and ridgelines; density 0 zones look exactly as before.
12. **Contact shadows** (VQ-D3): stand a character on flat ground — the shadow
   at the feet is crisp, not the soft blur the old single cascade gave;
   distant props still cast.
13. **Grounding AO** (VQ-A2): props and characters darken into corners and floor
   contact instead of floating on flat ambient.
14. **Specular calm** (VQ-A2): pan the camera across a shiny normal-mapped
   surface — highlights stay stable, no crawling sparkle.
