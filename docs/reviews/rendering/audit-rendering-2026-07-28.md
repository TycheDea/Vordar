# Rendering Audit — 2026-07-28

Extraction pass, not a fresh sweep: source material is an external expert
review (grok, 2026-07-27 — real-time rendering & visual quality, formerly
`docs/reviews/grok/03-*.md`, deleted after this extraction; see git history).
Every finding was re-verified against the current tree — including the
2026-07-25 world-space detail-layer rollout the review predates — checked
against what `docs/visual-quality.md` actually requires, and cross-checked
with the 2026-07-16 audit's landed queue. Rework-scale survivors are mirrored
in `reworks-rendering-2026-07-28.md`.

What the review confirmed holds up, verified: the PBR core is real
(Cook–Torrance with specular AA, MASK via alpha-to-coverage, BC5 z-reconstruct);
IBL + sky sample one environment with async decode; shadow craft (texel-snapped
concentric cascades, slope bias, PCF, outer fade); the post stack matches the
bar (HDR MSAA → resolve → Kawase → ACES → sRGB, egui after tonemap);
presentation is gameplay-literate (telegraph fill is a pure function of synced
server time); the art pipeline ships BC7/BC5 sidecars plus the triplanar
detail layer; verification culture is real (analytic offscreen suite,
adapter-locked FLIP goldens, content lint — including the VQ-E1 beats lint the
review claimed doesn't exist, its one outright factual error); and the dev
overlay meters the right knobs. One finding refuted: VQ-D1 reads "ACES/AgX" —
either curve satisfies it; no AgX obligation exists.

## Ideal end state

Lighting whose authority is the authored zone mood with day/night flowing
inside it; combat VFX whose three beats are data the lint enforces; a frame
graph that never pays for geometry twice without a measured reason; scaling
paths (skinning, uploads, streaming) budgeted by benches before enemy meshes
arrive; and profiling that never perturbs the frame it measures.

## Findings (implementation order)

Cross-type queue (reworks in `reworks-rendering-2026-07-28.md`):

> **finding 1 → rework 1 → finding 2 (after rework 1: the exposure driver
> implements its authority decision) → finding 3 → finding 4 → finding 5 →
> finding 6 → finding 7 → finding 8 → rework 2 (after findings 5 + 8:
> non-perturbing timers and the skinning bench supply its measurements) →
> rework 3 → rework 4 (trigger-gated: enemy-mesh enablement scheduled AND
> finding 8's bench shows a budget miss).**

### 1. Scheduled/Leap impact VFX are hardcoded client code, outside the data model and the beats lint

- **Evidence:** `game/vordar-game/src/vfx.rs:61-65` — `VfxDef` authors only
  `cast`; all `content/vfx/*.ron` files define cast only. The telegraph
  resolve burst is hardcoded in
  `client/vordar-client/src/telegraph.rs:66-98`. The existing lint
  `ability_vfx_beats_exist` (`game/vordar-game/tests/content_lint.rs:168-219`)
  already enforces cast for every ability and impact via `VfxTrail` for every
  `Projectile` ability — Scheduled/Leap impact is the uncovered residue.
  Separately: martial cast colors (cleave `(2.2, 0.8, 0.35)`, rend,
  onslaught) sit in VQ-A4's threat band (350°–25°) while player VFX is
  specified votive cool (190°–230°).
- **Ideal:** every beat of every ability is data the lint enforces; the
  telegraph resolve visual is authored per ability, not one hardcoded burst.
- **Gap:** the review's "no lint ties abilities to beats" was wrong — the
  gap is narrower: Scheduled/Leap impact lives in code, and the color
  language drifts. The VQ-A4 ranges are explicitly a proposal the user tunes
  at the B1 phase gate, so the warm-martial vs votive-cool call is a **user
  ruling to collect**, not a defect to auto-fix.
- **Suggestion:** extend `VfxDef` with an `impact` field; have the telegraph
  resolve path read it (keep the pure time→fill function); extend
  `ability_vfx_beats_exist` to require impact for Scheduled/Leap. Present
  the color question to the user alongside.
- **Path:** (1) `VfxDef.impact` + RON updates; (2) telegraph consumption;
  (3) lint extension, falsified once; (4) offscreen/content suites green;
  (5) user question on martial cast hue filed.

### 2. Nothing drives exposure — dusk and noon tonemap identically

- **Evidence:** `smirk/engine-renderer/src/facade.rs:140-146` `set_exposure`
  (feeds tonemap + bloom prefilter) — repo-wide, callers are offscreen tests
  only; `DayNightSystem` calls only `set_light`
  (`client/vordar-client/src/world_time.rs:53`). The bloom half of the
  review's complaint is stale: the prefilter is already display-referred
  (07-16 finding 9 — `tonemap.wgsl:52-53`, `bloom.rs:197`), so "what glows"
  tracks exposure correctly once something drives it.
- **Ideal:** target exposure derives from the HDRI's measured luminance
  (baked at manifest time — a manifest already ships,
  `client/vordar-client/src/presentation.rs:27`) and lerps with day/night
  inside whatever envelope rework 1 decides; manual override stays for art.
- **Gap:** driver and calibration are missing, not the plumbing.
- **Suggestion:** bake average luminance into the HDRI `.manifest.json`;
  read it in `set_environment`; drive exposure from `DayNightSystem`
  alongside ambient, per rework 1's authority model.
- **Path:** (1) manifest field + bake step; (2) driver; (3) offscreen
  "dusk vs noon mid-grey band" check in the existing analytic family;
  (4) goldens re-judged (visual change is the point — user eyeballs per
  VQ process).

### 3. Mesh streaming integrates one upload per frame in arrival order

- **Evidence:** `smirk/engine-renderer/src/mesh/store.rs:362`
  `MESH_UPLOADS_PER_FRAME = 1`; `store.rs:306-337` drains the decode channel
  in mpsc arrival order up to that budget; sole call site
  `mesh/sync.rs:315`. Decode is already off-thread; the stagger is GPU
  integration only. Current start-zone content (~8–10 assets) appears over
  ~10 frames. Correction to the review: collision is unaffected — nothing
  derives collision from render meshes; this is purely visual pop-in.
- **Ideal:** streaming budget is bytes (or ms) per frame, and the ground
  arrives first, so a zone fades in floor-up in a frame or two instead of
  prop-by-prop in arrival order.
- **Gap:** latency/presentation only — no frame-budget issue.
- **Suggestion:** byte budget per frame instead of count = 1; request the
  ground key before props on zone apply (request order is already the only
  priority mechanism — exploit it before building tier machinery).
- **Path:** (1) budget change; (2) request-order tweak in zone apply;
  (3) unit test on the integrate budget; offscreen suite green.

### 4. Contact shadows depend on an unguarded camera-targets-player invariant

- **Evidence:** `smirk/engine-renderer/src/shadow.rs:27`
  `CASCADE_HALF_EXTENTS = [24.0, 60.0, 160.0]`; `shadow.rs:85-111` `fit_vp`
  fits all three cascades concentrically around `camera.target`. The 24 m
  near cascade covers the player only because the one gameplay camera
  targets the player — nothing asserts it, and a future cinematic/spectate/
  AOE camera breaks feet-contact shadows silently.
- **Ideal:** the invariant is executable: an offscreen test proves a
  character offset from the look-at still receives near-cascade contact
  darkening, so the first camera mode that violates it fails a test instead
  of shipping washed feet.
- **Gap:** guard only — frustum-split CSM is deferred until camera modes
  actually diversify (see Not extracted).
- **Suggestion:** land the offscreen test; note the constraint in the
  shadow.rs header if absent.
- **Path:** (1) offscreen test (analytic tier); (2) falsify once by fitting
  around a displaced target; done.

### 5. GPU timing blocks the frame that emitted it; the particle pass silently owns the MSAA resolve

- **Evidence:** `smirk/engine-renderer/src/frame.rs:40`
  `GPU_TIMING_INTERVAL = 30`; `frame.rs:323-326` `timer.read_blocking` after
  submit while the F3 overlay is open; `gpu_timer.rs:119-131` `map_async` +
  `PollType::Wait`. Dev-only by design, but the hitch skews exactly the
  numbers being read and rework 2 depends on them. Adjacent micro-contract:
  the particle pass carries the MSAA resolve (`frame.rs:754-776`,
  `resolve_target` + `StoreOp::Discard`, and an empty particle pass still
  resolves) — skipping that pass would black-frame the game; nothing asserts
  or states it at the call site.
- **Ideal:** timestamps double-buffer — frame N reads N−1's results with no
  wait — and the resolve ownership is stated where someone would delete the
  pass.
- **Gap:** measurement fidelity for every GPU decision downstream.
- **Suggestion:** double-buffer the query staging; read the previous
  sample's results without polling; add the resolve-ownership comment (or
  debug assert) at the particle-pass call site.
- **Path:** (1) gpu_timer double-buffer; (2) comment/assert; (3) overlay
  numbers sanity-checked against a before capture; offscreen suite green.

### 6. Mesh/skinned/particle uploads rewrite from offset 0 every frame with no metering

- **Evidence:** `smirk/engine-renderer/src/frame.rs:335-384`
  `upload_gpu_buffers` — SDF uses `dirty_ranges` (`frame.rs:58-81`);
  mesh/skinned/particles rewrite the live set from 0 whenever non-empty.
  Corrections to the review: writes are sized to live counts, not buffer
  capacity, and the SDF dirty-range pattern does not transplant — the
  mesh/skinned lists are rebuilt by `pack_visible` every frame
  (`mesh/sync.rs:298-304`, `:398-399`), so "unchanged range" rarely exists.
- **Ideal:** upload traffic is visible on the dev overlay, and the common
  static case (camps, props, camera-only movement) skips writes whose bytes
  are identical to last frame's; persistent instance pools remain a rework
  decision gated on finding 8's measurements.
- **Gap:** bandwidth scales with live counts every frame, invisible today.
- **Suggestion:** add an upload-KB line to the dev overlay next to
  `skinned`/`particles`; skip a buffer's write when the packed slice equals
  the previous frame's bytes.
- **Path:** (1) metering; (2) identity skip + unit test; (3) verify the
  static-scene skip actually fires in zone review; offscreen suite green.

### 7. VQ-D4 promises an MSAA fallback that doesn't exist

- **Evidence:** `smirk/engine-renderer/src/post.rs:9-11` — `SCENE_SAMPLES =
  4`, comment calling the constant "the documented fallback seam"; every
  scene pipeline bakes it; no runtime downgrade path. VQ-D4 requires "MSAA
  4× with a documented fallback to 1× when unsupported."
- **Ideal:** the quality doc and the code agree. On the locked desktop
  target the WebGPU format-pair guarantee makes the 1× path dead code — so
  the honest reconciliation is amending the clause, not building an
  untestable fallback (VQ-G1's test-with-it discipline).
- **Gap:** doc/code divergence only.
- **Suggestion:** amend VQ-D4: "4× required; the desktop target guarantees
  it — 1× fallback deferred with portability." Revisit alongside the BC
  requirement if a non-desktop target ever enters scope.
- **Path:** (1) one clause edit in `docs/visual-quality.md`; done.

### 8. No bench gates skinned pose + upload cost before enemy meshes multiply it

- **Evidence:** every visible rig is posed on CPU per display frame
  (`smirk/engine-renderer/src/mesh/sync.rs:51-95`; far-LOD half-rate posing
  still re-uploads its cached palette, `sync.rs:129-142`) and the used joint
  prefix re-uploads each frame (`frame.rs:359-370`). Corrections to the
  review: posing is zero-alloc steady-state (07-16 finding 10, pinned by
  test), culled rigs never pose or upload, and only the used prefix is
  written — the structural cost claim stands, the churn framing was stale.
  A CPU pose bench exists (`benchmarks/benches/render_cpu.rs:58-66`,
  `joint_palette_40x64`); nothing covers upload or gates a budget.
- **Ideal:** a stress bench measures pose + upload at 40 and 128 rigs
  against a fixed ms budget and fails the gate when exceeded — the go/no-go
  number rework 4 (GPU skinning / dirty-skip / job pool) is gated on.
- **Gap:** VQ-F1 names 40 skinned @ 60 fps as the bar now; without the
  number, the skinning rework would be speculation.
- **Suggestion:** extend the render_cpu bench family with the upload-side
  measurement + a budget assertion wired like the existing bench gate.
  Criterion runs are heavy compute — respect the quiet-box caveat in
  `docs/benchmarks/BASELINE.md` and the bench-gate cadence rules.
- **Path:** (1) bench + budget; (2) baseline on a quiet box (user-side
  condition); (3) gate wired per the established bench-gate pattern.

## Not extracted

- 03-F4 (BC hard requirement, no portable fallback) — deferred: desktop-only
  is the locked target and CI runs the BC-free offscreen harness by policy.
  Verified for the record: the RGBA8 path is live for every non-DDS texture
  (`mesh/store.rs:67-73`), so the eventual fix is small — feature-detect BC,
  skip `.dds` sidecars when absent. Re-file with any non-desktop target.
- 03-F9 (transparent path kills instancing; OIT approximate) — deferred by
  the review's own recommendation; the accepted 07-16 rework-3 design, and
  current transparent content is near-nil. Re-file when alpha-heavy assets
  (foliage, glass) ship.
- 03-F11 (skinned characters skip the detail overlay; placeholder weapons) —
  deferred to the in-progress character-asset replacement: the new
  pipeline's texel density may not want the limestone tile at all, and the
  skinned pipeline sits at the 4-bind-group default cap
  (`shadow.rs:5-7`), so adding the detail group is a design decision
  recorded in rework 4's notes. Placeholder gear is explicitly permitted by
  VQ-A2; grey-box weapons must simply not appear in judged footage.
- 03-F14 (AgX not implemented) — **refuted**: VQ-D1 "ACES/AgX" is satisfied
  by the implemented, tested ACES; no selector promised.
- 03-F16 (soft particles sample MSAA sample 0 only) — acknowledged in-shader
  as deliberate (`particle_shader.wgsl:98-101`); fold a two-sample average
  into the next particle-shader touch, not a standalone task.
- 03-F17/F18 (goldens discipline, pass-ordering contract) — preserve-grade,
  verified accurate; F18's actionable micro (resolve-ownership assert) is
  absorbed into finding 5.
