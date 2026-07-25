# Dev-Loop Audit (Reworks) — 2026-07-25

Rework-scale companion to `audit-devloop-2026-07-25.md`: findings that need a
design pass before anyone writes code. Consumed by `/plan-rework`.

Both entries here come from the cross-cutting findings of
`research-agentic-loop-techniques-2026-07-25.md` — the two places where the
evaluation found a gap that no clause-sized edit can close.

## Ideal end state

The loop meters quality as well as cost. Today `scripts/token-report.ps1` and
the per-loop ledgers measure spend and nothing measures outcome, so every
finding has to invent its own one-shot proof metric and no two campaigns are
comparable. In the end state one script emits a per-campaign outcome vector that
`audit-devloop` reads instead of recomputing a hand census, a campaign can be
compared against the one before it, and a proposal to change the loop can be
judged against a number rather than an argument.

## Findings (implementation order)

Cross-type queue (mirrored verbatim from `audit-devloop-2026-07-25.md`):

> **~~finding 1~~ → ~~finding 2 (micro)~~ → ~~finding 3~~ → ~~finding 4 (user-decides)~~ →
> ~~finding 5~~ → ~~finding 6 (user-decides)~~ → ~~finding 7~~ → ~~finding 8 (micro)~~ →
> ~~rework 1 (user-decides)~~ → ~~finding 9~~ → ~~finding 10~~ → ~~finding 11 (micro)~~.**
> findings 1–8 done 2026-07-25 (`.claude` commits `c3884c5`..`8eee577`; finding 5
> has no commit — all its targets are unversioned — and measured 24→22 lesson
> notes, index −30%, 2 absorbed strikes, 2 merges).
> rework 1 done 2026-07-25 (plan-devloop-rework-1-2026-07-25.md, 8 steps,
> loop-final gate RED: 356 passed, 1 failed. The failure is
> `vordar-game::content_lint prop_material_matches_surface_class`, caused by a
> concurrent session's uncommitted `content/models/` files; this plan's six
> commits touch no Rust, WGSL or `content/`, so it is not attributable to the
> rework. premise-falsified: step 7's "diff removes ≥3 lines from the `:27`
> bullet" check, unsatisfiable because that file is one line per bullet;
> premise-falsified: step 6's expectation that devloop-2026-07-17 carries suite
> counts, that report having no `gate N/N` token at all).
> findings 9–11 done 2026-07-25 (`93bb011`, `bc3055e`; `.claude` `165a69c`,
> `4c00c6d`, `94efad6`; loop-final gate 422/422, which also clears the rework-1
> loop's red `content_lint` and confirms it was never attributable here).
> Trial baselines: finding 9 gate zero = 1184 anchors across `docs/reviews/`, of
> which 29 are genuinely stale and all 29 sit in closed pre-07-17 reports, so by
> user ruling it was narrowed to path-bearing anchors on one report at a time and
> its "names a measurable" check was cut as an unfixable keyword heuristic;
> finding 10 = 0 false positives over 427 tests with an empty allowlist, covering
> only the "asserts only literals and consts" half of its Gap, since "never calls
> a workspace crate path" needs type resolution a regex cannot supply.
> rework 2 is **parked**: its gate is a per-audit read-volume baseline that does
> not exist, takeable free on the next audit.
>
> Findings 1–4 close the durability holes: a stop that records its reason, a
> ceiling that can stop a runaway step, a policy for a worker that dies after
> editing. 2 goes early because it is one sentence and it forecloses a whole
> class of future finding. 5 must land as one batch — its four parts touch the
> same file and conflict if split — and 6 depends on it, because the eviction
> contract is what stops widened sourcing from inflating the index. 7 and 8 are
> the instrument and the wiring: 7 gives the audit a failure vocabulary, 8 gives
> the planner the error register it already has but cannot see. Rework 1 is the
> highest-leverage item in the source document and the only one needing a design
> pass; it goes before the trials because it is what would make their proof
> metrics real rather than anecdotal. 9–11 are trials, each gated on its own
> measurement — and **finding 9's gate zero costs nothing and can be taken at any
> time**, including before the queue starts.

### 1. (user-decides) The loop meters cost but never quality — a per-campaign outcome vector

- **Evidence:** four independent cluster reads in the source evaluation arrived
  here from different directions, and the gap is concrete in three places.
  (a) `scripts/token-report.ps1:1-7` is a **weekly Task Scheduler job**
  (`VordarTokenReport`, Sun 09:00) producing `docs/tokens/<date>.md` from
  `ccusage weekly --json` — week-granular and account-wide, with no per-campaign,
  per-loop or per-spawn attribution. Its exit code reflects whether the report
  was written, not whether spend was acceptable. `docs/tokens/` currently holds
  **one file** (`2026-07-23.md`), so there is not yet a time series to draw a
  before/after from.
  (b) Consequently `run-queue/SKILL.md:76-77` ("report the loop's tokens; name
  any outlier spawn") and `:88-90` (the campaign aggregate: tokens per loop and
  total, commit count, suite count first→last) have **no script behind them** —
  the orchestrator derives those by hand, every campaign.
  (c) `audit-devloop/SKILL.md:39` concedes the quality half outright by requiring
  every finding to invent its own bespoke one-shot proof metric — which is
  precisely a metric that cannot rank two candidate versions of the loop against
  each other.
  The cost of not having it is measured: **2 of 16 findings (12.5%) in the
  2026-07-17 campaign had mechanisms that measurement later contradicted**
  (`reworks-devloop-2026-07-17.md:105-115` and `:268-302`), and the audit finding
  is the only artifact in the loop with no oracle.
- **Ideal:** one script emits a per-campaign outcome vector, and the numbers in
  it are comparable across campaigns. `audit-devloop` reads it instead of
  recomputing a hand census. A proposal to change the loop is judged against the
  vector's movement.
- **Gap:** the loop has a denominator and no numerator. It can say what a
  campaign cost and cannot say whether the campaign was any good.
- **Tradeoffs:** three, and they are why this is a design pass rather than a fix.
  (a) **Scope is genuinely open.** The source document says only "a *scalar,
  comparable, cheaply re-evaluable* quality score over pipeline outcomes" and
  names `token-report.ps1`, the per-loop ledgers and `cargo nextest` counts as
  the denominator it would sit on. It states **no field list and no emission
  schema** — inventing one here would be exactly the unfounded specificity this
  document exists to catch, so the design pass has to derive it.
  (b) **Ruling 5 pulls against automating worth.** An automated fitness function
  is a machine deciding worth, and the source's cluster 5 records what happens
  when a self-improving loop optimizes a gameable objective. The vector must
  inform the user's judgement, not replace it — which constrains the design more
  than it looks.
  (c) **A metric can pass while the thing is broken** — the loop already has two
  lessons saying so (`tasks/lessons/2026-07-21-metric-must-detect-failure.md`,
  `2026-07-25-metric-cleared-picture-did-not-move.md`). A vector that cannot
  detect a bad campaign is worse than no vector, because it launders a bad
  campaign as a good one.
- **Suggestion:** a design pass that answers, in order: what quality dimensions
  are already recorded in artifacts the loop produces anyway (commits, struck
  queue notes with strike reasons, gate results, correction counts,
  plan-premise falsifications, findings-carried-forward ratios); which of those
  are cheap to extract deterministically; what the script emits and where it
  writes; and what `audit-devloop` deletes from its own method once the script
  exists. **The last question is the one that makes this worth doing** — if
  nothing gets deleted, this is an addition, and ruling 7 says an addition that
  subtracts nothing is worth less than one that does.
- **Path:**
  1. **DECIDED 2026-07-25 at the rework pause: inform-only.** A vector is wanted;
     it never gates. Plan steps 1–8 land
     (`plan-devloop-rework-1-2026-07-25.md`, committed `b187b47` — the approval
     pins to that SHA); the plan's conditional step 9 (consistency exit code) is
     **not built**, and the script's exit code means only "a vector was produced".
     No field may block a commit or a queue advance: under ruling 5 the vector
     informs the user's judgement and never substitutes for it.
  2. Design pass (`/plan-rework 1`): enumerate the outcome dimensions already
     recoverable from committed artifacts at zero marginal cost; reject any that
     need a transcript, since `.claude/CLAUDE.md` §9 destroys conversation state
     at every phase gate.
  3. Design the emission: one script, one output location, one format
     `audit-devloop` can read without re-deriving.
  4. Name what it makes obsolete in `audit-devloop/SKILL.md` — specifically the
     hand census at `:27` ("re-derive this every audit") and the per-finding
     bespoke measurable at `:39`, at least in part.
  5. **Measured before/after:** the campaign census currently recomputed by hand
     each audit is emitted by a script instead; and the 12.5% mechanism-error
     rate becomes a tracked series rather than a number someone counted once.
     Until this lands, that rate is the baseline any future verification proposal
     must move.

### 2. PARKED — Amortized cross-audit map

**Gate:** no baseline for `audit-*` read volume exists, so the break-even (map
refresh cost × campaigns vs read savings × 8 domains) **cannot be computed**.
The measurement is free — take it on the next audit from the transcript ledger —
and this entry gets a queue position only once it exists. Parked entries are
skipped and named at launch (`audit-base.md:52-54`, `run-queue/SKILL.md:29-31`).

- **Evidence:** eight `audit-*` skills sweep one 195-file / 41,759-line workspace
  (excluding `reference/` and `target/`), each rediscovering the same structure
  cold. `audit-devloop/SKILL.md:29` already directs the auditor to hunt for
  "what each spawn re-derives cold that a cheap artifact could hand it" — and
  **two audits have run under that clause and neither filed a map finding**,
  which is weak evidence the payoff is not obvious.
- **Ideal:** one maintained map, built once and read by all eight consumers, so
  the build is amortized across eight domains rather than repeated per sweep.
  This is a genuine reduction in total tokens, not a time-shift.
- **Gap:** unquantified in both directions. Nobody knows what an audit currently
  spends on file reads.
- **Tradeoffs:** **a stale map is worse than no map** — it would feed audits
  wrong `file:line` anchors, and `audit-base.md:18` makes concrete evidence
  non-negotiable, so a confidently wrong anchor corrupts the artifact the whole
  loop runs on. 65 commits landed in one campaign, so refresh cadence is the
  design's hard part, not its detail. Second: a map that duplicates rather than
  indexes the code is a second source of truth, which is an erasure violation.
  Third: the map itself is analysis, so ruling 3 puts its refresh pass on
  fable/opus at premium rates — the refresh is not free.
  **Explicitly rejected, and not to be revived with this entry:** the sleep-time
  / precompute-the-boot framing it came from. Under ruling 1, moving tokens to a
  different hour of the same weekly budget is not a win; and the boot itself is
  not compressible — our authored share of the 35–38k spawn boot is ~5k (~14%),
  the rest being harness prompt and tool schemas, already minimized by
  `ENABLE_TOOL_SEARCH` and served at a **0.9749 prefix-cache hit rate**. The one
  real idle cost that exists — 9 measured cache cliffs per week, up to ~734k
  write-priced tokens each at the orchestrator's peak context — is fixed by
  CLAUDE.md §9 compacting *before* the gap, a ~20× reduction requiring the
  adoption of nothing.
- **Suggestion:** take the baseline first. It is free, it is one number, and it
  decides whether there is anything here.
- **Path:**
  1. On the next `audit-*` run of any domain, record cache-create tokens
     attributable to file reads, from the transcript ledger. Zero extra spend.
  2. Repeat across ≥3 domains, since the amortization argument depends on the
     spread across consumers, not on one domain's number.
  3. Only if the summed read cost plausibly exceeds refresh cost × campaigns:
     `/plan-rework` this, designing the map's build trigger, its staleness guard,
     and the `audit-base.md` clause telling auditors to read it first.
  4. **Measured before/after:** cache-create tokens attributable to file reads
     per `audit-*` run, across ≥3 domains, before and after the map exists. The
     map earns its refresh pass only if the summed drop exceeds refresh cost ×
     campaigns.

## Carried forward from previous report

None. Every rework in `reworks-devloop-2026-07-17.md` is struck. That pair is
retained rather than deleted — see the header of `audit-devloop-2026-07-25.md`
for the recorded divergence from `audit-base.md:97-108`.

## Resolved since last report

Not applicable — no prior rework was open to resolve.
