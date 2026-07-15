# Dev-Loop Audit — 2026-07-15

First run of this audit. Telemetry corpus: 55 subagent transcripts from today
(48 finding-workers, 2 rework-planners, 5 sweeps) — a full hygiene
audit→implement cycle of 20 findings + 2 reworks, so the numbers describe the
pipeline at its current best. Aggregate: 6.59 agent-hours wall, 62.7% of it
model time, 37.3% tool time; 2,160 tool calls; ~1.04M output tokens; 311.8M
input tokens served from cache against only 19K uncached (the cache economy
works). Zero retry loops and near-zero read cost (Reads+Greps = 0.6% of tool
time across 908 calls) — the read-window and execute-don't-explore rules from
the 2026-07-14 analysis demonstrably held. The costs that remain are
structural: test-gate policy, one-class-at-a-time finding scoping, and worker
contract gaps that the orchestrator currently patches by hand.

Every finding carries a **Tradeoffs** bullet; the user decides adoption. Where
a finding edits `.claude/` files, note those are local-only (gitignored) — the
report is the committed artifact.

## Ideal end state

A named goal becomes verified, committed code with: workers spending tool time
only on gates that can actually fail for their diff; class-shaped cleanups
closed by a mechanical lint instead of repeated audit passes; the worker
contract complete enough that the orchestrator never patches it per-prompt;
routing sending each step to the cheapest model that lands it, with the miss
rate as a tracked number; and the user's attention spent only on genuine
decisions, batched where a loop is running.

## Findings (implementation order)

Cross-type queue (mirrored in `reworks-devloop-2026-07-15.md`):

> **finding 1 → finding 2 → finding 3 → finding 4 → finding 5 → finding 6 →
> finding 7 → finding 8 → finding 9 → finding 10 → finding 11 → rework 1
> (parked: gated on findings 1+4 landing first — parallelism multiplies
> whatever per-worker cost policy exists, so fix the policy before
> multiplying it).**
>
> Findings 1–4 first: they are the measured hot spots (test-gate policy is
> 51% of tool time; the pattern-gate finding removes whole audit passes).
> 5–11 are independent rule/config edits ordered by impact.

### 1. Test-gate policy: scope the suite to the diff, stop paying for baselines

- **Evidence:** 55 `cargo nextest run --workspace` invocations today (median
  56.9s, ≈3,000s total — the single largest tool cost, 51% of tool wall
  including the scoped runs). 8 of them were *baselines* — runs before the
  worker's first edit, in transcripts where the orchestrator already knew HEAD
  was green from the previous commit's gate. Most workers ran `--workspace`
  even for single-crate diffs (e.g. comment-only purges of one crate). The
  cross-crate compile risk that motivates the wide gate is already covered by
  `cargo check --workspace --all-targets`, which costs 2.4% of tool time.
- **Ideal:** the orchestrator's spawn prompt states "HEAD is green at N/N —
  do not re-establish a baseline"; the worker's gate is `cargo check
  --workspace --all-targets` (always) + `cargo nextest run -p <touched
  crates>` (default) + one full `--workspace` run only when the diff crosses
  crates or touches shared test infrastructure; the loop's final item always
  runs the full gate once.
- **Gap:** every worker pays ~55s per full run regardless of diff shape;
  baselines re-prove what the orchestrator already knows.
- **Tradeoffs:** *Wins:* cuts the dominant tool cost — scoped server-crate
  runs are ~38-40s, engine-crate runs seconds; baselines vanish (8×55s
  today), plus the model turns that watch them. *Losses:* a scoped run can
  miss a behavioral (not compile) cross-crate regression until the loop-final
  full gate — failures surface one commit late and need bisecting back; the
  baseline habit once proved a flake pre-existed (`rend_kills_camped_enemy`,
  2026-07-14) — without it, pre-existing flakes get attributed to the current
  diff until rerun; the rule adds prompt/agent-file complexity.
- **Suggestion:** encode in `finding-worker.md` (gate section) + the
  implement-finding spawn prompt gains the "HEAD green at N/N" line.
- **Path:** (1) edit the two files; (2) proof: next loop's telemetry shows
  workspace-run count ≈ loop-final runs + genuinely cross-crate steps, and
  median worker wall drops by ≥40s for single-crate findings.

### 2. Ban unscoped filesystem scans in workers

- **Evidence:** the two costliest tool calls today were an unscoped
  `grep -rn ... C:/Users/egm_8/IdeaProjects/vordar` and a `find` that both
  pegged the 120s Bash timeout; a third unscoped `grep -r .` cost 70s. All
  three scanned `target/` (13.8 GiB). Total ≈310s for what ripgrep answers in
  under a second. finding-worker.md already prefers the Grep tool; it doesn't
  forbid raw recursive grep/find.
- **Ideal:** workers use the Grep/Glob tools (gitignore-aware) or explicitly
  scoped paths; raw `grep -r`/`find` from the repo root is named as forbidden.
- **Gap:** one sentence missing from the agent file.
- **Tradeoffs:** *Wins:* removes a 70–120s stall class entirely. *Losses:*
  none of substance — the Grep tool covers every observed use.
- **Suggestion:** add the sentence to `finding-worker.md`'s tool notes.
- **Path:** (1) edit; (2) proof: next telemetry's top-10 contains no
  filesystem scan.

### 3. Class-shaped findings need a pattern gate, not evidence file lists

- **Evidence:** the provenance purge took FOUR passes to converge: findings
  4–6 (2026-07-14) cleaned their evidenced file lists; findings 2–5
  (2026-07-15) cleaned the 71 stragglers outside those lists; finding 19
  caught manifests and never-swept headers; finding 20 caught two bench
  headers. Each extra pass cost a sweep, an addendum, worker spawns
  (~100–150k tokens per pass) and user wall time — while the eventual closure
  proof was a 2-second workspace grep. The audits scoped the class by file
  lists because no mechanical definition of "clean" existed.
- **Ideal:** a checked-in `scripts/lint-comments` (ripgrep over the forbidden
  patterns — finding/rework/audit citations, phase tags, used-to-be/before-
  the-fix, WEAKPOINTS-style doc tags — with an allowlist for the spec-clause
  exceptions like `VQ-*`-anchoring-a-constraint and `DESIGN.md §N`). Audits
  cite the lint output as evidence; a purge finding's Path ends "lint reports
  zero"; workers run it as part of the gate for comment-touching findings.
- **Gap:** class cleanups converge by repeated human-shaped sweeps instead of
  one mechanical definition.
- **Tradeoffs:** *Wins:* collapses N audit passes to one; closure becomes
  checkable in seconds; the VQ-style boundary rulings become executable
  (allowlist) instead of re-litigated. *Losses:* ~30–60 min to build and
  calibrate; the allowlist needs upkeep when new spec docs appear; a lint can
  only *flag* — deciding whether a tag anchors a constraint still needs a
  reader, so false positives will page a human; one more script to keep
  honest.
- **Suggestion:** build the script; reference it from audit-hygiene's hunt
  list and finding-worker's gate note.
- **Path:** (1) script + allowlist seeded from this cycle's rulings; (2)
  proof: run it on HEAD → zero hits; introduce a seeded violation → caught.
  Next hygiene audit's purge section cites lint output instead of file lists.

### 4. Worker contract gaps the orchestrator patched by hand

- **Evidence:** three per-prompt patches recurred today: (a) one worker
  self-committed (`36c4d22`, finding 7) because commit ownership lives in
  session convention, not `finding-worker.md` — every later spawn carried a
  manual "Do not run `git commit`" line; (b) compile-gate wording is
  inconsistent — workers variously ran `cargo check -p X`, `--workspace`, or
  `--all-targets`; finding 14's orphaned `Vec3` import passed a lib-only
  check and reached the orchestrator gate before being caught; (c) worker
  final reports vary from 15 to 60+ lines — shown verbatim per session rule,
  they compacted the orchestrator context twice today.
- **Ideal:** `finding-worker.md` states: never commit (the orchestrator
  does); the canonical compile gate is `cargo check --workspace
  --all-targets` (2.4% of tool time — cheap enough to be unconditional); the
  final report is capped at ~20 lines (files changed one-liners +
  verification output + flags), everything else stays in the transcript.
- **Gap:** contract lives partly in per-spawn prompt text that must be
  remembered.
- **Tradeoffs:** *Wins:* removes a manual line from every spawn; kills the
  orphaned-import escape class; slows orchestrator context growth (fewer
  compactions → fewer cold re-reads). *Losses:* terser reports carry less
  detail for the user reading along — flags must be trusted to surface what
  matters; `--all-targets` adds a second or two per check.
- **Suggestion:** three edits to `finding-worker.md`.
- **Path:** (1) edit; (2) proof: next loop needs zero per-prompt contract
  patches; no self-commits; report lengths ≤ cap.

### 5. Routing calibration: widen haiku's bracket to planner-dictated steps

- **Evidence:** today's ledger — haiku: 19 launches, ≈56k tokens average,
  including verbatim moves, enumerated deletions, a Cargo-feature-less
  extraction (rework-1 steps 1–3: clock.rs move, impair merge, handshake
  naming) — one follow-up needed all day (finding 8's misnamed split test).
  Sonnet: 23 launches, ≈120k average — needed follow-ups too (finding 14's
  orphaned import). Rework-2 steps 1–5 (extractions with plan-dictated
  signatures and verbatim statement order) went to sonnet at 80–128k each;
  the equally-planned rework-1 steps went to haiku at 50–68k and landed
  identically clean. The differentiator that predicted success was plan
  specificity, not code sensitivity.
- **Ideal:** the routing rule in `implement-finding/SKILL.md` names "a
  /plan-rework step whose Path dictates signatures/order" as haiku territory
  alongside enumerated mechanical edits; sonnet keeps prose rewrites
  (constraint-core purges), open placement choices, and diagnosis.
- **Gap:** ~5 rework-2 steps paid sonnet rates for haiku-shaped work
  (~250k tokens of headroom today).
- **Tradeoffs:** *Wins:* ~half cost on planned steps; planners already absorb
  the thinking, so this is the tiering working as designed. *Losses:* a haiku
  miss on a borrow-sensitive extraction costs a follow-up or a sonnet re-run
  (observed follow-up cost 60–106k tokens) and a user-visible hiccup; the
  measured miss data is one day deep — the bracket could be wrong.
- **Suggestion:** one bullet edit in the skill's routing list.
- **Path:** (1) edit; (2) proof: next planned rework's steps route ≥half to
  haiku with follow-up rate ≤ today's (1 in 19).

### 6. Stale IDE diagnostics after file moves burn orchestrator attention

- **Evidence:** ~12 episodes today where post-move diagnostics showed phantom
  errors (unresolved modules, duplicate definitions) for code cargo compiled
  cleanly; each cost an orchestrator verification round. Worse, the one real
  warning of the day (finding 14's orphaned import) arrived mixed into a
  phantom batch — signal drowned in noise. The rustrover-index MCP exposes
  `ide_sync_files` but nothing invokes it after moves.
- **Ideal:** the convention (orchestrator-side note) is: after a structure
  move, cargo is the only oracle; diagnostics regain authority after an
  `ide_sync_files` call or naturally on next index refresh.
- **Gap:** an ambient channel that is wrong exactly when the pipeline is most
  active.
- **Tradeoffs:** *Wins:* removes ~12 verification rounds/loop and the
  drowned-signal risk. *Losses:* systematically distrusting diagnostics can
  delay noticing a genuine IDE-only hint (unused import warnings arrive
  faster there than from cargo); an extra MCP call per move if syncing.
- **Suggestion:** record the rule in the orchestrator-facing notes of
  `implement-finding/SKILL.md`; optionally call `ide_sync_files` after
  move-heavy commits.
- **Path:** (1) note; (2) proof: next move-heavy loop shows zero phantom-
  diagnostic verification rounds in the orchestrator transcript.

### 7. Dev profile is stock; test debuginfo costs 13.8 GiB and link time

- **Evidence:** the workspace has zero `[profile.dev]`/`[profile.test]`
  tuning (only `[profile.bench] debug = true`). Partial-cleaning just two
  crates deleted 18,842 files / 13.8 GiB — almost all integration-test
  binaries carrying full debuginfo (debug=2 default). 36 test binaries exist;
  9 link with zero runnable tests (chapter-01/02, engine-audio,
  vordar-benches lib, wan_profiles' empty sibling targets, bins).
- **Ideal:** `[profile.dev] debug = 1` (line tables only — panics and
  backtraces stay readable) or `split-debuginfo`, cutting test-binary size
  and link time; `test = false`/`doctest = false` on lib targets that ship no
  tests, removing the zero-test binaries from every suite compile.
- **Gap:** untuned defaults tax every worker's compile-link cycle and the
  disk.
- **Tradeoffs:** *Wins:* smaller target/, faster links (unquantified — the
  Path measures it), faster `cargo clean` cycles. *Losses:* debug=1 degrades
  debugger variable inspection (the user debugs via feel-checks and logs, but
  a future debugger session would notice); target-pruning flags must be
  maintained as crates gain tests; profile changes invalidate the current
  cache once (one slow rebuild).
- **Suggestion:** measure first: full-link time before/after `debug = 1` on
  one heavy test binary, then decide.
- **Path:** (1) before/after link-time measurement of `vordar-server::e2e`;
  (2) if adopted: profile edit + target flags; proof = measured link delta
  and target/ size delta recorded in the commit.

### 8. The doc-test gate runs 3.6s per invocation for zero doctests

- **Evidence:** `cargo test --doc --workspace` = 3.649s warm, 0 doctests
  exist anywhere; the worker gate names it, so it ran ~50 times today
  (~3 min/day of pure freshness-check).
- **Ideal:** the standard worker gate drops it; the loop-final/audit gates
  keep it (so a future first doctest can't rot unnoticed for long).
- **Gap:** a per-worker cost with a zero-payload denominator.
- **Tradeoffs:** *Wins:* 3.6s + a model turn per worker. *Losses:* a doctest
  added between loop-final gates would go unchecked by intermediate workers —
  currently a theoretical loss (zero doctests, none planned).
- **Suggestion:** edit the gate wording in `finding-worker.md`.
- **Path:** (1) edit; (2) proof: telemetry shows doc-test runs ≈ one per
  loop, not one per worker.

### 9. Exclusive-test isolation: cost now measured, flake rate now tracked

- **Evidence:** the three exclusive closed-loop tests serialize ~6.45s of
  every full run (3.37 + 2.06 + 1.02s with all other cores idle) — the
  isolation adopted 2026-07-14 measured for the first time. Despite it,
  `rend_kills_camped_enemy` still flaked once in ~40 full runs today (in a
  worker's pre-edit baseline; clean on rerun) — ≈2.5% under load.
- **Ideal:** the number is tracked, not re-litigated: isolation stays (its
  cost is 6.45s; a flake costs a 40s rerun plus attribution confusion), and
  if the rend flake recurs the fix is the test's internal budget, not more
  scheduling.
- **Gap:** none to fix today — this finding records the baseline numbers the
  config header lacked.
- **Tradeoffs:** *Wins:* future decisions get numbers. *Losses:* none.
- **Suggestion:** append the measured figures to `.config/nextest.toml`'s
  header comment.
- **Path:** (1) one comment edit; (2) proof: header states 6.45s/run and the
  observed flake rate with today's date.

### 10. Audit reruns on the same day have no naming/mechanics rule

- **Evidence:** today's audit→implement→re-audit loop hit the date-named
  file convention: the third and fourth passes had nowhere to go and were
  grafted onto `audit-hygiene-2026-07-15.md` as findings 19/20 addenda —
  workable, but improvised, and the queue note needed a prose explanation.
- **Ideal:** `audit-base.md` states the rerun rule: a same-day rerun extends
  the day's report with an explicitly-labeled addendum section and queue
  extension (exactly what was improvised), so every audit converges the same
  way.
- **Gap:** one unwritten convention.
- **Tradeoffs:** *Wins:* removes improvisation from convergence loops.
  *Losses:* none.
- **Suggestion:** one paragraph in `audit-base.md`.
- **Path:** (1) edit; (2) proof: next convergence loop needs no format
  improvisation.

### 11. User-decides findings stall or get defaulted inside autonomous loops

- **Evidence:** finding 18 (test-support shape) was written as "user picks
  a/b/c", but it came up mid-autonomous-loop where asking would block; the
  orchestrator picked (a)+(b) and flagged it after the fact. The audit format
  has no marker distinguishing "worker can land this" from "this contains a
  decision".
- **Ideal:** audits tag such findings (e.g. "(user-decides)" in the title,
  like "(docs-only)"); when the user launches a loop, the orchestrator asks
  the batched questions for every tagged finding in the queue up front, then
  runs without stalls.
- **Gap:** decisions surface at the worst time (mid-loop) instead of at
  launch.
- **Tradeoffs:** *Wins:* user attention spent once, at a natural decision
  point; no mid-loop defaults taken on the user's behalf. *Losses:* answers
  given up front can be invalidated by earlier findings' outcomes (rare —
  queues are ordered to avoid this); one more format convention.
- **Suggestion:** title-tag rule in `audit-base.md` + a batching note in the
  loop behavior of `implement-finding/SKILL.md`.
- **Path:** (1) edits; (2) proof: next loop containing a tagged finding asks
  at launch and completes with zero mid-loop decision defaults.

## Same-day rerun addendum (second pass — token axis)

Findings 12–17 were added by the same-day second-pass sweep, run after the
user re-weighted the ordering axis to token spend first (parallel-worker
rework declined on the same ground). Telemetry corpus: the session ledger of
today's game-architecture loop — 20 primary worker spawns, 1 resume
correction, 1 rework-planner, ≈2.118M subagent tokens (haiku 3×/165.5k raw,
sonnet 11×/1,310.4k, opus 5×/370.9k, planner 190.7k, correction 80.4k) — plus
the morning devloop loop (11 spawns + 1 correction, 651.8k). Every number
below is from recorded per-spawn `subagent_tokens`/`tool_uses` usage lines,
not estimates.

Queue extension (mirrored in `reworks-devloop-2026-07-15.md`):

> **finding 12 → finding 13 → finding 14 → finding 15 → finding 16 →
> finding 17 (user-decides — ask at loop launch).**
>
> 12 first: it is the single largest weighted-spend lever and changes how
> every later finding's own implementation gets routed.

### 12. Opus routing is ~54% of weighted spend and its depth went unused — route sonnet-first, escalate on failure

- **Evidence:** 5 of today's 20 game-arch spawns went to opus (findings 1, 4,
  9, 13; plan step 5) totaling 370.9k raw tokens — 24% of raw but, weighted
  by model price (opus ≈ 5× sonnet per token), ≈54% of the loop's weighted
  spend (≈1.85M sonnet-equivalents of ≈3.4M total). Outcomes: all five landed
  first-pass with zero fail-first debugging cycles beyond what sonnet spawns
  handled elsewhere the same day (sonnet took the 69–81-tool multi-crate
  steps — findings 3, 7, 11, plan step 3 — at 141–179k each, zero misses).
  The routing rule sent them to opus on *anticipated* subtlety ("timing-
  sensitive", "engine-wide blast radius"); the anticipation never cashed out.
- **Ideal:** the router pays the opus premium only on demonstrated need: a
  step a cheaper worker already missed, or a Path that names an actual
  debugging activity (a failing timing test to diagnose, a race to find) —
  not adjectives about the code's importance. Sonnet is the default even for
  scheduler/netcode steps whose Path dictates the design.
- **Gap:** the current bracket prices fear, not evidence; today that fear
  cost ≈1.5M sonnet-equivalents of premium for zero prevented failures.
- **Tradeoffs:** *Wins:* ~5× cost cut on every reclassified step; today that
  was 5 spawns. *Losses:* a sonnet miss on a genuinely subtle step costs a
  follow-up (observed follow-up cost 59–106k) plus an escalated opus re-run —
  if sonnet's miss rate on ex-opus steps exceeds ~1 in 6, the savings invert;
  the miss also costs a user-visible hiccup and wall time (which the user has
  explicitly deprioritized).
- **Suggestion:** rewrite the opus bullet in `implement-finding/SKILL.md`'s
  routing list: opus = "a previous worker failed this step, or the Path names
  a concrete failing-behavior diagnosis (not code sensitivity)". `fable`
  keeps its current reserve wording.
- **Path:** (1) one bullet edit; (2) proof: next loop's ledger shows opus
  spawns ≤1 per 15 findings with follow-up rate on ex-opus-shaped steps ≤ 1
  in 6.

### 13. Every worker re-reads the report the orchestrator just read — embed the finding text in the spawn task

- **Evidence:** the spawn template commands "Read the finding's full section
  from that file first" — so all 20 workers today opened the 424-line audit
  (or 476-line plan) to extract their ~30-line section, and most read beyond
  it for context. The orchestrator had *already read the same section* to
  route the model (the skill requires it). The cheapest spawns measure the
  fixed boot cost this contributes to: 37.5–38.8k tokens for one-comment
  diffs (morning findings 2, 8, 9 — diffs under 10 lines each).
- **Ideal:** the routing read is reused: the orchestrator pastes the finding's
  verbatim section into the spawn task; the worker is told the section is
  complete and the report file is off-limits (its Evidence file:line pointers
  are what it explores, not the report).
- **Gap:** the same ~30 lines are paid for twice per finding — once in
  orchestrator context, once in worker context plus the worker's surrounding
  overread — ~20× per loop.
- **Tradeoffs:** *Wins:* kills one Read per spawn and the overread tail;
  bounds what a worker knows to exactly its finding (less drift). *Losses:*
  orchestrator output grows ~1–2k per spawn (net still positive); a worker
  that legitimately needs a neighboring finding's context (rare — queue
  dependencies are supposed to be resolved by ordering) must ask or miss it;
  the template change must keep the "execute its Path faithfully" framing or
  workers lose their contract anchor.
- **Suggestion:** edit `implement-finding/SKILL.md`'s spawn template (embed
  section, forbid report re-read) and `finding-worker.md`'s reading rules to
  match.
- **Path:** (1) two edits; (2) proof: next loop's cheapest spawn drops below
  ~30k and no worker transcript shows a report/plan Read.

### 14. Resume-corrections replay a whole transcript to make a 3-tool fix — small corrections get fresh minimal spawns

- **Evidence:** both corrections today were transcript resumes: the morning
  misplaced-file fix cost 59.4k (6 tools), and the game-arch step-4 orphan
  deletion cost 80.4k for 3 tool calls — a `Remove-Item`, a `cargo check`,
  and a scoped test run — because resuming replays the agent's full prior
  context (~65k) under the new instruction.
- **Ideal:** a correction whose instruction is self-contained (delete this
  file, rename this test, re-run this gate) goes to a fresh haiku spawn with
  a 5-line task; resumes are reserved for corrections that genuinely need the
  original working context (a design misunderstanding, a partial edit to
  continue).
- **Gap:** the correction channel costs 10–25× its work when the work is
  mechanical.
- **Tradeoffs:** *Wins:* ~60–70k saved per mechanical correction (today:
  2 of 2 qualified). *Losses:* a fresh spawn lacks the original's context —
  if the orchestrator's correction brief is wrong or incomplete, the fresh
  worker can't catch the discrepancy the way the original (which knows what
  it did) would; writing the self-contained brief is orchestrator output
  spend (~0.5k).
- **Suggestion:** a note in `implement-finding/SKILL.md`'s orchestrator
  rules: corrections enumerable in ≤5 lines with no dependence on the prior
  transcript spawn fresh (haiku unless the fix itself is subtle); otherwise
  resume.
- **Path:** (1) one note; (2) proof: next mechanical correction's ledger
  entry lands under 20k.

### 15. "Crosses crates" was judged three different ways — make the full-gate rule mechanical and worker-owned

- **Evidence:** today's cross-crate diffs split three ways: finding 7's
  worker ran the full workspace gate itself (correct); findings 5 and 8's
  workers argued domain containment and skipped it; findings 3, 6, 11's
  workers ran scoped suites only — so the orchestrator ran the full gate
  before committing, four times total. Each ambiguity costs an orchestrator
  decision turn, and the duplicated gates cost ~40s each (wall, cheap in
  tokens — the real cost is the judgment churn and inconsistent history).
- **Ideal:** one mechanical rule both sides apply identically: `git status`
  touches files under ≥2 workspace crate roots → the WORKER runs the full
  gate once before reporting; the orchestrator never re-runs a gate a worker
  reported green.
- **Gap:** a rule that needs judgment gets three interpretations across one
  loop.
- **Tradeoffs:** *Wins:* zero duplicate gates, zero orchestrator gate turns,
  consistent commit evidence. *Losses:* a strictly mechanical trigger
  full-gates some diffs a human would wave through (e.g. two crates' comment
  headers), costing ~40s wall each — accepted, since wall is the cheap axis.
- **Suggestion:** replace the conditional wording in `finding-worker.md`'s
  gate section with the ≥2-crate-roots trigger; add the "never re-run a
  reported-green gate" line to `implement-finding/SKILL.md`.
- **Path:** (1) two edits; (2) proof: next loop shows zero orchestrator-run
  workspace gates.

### 16. Audit sweeps read test-module bodies they don't judge — window them out

- **Evidence:** today's game-architecture audit (main context) read ~25
  source files end-to-end; in the ten with `#[cfg(test)]` modules, test
  bodies were ~37% of lines read (e.g. scheduler.rs 168 of 427, enemies/mod
  378→185, projectile 281→111, buff 228→106) and produced zero findings —
  every finding cited non-test code. Audit context is the most expensive
  context in the pipeline (longest-lived, feeds summarization).
- **Ideal:** audit sweeps read implementation to the end of the impl and stop
  at `#[cfg(test)]` unless the audit's scope is test quality (project-meta)
  or a finding needs a test as evidence — then window into it deliberately.
- **Gap:** ~a third of audit read volume buys nothing on most domains.
- **Tradeoffs:** *Wins:* ~30–40% less main-context read volume per audit at
  unchanged coverage of the code being judged. *Losses:* tests sometimes
  reveal intent the impl hides (a test named for a bug documents the
  invariant) — the auditor must notice the test module exists and choose,
  which is one more judgment; a lazy application could under-read genuinely
  test-relevant domains.
- **Suggestion:** one line in `audit-base.md`'s Method ("read impl; stop at
  test modules unless the domain or a specific finding needs them").
- **Path:** (1) one edit; (2) proof: next audit's read ledger shows test-body
  lines <10% of volume with finding count unaffected.

### 17. (user-decides) Micro-findings pay a ~40k spawn to land a <10-line diff — an inline orchestrator path would skip the boot entirely

- **Evidence:** the morning loop's smallest findings — one added sentence
  (finding 2, 37.7k), one comment block (finding 9, 37.5k), one gate-wording
  edit (finding 8, 38.8k) — each cost a full worker boot for a diff under 10
  lines: >100× the tokens of the change itself. The boot floor (~35–38k) is
  the irreducible cost of ANY spawn, however routed.
- **Ideal:** findings an audit tags "(micro)" — strictly enumerated, single
  file, no test to write beyond an existing gate — are applied by the
  orchestrator inline (Edit + the scoped gate), no spawn at all.
- **Gap:** the orchestrator-never-implements contract, which exists to keep
  the orchestrator's context clean and its judgment un-conflicted, currently
  has no floor below which its own overhead dominates.
- **Tradeoffs:** *Wins:* ~35k saved per micro-finding (3 today ≈ 105k).
  *Losses:* this erodes the contract that has kept the orchestrator honest —
  inline edits grow orchestrator context, skip the worker's independent
  fail-first discipline, and create a judgment call ("is this really micro?")
  exactly where the pipeline currently has none; a misjudged "micro" that
  needed a test lands untested. This is a real contract change, hence
  user-decides, asked at loop launch per finding 11's convention.
- **Suggestion:** if adopted: audits may tag "(micro)" under stated criteria;
  `implement-finding/SKILL.md` gains the inline path for tagged findings
  only. If declined: strike this finding; the boot floor stays the price of
  the clean contract.
- **Path:** (1) user decision; (2) if adopted, two edits; proof: micro-tagged
  findings land at <5k tokens each in the next loop's ledger.

## Carried forward from previous report

None — first run of this audit (the addendum extends the same day's report).

## Resolved since last report

First-pass findings 1–11 all landed the same day (2026-07-15): the repo-file
changes in commits `b77b46f` (dev profile, nextest header, lint-comments
script) and the `.claude/` config edits for gates, routing, contracts, and
conventions. Rework 1 (parallel execution) had its gate met by the
game-architecture serial loop and was **declined by the user** — token spend
outranks wall time; recorded in the reworks file and the skill's ordering
axis.

Worth recording as *working as designed* from the second pass's ledger: zero
baseline re-establishments (the "HEAD green at N/N" line held across all 20
spawns); scoped gates held (full runs only on cross-crate diffs and
loop-final); the widened haiku bracket held for plan-dictated steps (2 of 3
haiku spawns clean, the miss was a rename-leaves-orphan, not a routing
error); worker reports stayed ~15–35 lines (cap mostly held, no compaction
events this loop); and the first-pass findings' own implementation loop ran
at 651.8k tokens for 11 findings — cheaper per finding than the game-arch
loop, consistent with its smaller diffs.
