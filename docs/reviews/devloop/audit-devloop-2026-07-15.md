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

## Carried forward from previous report

None — first run of this audit.

## Resolved since last report

None — first run. Worth recording as *working as designed* from today's
telemetry: zero retry loops (the 5-run stability cap and probe rules held);
Reads/Greps at 0.6% of tool time across 908 calls (read windows held); cache
serving 311.8M of 311.9M input tokens; the two-tier plan-absorbs-thinking
design showing up as haiku executing planner-dictated extractions cleanly.
