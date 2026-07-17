# Rust & Tooling Audit — 2026-07-17

First audit in this domain — no prior reports to carry forward.

## Ideal end state

`cargo clippy --workspace --all-targets` exits 0 and is part of the standing
gate, backed by a single `[workspace.lints]` table every crate inherits. The
dependency tree contains nothing unused: every entry in every manifest is
consumed, shared versions live in `[workspace.dependencies]` exclusively, and
no dependency drags a parallel math/hash/windows-sys stack for a function the
codebase could write in six lines. The toolchain and the handful of
version-pinned deps are current, with pin comments that state a reason that is
still true. Panics on invariant violations name the invariant. The criterion
suite's saved baseline matches the tree it claims to describe.

## Findings (implementation order)

Cross-type queue (no rework-scale findings this audit — the queue is all
fixes; `reworks-rust-tooling-2026-07-17.md` mirrors this note):

> **~~finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 (user-decides — ask at loop launch) → finding 8 → finding 9 →
> finding 10 → finding 11 (docs-only, micro) → finding 12~~.**
>
> Finding 1 goes first because it is the infrastructure every later diff is
> verified against. 2–3 shrink the tree before anything recompiles it
> repeatedly. 7 precedes 9 (edition work should run on the newest toolchain)
> and 12 (a toolchain bump shifts codegen, so re-baselining before it would be
> wasted). 12 is last because findings 2, 3, 7, 9, and 10 each move numbers.
>
> Findings 1–12 done 2026-07-18 (one commit each; finding 7 also fixed a
> direct rustc-1.97.1-caused clippy regression it introduced; finding 10
> measured dev-profile dep opt-level and reverted — see its Measured note
> above; finding 12 also fixed a pre-existing, unrelated bench-registry
> break; loop-final gate 408/408). Filed findings 13 and 14 during finding
> 7's proof pass — not in this queue, tracked in
> `reworks-rust-tooling-2026-07-17.md`.

### 1. No lint configuration exists anywhere, and `cargo clippy --workspace --all-targets` does not even exit 0

- **Evidence:** zero `#![warn(...)]`/`#![deny(...)]` attributes and zero
  `[lints]` tables across all 14 crates (workspace-wide grep, 2026-07-17). A
  default-level clippy run produces **84 unique warning sites** (22 in
  engine-renderer, 14 engine-core, 12 each vordar-client/engine-app, 10
  vordar-server, 8 vordar-game, 5 engine-physics, 1 engine-net) and **exits
  101**: `smirk/engine-renderer/src/particle_pipeline.rs:277` trips
  deny-by-default `clippy::erasing_op` on `0 * cell_px` — intentional
  grid-coordinate arithmetic in a test (cell (2, 0)), but it makes the whole
  command red, so clippy can never be gated as-is. Warning classes observed:
  ~25× `new_without_default`, ~10× `type_complexity`
  (`smirk/engine-core/src/traits.rs:75,84,91`,
  `smirk/engine-app/src/scheduler.rs:100`), 5× `result_large_err` on
  `PrefabError` (carries `ron::error::SpannedError` by value,
  `smirk/engine-core/src/prefab.rs:73-264`), ~9× `manual_is_multiple_of`, 2×
  `items_after_test_module` (`smirk/engine-renderer/src/lib.rs:37`,
  `dev_overlay.rs:36`), plus singletons (`chapter_registry.rs:46`
  inefficient `contains`, `locomotion.rs:93` doc-comment formatting).
- **Ideal:** one `[workspace.lints]` table in the root manifest (rust +
  clippy groups), `lints.workspace = true` in every member, warnings fixed or
  deliberately allowed with a reason, and `cargo clippy --workspace
  --all-targets` green in the standing gate so drift can never re-accumulate.
- **Gap:** 84 sites of accumulated drift, no mechanism to stop the 85th, and
  a deny-level false positive blocking gate adoption.
- **Suggestion:** add `[workspace.lints]` (start from default clippy; do not
  adopt pedantic wholesale — a pedantic pass produced 1,429 hits, almost all
  style noise, and its one "suspicious operator" flag on
  `smirk/engine-net/src/clock.rs:111` is a false positive: `n·Σxx − Σx·Σx` is
  the textbook least-squares denominator). Rewrite the `0 * cell_px`
  expression (or `#[allow(clippy::erasing_op)]` with the constraint stated),
  apply the mechanical fixes (`cargo clippy --fix` covers ~40), introduce
  type aliases for the flagged closure types, and add the clippy run to the
  gate the loop-final check already uses.
- **Path:** (1) root `[workspace.lints]` + 14× `lints.workspace = true`;
  (2) clear the erasing_op site; (3) `cargo clippy --fix` + the manual remainder, crate
  by crate; (4) wire `cargo clippy --workspace --all-targets` into the gate;
  proof: the command exits 0 with zero warnings and a seeded regression (any
  new `unwrap_or(false)`-style drift) is caught locally.

### 2. Four dead dependencies: `slotmap`, `rand`, `gilrs`, and `kira` feeding an empty stub crate

- **Evidence:** `slotmap = "1.1.1"` in root `[workspace.dependencies]`
  (Cargo.toml:28) is referenced by **no member manifest** (workspace-wide
  grep: only Cargo.toml + Cargo.lock match). `rand = "0.10.0"`
  (smirk/engine-renderer/Cargo.toml:26) has **zero** Rust usage — the only
  greps matching "rand" in the crate are a WGSL variable name
  (`ssao.wgsl:103`); it drags `chacha20`, `rand_core 0.10`, `getrandom 0.4`,
  and a second `cpufeatures`. `gilrs = "0.11"` (same manifest, line 22) has
  zero usage anywhere in the crate; it drags `gilrs-core` + windows bindings.
  `smirk/engine-audio/src/lib.rs:1` is literally `// engine-audio — empty
  stub crate; audio not yet built.` — yet its manifest pulls `kira = "0.12"`,
  which compiles the entire symphonia decode stack (8 symphonia crates, cpal,
  bitflags 1.x — visible in `cargo tree -d`) into every workspace build for a
  crate with no code.
- **Ideal:** every manifest entry is consumed; the dep tree carries no
  compile cost for features that don't exist yet.
- **Gap:** four dead entries, three duplicate-version chains
  (`cpufeatures`×2, `getrandom`×3, `rand_core`×2 — the rand-0.10 chain is
  one of each), and the symphonia stack built on every clean build.
- **Suggestion:** delete all four. `kira` returns to engine-audio's manifest
  in the commit that writes engine-audio's first real code; the crates.io
  verification comment goes with it.
- **Path:** (1) remove the four entries (root + engine-renderer +
  engine-audio manifests); (2) `cargo check --workspace --all-targets` +
  full gate; proof: `cargo tree -d` no longer lists rand 0.10's chain or the
  symphonia stack, and the gate stays green.

### 3. `parry3d` exists to evaluate one six-float comparison, and pays a second `glam` for it

- **Evidence:** the entire consumption of `parry3d = "0.26"` is
  `smirk/engine-physics/src/aabb.rs` — 22 lines wrapping
  `ParryAabb::intersects` for center+half-extents boxes; the file itself
  documents the cost: "parry3d 0.26 uses its own glam re-export (0.30.x),
  our workspace uses glam 0.32. Bridge by constructing from raw f32
  components" (aabb.rs:12-13). `cargo tree -d` shows parry3d dragging `glam
  0.30.10` (a full second copy of the workspace's core math crate), `simba`,
  `spade`, `rstar`, `glamx`, `approx`, `num-complex`, `ordered-float`,
  `wide`, `safe_arch`, and a second `heapless`/`hash32` chain.
  `docs/benchmarks/WEAKPOINTS.md`'s 2026-07-04 pass already measured that
  narrowphase cost is world.get fetches, "parry3d removal is a dep-weight
  win only". The crate's three unit tests (aabb.rs:28-48) already pin the
  exact semantics, including the `<=` touching-faces case.
- **Ideal:** the AABB overlap test is six `<=` comparisons on workspace
  `glam` types; engine-physics has no external math dependency.
- **Gap:** a duplicated math ecosystem (~12 transitive crates) for a
  function whose behavior is already pinned by local tests.
- **Suggestion:** hand-roll `overlaps` (`(a.min <= b.max && b.min <= a.max)`
  per axis, `<=` preserved), store `min`/`max` as `glam::Vec3`, delete the
  parry3d dependency and the bridge comments.
- **Path:** (1) rewrite `Aabb` on glam types with the three existing tests
  unchanged (they are the behavior contract — touching faces stay
  overlapping); (2) drop the manifest entry; (3) full gate + a
  `physics_pipeline`/`full_tick` bench sanity run (numbers should be flat —
  WEAKPOINTS already established the cost lives elsewhere); proof: tests
  green, `cargo tree` shows no parry3d/glam-0.30 subtree.

### 4. Dead traits `Spawnable`/`EntityLifecycle`, and engine-core's header documents two traits that never existed

- **Evidence:** `smirk/engine-core/src/traits.rs:57-68` declares `Spawnable`
  and `EntityLifecycle`; a workspace-wide grep finds no implementor and no
  consumer — the only references are the declarations and comments.
  `smirk/engine-core/src/lib.rs:4` and `:18` claim the crate owns "Base
  traits: Spawnable, EntityLifecycle, Collidable, Renderable" — `Collidable`
  and `Renderable` do not exist anywhere in the workspace. The comment at
  traits.rs:41 ("Passed to Spawnable::spawn and EntityLifecycle hooks")
  describes `SpawnContext`'s real consumers wrongly — its real consumers are
  `spawn_prefab` and the queue closures.
- **Ideal:** engine-core's public surface contains only what the engine
  actually uses; its module docs state what is true.
- **Gap:** two dead public traits inviting speculative implementations, and
  a module header asserting four traits of which two are dead and two are
  fiction.
- **Suggestion:** delete both traits; rewrite the lib.rs owns-list and the
  SpawnContext comment to name the real consumers.
- **Path:** (1) delete + fix comments; (2) `cargo check --workspace`
  confirms nothing consumed them; proof: gate green.

### 5. Version-locked and shared deps pinned per-crate instead of in `[workspace.dependencies]`

- **Evidence:** `smallvec = "1"` appears directly in
  smirk/engine-core/Cargo.toml:13 **and** game/vordar-game/Cargo.toml:15;
  smirk/engine-renderer/Cargo.toml:27 declares `hecs = "0.11.0"` directly
  while every other crate uses `hecs = { workspace = true }`; `wgpu = "29"`
  is pinned independently in engine-renderer (line 20) and vordar-client's
  dev-deps (line 44); `egui-wgpu = "0.34"`/`egui-winit = "0.34"`
  (engine-renderer:31-32) are version-locked to the workspace's `egui`
  entry but live outside it, so an egui bump is a three-manifest edit that
  can silently skew. Root comment "Shared versions — all crates pull from
  here" (Cargo.toml:21) states a rule the tree violates. Also:
  game/chapter-01/Cargo.toml disables `doctest` but not `test` despite
  having zero `#[test]`s, while chapter-02 disables both — the stated
  convention ("skip building the empty test binaries") applied to one twin
  and not the other.
- **Ideal:** every dep used by ≥2 crates, and every version-locked family
  (egui/egui-wgpu/egui-winit, wgpu), has exactly one version declaration.
- **Gap:** five entries where a bump requires finding N manifests, against a
  workspace whose own comment promises one.
- **Suggestion:** move `smallvec`, `wgpu`, `egui-wgpu`, `egui-winit` into
  `[workspace.dependencies]`; switch engine-renderer's `hecs` to
  `{ workspace = true }`; add `test = false` to chapter-01's `[lib]`.
- **Path:** (1) manifest sweep; (2) `cargo check --workspace --all-targets`;
  proof: `Cargo.lock` unchanged (same resolved versions), gate green.

### 6. `notify` is two majors old and compiles into headless server builds for a window-config watcher

- **Evidence:** `notify = "6"` (smirk/engine-app/Cargo.toml:20) — current
  stable is 8.2.0; v6.1.1 drags the workspace's fourth `windows-sys`
  (0.48.0) and `windows-targets 0.48.5` duplicates (`cargo tree -d`). Its
  sole use is `App::configure`'s hot-reload watcher for `WindowConfig` /
  engine.ron (smirk/engine-app/src/app.rs:59-63, 122-134) — a windowed-client
  concern — yet the dep and the `config_watcher` field are unconditional, so
  `vordar-server` (which builds engine-app `default-features = false`
  precisely to shed winit) still compiles notify.
- **Ideal:** headless builds carry no file-watcher; the watcher that windowed
  builds carry is current.
- **Gap:** one stale major dragging a duplicate windows-sys chain into every
  build including the dedicated server.
- **Suggestion:** make `notify` optional, enabled by the existing `winit`
  feature (the config it watches is a window config; no new feature name
  needed), cfg-gate `config_watcher` + the watcher setup, and bump to 8.
- **Path:** (1) manifest: `notify = { version = "8", optional = true }`,
  `winit = ["dep:winit", "dep:notify"]`; (2) cfg-gate the field, `configure`'s
  watcher block, and the poll site; (3) `cargo check -p vordar-server` proves
  the headless graph lost notify; proof: gate green, windowed client still
  hot-reloads (existing behavior, user feel-check not required — the watcher
  logs on start).

### 7. (user-decides) Toolchain is 4½ months stale, and it is the stated blocker on the `rusqlite` pin; `criterion` is three minors behind

- **Evidence:** `rustc 1.94.0 (2026-03-02)` is the active toolchain (no
  `rust-toolchain.toml` pins it — it is simply the last `rustup update`).
  Root Cargo.toml:41-42 pins `rusqlite = "0.37"` because "0.40's
  libsqlite3-sys needs a newer rustc (unstable cfg_select on 1.94) — bump
  when the toolchain moves." That condition has expired: `cfg_select!` was
  stabilized in **Rust 1.95.0, released 2026-04-16**; current rusqlite is
  0.40.1. `criterion = "0.5"` (root:59) — current is 0.8.2 (MSRV 1.86).
  `getrandom`, `sha2`, `ctrlc`, `postcard`, `gltf`, `image`, `egui 0.34`,
  `wgpu 29`, `quinn 0.11` are current or current-enough; no other pin
  comment has expired.
- **Ideal:** the toolchain is current stable; every pin comment states a
  reason that is still true; the bench harness is on its current major.
- **Gap:** the one dep pinned "until the toolchain moves" stayed pinned
  because the toolchain silently stopped moving; nothing in the repo records
  which toolchain the gate is expected to run on.
- **Suggestion:** `rustup update` (user's machine-level action — hence
  user-decides, asked at loop launch), then bump rusqlite to 0.40 and
  criterion to 0.8 in one pass; rewrite the rusqlite comment (the pin
  dissolves entirely). For criterion, verify the
  `default-features = false, features = ["cargo_bench_support"]` headless
  configuration against the 0.8 feature list before landing — the no-plotters
  rationale (root:58) must survive the bump. Optionally add
  `rust-toolchain.toml` so the expected toolchain is recorded; with a
  single dev machine this is documentation, not enforcement — the user may
  decline it.
- **Path:** (1) user: `rustup update` to ≥1.95; (2) bump rusqlite, run the
  server e2e + persistence tests (db.rs's bundled sqlite jumps versions);
  (3) bump criterion, confirm `--save-baseline`/`--baseline`/`--quick` CLI
  still accepted by a smoke `cargo bench -p vordar-benches -- --quick`;
  (4) update both pin comments; proof: full gate + one bench smoke run
  green.

### 8. A missing-resource panic is an anonymous `unwrap` at 100+ sites

- **Evidence:** the dominant panic idiom across every runtime crate is
  `resources.get_mut::<T>().unwrap()` — e.g. 27 pre-test sites in
  server/vordar-server/src/net/receive.rs alone (`:65,66,68,91,97,...`), 13
  in client/vordar-client/src/net/lifecycle.rs, plus engine-app/renderer
  equivalents. `Resources::get`/`get_mut`
  (smirk/engine-core/src/traits.rs:26-32) return `Option`, so every caller
  unwraps; when the invariant breaks (plugin forgot an insert), the panic
  reads `called Option::unwrap() on a None value` with no resource type
  name — the one fact the debugger needs. Sites that bother with `.expect()`
  hand-write the type name (receive.rs:61: "ClassLibrary not in resources"),
  proving the need; ~90% of sites don't bother.
- **Ideal:** a missing-resource panic names the missing type unconditionally,
  with zero per-site effort — the pattern Bevy/hecs users expect from a
  resource map.
- **Gap:** the API pushes the diagnostic cost to every call site, and call
  sites predictably decline to pay it.
- **Suggestion:** add `Resources::expect<T>` / `expect_mut<T>` that panic
  with `std::any::type_name::<T>()` ("resource `vordar_server::NetServerState`
  not inserted — missing plugin?"), keep `get`/`get_mut` for genuinely
  optional lookups, and mechanically migrate the `.unwrap()` sites
  (regex-scale; the handwritten `.expect("...")` sites migrate too and drop
  their strings).
- **Path:** (1) add the two methods + unit test asserting the message names
  the type; (2) mechanical migration per crate; (3) full gate; proof: a
  deliberate missing-insert in a scratch test names the type in its panic.

### 9. The workspace is on edition 2021

- **Evidence:** all 14 manifests declare `edition = "2021"`; edition 2024
  has been stable since Rust 1.85 (Feb 2025) — a year and a half. The
  workspace also carries `resolver = "2"` (root:18) which edition-2024
  packages default past (resolver 3 / MSRV-aware resolution).
- **Ideal:** current edition, so new code is written against current idioms
  and the workspace never needs a compound migration later.
- **Gap:** fourteen crates of migration debt accruing interest with every
  new file written under 2021 rules.
- **Suggestion:** `cargo fix --edition` per crate, then flip `edition`, on
  the post-finding-7 toolchain, behind the finding-1 lint gate; land as one
  commit per crate or one workspace commit, whichever the diff size favors.
- **Path:** (1) `cargo fix --edition` sweep (expect mostly no-ops in this
  codebase — no `static mut`, minimal `unsafe`); (2) flip all 14 editions +
  set workspace `resolver = "3"`; (3) full gate incl. e2e; proof: gate
  green on edition 2024.

### 10. Dev-profile dependency codegen is untuned while the e2e suite's whole failure mode is CPU starvation

- **Evidence:** root Cargo.toml's only dev-profile tuning is `debug = 1`
  (:65-66). The devloop campaign (docs/reviews/devloop/, 2026-07-17)
  spent its entire rework budget on e2e tests starving under CPU load —
  those tests run the real server sim in the **dev** profile via nextest,
  where glam/hecs/quinn/rustls compile at opt-level 0. A
  `[profile.dev.package."*"] opt-level = 1` (deps only — workspace crates
  stay at 0, so incremental iteration is untouched) is the standard lever;
  whether it pays here is an empirical question this audit cannot settle
  from code.
- **Ideal:** the dev profile is a measured choice: either tuned because it
  buys real e2e headroom, or left alone because measurement showed it
  doesn't — not defaulted.
- **Gap:** the one profile knob that could cheaply widen the sim's CPU
  margin under test load has never been measured.
- **Suggestion:** measure, then adopt or discard on the numbers — this is a
  2-line reversible change with an objective criterion, not a preference.
- **Path:** (1) record current: clean `cargo nextest run --workspace` wall
  time and the e2e suite's wall time, idle; (2) add
  `[profile.dev.package."*"] opt-level = 1`, rebuild, re-measure both plus
  one `stress-suite.ps1 -Load 3.0` sensitive-set pass; (3) adopt iff e2e
  wall improves ≥10% and clean-build cost stays acceptable (record both in
  the commit); otherwise revert and record the numbers in this report's
  successor so it is never re-litigated blind.
- **Measured 2026-07-17:** clean full-workspace `cargo nextest run
  --workspace` (compile+test, 20-logical-core machine): 110.0s at
  opt-level 0 deps (compile 54.6s + test 54.6s) vs 156.0s at opt-level 1
  deps (compile 101s + test 53.7s) — clean-build wall +42%, compile-only
  +85%. Idle e2e-scope wall (the 3 exclusive-group tests:
  `rend_kills_camped_enemy`,
  `net::e2e::kicked_connection_reconnects_and_relogs_in`,
  `net::e2e::onslaught_dash_replay_never_snaps_at_150ms_rtt`): 7.74s at
  opt-level 0 vs 7.61s at opt-level 1 — a 1.7% improvement, far short of
  the ≥10% bar (the sensitive-set stayed 5/5 green under `stress-suite.ps1
  -Load 3.0` at opt-level 1, but that was already true pre-tuning per the
  devloop campaign's sim-pacing fixes, so it isn't evidence for this
  knob). Reverted; `[profile.dev]` is unchanged from `debug = 1`.

### 11. (docs-only) (micro) The bench profile keeps symbols "for future flamegraph work" but no profiling recipe exists

- **Evidence:** root Cargo.toml:68-69 keeps `debug = true` in
  `[profile.bench]` explicitly for flamegraphs; `docs/benchmarks/BASELINE.md`'s
  "How to run" (:16-37) documents baselines, soak, loss probes — no
  profiling command anywhere in the repo.
- **Ideal:** the paid-for symbols have a documented consumer: one command a
  future perf pass copy-pastes.
- **Gap:** the profile comment promises a workflow the docs never deliver.
- **Suggestion:** add a "Profiling" entry to BASELINE.md's How-to-run block:
  on this Windows box, `cargo flamegraph --bench full_tick -p vordar-benches
  -- --bench --profile-time 10` (blondie/ETW needs an elevated shell — say
  so), or the equivalent Superluminal invocation if the user prefers it.
- **Path:** (1) verify the chosen command actually produces a flamegraph
  once; (2) add the block; proof: the documented command runs.

### 12. The criterion saved baseline predates five campaigns of perf-relevant change

- **Evidence:** the last full-suite `--save-baseline main` is recorded
  2026-07-04 (docs/benchmarks/BASELINE.md:42-50, machine block: rustc
  1.94.0, Date 2026-07-04). Since then: networking reworks 7 and 11
  (07-16), the game-architecture reworks (07-15/16), the full rendering
  campaign (07-16 — which itself edited BASELINE.md sections piecemeal:
  render_cpu, asset_load, frustum_classify), and the devloop campaign.
  BASELINE.md:39-40's own rule: "Update it after any change that moves a
  number." The weakpoints pass explicitly fixed this same "mixed-vintage
  baseline problem" once (tasks/todo.md, item 9, 2026-07-04) — it has
  recurred.
- **Ideal:** `--baseline main` comparisons measure the optimization being
  tested, not thirteen days of unrelated drift; the header's
  rustc/date describe the tree the numbers came from.
- **Gap:** the durable record is a patchwork of 07-04 full-suite numbers and
  07-16 single-bench updates against different trees.
- **Suggestion:** after findings 2, 3, 7, 9, 10 settle (each moves codegen or
  dep code), re-run the full suite with `--save-baseline main`, refresh
  every table plus the machine block in one commit. Goes last for exactly
  that reason.
- **Path:** (1) `cargo bench -p vordar-benches -- --save-baseline main`
  (~6 min per BASELINE.md); (2) update tables + header; proof: BASELINE.md's
  date, rustc, and every table row come from one tree.

## Carried forward from previous report

None — first rust-tooling audit.

## Resolved since last report

None — first rust-tooling audit.
