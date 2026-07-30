# Rendering Audit (Reworks) — 2026-07-28

Rework-scale companion to `audit-rendering-2026-07-28.md`: findings that need
a design pass before anyone writes code. Consumed by /plan-rework. Source
provenance and verification discipline are stated in the audit header.

## Ideal end state

A lighting model where the zone's authored mood is the authority and
day/night, events, and exposure all modulate inside it; a frame graph whose
depth work is paid once, chosen by measurement; telegraphs that read as
grounded threat decals under every light; and a skinned-character path whose
scaling strategy was picked from bench numbers, not fear.

## Findings (implementation order)

Cross-type queue (mirrored verbatim from `audit-rendering-2026-07-28.md`):

> **finding 1 → rework 1 → finding 2 (after rework 1: the exposure driver
> implements its authority decision) → finding 3 → finding 4 → finding 5 →
> finding 6 → finding 7 → finding 8 → rework 2 (after findings 5 + 8:
> non-perturbing timers and the skinning bench supply its measurements) →
> rework 3 → rework 4 (trigger-gated: enemy-mesh enablement scheduled AND
> finding 8's bench shows a budget miss).**

### 1. Lighting authority: zone dusk vs `DayNightSystem` noon (VQ-A5 vs VQ-D5)

- **Evidence:** zone apply stamps the dusk key matched to the baked HDRI sun
  (`client/vordar-client/src/presentation.rs:34-35` `SUN_DIR`/`SUN_COLOR`
  derived from `castilian_plateau_dusk_2k.manifest.json`; `:111`
  `set_light` on apply). Once world time syncs, `DayNightSystem` overwrites
  direction/color/ambient every frame from a full day cycle
  (`client/vordar-client/src/world_time.rs:53`;
  `game/vordar-game/src/world/mod.rs:108-114` — day key `(1.0, 0.95, 0.85)`,
  ambient up to 1.0) against the unchanged dusk cubemap. VQ-A5: bright
  noon-neutral scenes are off-theme. Verified sharper than the source
  review put it: the sandbox is structurally immune (`DayNightSystem` is
  registered only on the networked path and waits for sync), so offline
  review always looks on-theme while networked play always drifts — the
  worst shape for catching it by eye.
- **Update (2026-07-30):** `5933c31` landed per-zone `ZoneVisuals` lighting
  fields (`sun_azimuth_deg`/`sun_elevation_deg`/`sun_color`/`sun_intensity`/
  `ambient`/`exposure`, `game/vordar-game/src/world/zones.rs`) — the
  hardcoded `SUN_DIR`/`SUN_COLOR` constants this evidence cites no longer
  exist in `presentation.rs`; zones now author their own key
  (`content/zones/zones.ron`, G1 gate:
  `docs/reviews/town/g1-lighting-2026-07-30.md`). This is plumbing only, not
  a fix: `DayNightSystem` still overwrites the authored key every frame once
  world time syncs. The per-zone fields are the re-entry point option (b)
  below needs; the core VQ-A5-vs-VQ-D5 fight is unchanged.
- **Ideal:** the zone's authored key is the envelope for *mood*; day/night
  keeps flowing through the `DayNightSystem` seam (VQ-D5 requires the cycle)
  but its output is remapped into the zone envelope — exposure/ambient
  scale, hue clamped to the warm band — or zones author per-time sun tables
  / HDRI swaps. A noon-white key never rides a dusk irradiance.
- **Gap:** IBL + sun coherence is what sells the dark-fantasy register; the
  current fight makes every networked minute drift off the direction the
  art pass locked.
- **Suggestion:** design pass must reconcile VQ-A5 with VQ-D5 and choose:
  (a) dusk-envelope remap of `day_night_light` output (cheapest, keeps one
  HDRI), or (b) per-zone sun tables + HDRI swaps per time band (heavier,
  most correct). Either way add the offscreen assertion: under the default
  start-zone path, the sun color stays inside the VQ-A4 warm band at every
  day fraction (zone_review already reuses `SUN_DIR`/`SUN_COLOR`, so the
  seam exists). Audit finding 2 (exposure driver) consumes this decision.
- **Path:** design doc → user go/stop on (a) vs (b) → fix-sized steps
  (remap or tables, assertion, event-tint interaction re-checked).

### 2. Depth prepass pays full opaque geometry for SSAO alone

- **Evidence:** with SSAO on (default, `state.rs:209`), every opaque
  SDF/mesh/skinned draw records into a single-sample prepass depth
  (`smirk/engine-renderer/src/frame.rs:278-284`, `:485-495`, clear 1.0),
  SSAO runs, then the main MSAA pass clears its own depth and redraws
  everything (`frame.rs:571-577`). Verified constraints the source review
  missed: (1) direct reuse is blocked by the sample-count mismatch — the
  prepass is 1×, main depth is 4× (`post.rs:41`); Early-Z means an *MSAA*
  prepass, not a repoint. (2) `shade_pbr` consumes blurred AO inside the
  main pass, so a same-frame SSAO from main depth is impossible without
  splitting the main pass — the real alternative is previous-frame depth
  (one-frame AO lag). (3) This shape is the 07-16 rework-6 design, so this
  is a measured revisit of a chosen tradeoff, not a regression hunt.
- **Ideal:** depth work paid once: either an MSAA prepass with the main pass
  at `Equal`/`LessEqual` and no clear, or the prepass deleted and SSAO fed
  from previous-frame depth with the lag accepted.
- **Gap:** at the VQ-F1 stress point (40 skinned + environment), geometry
  bandwidth and vertex/skinning cost are paid twice — the largest structural
  GPU cost in the frame graph, growing linearly with content density.
- **Suggestion:** design pass weighing (a) MSAA Early-Z prepass vs
  (b) prev-frame-depth SSAO (deletes the prepass outright — strictly less
  code). Decide from per-pass GPU timer numbers on the stress scene, taken
  after audit finding 5 (non-blocking timers) and finding 8 (skinning
  bench) land.
- **Path:** design doc with both variants costed → user go/stop → fix-sized
  steps with before/after `gpu shadow`/`gpu main` numbers and FLIP goldens
  re-judged.

### 3. Telegraphs are emissive SDF discs, not grounded threat decals (VQ-E4)

- **Evidence:** the telegraph prefab is a scaled SDF cylinder
  (`content/prefabs/telegraph.ron`) with an HDR dim→bright lerp
  (`client/vordar-client/src/telegraph.rs:23-26`, flat scale at `:40`). No
  ground projection, no terrain depth-test (on the hill skirt the disc can
  interpenetrate), no edge ring, no secondary non-hue channel for colorblind
  separation. The fairness model — fill as a pure function of synced server
  time — is exactly right and must survive any visual redesign.
- **Ideal:** a ground-projected decal or ring, depth-tested against terrain,
  dark core + threat rim, with a non-hue secondary channel (pulse/chevron),
  driven by the same pure time→fill function.
- **Gap:** VQ-E4 demands clear ground contrast; emissive red-orange works at
  dusk but washes on bright stone and fails colorblind separation with hue
  alone. No decal system exists in the renderer today — hence rework-scale.
- **Suggestion:** design pass choosing the projection mechanism (mesh decal
  vs soft-particle ring vs stencil volume) against the renderer's existing
  pass structure; requirements above are the acceptance list; add a sandbox
  feel-check screenshot pair (mid-fill / resolve) to the judged set.
- **Path:** design doc → user go/stop → fix-sized steps; keep
  `TelegraphFillSystem` pure.

### 4. Skinned pose/upload scaling strategy (trigger-gated)

- **Evidence:** CPU pose per visible rig per display frame + used-prefix
  joint upload each frame (`mesh/sync.rs:51-95`, `:129-142`;
  `frame.rs:359-370`; caps at `skinned_pipeline.rs:44-45`). Already
  verified sane: zero-alloc steady state, culled rigs fully skipped,
  far-LOD half-rate posing. The open question is only whether this scales
  to enemy-wave counts within VQ-F1's budget.
- **Ideal:** the scaling decision — dirty/no-change palette skip, job-pool
  posing, or GPU/compute skinning — is made from audit finding 8's bench
  numbers at 40/128 rigs, not preemptively.
- **Gap:** none until the trigger fires: **enemy-mesh enablement is
  scheduled AND the bench shows a budget miss.** Do not plan this rework
  before both hold.
- **Suggestion:** when triggered, weigh the three options in cost order
  (dirty-skip → job pool → GPU skinning) and note the constraint recorded
  from the detail-layer verification: the skinned pipeline sits at the
  4-bind-group default cap (`shadow.rs:5-7`), which any new bind group
  (compute skinning outputs, detail textures for characters) must resolve —
  bundle the skinned-detail decision from the audit's Not-extracted list
  into this design pass.
- **Path:** trigger check → design doc → user go/stop → fix-sized steps
  gated by the finding-8 bench.
