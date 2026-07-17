# Dev-Loop Audit — 2026-07-17

Second run of this audit. Telemetry corpus: the 76 subagent transcripts of the
2026-07-16/17 rendering campaign (41 sonnet workers, 18 haiku workers, 7 fable
planners, 1 opus planner, 1 haiku probe, 9 synthetic husks from API failures) —
a full audit→10-fixes→6-reworks cycle, 65 commits, suite 328→400. Aggregates:
9.94 agent-hours wall, tool share 19.7% (was 37.3% on 07-15 — the gate and
read rules landed and held); subagent output 1,993k tokens, cache-create
17.58M; orchestrator (main context, fable): 646.6k output tokens across the
two campaign days, peak context 734k, 2 compactions. Zero opus workers, zero
baseline re-establishments, zero self-commits, both corrections went through
the fresh-haiku path (9.4k tokens combined vs the ~60–80k resume cost they
replaced) — every 2026-07-15 rule held under a 10× longer campaign. What
remains is a different cost profile: the orchestrator's own premium-tier
spend, the read channels the report-ban missed, and gate/plan shapes that let
one silent breakage and two mega-steps through.

Every finding carries a **Tradeoffs** bullet; the user decides adoption.
Findings editing `.claude/` files are local-only (gitignored, pushed to
TycheDea/ClaudeConfig); the report is the committed artifact.

## Ideal end state

Each pipeline tier runs on the cheapest model that lands its work — including
the orchestrator, whose loop-time role is mechanical enough that premium-tier
tokens are spent only where judgment is real (audits, blockers, corrections).
Workers touch no file larger than the window they need; every gate that can
fail for a diff shape actually runs for that shape, and nothing else does;
plans slice work so no single step balloons to 7× the median; and an API
outage or flake costs one recorded recovery step, not 45 minutes of retries.

## Findings (implementation order)

Cross-type queue (mirrored in `reworks-devloop-2026-07-17.md`):

> **~~finding 1 (user-decides — ask at loop launch) → finding 2 → finding 3 →
> finding 4 → finding 5 → finding 6 → finding 7 → finding 8 → finding 9
> (micro) → finding 10~~ → ~~rework 1 (user-decides; after finding 5: a queue
> runner must embed the planner-fallback convention finding 5 writes)~~.**
> Findings 1–10 done 2026-07-17 (1–5, 7: pipeline rules, pushed to
> ClaudeConfig; 6: allowlist content-keying; 8: measurement contradicted the
> stated failure mode — fixed the real cause, filed rework 2; 9: micro, applied
> inline; 10: dedicated sessions declined — plugin disabled, measurement folded
> into the next loop).
> Rework 1 done 2026-07-17 (plan-devloop-rework-1-2026-07-17.md, 3 steps; pause-on-plan queue runner: a run-queue skill chains plan-rework + implement-finding by invocation, rework-planner pinned to fable so an opus queue session cannot downgrade plans).
>
> Findings 1–4 lead on the token axis (orchestrator tier, worker read
> channels, silent-breakage prevention, mega-step shape). 5 is the
> attention-axis fix for the measured 45-minute stall. 6–9 remove small
> recurring frictions; 10 is wall-only. Rework 1 goes last because it
> packages conventions the fixes establish.

### 1. (user-decides) The orchestrator runs loops on the premium tier — 646.6k fable output tokens in two days, most of it mechanical

- **Evidence:** main-session ledger 2026-07-16/17: 604.7k + 41.9k output
  tokens on fable (the tier above opus), 561 assistant turns, against 1,993k
  output across ALL subagents (mostly sonnet). The 07-16 turns break down as
  ~55 spawn-watch-commit cycles (spawn prompt with pasted section, completion
  handling, `git status`/`diff --stat`, commit with a long message, queue
  strikes) plus the audit itself. The audit and the campaign's three genuine
  judgment calls (529-storm diagnosis, rework-4 breakage triage, the mid-turn
  stop order) are fable-grade; the per-finding cycle — route by the routing
  list, paste, watch, commit — is not, and it is the bulk of the turns.
- **Ideal:** premium-tier output is spent only on audits and judgment;
  loop-time orchestration runs on a tier whose price matches its mechanics.
- **Gap:** at ~5×+ sonnet pricing, the orchestrator's loop-time output alone
  plausibly outweighs the entire worker fleet's spend on 07-16.
- **Tradeoffs:** *Wins:* the single largest token lever found this audit —
  a sonnet-run loop cuts the orchestrator's per-finding cycle cost ~5×; the
  deep thinking already lives in plans and findings by design (the
  pipeline-tiers principle applied to its last untiered layer). *Losses:*
  the three judgment calls above happened mid-loop, unplanned — a sonnet
  orchestrator handles them worse or escalates late; model switching
  (`/model sonnet` at loop launch, back to fable for audits) is a manual
  step the user must remember, and a forgotten switch-back degrades the next
  audit; corrections routing (finding-worker vs resume vs inline) is genuine
  judgment that would also downgrade.
- **Suggestion:** if adopted: a one-line convention in
  `implement-finding/SKILL.md` — "loops of pre-planned findings may run
  under a sonnet session; audits and plan reviews stay on fable" — and the
  user switches at launch. If declined: record that loop-time fable spend is
  accepted as the price of mid-loop judgment.
- **DECIDED 2026-07-17: opus for loops.** The user chose the middle option —
  implement loops run under an opus session (`/model opus` at loop launch,
  back to fable for audits and plan reviews): cheaper than fable on the
  mechanical cycle, stronger mid-loop judgment than sonnet. The skill
  convention line should name opus, not sonnet.
- **Path:** (1) ~~user decision~~ decided (opus); (2) one skill line + trying
  it on the next loop; proof: that loop's main-session output ledger shows
  the per-finding orchestrator cycle at opus pricing with zero escalation
  incidents, or names the incident that justifies revisiting.

### 2. Workers still read large review files the ban doesn't cover — 23 whole-report reads this campaign

- **Evidence:** the spawn-task ban (`implement-finding/SKILL.md:89` "do NOT
  open the report or plan file itself") stopped reads of the worker's OWN
  report — but transcripts show 10 worker reads of
  `audit-rendering-2026-07-16.md` (the origin audit their rework plan cites —
  a third file the ban never names) and 13 worker reads of
  `reworks-rendering-2026-07-16.md`. The reworks reads have a legitimate
  trigger: `finding-worker.md:37-45` requires appending rework-scale
  remainders there, and Edit requires a prior Read — so each append paid a
  full read of a ~450-line report to learn one number (the next free finding
  index).
- **Ideal:** a worker's review-file exposure is exactly its pasted section
  plus, when appending, the tail window of the reworks file.
- **Gap:** ~23 large-file reads (~150k+ cache-create) per campaign through
  two channels the ban's wording misses.
- **Tradeoffs:** *Wins:* kills both channels for one wording change; append
  behavior stays intact. *Losses:* a worker whose finding genuinely
  mis-states its origin context loses the ability to consult the source
  audit and must flag instead (the flag is the designed behavior, but it is
  one round slower); tail-window appends assume the last finding sits in the
  last ~40 lines (true for every report generated under audit-base's
  format).
- **Suggestion:** two edits to `finding-worker.md`: (a) extend the
  do-not-open rule from "the report or plan file" to "any file under
  `docs/reviews/`, except as (b)"; (b) in the append instruction: "Read only
  the file's tail (offset within ~40 lines of the end) to get the next free
  number and the Edit anchor — never the whole report." Mirror the ban
  extension in the spawn-task template in `implement-finding/SKILL.md`.
- **Path:** (1) two edits; (2) proof: next loop's transcripts show zero
  whole-file `docs/reviews/` reads by workers (windowed tail reads only).

### 3. A content-only diff passed a check-only gate and shipped two broken tests — content files need a mechanical gate trigger

- **Evidence:** rework-4 step 3 committed DDS sidecars under `content/`
  with its plan-prescribed gate (`cargo check` — clean, since no code
  changed), silently breaking `zone_ground_renders_with_texture_variation`
  and `ground_sets_within_dimension_cap` (both `find()`-matched texture
  files by substring and picked up the new `.dds`). Caught one commit later
  by step 4's full gate (371/373), costing that worker a breakage diagnosis
  it didn't cause (its spawn ran 227 turns / 78.6k output, the campaign's
  third-largest) plus a haiku correction spawn (166s, 7.6k) and orchestrator
  triage. The full-gate trigger in `finding-worker.md:74-77` counts
  "workspace crate roots" — `content/` and `tests/data/` are not crate
  roots, so a pure content diff never trips it.
- **Ideal:** any diff that adds or changes files under `content/` or a
  `tests/data/` fixture directory runs the full workspace gate — content is
  consumed cross-crate by lints and render tests by design, so the crate-root
  count is the wrong detector for it.
- **Gap:** one file-class hole in an otherwise mechanical trigger; it fired
  exactly once this campaign and cost roughly a worker's worth of cleanup.
- **Tradeoffs:** *Wins:* closes the only silent-breakage escape observed in
  two audits; the full gate is 45.6s — cheap on the wall axis the user has
  deprioritized anyway. *Losses:* content-heavy loops (future asset drops)
  full-gate every step, adding ~45s each even for assets nothing consumes
  yet; the rule adds one more clause to the gate section.
- **Suggestion:** extend the trigger sentence in `finding-worker.md` with
  "or your diff adds/changes files under `content/` or a test fixture
  directory (`tests/data/`)"; add the matching sentence to rule 2 of
  `rework-planner.md` (a step whose diff is content-only must still name
  the full gate in its Path).
- **Path:** (1) two edits; (2) proof: next content-touching step's transcript
  shows the worker running the full gate unprompted; no content-induced
  breakage surfaces a commit late again.

### 4. Two mega-steps cost 290k output tokens (7×/4× the loop median) — planners need a split signal for multi-pass GPU work

- **Evidence:** rework-6 step 5 (depth prepass + half-res SSAO + blur + 
  readback seam): 180.3k output, 566.9k cache-create, peak context 338.2k,
  221 turns, 2,178s — the campaign's costliest spawn by 2.3×. Step 6 ran
  109.5k/275.3k peak/196 turns. Campaign median sonnet worker: ~27k output.
  Both steps landed clean, but each bundled multiple new GPU passes
  (pipelines + WGSL + offscreen harness + calibration) into one worker whose
  context grew past 275k — the zone where every further turn's cache write
  is priced on a huge transcript. `rework-planner.md:53-58` (rule 1) says
  "fix-sized… if a step needs its own design discussion, it is too big" —
  design discussion is the wrong detector for graphics steps whose cost is
  iterative calibration, not design.
- **Ideal:** the planner's sizing rule names the observable predictor: a
  step that introduces two or more new render passes/pipeline families, or
  a new pass plus its consumer calibration, splits at the pass boundary
  (each pass provable via its own readback), keeping worker contexts in the
  ~100k band where the rest of the campaign ran.
- **Gap:** the two outliers were 14.5% of the campaign's entire subagent
  output; a pass-boundary split caps the shape.
- **Tradeoffs:** *Wins:* smaller peak contexts (cheaper cache writes per
  turn, no flirting with compaction at 338k), independently provable passes,
  a failure in pass B doesn't sit on pass A's 100k transcript. *Losses:*
  each extra step costs a ~35k spawn boot plus the green-state scaffolding
  a half-built feature needs (the depth prepass alone must earn a test
  before SSAO exists to consume it); total output might not drop much if
  calibration iterations, not context, dominated — the numbers can't be
  fully separated from this side of the transcript; one more sizing clause
  for planners to weigh.
- **Suggestion:** add the pass-boundary sentence to rule 1 of
  `rework-planner.md`, citing the measured shape ("a two-pass step measured
  180k output vs a 27k median, 2026-07-16").
- **Path:** (1) one edit; (2) proof: the next multi-pass rendering rework's
  plan slices at pass boundaries, and its loop ledger shows no spawn above
  ~2× median output.

### 5. The 529 storm cost 45 minutes and 8 dead spawns before an improvised recovery — codify the probe-and-override rule

- **Evidence:** 2026-07-16 19:23–20:08: eight rework-planner spawns died to
  sustained `529 Overloaded` (eight identical 19.9KB synthetic transcripts,
  ~200s each) across escalating background-sleep backoffs, because
  `rework-planner.md` pins no model and inherited the session's fable, whose
  subagent capacity was what was overloaded. Recovery was improvised and
  worked first try: a 1-turn haiku probe ("Reply with the single word: ok",
  20:08) proved the API path fine, isolating the failure to the model tier,
  and a one-off `model: "opus"` override landed the plan immediately
  (785s, 51.1k output). Nothing records this playbook; the next storm pays
  the 45 minutes again.
- **Ideal:** spawn-level API-failure recovery is a written convention: after
  the second consecutive pre-edit 5xx death of the same spawn, probe with a
  1-turn haiku no-tool task; probe green ⇒ the tier is down, respawn with
  the documented fallback (`opus` for planners, `sonnet` already default for
  workers); probe red ⇒ back off long and tell the user.
- **Gap:** an unwritten playbook that already proved itself.
- **Tradeoffs:** *Wins:* caps any future tier outage at ~2 dead spawns + one
  probe (~46k cache-create, 2s) instead of 8 + 45 minutes of user wall;
  the fallback quality question is settled per-agent in advance (opus
  planned rework 6 acceptably — 7 steps, all landed). *Losses:* an opus
  fallback plan is still a downgrade from fable's design depth — a storm
  during a genuinely subtle rework bakes that downgrade in silently unless
  the rule also says "tell the user which model planned it" (include that);
  two dead spawns are still paid before the probe triggers.
- **Suggestion:** add the probe-and-override paragraph to
  `plan-rework/SKILL.md` (orchestrator-side) and a matching one-liner to
  `implement-finding/SKILL.md`; the planner keeps inheriting fable as its
  default.
- **Path:** (1) two edits; (2) proof: the next capacity incident's ledger
  shows ≤2 dead spawns, one probe, one fallback spawn, and the user told
  which model produced the artifact.

### 6. The lint-comments allowlist is line-pinned and churned 13 entries in one campaign — key it on content, not line numbers

- **Evidence:** `scripts/lint-comments-allowlist.txt` holds 44 `path:line`
  pins; `scripts/lint-comments.sh:40-51` matches hits by exact line number.
  Any insertion above a pinned line shifts every pin below it: stale pins
  simultaneously un-allowlist legitimate VQ lines (false hits the worker
  must re-pin) and keep allowlisting whatever line drifted into the slot
  (a masked violation). Campaign churn: 11 entries in `f9ad204`, 1 in
  `28b8253`, 1 in `36c3070` — three comment-touching findings each paid a
  re-pin round. 32 of the 44 pins sit in one file,
  `game/vordar-game/tests/content_lint.rs`, whose VQ density is by
  construction: it is the test that enforces the VQ clauses.
- **Ideal:** the allowlist key survives line drift: `path:normalized-line-
  content` (trimmed, whitespace-collapsed), so an entry moves with its line
  and dies with it; `content_lint.rs` is exempted wholesale in the script
  (every VQ tag there anchors an assert — the spec-clause exception as a
  file property), collapsing 32 entries.
- **Gap:** the mechanical gate that killed sweep-convergence passes now
  generates its own per-finding maintenance.
- **Tradeoffs:** *Wins:* zero re-pin rounds; a masked-drift violation class
  disappears; the list shrinks to ~12 self-maintaining entries. *Losses:*
  content-keying allowlists every copy of an identical line in that file
  (acceptable: the exception is about the line's text anchoring a
  constraint); the wholesale `content_lint.rs` exemption means a genuinely
  provenance-shaped comment in that one file would pass the script and wait
  for a human sweep; ~30 minutes of script work plus regenerating the list.
- **Suggestion:** rework the check loop in `lint-comments.sh` to normalize
  and compare line content; regenerate the allowlist in the new format; add
  `tests/content_lint.rs` to the script's exemptions with a comment stating
  why.
- **Path:** (1) script + list edit; (2) proof: HEAD scans zero-hit; inserting
  a line above a pinned one still scans zero-hit (the drift case); a seeded
  provenance comment elsewhere is still caught.

### 7. Dependency-source and target/ lookups burned three 120s Bash timeouts — the scan ban misses the two big trees outside the repo

- **Evidence:** the campaign's three costliest tool calls all pegged the
  120s Bash timeout: `grep -rn … ~/.cargo/registry/src/*/wgpu-naga-bridge-29.0.0`
  and `find ~/.cargo/registry/src -maxdepth 1 -iname "naga-*"` (both in the
  rework-6 step-3 worker) and `grep … target/debug` (rework-4 step 4) —
  ~360s plus the failed-turn retries. `finding-worker.md:104-109` bans
  unscoped scans *from the workspace root* and even names the registry path
  as the place to "go directly" — but says nothing about HOW: bash
  grep/find over multi-GiB trees is the failure mode regardless of scoping,
  while the Grep/Glob tools answer the same questions in under a second
  (all 419 campaign Grep-tool calls together cost 53s).
- **Ideal:** the rule names tools, not just paths: `target/` is never
  searched (build artifacts; versions come from `Cargo.lock`); dependency
  sources are located with Glob (`~/.cargo/registry/src/*/<crate>-*`), then
  searched with the Grep tool passing that directory as `path`.
- **Gap:** two sentences of recipe missing from an otherwise-working rule.
- **Tradeoffs:** *Wins:* removes the last observed 120s-stall class (~360s
  + retry turns per incident). *Losses:* none of substance — the Grep tool
  covers every observed use, including the naga-internals lookup.
- **Suggestion:** replace the dependency-sources sentence in
  `finding-worker.md` with the Glob-then-Grep recipe and an explicit
  "never search `target/`".
- **Path:** (1) one edit; (2) proof: next campaign's top-10 tool events
  contain no filesystem scan.

### 8. rend_kills_camped_enemy flaked again post-isolation — the recorded decision now triggers: widen the test's internal budget

- **Evidence:** `.config/nextest.toml:17-22` records the 2026-07-15
  decision: isolation is retained, and "future flakes get fixed via
  test-internal budget tuning, not scheduling." The flake recurred this
  campaign (~2 events across ~30 full runs — rework-6 loop-final among
  them, green in isolation and on rerun), so the trigger condition is met.
  Transcript census (all-time): 12 FAIL mentions across both its e2e homes.
  It currently passes at 3.68s under exclusive scheduling; the failure mode
  is its closed-loop bot missing a wall-clock kill deadline under CPU load.
- **Ideal:** the test's internal deadline carries enough margin that CPU
  contention inside an exclusive slot cannot starve it below the kill
  threshold; the nextest header's numbers get today's date and the new
  margin.
- **Gap:** a ~2% flake that costs a 40s rerun plus attribution turns every
  loop-final it hits — and it hits the loop-final disproportionately, the
  one gate whose result gets written into the commit message.
- **Tradeoffs:** *Wins:* removes the last recurring flake the pipeline
  carries; honors the already-recorded decision instead of re-litigating.
  *Losses:* a widened budget makes the test slower to fail when a REAL
  regression slows rend kills — the margin trades detection sharpness for
  stability; the diagnosis needs a measured timing profile first (which the
  Path names), not a blind constant bump.
- **Suggestion:** one finding-worker run: instrument the test's kill
  timeline (5 looped runs), set the deadline from measured worst-case plus
  margin, update the nextest header comment's figures.
- **Path:** (1) measure; (2) budget edit + header update; proof: 5
  consecutive green loops with zero rend flakes in transcripts, and the
  header states the new margin with its measured basis.

### 9. (micro) Two tests share one temp GLB path — rename one side

- **Evidence:** `smirk/engine-renderer/src/mesh/anim_import.rs:179` and
  `smirk/engine-renderer/src/mesh/gltf_import.rs:445` both write
  `temp_dir()/vordar_mesh_test_skinned.glb`; under plain `cargo test -p
  engine-renderer` (multithreaded libtest, which workers legitimately run
  as their scoped gate) the writes race. Census:
  `loads_skinned_animated_glb` FAILED 6 mentions + 8 GLB-related panics
  across transcripts. Neighboring tests already use unique names
  (`store.rs:441,501-502`).
- **Ideal:** every temp fixture path is test-unique, matching the store.rs
  pattern.
- **Gap:** one filename collision generating recurring worker-facing flakes.
- **Tradeoffs:** *Wins:* removes the race for a one-line rename. *Losses:*
  none.
- **Suggestion:** rename anim_import.rs:179's file to
  `vordar_anim_test_skinned.glb`. Single file, strictly enumerated, existing
  suite covers it — the orchestrator applies it inline.
- **Path:** (1) one-line edit + `cargo test -p engine-renderer --lib`;
  proof: no shared temp filenames remain (grep) and the flake never recurs.

### 10. Every Edit costs a flat ~5.05s regardless of file type — 53 minutes of campaign wall on one tool

- **Evidence:** 626 Edit calls at 5.1s average = 3,178s, 45% of all tool
  wall — the #2 tool cost behind Bash. The latency is uniform: median
  5.055s, p90 5.094s, identical for `.rs` (4.98s), `.wgsl` (4.88s), `.md`
  (4.41s), `.toml`/`.ron`/`.txt` (~5.0s) — while Write averages 0.9s and
  Read 0.03s. A flat cross-language constant rules out rust-analyzer
  compile time (the enabled `rust-analyzer-lsp` plugin declares `.rs`
  only); the shape matches a fixed post-edit diagnostics wait, either from
  the LSP plugin firing unconditionally or the connected RustRover IDE's
  file-sync integration — the same channel whose diagnostics the pipeline
  already treats as unreliable after moves (`implement-finding/SKILL.md:51-56`).
- **Ideal:** Edit costs what Write costs (~1s); post-edit diagnostics are
  either off during headless loops or fast enough not to gate the turn.
- **Gap:** pure wall (zero tokens), so last on this audit's axis — but it
  is 45% of tool time bought by a channel the pipeline distrusts anyway.
- **Tradeoffs:** *Wins:* ~4s × ~600 edits ≈ 40+ minutes of loop wall per
  campaign if eliminable. *Losses:* if the cause is the LSP plugin,
  disabling it loses real-time `.rs` diagnostics workers occasionally
  benefit from (the finding-14 orphaned import of 07-15 was caught by this
  channel); if it's the IDE connection, closing RustRover during loops
  kills the rustrover-index MCP tools CLAUDE.md prefers for navigation; and
  the constant may be harness-intrinsic (unfixable locally), which the
  experiment below establishes cheaply.
- **Suggestion:** run the discriminating experiment before changing
  anything: one worker-scale session with the LSP plugin disabled, one with
  RustRover closed, comparing Edit averages against the 5.05s baseline.
- **DECIDED 2026-07-17: fold the measurement into the next real loop.** The
  user declined the two dedicated sessions — they spend tokens (the binding
  axis) to buy wall (the deprioritized one). Instead `rust-analyzer-lsp` was
  disabled in `~/.claude/settings.json` on 2026-07-17, so the next loop's
  telemetry discriminates the plugin at zero extra cost: Edit drops toward
  ~1s ⇒ the plugin was the cause and it stays off; Edit stays ~5.05s ⇒ the
  plugin is exonerated, leaving the RustRover file-sync and harness-intrinsic
  hypotheses, which the successor report tests only if the wall axis has by
  then started to bind.
- **Path:** (1) ~~two measurement sessions~~ plugin disabled 2026-07-17;
  (2) read the next loop's Edit average against the 5.05s baseline and adopt
  or exonerate per the decision above; proof: next campaign's Edit average.

## Carried forward from previous report

None unresolved — all 17 findings of `audit-devloop-2026-07-15.md` landed the
same day and their rules held under this campaign's 10× load (verified in
this report's preamble). Rework 1 (parallel execution) was declined
2026-07-15 with a standing do-not-refile condition; see Resolved below.

## Resolved since last report

- **2026-07-15 findings 1–17:** all verified working under the rendering
  campaign: scoped gates (18 workspace runs across 68 live spawns vs 55/55
  before), zero baselines, zero self-commits, capped reports, embedded
  finding sections (own-report reads: zero — the residual channels are new
  finding 2), sonnet-first routing (zero opus workers), fresh-spawn
  corrections (2 of 2), micro path (used inline), same-day rerun rule and
  user-decides batching (unexercised this campaign, still in force).
- **2026-07-15 rework 1 (parallel execution):** remains declined; the
  condition ("do not re-file while token spend outranks wall time") still
  holds and this campaign's serial loops stayed within budget expectations
  (283k–1.33M tokens per loop, reported per loop).
- The prior report and its reworks file are superseded and deleted per
  audit-base; no plan files existed for the declined rework.
