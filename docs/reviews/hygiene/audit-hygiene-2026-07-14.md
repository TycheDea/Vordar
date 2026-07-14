# Code Hygiene Audit — 2026-07-14

First run of this audit: no prior `docs/reviews/hygiene/` reports exist, so there is no
carried-forward section. Sweep covered every workspace crate (`smirk/*`, `game/*`,
`server/*`, `client/*`, `benchmarks/`), all test binaries, and the `content/` tree
(layout/naming only). Companion reworks file: `reworks-hygiene-2026-07-14.md`.

## Ideal end state

Every file's name predicts its contents on the first guess; no file is so large its
structure hides (the three 1,500+-line units are decomposed along their real seams).
Comments state only constraints and non-derivable whys — provenance (finding numbers,
rework citations, "used to be") lives in git history and `docs/reviews/`, not in source.
Test helpers exist once, dead scaffolding is gone, and one naming convention governs
systems, resources, tests, files, and content folders.

## Findings (implementation order)

> **Cross-type queue** (spans this file and `reworks-hygiene-2026-07-14.md`; mirrored
> there verbatim):
> **finding 1 → ~~rework 1~~ → rework 2 → rework 3 → finding 2 → finding 3 → rework 4 →
> finding 4 → finding 5 → finding 6 → finding 7 → finding 8 → finding 9 → finding 10 →
> finding 11 → finding 12 → finding 13 → finding 14 → finding 15 → finding 16.**
> Finding 1 first: every comment cleanup (4-7) applies the policy it defines, and every
> worker diff from then on writes under it. Reworks 1-3 before findings 4-6 and 10/12:
> those findings' diffs land inside the files the reworks restructure — splitting first
> means cleaning once. Finding 2 before finding 3: the split decides which binaries the
> consolidated helpers serve. Finding 3 before rework 4: consolidation gives the shared
> test crate one source of truth to lift instead of scattered copies. The rest are
> independent; ordered by impact.

### 1. Comment policy: constraint-or-why only, and it lands in CLAUDE.md

- **Evidence:** the workspace has a strong implicit style — module headers stating intent
  plus scheduling constraints, inline whys (`client/vordar-client/src/weapons.rs:204-206`
  cm→m scale; `smirk/engine-renderer/src/post.rs:9-11` WebGPU 4× MSAA guarantee;
  `server/vordar-server/src/db.rs:218-221` Drop-order invariant;
  `game/vordar-game/src/motion/movement.rs:9-15` PLAY_RADIUS rationale) — but no written
  policy, and ~200 provenance/change-log comments violating that style have accreted
  (findings 4-6 count them per crate).
- **Ideal:** a written comment discipline in the project `CLAUDE.md` that every human and
  agent diff follows: a comment exists only to state (a) a constraint the code cannot
  show, (b) a why that is not derivable from the code, or (c) a module header giving
  intent and scheduling/ownership contracts. Forbidden: narration of the next line,
  PR/change-log talk (finding/rework/audit citations, "used to be", "now we", roadmap
  tags like `VQ-*`/`Phase N` used as provenance), restated signatures, stale claims, and
  TEMP scaffolding that outlives its purpose. Provenance belongs to git history and
  `docs/reviews/`.
- **Gap:** the style exists only by imitation; nothing stops the provenance-tag habit
  that produced ~200 violations, and workers are explicitly told today to cite findings
  in comments by example.
- **Suggestion:** add a short "Comment policy" section to
  `C:\Users\egm_8\IdeaProjects\vordar\.claude\CLAUDE.md` codifying the allowed classes
  and the forbidden classes above, naming one exemplar file per class (e.g.
  `engine-app/src/scheduler.rs` headers, `db.rs` invariants).
- **Path:** (1) write the policy section into CLAUDE.md; (2) verification: the section
  exists, names the allowed/forbidden classes, and findings 4-6 can each cite it as
  their rule; the compile/test gate is untouched (docs-only).

### 2. Split e2e.rs into concern-named test binaries (docs-only structure, mechanical)

- **Evidence:** `server/vordar-server/tests/e2e.rs` is 1,332 lines and its header
  (L1-12) still advertises only "phase1/2/3/6". Contents group cleanly: connectivity/
  movement (L28-158), NPC/replication/AOI (L160-178, 376-481), combat/mechanics
  (L180-374, 483-611), persistence (L613-868), security/metrics (L870-1064), wire format
  (L1066-1332). The multi-zone concern was already split out into `zones.rs` — precedent
  exists.
- **Ideal:** one test binary per concern — `e2e_combat.rs`, `e2e_persistence.rs`,
  `e2e_security.rs`, `e2e_wireformat.rs`, with the connectivity/AOI core keeping
  `e2e.rs` — so a failure's binary names its subsystem and test edits stop colliding in
  one file.
- **Gap:** five concerns share one file; the header lies about what's inside.
- **Suggestion:** mechanical move of whole `#[test]` fns along the section seams above;
  each new binary gets `mod common;` and a header stating its concern. No assertion
  changes.
- **Path:** (1) create the four new files, move tests verbatim, fix imports; (2) update
  the `e2e.rs` header to what remains; (3) green gate: `cargo nextest run -p
  vordar-server` shows the same total test count (35 db + 24 moved/remaining e2e + rest)
  all passing, zero behavior change.

### 3. Consolidate duplicated server test helpers into tests/common

- **Evidence:** `RejectMirror` (`tests/e2e.rs:874-883`) and `BusyMirror`
  (`tests/soak.rs:60-69`) are the same metric-mirror pattern — e2e's comment even says
  "same smuggling trick as the soak harness's BusyMirror". Multi-zone bring-up is
  hand-rolled three times (`zones.rs:156-174`, `shutdown.rs:110-125`,
  `watchdog.rs:53-82`) despite `spawn_zone_server` existing (`zones.rs:18-37`). The raw
  login-probe loop is duplicated at `e2e.rs:960-979` and `e2e.rs:1024-1046`; the
  join-with-deadline mpsc pattern at `shutdown.rs:61-67,168-176` and
  `watchdog.rs:126-134`; percentile fns `p99` (`soak.rs:71-74`) and `pct`
  (`loss.rs:29-31`).
- **Ideal:** each pattern exists once in `tests/common/mod.rs`: a generic
  `MetricMirror`, a parameterized zone bring-up (flag/supervisor injectable), a
  login-probe helper, a join-with-deadline helper, one percentile fn.
- **Gap:** five patterns × 2-3 copies each; a change to any (e.g. the zone bring-up
  when the supervisor changes) must be found in three places.
- **Suggestion:** lift into common with the smallest general signature that serves all
  call sites; delete the copies.
- **Path:** (1) add the five helpers to `tests/common/mod.rs`; (2) replace each inline
  copy; (3) green gate: full `cargo nextest run -p vordar-server` passes with identical
  test counts; the diff deletes more lines than it adds.

### 4. Purge provenance comments: engine crates

- **Evidence:** 123 finding/date/rework citations across 26 `smirk/` files. Heaviest:
  `engine-net/src/server.rs` ~18 (e.g. L49 "…(networking audit 2026-07-11, finding 14
  step 2)", L483 "A version mismatch used to be a silent close…finding 16"),
  `engine-net/src/client.rs` ~20 (e.g. L513 "(finding 17 — previously a strictly
  monotonic FIFO delay…)"), plus `common.rs:26`, `metrics.rs:19-31`,
  `impair.rs:7-8,118,165,178,193`, `lib.rs:14`. engine-renderer carries `VQ-*`/`Phase N`
  provenance tags (`lib.rs:99,104,114`, `post.rs:1-3`, `camera.rs:39-40,173`,
  `mipgen.rs:6`, `bloom.rs:1`, `ibl.rs:1`, `shadow.rs:1`, `tangent.rs:1`,
  `particle_pipeline.rs:1`, `mesh.rs:48,61`). Also `engine-physics/src/lib.rs:53-56`
  ("Phase 3 review wrinkle, closed in Phase 7.5") and `engine-core/src/prefab.rs:177-178`
  ("networking rework 5, finding 4…").
- **Ideal:** the mechanism explanations stay (many are exemplary); the citation tags and
  "used to be" history go. A reader learns the constraint, not the changelog.
- **Gap:** provenance is grafted onto real constraints, so every future reader parses
  history to find the rule; tags go stale the moment reports move (they already point at
  pre-reorg paths in some files).
- **Suggestion:** per file, rewrite each tagged comment to its constraint core; delete
  pure-history lines. Batch per crate in one diff. Policy: finding 1.
- **Path:** (1) engine-net (server, client, common, metrics, impair, lib); (2)
  engine-renderer tag sweep; (3) engine-physics + engine-core single instances; (4)
  green gate: `cargo check --workspace` zero warnings, `cargo nextest run --workspace`
  green — comments only, zero behavior change.

### 5. Purge provenance comments: server, protocol, and test headers

- **Evidence:** `server/vordar-server/src/net_plugin.rs` ~35+ citations (L72-74,
  L388-391 quoting a full report path, L493-494 "this used to kick on bare name match…",
  L795 inline doc path, L1192-1193 "…the dead NetMetrics facade claimed but never
  provided"). `db.rs` test doc-comments ~10 ("Regression test for finding 13…").
  `lib.rs:80-90` change-log ("`main` used to discard…Since rework 10…").
  `game/vordar-protocol/src/lib.rs` ~20 field/variant tags ("(protocol vN, networking
  rework N finding M)" at L76-77, L88, L96, L124-126, L156, L182, L187). Test-file
  headers are change-logs: `e2e.rs:121-126,274-280,795-801,1066-1074,1102-1111`,
  `loss.rs:113-135` (a 23-line prose changelog saying "see git history of this
  comment"), `watchdog.rs:1-8`, `shutdown.rs:1-7`.
- **Ideal:** protocol fields say what the field means and its invariant; tests' doc
  comments state the behavior they pin (the why), not which finding spawned them.
- **Gap:** same as finding 4 — history where constraints should be.
- **Suggestion:** rewrite to constraint core; for tests, keep the scenario rationale
  ("duplicates are deduped by design, so a reject needs a future-stamped seq"), drop the
  finding numbering. Policy: finding 1. Do after finding 2 so headers are rewritten in
  their final files.
- **Path:** (1) net_plugin.rs, db.rs, lib.rs; (2) vordar-protocol; (3) the five test
  binaries' headers and per-test docs; (4) green gate: `cargo nextest run --workspace`
  green, comments only.

### 6. Purge provenance comments: client

- **Evidence:** `client/vordar-client/src/net.rs` ~15 citation tags (L66-68, L174, L219,
  L228, L244, L282-283, L464, L469, L475, L480, L518-519) plus L1243 narrating the
  deleted `NetLerpSystem`; two test doc-comments are review memos — L1568-1587 (20 lines
  explaining an assertion window shrink, citing reworks finding 11) and L2126-2132
  (quoting a plan path).
- **Ideal:** as findings 4-5. The L1568-1587 memo becomes two sentences: the window
  asserted and why the resync point bounds it.
- **Gap:** same class; net.rs is also the file every future networking diff reads.
- **Suggestion:** rewrite after rework 1 lands (the comments move with their code into
  the new modules; clean once, in place). Policy: finding 1.
- **Path:** (1) sweep the post-split net modules; (2) green gate:
  `cargo nextest run -p vordar-client` green, comments only.

### 7. Remove TEMP debug scaffolding

- **Evidence:** `client/vordar-client/src/body.rs:66-68` — "TEMP (anim feel-check)…
  Remove with the MeshRenderSyncSystem pose log" plus its `log::info!`;
  `smirk/engine-renderer/src/mesh.rs:776` `log_accum` field, L796-803 and L869-874 the
  accompanying log blocks ("Remove once the character animates on screen" — the
  character animates on screen); `smirk/engine-renderer/src/lib.rs:747-752` a multi-line
  TODO describing a future refactor (subsumed by rework 3).
- **Ideal:** no self-flagged-for-removal code survives its purpose.
- **Gap:** three pieces of debug scaffolding shipped past their expiry, one TODO that
  duplicates a filed rework.
- **Suggestion:** delete all four; log-only output, no gameplay behavior. The TODO's
  content is covered by rework 3 in the reworks file.
- **Path:** (1) delete; (2) green gate: `cargo check --workspace` zero warnings
  (`log_accum` removal must not orphan a field read), `cargo nextest run --workspace`
  green.

### 8. vordar-game placement: race system and Mechanic get their own files

- **Evidence:** `game/vordar-game/src/player/class.rs` (384 lines) holds the entire race
  system — `RaceId`, `RaceDef`, `RaceModel`, `RaceLibrary`, `PoseParams` — under a file
  named "class". `game/vordar-game/src/combat/mod.rs:23-32` defines the `Mechanic` type
  inline while every sibling combat component (`ContactDamage`, `Projectile`,
  `LeapImpulse`, `BuffStack`, `CombatStats`) has its own file.
- **Ideal:** `player/race.rs` and `combat/mechanic.rs`; `class.rs` contains classes.
- **Gap:** two names that don't predict their contents; the race system is invisible to
  a newcomer scanning filenames.
- **Suggestion:** mechanical move with `pub use` re-exports preserved so no caller
  changes; module docs move with the code.
- **Path:** (1) extract `race.rs`, (2) extract `mechanic.rs`, (3) green gate:
  `cargo check --workspace` zero warnings, `cargo nextest run --workspace` green, diff
  is moves + mod decls only.

### 9. server lib.rs: zone supervisor into its own module

- **Evidence:** `server/vordar-server/src/lib.rs:80-169` carries a complete second
  responsibility beside app assembly: `supervise_zone`, `join_zone_threads`,
  `next_strikes`, `panic_message`, `MAX_ZONE_RESTARTS`, `HEALTHY_UPTIME`.
- **Ideal:** `src/supervisor.rs` owning restart policy; `lib.rs` assembles apps.
- **Gap:** crate root mixes assembly and supervision; the restart policy is findable
  only by reading lib.rs top to bottom.
- **Suggestion:** mechanical move, `pub use` for the two entry points `main.rs` uses.
- **Path:** (1) move; (2) green gate: `cargo nextest run -p vordar-server` green
  (watchdog/shutdown binaries exercise the moved code), diff is a move.

### 10. engine-renderer small placements, missing module headers, pipeline.rs name

- **Evidence:** `post.rs` holds the sky pipeline (`create_sky_pipeline` L249,
  `create_sky_bind_group_layout` L367 — scene rendering, not post) and `GpuTimer`
  (L297-364, dev instrumentation). Five modules open with no purpose header where every
  other module has one: `texture.rs`, `pipeline.rs`, `mesh_pipeline.rs`, `camera.rs`,
  `instance.rs`. `pipeline.rs` holds the SDF-primitive pipeline while its siblings are
  name-qualified (`mesh_pipeline.rs`, `skinned_pipeline.rs`, `particle_pipeline.rs`).
- **Ideal:** sky in `sky.rs`, `GpuTimer` in a timing/dev module, headers on all modules,
  `sdf_pipeline.rs` matching the sibling convention.
- **Gap:** post.rs under-predicts its contents; the unqualified `pipeline.rs` is the odd
  one out; headerless modules break the crate's own documentation convention.
- **Suggestion:** three mechanical moves/renames plus five constraint-style headers.
  Coordinate with rework 3 (this finding's targets are outside lib.rs/mesh.rs, so it can
  land before or after it — after keeps line refs stable).
- **Path:** (1) moves + rename with re-exports; (2) headers; (3) green gate:
  `cargo check --workspace` zero warnings, offscreen tests green.

### 11. app_loop.rs: config persistence belongs to config.rs

- **Evidence:** `smirk/engine-app/src/app_loop.rs:85-107` — the `CloseRequested` arm
  serializes `WindowConfig` to RON (with an inline RON header string) and writes it to
  disk; L154-177 runs config hot-reload re-parsing. `config.rs` owns config.
- **Ideal:** app_loop handles window events and delegates persistence/parsing to
  `config.rs` functions.
- **Gap:** serialization format knowledge lives in the event loop; two files must change
  to alter the config format.
- **Suggestion:** extract `save_window_config`/`reload_config` into config.rs; app_loop
  calls them.
- **Path:** (1) extract; (2) green gate: `cargo check --workspace` zero warnings —
  behavior identical (same bytes written on close).

### 12. client placements: presentation.rs grab-bag, NetMotion home, react.rs name

- **Evidence:** `client/vordar-client/src/presentation.rs` (282 lines) holds three
  unrelated concerns: `ZoneDressingSystem` (zone dressing), `HudSyncSystem` (L160-211 —
  the producer of `HudState`, whose consumer lives in `ui/minimap.rs`), and
  `SandboxCastSystem` (offline ability casting — gameplay input, not presentation).
  `NetMotion` is defined in `locomotion.rs:96` but is the net-derived motion component
  written by the interpolation code. `react.rs` contains hit/death reaction presentation
  (`HitReactSystem`, `CorpseOnDeathSystem`, `CorpseTtlSystem`, `spawn_corpse`) — "react"
  reads as the UI framework.
- **Ideal:** `HudSyncSystem` beside its data in `ui/`; `SandboxCastSystem` with the
  sandbox binary's concerns; `NetMotion` defined where net writes it; `react.rs` named
  for what it is (e.g. `hit_react.rs`).
- **Gap:** three names that under- or mis-predict contents.
- **Suggestion:** mechanical moves + one file rename; do after rework 1 (NetMotion's
  ideal home is the post-split interpolation module).
- **Path:** (1) moves/rename with re-exports; (2) green gate: `cargo check --workspace`
  zero warnings, client tests green.

### 13. Dead code removal

- **Evidence:** `client/vordar-client/src/locomotion.rs:154 trigger_attack` and `:185
  trigger_death` — zero callers workspace-wide (verified by grep; the live path is
  `trigger_attack_clip`). `game/chapter-01/src/enemies/{grunt.rs:8, cinder_imp.rs:8,
  mossback.rs:7, sentinel.rs:10}` — four `pub const PREFAB` with zero references
  (verified; grunt.rs:4 claims the const is "load-bearing" — the load-bearing string is
  the RON filename, making the comment a stale claim); all four `register(_registry)`
  hooks are empty bodies called by `enemies/mod.rs::register_behaviors` L16-21 — pure
  scaffolding, zero behavior. Commented-out module decls: `engine-audio/src/lib.rs:17-18`
  (`// pub mod manager;` `// pub mod assets;` under a doc block describing an
  AudioManager that does not exist), `engine-core/src/lib.rs:19` (`// pub mod assets;`).
- **Ideal:** dead code is deleted (git history preserves it); doc blocks describe what
  exists.
- **Gap:** two dead fns, four dead consts, an empty registration chain, and doc/decls
  describing absent code.
- **Suggestion:** delete `trigger_attack`/`trigger_death`; delete the PREFAB consts,
  empty `register` fns, and `register_behaviors` (and its call); make engine-audio's doc
  block one honest line ("empty stub crate; audio not yet built") and drop the
  commented-out decls. Whether engine-audio stays a workspace member is
  audit-rust-tooling's call — this finding only makes the file honest.
- **Path:** (1) delete client fns; (2) delete chapter-01 scaffolding; (3) honest stub
  docs; (4) green gate: `cargo check --workspace` zero warnings (deletions must not
  orphan imports), `cargo nextest run --workspace` green.

### 14. Naming convergence: WorldTimeRes and test-name conventions

- **Evidence:** `game/vordar-game/src/world/mod.rs:24 WorldTimeRes` is the only resource
  in the workspace carrying a `Res` suffix (`NetServerState`, `ClassLibrary`,
  `BehaviorRegistry`, `ActiveChapter`… are all unsuffixed). Test names mix three
  conventions in one suite: roadmap-phase (`phase1_…` through `phase7_5_…`),
  finding-number (`finding18_invalid_intent_increments_reject_counter`, e2e.rs:892), and
  plain descriptive (`relog_restores_exact_cooldown_remainder`) — the phase/finding
  prefixes are provenance in names.
- **Ideal:** `WorldTime` (or the convention-consistent bare noun), and one descriptive
  test-naming convention: the name states the behavior pinned, nothing else.
- **Gap:** one suffix deviation; two provenance conventions that stop meaning anything
  the day the roadmap doc goes stale.
- **Suggestion:** rename `WorldTimeRes`; rename phase/finding-prefixed tests to their
  descriptive core (e.g. `phase7_5_rend_kills_camped_enemy` → `rend_kills_camped_enemy`,
  `finding18_…` → `invalid_intent_increments_reject_counter`). Renames must update every
  reference: `docs/benchmarks/BASELINE.md` run commands and any `-E 'test(…)'` filters.
- **Path:** (1) `WorldTimeRes` rename (mechanical, compiler-checked); (2) test renames +
  BASELINE.md reference sweep (grep for each old name repo-wide); (3) green gate:
  `cargo nextest run --workspace` green with the same test count, BASELINE commands
  copy-paste-run correctly.

### 15. content/ tree naming normalization

- **Evidence:** one concept, three separators: `content/chapters/chapter01/` vs crate
  `game/chapter-01` vs lib name `chapter_01` (`game/chapter-01/Cargo.toml:7`). Plural is
  the folder norm (`chapters, classes, models, prefabs, races, textures, zones, vfx`)
  with unexplained singulars `config`, `source`, `world` — and `world/` (events.ron) vs
  `zones/` (zones.ron) splits two world-description files across a singular and a plural
  folder. `content/models/` mixes shipped character models (`human.glb, dwarf.glb,
  elf.glb, valkyrie.glb`) with test/sample assets (`avocado.glb`, `fox.glb`,
  `vroid_test01.glb`) while `content/source/test/` exists for exactly that
  (`DamagedHelmet.glb`, `MetalRoughSpheres.glb`). Two loose `floor_tile*.dds` sit at
  `content/textures/` root beside structured subdirs (`env/`, `ground/mud_leaves/`).
  Entity-prefab naming is asymmetric: `content/prefabs/player.ron` (human-class body,
  named by role) vs `content/prefabs/ravager.ron` (named by class).
- **Ideal:** one separator rule for chapter ids in content (`chapter01` — matching the
  RON-referenced paths — with the crate/lib names left to cargo convention and the rule
  written down); folder = schema, so stem collisions across folders (`human.ron` in
  classes/ and races/) are fine and the rule says so; test assets live under
  `content/source/test/`; loose textures live in their subdir; entity prefabs named by
  one rule (by class: `player.ron` → `human.ron`).
- **Gap:** three separator styles, unexplained singular/plural, strays in shipping
  folders, two naming rules for the same prefab kind.
- **Suggestion:** move the strays, rename `player.ron` → `human.ron`, merge
  `content/world/` into `content/zones/` (or vice versa — one folder for world
  description), and write the layout rules into `content/`'s guide (or CLAUDE.md).
  Every rename must update its loaders: RON path strings, prefab-name references
  (`sandbox.rs:30` spawns "ravager"; net_plugin.rs `PLAYER_PREFAB="ravager"` — grep each
  stem before moving).
- **Path:** (1) inventory references per moved/renamed file (grep stem); (2) execute
  moves + reference updates; (3) write the rules; (4) green gate: `cargo nextest run
  --workspace` green — `content_lint.rs` and the zone/content e2e tests are the proof
  the tree still loads.

### 16. smirk/ root strays: orphaned integration toolkit and a tracked binary

- **Evidence:** `smirk/integration/` — a 17-file git-tracked PowerShell "Integration
  Toolkit" (scripts/templates/trackers) whose own README references plan documents that
  do not exist in the repo; nothing in the workspace references the directory (grep
  confirmed); its tracker state contains a placeholder dummy task. `smirk/texconv.exe` —
  a 966 KB tracked Windows binary at the engine-workspace root. (`smirk/.idea/` is
  correctly gitignored and untracked — no action.)
- **Ideal:** the engine root contains engine crates; tools that are used live under
  `scripts/` with a reference from docs; binaries are fetched, not tracked.
- **Gap:** an orphaned toolkit and a binary blob sit among the engine crates.
- **Suggestion:** delete `smirk/integration/` (git history preserves it; it is
  unreferenced and stale). Relocate the texconv dependency out of the tree — whether it
  becomes a documented download step or a script-fetched tool is
  audit-project-meta/content-pipeline territory (it belongs to the asset pipeline);
  this finding's scope is that it must not live tracked at `smirk/` root.
- **Path:** (1) delete `smirk/integration/`; (2) move `texconv.exe` handling into the
  asset-pipeline scripts' story (coordinate with `scripts/asset-pipeline/` usage — grep
  for `texconv` first and update the caller); (3) green gate: `cargo nextest run
  --workspace` green; asset-pipeline script still runs (its own check), repo loses ~1 MB
  of tracked non-source.

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run of this audit.
