# Code Hygiene Audit — 2026-07-15

Second run. The 2026-07-14 run's entire queue (16 findings, 4 reworks) landed between
the two runs; see "Resolved since last report". This run swept the whole workspace
fresh: every crate's module tree read first, contents predicted from names, every
surprise recorded. The dominant theme: the provenance purges cleaned exactly the file
lists their findings evidenced, and the tags survived everywhere else — test binaries,
bench targets, WGSL shaders, the `game/` crates, and (heaviest of all) the new
`testing/test-support` crate, which inherited the old harness comments verbatim when
the code moved. 71 citation/roadmap-tag occurrences remain across 31 `.rs` files, plus
the WGSL population.

## Ideal end state

Every comment in the workspace states a constraint or a why the code cannot show —
zero provenance tags, zero pointers to files that no longer exist, zero claims the
tree contradicts. Every file's name predicts its contents on first guess, every module
has one responsibility, and nothing compiled into a shipping binary exists only for
tests. The repo root and content tree contain only what a newcomer would expect to
find there, named by what it is.

## Findings (implementation order)

Cross-type queue (mirrored in `reworks-hygiene-2026-07-15.md`):

> **finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 → finding 8 → finding 9 → finding 10 → finding 11 → finding 12 →
> finding 13 → finding 14 → finding 15 → finding 16 → finding 17 → finding 18 →
> ~~rework 1~~ → ~~rework 2~~ → finding 19.**
>
> Finding 19 was added by the same-day third-pass sweep after the queue above
> landed; it closes the residue that pass found.
>
> Findings 1–5 (comment work) go first: they edit only comments, and doing them before
> the placement findings and reworks means moved code carries clean comments (clean
> once). Finding 2 before finding 10: both edit `bot.rs`; purge the comments before the
> constructor dedup reshapes the file. Findings 3 and 6 before finding 14: they fix
> `gltf_import.rs` comments before the file splits. Finding 4 before rework 2: the one
> `receive.rs` comment fix lands before the file is restructured. Finding 5 before
> finding 11: `class.rs`'s backward-compat comment dies with the re-export it annotates.
> Everything from finding 6 on is ordered by impact; the reworks close because nothing
> depends on them.

### 1. Stale claims and dead pointers: comments the tree contradicts

- **Evidence:**
  - `smirk/engine-physics/src/resolve.rs:1` — header says "reads CollisionStarted
    events each frame" but `run()` (L13-16) reads nothing and does nothing; L4 admits
    it's a stub. The first line is false against the body.
  - `smirk/engine-renderer/src/menu.rs:333-334` — "handled in
    `apply_pending_menu_actions`" — no such function; the real one is
    `apply_menu_actions` (`menu_actions.rs:11`).
  - `client/vordar-client/src/vfx.rs:28` — "see react.rs" — the file is now
    `hit_react.rs`.
  - `client/vordar-client/src/locomotion.rs:6` — "(see net.rs)" — `net.rs` is now the
    `net/` module family.
  - `client/vordar-client/src/presentation.rs:1-3` — module doc claims it owns "the
    minimap data feed"; the feed lives in `ui/minimap.rs`, this module only exposes
    `HudHidden` and `ZoneDressingSystem`.
  - `game/vordar-game/src/enemies/behavior.rs:6-8` — doc says archetypes that outgrow
    data "register their own EnemyBehavior (see chapter-01's enemies/ modules — the
    archetype files that own custom behaviors)"; no chapter-01 enemies module contains
    an `EnemyBehavior` impl (all four are doc-only stubs).
  - `smirk/engine-renderer/src/instance_sync.rs:14-25` — module doc says "keeps the
    SDF InstancePool in sync," but `SaveTransformSystem` in the same file never touches
    `InstancePool` (it snapshots `PreviousTransform` for interpolation); also the
    `RenderSlotDespawnSystem` doc at L93-95 states "must run before DespawnFlushSystem"
    twice.
  - `client/vordar-client/src/telegraph.rs:1-5, 17-22, 49-52` — the same claim
    (client-local, never replicated, fill pure in synced time) restated verbatim three
    times; and both `telegraph.rs:1` and `client/vordar-client/src/net/lifecycle.rs:1`
    open with `///` module headers that silently doc-attach to the following `use`
    item, where every sibling uses `//`.
- **Ideal:** every pointer resolves, every claim matches the code, every constraint is
  stated once at the place a reader needs it.
- **Gap:** eight comments actively lie or misdirect; the telegraph invariant costs
  three reads to learn it's one fact.
- **Suggestion:** fix each in place: correct the pointers, rewrite `resolve.rs`'s
  header to say what the stub is, scope `presentation.rs`'s and `instance_sync.rs`'s
  docs to what the files hold, rewrite `behavior.rs`'s doc to describe the actual
  extension mechanism (a chapter crate registers an `EnemyBehavior` into
  `BehaviorRegistry` — no crate currently does), keep telegraph's claim in the module
  header only, and convert the two `///` headers to `//`.
- **Path:** (1) edit the nine sites; (2) green gate: `cargo check --workspace` zero
  warnings, `cargo nextest run --workspace` green — comments only.

### 2. Purge provenance comments: testing/test-support

- **Evidence:** the heaviest cluster in the sweep; `bot.rs` reads like a changelog.
  Field and method docs citing reworks/findings/audits: `bot.rs:33-37, 50-51, 57-59,
  65-68, 75-76, 96-97, 99-101, 102-106, 123-133, 207-209` (the last also "before this,
  only downstream loss could be simulated"); inline citations inside `pump`/`send_move`
  at `:276-279, 288, 293-296, 305-309, 384-386`; bare `(v8)`/`(v12)` tags at `:92-95,
  284, 323, 340-341`. `server.rs:26-27` cites "networking rework plan 2026-07-12,
  finding 4".
- **Ideal:** the harness docs state what each field/preset means and the invariant it
  carries (e.g. `move_ring` holds the last-3 redundancy batch the server dedupes by
  seq) — no report citations, no wire-version archaeology.
- **Gap:** every future protocol change forces a reader through four reports' worth of
  history to find the rule; the tags already point at pre-reorg report paths.
- **Suggestion:** rewrite each to its constraint core exactly as the earlier purges
  did; where a `(vN)` tag pins a real wire invariant, keep the invariant and drop the
  tag. Comments only — finding 10 reshapes the constructors afterwards.
- **Path:** (1) bot.rs sweep; (2) server.rs one-liner; (3) green gate:
  `cargo nextest run --workspace` green, comments only.

### 3. Purge provenance comments: smirk test binaries, benches, WGSL, renderer stragglers

- **Evidence:**
  - engine-net tests: `tests/handshake.rs:1-6`, `tests/flood_control.rs:1-11`,
    `tests/wan_profiles.rs:1`, `tests/impairment.rs:1-15, 31-33, 72-75`,
    `tests/crowd_snapshot.rs:1-7` — every header is "Regression test for the
    networking audit 2026-07-11, finding N" plus used-to-be/formerly/old narration;
    `src/server.rs:749` — "before the fix the leaked endpoint still owned the port".
  - engine-renderer tests: `tests/offscreen.rs:1, 123, 155, 279, 309, 311, 345, 373,
    411, 449, 451, 523` — pervasive `VQ-*` and "Phase 1/2/3/4" section/test tags;
    `tests/egui_probe.rs:1-10` — past-bug narration (keep the constraint: egui must
    consume its events before the game reads input; drop the story).
  - engine-renderer src stragglers the prior purge's file list missed:
    `src/state.rs:58, 63, 73, 107, 144, 167, 187`, `src/frame.rs:161, 266, 340, 418,
    425`, `src/facade.rs:118, 135, 174`, `src/texture.rs:30, 38, 97`,
    `src/offscreen.rs:1, 408`, `src/mesh/store.rs:32`, `src/mesh/sync.rs:172`,
    `src/mesh/gltf_import.rs:575`, `src/ibl.rs:217-218` ("degrades to roughly the old
    flat ambient").
  - WGSL sources (same policy — they are renderer source): `shader.wgsl:24,52,150,158`,
    `mesh_shader.wgsl:2,26,68,213`, `skinned_mesh_shader.wgsl:27,67,220`,
    `bloom.wgsl:1`, `ibl.wgsl:1`, `mipgen.wgsl:3`, `particle_shader.wgsl:1`,
    `sky.wgsl:1`, `shadow.wgsl:1`, `tonemap.wgsl:1`.
  - client test binary: `client/vordar-client/tests/ground_render.rs:1` — "Phase 6
    offscreen verification (VQ-G1)".
  - benches: `benchmarks/benches/render_cpu.rs:1-9` (Phase 8/VQ-F1 + roadmap talk),
    `benches/protocol.rs:11-12, 23, 44-45`, `benches/client_netcode.rs:70-71`
    (protocol-vN + rework/finding citations).
  - marginal reframes in the same pass: `smirk/engine-app/src/flush.rs:56`
    ("Regression:" framing — keep the invariant, drop the label),
    `smirk/engine-physics/src/cell_update.rs:3-6` (justifies placement via test
    history — restate as the staleness bound itself).
- **Ideal:** test headers state the behavior pinned and why it matters; bench headers
  state what cost is measured and the budget it guards; shader comments state the
  pass's contract. The policy names `VQ-*` and `Phase N` tags as forbidden provenance —
  where a tag anchors a real spec clause, the constraint text stays and the tag goes
  (the same rewrite rule the earlier purges used).
- **Gap:** ~50 tags across 16 renderer files plus five engine-net test headers that
  are pure audit archaeology.
- **Suggestion:** same constraint-core rewrite, batched per file. Rust and WGSL in one
  pass.
- **Path:** (1) engine-net tests + server.rs test comment; (2) renderer src + tests +
  WGSL; (3) ground_render.rs + three benches; (4) flush.rs/cell_update.rs reframes;
  (5) green gate: `cargo nextest run --workspace` green (offscreen tests recompile the
  shaders), `cargo bench -p vordar-benches --bench protocol -- --test` compiles the
  touched benches; comments only.

### 4. Purge provenance comments: server crate remainder

- **Evidence:** `src/main.rs:54` ("(finding 3)"), `:83-84` ("(rework 10)"),
  `:115-116` ("used to be silently swallowed (networking audit 2026-07-11, finding
  18)") — main.rs was in no prior finding's evidence list. `src/net/receive.rs:471-473`
  ("real death/respawn design lands with later phases"). Tests: `tests/e2e.rs:75, 147,
  206, 284, 309` (Phase-N prefixes), `tests/e2e_persistence.rs:8, 48, 100, 137`,
  `tests/e2e_combat.rs:9, 100` ("Phase 7.5 (Ravager rework)"), `tests/zones.rs:1, 24,
  38, 81, 191-192` (Phase tags, "The roadmap's Verify line", a "(lessons.md)" citation
  to a gitignored local file, a full plan-path citation plus "cooldowns now persist"),
  `tests/soak.rs:1, 118, 222, 265` (Phase tags + dated audit citations),
  `tests/loss.rs:1, 10` ("gap C — the WEAKPOINTS #4 evidence" — external-doc
  provenance; the p50/p99 rationale itself is legitimate and stays).
- **Ideal:** scenario comments open with the scenario ("Disconnect saves the character;
  reconnecting restores…"), not the roadmap phase that shipped it.
- **Gap:** the Phase-N prefixes are the exact naming-convention problem finding 14
  (2026-07-14) removed from test *names*, still present in test *comments*.
- **Suggestion:** strip the prefixes and citations, keep every scenario description
  verbatim after the colon; rewrite the three main.rs comments to their constraint
  core (the ShutdownFlag contract, why zone threads rerun under supervision, why panics
  must be joined).
- **Path:** (1) main.rs + receive.rs; (2) the six test files; (3) green gate:
  `cargo nextest run -p vordar-server` green, comments only.

### 5. Purge provenance comments: game crates and client remainder

- **Evidence:** game/vordar-game (never swept): `src/enemies/mod.rs:21, 61, 184`
  (Phase 7.5 ×2, "for now"), `src/combat/projectile.rs:1`, `src/combat/mechanic.rs:9`
  ("roadmap P11"), `src/events.rs:22` ("later (roadmap)"), `src/world/zones.rs:1, 29`
  (Phase 7, VQ-A5), `src/world/camp.rs:2`, `src/world/chapter.rs:97`,
  `src/world/wave_spawner.rs:1` ("replaces the old hard-coded
  SetupSystem/EnemySpawnerSystem" — names a design that no longer exists in the tree),
  `src/player/race.rs:15, 122, 150` (Phase-C ×3), `src/player/class.rs:15, 218, 232`
  ("re-export for backward compatibility", Phase-C, Phase D), `src/plugin.rs:41`
  (VQ-E1), `src/vfx.rs:5-10, 16, 31, 59, 162` (VQ tags, "legacy tinted spark burst",
  "Pre-Phase-7 prefabs"). game/vordar-game/tests/content_lint.rs:163, 194 (Phase 6,
  "until Phase 5 makes them…"). Client stragglers: `src/net/mod.rs:1` ("Phase 2
  model:" as a design label), `src/ui/minimap.rs:3` ("Moved out of the engine
  (Phase 8)") and `:41` ("(networking audit 2026-07-11, finding 7)").
- **Ideal:** as findings 2–4; the game crate's comments are otherwise the best in the
  workspace (determinism whys, DESIGN.md § references — which stay).
- **Gap:** ~30 tags in the crate every gameplay feature will be built on.
- **Suggestion:** constraint-core rewrite, batched per file.
- **Path:** (1) vordar-game src + content_lint; (2) the three client files; (3) green
  gate: `cargo nextest run --workspace` green, comments only.

### 6. TEMP scaffolding past expiry: mesh probe and glTF diagnostics

- **Evidence:** `client/vordar-client/src/bin/sandbox.rs:32-33` — "Static-mesh probe
  (Phase A)… Remove once real props land." Real props landed
  (`content/zones/zones.ron` scatters rocks and dead trees), so the guard expired; the
  probe spawn and `content/prefabs/mesh_probe.ron` are dev scaffolding shipped in
  content. `smirk/engine-renderer/src/mesh/gltf_import.rs:623-627, 655-658` —
  "DIAGNOSTIC (the 'half under the field' report)… prime suspect" blocks narrating a
  closed bug hunt.
- **Ideal:** no self-flagged-for-removal code survives its stated condition; import
  code asserts its invariants instead of narrating old investigations.
- **Gap:** one expired probe (bin + prefab), two diagnostic comment blocks.
- **Suggestion:** delete the sandbox probe spawn and `mesh_probe.ron`; rewrite the two
  gltf_import blocks to the invariant they guard (root-motion offset must come from the
  skeleton root, not the mesh origin) or delete if the assertion already states it.
  Sandbox-only visual probe — no gameplay behavior.
- **Path:** (1) delete + rewrite; (2) green gate: `cargo check --workspace` zero
  warnings, `cargo nextest run --workspace` green (gltf_import tests cover the import
  paths).

### 7. Root strays: `example` and `req.md`

- **Evidence:** `example` — a tracked, extensionless 630-byte orchestrator-prompt
  scratch file at the workspace root (last touched by the docs-reorg commit `14d1835`,
  which updated a path inside it). `req.md` — a tracked, empty file at root (from
  commit `442867c` "minor changes").
- **Ideal:** the root contains the workspace manifest, config, and top-level
  directories — nothing else.
- **Gap:** two tracked scratch leftovers.
- **Suggestion:** delete both (git history preserves them). If `example` is still a
  useful prompt reference it belongs in local `.claude/` notes, which are deliberately
  untracked.
- **Path:** (1) `git rm example req.md`; (2) green gate: nothing references either
  (grep confirmed only the audit reports mention them).

### 8. content/ naming follow-ups: the statue named like a fixture, loose source tiles

- **Evidence:** `content/models/vroid_test01.glb` — shipped live content (the
  start-zone statue, `content/zones/zones.ron:25`) named `_test01`, a stem carried
  straight from its pipeline source (`source/characters/vroid/test01_mixamo_upload.fbx`).
  The name violates the tree's own rule that `models/` holds shipped assets and
  test fixtures live in `source/test/`. `content/source/` root holds loose
  `floor_tile.png`/`floor_tile2.png` beside organized subfolders.
  `content/source/characters/vroid/clips/` is an empty directory.
- **Ideal:** every shipped asset is named by what it is in the world; source raws are
  filed by kind.
- **Gap:** a live prop reads as a throwaway; two raws and an empty dir clutter
  `source/`.
- **Suggestion:** rename the statue by role (e.g. `statue_vroid.glb` or the art
  team's name for it) updating `zones.ron:25` and `content/source/CREDITS.md:15`;
  file the two pngs under `source/` (e.g. `source/textures/`); remove the empty dir.
  Whether the look-test statue should remain in the start zone at all is an art call —
  audit-content-pipeline territory, out of scope here.
- **Path:** (1) rename + reference updates (grep the stem first); (2) tidy source/;
  (3) green gate: `cargo nextest run --workspace` green — content_lint and the zone
  tests prove the tree still loads.

### 9. Dead field: `Time.server_offset_micros` is write-only

- **Evidence:** `smirk/engine-app/src/time.rs:13` defines the field;
  `client/vordar-client/src/net/lifecycle.rs:145` writes it every tick; a workspace
  grep finds zero reads — every consumer calls `NetClient::server_offset_micros()`
  directly. It is also a networking concept bolted onto the foundational timing
  resource of a crate with no networking dependency.
- **Ideal:** `Time` carries frame timing; server-clock offset lives with the net
  client that owns it (`NetClientState` already exposes `server_now_micros`).
- **Gap:** a dead field that also misplaces a concern — the worst kind of API noise,
  because a reader assumes a written field is read.
- **Suggestion:** delete the field, its `Default` init, and the mirroring write in
  `lifecycle.rs:143-145`.
- **Path:** (1) delete; (2) green gate: `cargo check --workspace` zero warnings,
  `cargo nextest run --workspace` green — nothing can regress, nothing read it.

### 10. Bot constructor ladder: one struct literal, one seam

- **Evidence:** `testing/test-support/src/bot.rs:164-189` and `:222-247` — the full
  24-field `Bot { … }` literal is duplicated verbatim in `try_connect_as` and
  `connect_full_as`; the 8-constructor ladder (`connect`, `connect_with_latency`,
  `connect_as`, `try_connect_as`, `connect_with_latency_as`, `connect_impaired_as`,
  `connect_upstream_impaired_as`, `connect_full_as`) has no shared construction seam,
  so every new field must be added in two places.
- **Ideal:** one private `fn new(conn: …) -> Bot` (or a `Default` for the state
  fields) that both entry points call; the ladder keeps its explicit preset names.
- **Gap:** a guaranteed future drift point in the crate every integration test
  depends on.
- **Suggestion:** extract the literal into one private constructor; keep all public
  signatures unchanged.
- **Path:** (1) extract; (2) green gate: `cargo nextest run --workspace` green — every
  e2e test constructs Bots through the ladder.

### 11. Post-split re-export retirement and visibility tightening

- **Evidence:** `client/vordar-client/src/locomotion.rs:92` re-exports
  `crate::net::NetMotion`, and `net/mod.rs:33` already re-exports it from
  `interpolate.rs:44` — three modules appear to own one type.
  `game/vordar-game/src/player/class.rs:16` re-exports `race::{RaceId, RaceLibrary,
  RaceModel}` "for backward compatibility" — the callers are few and enumerable.
  `server/vordar-server/src/net/mod.rs:37-38` — `SnapshotBroadcastSystem` and
  `MechanicResolveSystem` are `pub` but only referenced inside `install()` and the
  internal bench seam; nothing outside the crate constructs them.
- **Ideal:** one canonical path per type (`crate::net::NetMotion`,
  `player::race::RaceId`); visibility as narrow as the callers.
- **Gap:** transition-era re-exports that outlived the transition; two systems wider
  than any caller.
- **Suggestion:** migrate the few callers, delete both re-exports, drop the two
  systems to `pub(crate)`.
- **Path:** (1) grep callers, migrate, delete; (2) tighten visibility; (3) green gate:
  `cargo check --workspace` zero warnings, `cargo nextest run --workspace` green.

### 12. Test naming: two Systems without the suffix, one name that names the file

- **Evidence:** `server/vordar-server/tests/soak.rs:31` `struct PhaseMeter` and
  `tests/watchdog.rs:23` `struct PanicOnce` implement `System` without the `…System`
  suffix every other system carries (the test-side exemplar is `KillPlayersSystem`,
  `e2e.rs:261`). `tests/e2e.rs:25` `fn end_to_end` names the binary, not the behavior
  pinned.
- **Ideal:** conventions hold in tests exactly as in src; test names state the
  behavior.
- **Gap:** three deviations, all test-side.
- **Suggestion:** rename to suffix-carrying names (e.g. `TickPhaseMeterSystem`,
  `PanicOnceSystem`) and `end_to_end` to its behavior (it pins login → move →
  replicate round-trip). Renames must sweep `docs/benchmarks/BASELINE.md` and the
  `.config/nextest.toml` exclusive filter list per its header (none of these three
  appear there today — verify at implementation time).
- **Path:** (1) rename + reference sweep; (2) green gate:
  `cargo nextest run --workspace` green at unchanged count.

### 13. chapter.rs: registry machinery and content data model share a file

- **Evidence:** `game/vordar-game/src/world/chapter.rs` (212 lines) holds the
  linked-chapter registry (`ChapterModule`, `ChapterRegistry`, `find`/`deps_of`/
  `install`/`install_all_content`, L17-88) and the chapter content model
  (`ChapterDef`/`SpawnConfig`/`WaveDef`/`InitialSpawn`/`CampDef`/`ActiveChapter`/
  `load_chapter`/`camp_slot_pos`, L90-173) — crate-linking machinery and spawn-content
  schema sharing a file only because both say "chapter".
- **Ideal:** `world/chapter.rs` (or `chapter/mod.rs`) holds the content model;
  the registry lives in its own file named for what it does (e.g.
  `world/chapter_registry.rs`).
- **Gap:** a newcomer looking for "how chapters link into the binary" and one looking
  for "what a wave is" both land in the same 212 lines.
- **Suggestion:** mechanical split with re-exports preserved so no caller changes.
- **Path:** (1) extract; (2) green gate: `cargo check --workspace` zero warnings,
  `cargo nextest run --workspace` green, diff is a move.

### 14. gltf_import.rs: animation import is a second responsibility

- **Evidence:** `smirk/engine-renderer/src/mesh/gltf_import.rs` (730 lines, ~457
  non-test) holds static-geometry import (`visit_node`, `read_material`, `to_rgba8`)
  and skeletal-animation import (`extract_skeleton`, `extract_clips`,
  `keyframe_values` — ~165 lines). The runtime side already splits exactly there
  (`anim.rs` vs the mesh pipeline).
- **Ideal:** `mesh/gltf_import.rs` imports geometry/materials; a sibling (e.g.
  `mesh/anim_import.rs`) imports skeletons/clips, mirroring the runtime split.
- **Gap:** the file under-predicts half its contents and is the largest non-test
  source in the renderer.
- **Suggestion:** mechanical move of the three functions and their helpers/tests, with
  re-exports keeping `mesh::gltf_import::load_gltf`-style call sites unchanged.
- **Path:** (1) extract after findings 3/6 clean its comments; (2) green gate:
  `cargo test -p engine-renderer` green (the gltf_import tests move with their code),
  `cargo nextest run --workspace` green.

### 15. Engine small placements, dead weight, and one duplicated algorithm

- **Evidence:**
  - `smirk/engine-physics/src/lib.rs:28-41` — `PhysicsStatsSystem` (dev-overlay
    counter publisher) lives in the plugin-registration file.
  - `smirk/engine-renderer/src/ibl.rs:252-254` — `cube_view_of` is a one-line wrapper
    around `cube_view`; two names for one op (`cube_view` at 138/201/409,
    `cube_view_of` at 191-192).
  - `smirk/engine-renderer/src/menu.rs:338` and `menu_actions.rs:93` — two independent
    `std::process::exit(0)` routes for Quit (keyboard vs click).
  - `smirk/engine-app/src/app.rs:214-250`, `app_loop.rs:28-45`, and
    `smirk/engine-renderer/src/menu_actions.rs:56-65` — three copies of the
    pick-closest-video-mode fitting logic.
- **Ideal:** stats publisher in its own file; one name per op; one Quit path (keyboard
  emits `MenuAction::Quit`, `apply_menu_actions` owns the exit); one video-mode-fit
  function owned by engine-app (which owns the window), called by all three sites.
- **Gap:** four small frictions, each a future drift point.
- **Suggestion:** four bounded edits. Also noted, deliberately left: `XorShift32` in
  `client/vordar-client/src/vfx.rs:67-87` is the third unrelated PRNG in the workspace
  (with test-support's `Lcg` and `ground.rs`'s value-noise hash) — consolidating would
  couple production code to test-support or invent a util crate; the duplication is
  cheaper than the coupling. Flagged so the tradeoff is on record; user decides.
- **Path:** (1) move `PhysicsStatsSystem` to `stats.rs`; (2) delete `cube_view_of`,
  update its two callers; (3) route menu.rs's Quit through `MenuAction::Quit`; (4)
  extract the video-mode fit into engine-app (e.g. `config.rs` or `window.rs`), call
  it from the three sites; (5) green gate: `cargo check --workspace` zero warnings,
  `cargo nextest run --workspace` green (offscreen + egui probe cover menu paths).

### 16. offscreen.rs: a test-only renderer compiled into shipping builds

- **Evidence:** `smirk/engine-renderer/src/offscreen.rs` (474 lines —
  `OffscreenRenderer`, `HeadlessGpu`, `SceneTarget`) exists to serve integration tests
  but is a plain `pub` module (`lib.rs:16`), so it compiles into production binaries.
  The workspace already has the precedent for this exact problem:
  `vordar-server`'s bench seam is feature-gated (`bench-internals`).
- **Ideal:** test-only surface exists only in test builds: `#[cfg(feature =
  "offscreen")]` on the module, the feature enabled by the integration tests and any
  consumer that genuinely needs headless rendering.
- **Gap:** ~474 lines and a second render path in every shipping binary.
- **Suggestion:** mirror the bench-internals pattern. Check consumers first: if the
  client's ground_render test or content tools use it, they enable the feature in
  their dev-dependencies.
- **Path:** (1) grep consumers of `offscreen::`; (2) gate the module + wire features;
  (3) green gate: `cargo nextest run --workspace` green,
  `cargo build --release -p vordar-client -p vordar-server` then confirm the symbols
  are absent (the feature is off by default).

### 17. Renderer long-function seams: frame.rs passes, state.rs init

- **Evidence:** `smirk/engine-renderer/src/frame.rs` (538) — one `RenderSystem::run`
  spans dirty-range upload → egui frame → shadow pass → main pass → sky → particles →
  bloom/tonemap → egui pass → present. `src/state.rs` (384) — `RendererState` is a
  60+-field struct with one monolithic `init` whose per-subsystem seams are already
  comment-sectioned (meshes / skinned / particles / HDR+post / shadows / IBL /
  textures / egui).
- **Ideal:** `run` reads as a frame graph — one named private method per pass; `init`
  delegates to per-subsystem constructors. Same file, no module changes.
- **Gap:** the passes exist as comment sections instead of functions, so the structure
  is invisible to navigation (outline, go-to-symbol) and every edit scrolls.
- **Suggestion:** extract-method only, no behavior or ordering change; the offscreen
  suite pins output.
- **Path:** (1) frame.rs pass methods; (2) state.rs init helpers; (3) green gate:
  `cargo test -p engine-renderer` (31 unit + 13 offscreen) green,
  `cargo nextest run --workspace` green.

### 18. test-support crate shape: util grab-bag, glob re-exports, single-child dir

- **Evidence:** `testing/test-support/src/util.rs` holds four unrelated
  responsibilities under an uninformative name (`workspace_root` cwd mutation,
  `percentile` stats, `join_with_deadline` threading, `Lcg` PRNG).
  `src/lib.rs:8-10` glob-re-exports all three modules flat, hiding which module owns a
  symbol. `testing/` contains exactly one crate.
- **Ideal:** either the flat harness namespace is the documented convention (one line
  in lib.rs saying so) or symbols are imported by module; `util` is honest but
  predicts nothing.
- **Gap:** minor — the crate is three files and greppable; the cost is a newcomer's
  first minute, not drift.
- **Suggestion:** options, user decides: (a) leave as-is and document the flat
  namespace as the harness convention in lib.rs's header (cheapest; the crate is
  small); (b) split util.rs into named files (`fs.rs`/`stats.rs`/`rng.rs`) keeping the
  flat re-exports; (c) also flatten `testing/test-support/` to `test-support/` at the
  workspace root (loses the category dir that future testing crates would share).
  Wins: navigability matches the rest of the workspace. Losses: churn in a crate whose
  whole point is to be boring; (c) touches workspace members and every path reference.
- **Path:** (1) user picks a/b/c; (2) if b/c: mechanical move, green gate
  `cargo nextest run --workspace` green at unchanged count.

### 19. Third-pass residue: manifest citations, two "before the fix" narrations, five phase-tagged client headers

Added by the same-day third-pass sweep (run after findings 1–18 and both reworks
landed). The sweep also settled a policy boundary: `docs/visual-quality.md` exists as
a living spec, so `VQ-*` references that anchor a stated constraint (content_lint's
budget asserts, "HDR emissive (VQ-C3)", `ground_render.rs:39`'s assert message) are
spec-clause references — the allowed class `DESIGN.md §N` belongs to — and are NOT
findings; the forbidden use is a tag riding no constraint or explaining when code was
written. That ruling is now written into the comment policy (project CLAUDE.md §5).

- **Evidence:**
  - Manifest comments (no purge finding ever evidenced Cargo.tomls):
    `client/vordar-client/Cargo.toml:44` "(hygiene finding 16)" and `:47` "net.rs's
    reconnect test (networking audit 2026-07-11, finding 7)" (also a stale `net.rs`
    pointer); `smirk/engine-renderer/Cargo.toml:7` "(hygiene finding 16)";
    `server/vordar-server/Cargo.toml:38` "(hygiene rework 4, finding 1)".
  - "Before the fix" narration: `server/vordar-server/tests/watchdog.rs:86` ("before
    the fix there is no supervisor…"), `server/vordar-server/tests/shutdown.rs:54`
    ("before the fix, run_headless(_, None) never returns…").
  - Phase tags in never-swept client headers: `src/ground.rs:1` "(Phase 6)" plus
    "replacing the SDF slab" change-log; `src/credentials.rs:1` "(networking rework 1,
    finding 3)"; `src/body.rs:4,7` "Phase-B runtime" / "the pre-Phase-C path";
    `src/net/mod.rs:52` "reproduces the Phase 1 server-driven feel";
    `src/presentation.rs:1` "(Phase 7.5)"; `src/bin/vordar.rs:3` "(Phase 2)".
- **Ideal:** as findings 2–5 — constraint stays, citation/phase/history framing goes
  (e.g. watchdog: "without a supervisor the zone thread stays dead and every retry
  times out"; body.rs: "the skinned-mesh runtime animates it" / "the SDF path").
- **Gap:** 14 sites across 10 files, all in surfaces (manifests, two test bodies,
  client file headers) outside every earlier finding's evidence list.
- **Suggestion:** constraint-core rewrite, one pass.
- **Path:** (1) edit the 14 sites; (2) green gate: `cargo check --workspace` zero
  warnings, `cargo nextest run --workspace` green — comments only.

### 20. Fourth-pass residue: two bench headers still cite WEAKPOINTS gaps

- **Evidence:** `benchmarks/benches/prefab_spawn.rs:1` ("Prefab spawn cost —
  WEAKPOINTS gap A") and `benchmarks/benches/client_netcode.rs:1` ("Client netcode
  hot paths — WEAKPOINTS gap B") — the same external-doc provenance class finding 4
  stripped from `loss.rs`; finding 3's evidence cited these files only for their
  protocol-vN lines, so the headers survived.
- **Ideal:** the headers state what cost is measured and the budget guarded, minus
  the gap tags.
- **Gap:** two sites.
- **Suggestion:** strip the tags, keep the measured-cost rationale.
- **Path:** (1) edit both headers; (2) green gate: `cargo check --workspace` zero
  warnings — comments only.

Scope handoff recorded by the fourth pass: the remaining Phase tags live in
`content/` file contents (`zones/zones.ron:1`, `races/{human,dwarf,elf,valkyrie}.ron`
pre-Phase-C fallback notes, `chapters/chapter01/chapter.ron:1`,
`source/CREDITS.md:23`, `source/characters/mixamo/SHOPPING_LIST.md`) — content file
contents are audit-content-pipeline territory per this audit's scope boundary
(hygiene owns the content tree's layout and naming only), so they are recorded here
as a handoff, not findings. SHOPPING_LIST.md additionally describes a genuinely
pending manual step (the blocked Mixamo downloads) and must not be blindly purged.

## Carried forward from previous report

None — the full 2026-07-14 queue was implemented and verified before this run.

## Resolved since last report

All sixteen 2026-07-14 findings and all four reworks, re-verified this run by fresh
greps and the sweep itself:

1. **Comment policy defined** — lives in the project CLAUDE.md §5 (local, per config
   policy); this report's findings 2–5 apply it to the areas the first purge round
   didn't evidence.
2. **e2e.rs split into concern-named binaries** — `e1228a2`; the five binaries carry
   honest headers (their remaining Phase-prefix comments are finding 4 here).
3. **Server test helpers consolidated** — `4bf38fd`, then superseded structurally by
   rework 4 (test-support crate).
4. **Engine provenance purge** — `6607ec7` (its evidenced file list is clean; the
   straggler population outside that list is finding 3 here).
5. **Server/protocol/test-header purge** — `e847130` (same caveat: finding 4 here).
6. **Client purge** — `69162cc` (stragglers are in findings 1 and 5 here).
7. **TEMP scaffolding removed** — `14500b8` (body.rs log, mesh sync pose logs).
8. **race.rs and mechanic.rs extracted** — `933bc25`.
9. **Zone supervisor into supervisor.rs** — `64b4887`.
10. **Renderer placements/headers/sdf_pipeline rename** — `b4028f9`.
11. **Config persistence into config.rs** — `b5ce7b5`.
12. **Client placements (HudSync/Sandbox/NetMotion/hit_react)** — `71cf2b0` (the
    transition re-exports it added are retired by finding 11 here).
13. **Dead code removal** — `6c511f7`.
14. **Naming convergence (WorldTime, test prefixes)** — `8bcbff0` (comment-side Phase
    prefixes are finding 4 here).
15. **content/ normalization** — `960f00e` (follow-ups are finding 8 here).
16. **smirk/ strays** — `01e3991`.
- **Rework 1 (client net.rs decomposition)**, **rework 2 (server net_plugin.rs
  decomposition)**, **rework 3 (renderer lib.rs/mesh decomposition)**, **rework 4
  (testing/test-support crate)** — all landed across their plan-step commits
  (`plan-hygiene-rework-{1,2,3,4}-2026-07-14.md`), each verified at the time and
  re-confirmed by this sweep's clean-area findings.
