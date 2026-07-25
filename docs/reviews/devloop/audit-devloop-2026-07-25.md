# Dev-Loop Audit — 2026-07-25

Not a sweep. This report converts the decision queue of
`research-agentic-loop-techniques-2026-07-25.md` — 41 published techniques
evaluated against this loop, then six of them re-read paper-by-paper in pass two
— into findings `/run-queue` can walk. Every finding below cites its source
entry in that document; the evidence chain runs report → research entry →
primary source.

**Divergence from `audit-base.md:97-108`, recorded rather than silent:** the
2026-07-17 pair is *not* deleted. It is fully struck (all 10 findings and all
6 reworks done) and would normally be superseded, but it is the telemetry corpus
of record — 76 subagent transcripts, the 180k mega-step, the 529 storm — and both
the research document and the findings below cite it by line about fifteen times.
Deleting it would orphan those anchors to git history. It stays until a real
devloop sweep re-derives its measurements.

Findings editing `.claude/` files are local-only (gitignored, pushed to
TycheDea/ClaudeConfig); the report is the committed artifact. Every finding
carries a **Tradeoffs** bullet; the user decides adoption.

## Ideal end state

Every mechanism in `.claude/` is either the best published option for its job or
a deliberate, recorded divergence from it — and the loop knows which, because the
reason is written down where the next sweep will find it. A stop is resumable
from files alone, including *why* it stopped. A step has a ceiling it can trip.
The lessons index is the one memory tier with an eviction contract rather than
the one without. And the loop's own failure modes are named from a published
taxonomy instead of re-invented one incident at a time — while every number that
taxonomy carries is restated against its own denominator before it is trusted.

## Findings (implementation order)

Cross-type queue (mirrored in `reworks-devloop-2026-07-25.md`):

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

### 1. Record the failure *reason* in the queue note on every stop

- **Evidence:** `run-queue/SKILL.md:79-86` names four stop conditions and closes
  with the claim that "Every stop leaves the workspace green and the queue note
  accurate, so a later 'run the queue' resumes from the files alone" — but what
  the note records is **position** (`:29-31`: "your position is the first
  unstruck entry"), never failure type, never what was already attempted. The
  rest lives in the conversation that `.claude/CLAUDE.md` §9 orders you to
  `/compact` or `/clear` at the next phase gate. The loop has a precedent for
  what that costs: the 529 storm at `audit-devloop-2026-07-17.md:194-225` burned
  45 minutes and 8 dead spawns, and none of the diagnosis survives outside that
  report's prose.
- **Ideal:** a stop is fully resumable from files alone — the resuming session
  knows which item, at which tier, failed how, against what already-tried, and
  what must be true before a retry is worth spending a spawn on.
- **Gap:** writing it costs **~150–300 output tokens, once per stop** (roughly
  once per campaign). Not writing it costs, on the first blind re-attempt after a
  `/clear`, one wasted spawn boot (**35–38k**) plus up to a median step's output
  (**27k**) ≈ **60k**. That is a ~200× return on the first avoided re-attempt,
  and it is the cheapest item in the whole source document.
- **Tradeoffs:** a **stale line** is the new failure mode — if the user fixes the
  blocker by hand and does not clear the note, the next run skips or mis-routes
  an item that is now fine. The mitigation is not a flag but the mechanism
  already in the contract: the resume must **re-verify the named gate** before
  honoring the line, exactly as `audit-base.md:24` re-verifies every
  carried-forward finding. Cost of that re-verification is one gate run, which
  the loop was going to pay anyway.
- **Suggestion:** import H-RePlan's Cross-Layer Failure Envelope (research doc
  7.1, arXiv:2606.20487) as five fields on one line, not as a subsystem: the
  item, the model tier used, the failure category, what was already attempted,
  and the gate that must clear before a resume retries it. No new agent, no new
  script, no new file.
- **Path:**
  1. `run-queue/SKILL.md`, Stopping section (`:79-86`): add the requirement that
     every self-initiated stop appends one such line under the queue note **in
     both report files** (the note is mirrored, per `audit-base.md:47-55`).
  2. Same section: add that a resume re-verifies the named gate before honoring
     the line, and clears the line once the gate is green.
  3. `audit-base.md:47-55`: extend the queue-note convention to name the stop
     line as part of the note's contract, so an audit writing a fresh report
     carries it.
  4. **Measured before/after:** re-spawns of an item whose stop line names an
     unmet gate. Baseline: structurally 1 wasted 35–38k boot per resumed stop,
     currently unmeasured because the information does not exist. Target 0.

### 2. (micro) Write down "one routing level, never two"

- **Evidence:** the loop already runs flat routing and has never said so.
  `audit-base.md:1-8` is a shared body pulled in by an invoked audit skill — a
  flat include, not a router; the eight `audit-*` skills sit in context as
  descriptions only (~29.9KB of body that never loads until invoked); and
  `ENABLE_TOOL_SEARCH` is on. Routing depth between an invoked skill and the text
  it acts on is **1, across all 11 project skills**. Nothing records that this is
  a decision rather than an accident.
- **Ideal:** the property is written down once, so the next plausible-sounding
  proposal to add a routing layer is struck on sight instead of re-argued.
- **Gap:** the tempting violations are concrete and close at hand — a meta audit skill that
  routes to the eight `audit-*` skills, or a repo-map skill routing to per-crate
  sub-skills. Research doc 2.3 (arXiv:2607.17598) measures that shape at up to
  **−0.27 absolute accuracy**: En.MC 0.9126 flat vs **0.6398 hierarchical**, with
  Claude Code among the three harnesses tested. It is not a neutral refactor.
- **Tradeoffs:** the rule only forbids, so it forecloses a design we might
  otherwise want — specifically any future index-the-codebase-into-skills work.
  Priced honestly: at our corpus size (195 Rust files / 41,759 lines excluding
  `reference/`) we sit at the single-document end of that paper's curve, where a
  skill-based map is predicted neutral at best. Losing it costs nothing measured.
- **Suggestion:** one sentence in `audit-base.md`. Not a new file, not a new
  section — it belongs with the other contract clauses.
- **Path:**
  1. Add to `audit-base.md`: "Exactly one routing level between an invoked skill
     and the text it acts on; never a skill whose job is to select another skill."
  2. **Measured before/after:** routing depth stays at 1 (today: 1, verified
     across all 11 skills). If a repo map is ever proposed, the flat variant is
     the only one to be built.

### 3. Budget ceiling and an `exhausted` terminal state

- **Evidence:** `run-queue/SKILL.md:79-86` names four stop conditions — a worker
  failure surviving one correction round, a gate red after one rerun, the API
  path down, a plan raising an unanswered question. None of them is a budget.
  The only budget bound in the loop is static and planner-side
  (`rework-planner.md:56-64`: split at pass boundaries, keep workers in the
  ~100k band, citing the measurement directly) — **the worker at runtime has no
  ceiling it can trip.** The measured consequence: one step ran to **180k output
  (2026-07-16)**, 7× the 27k campaign median, with no mechanism that could have
  stopped it.
- **Ideal:** the loop's stop conditions are one named terminal-state set, and a
  runaway step halts itself instead of running to completion.
- **Gap:** two independent sources converge on the same missing piece — research
  doc 1.1 (arXiv:2607.00038) supplies the terminal-state vocabulary (success,
  clean no-op, blocked, stalled, **exhausted**, with "an error or an exhausted
  budget never counts as success"), and 1.5 BAGEN (arXiv:2606.00198) supplies the
  deterministic half of the argument. Our four existing conditions already map
  onto *blocked* and *stalled*; only *exhausted* is absent. A 100k ceiling on
  that one 180k outlier saves ~80k — at ~1 outlier per ~16-item campaign against
  283k–1.33M per loop that is ~0.5–2% of campaign spend, but the arithmetic is
  one-sided because the ceiling has **no running cost**.
- **Tradeoffs:** two real risks. A ceiling that fires mid-diff leaves a
  half-landed step, so it must **halt-and-report and never commit** — which is
  what every other stop condition already does. And a ceiling set too low turns
  legitimate large steps into spurious stalls, re-spending the 35–38k boot on the
  retry. The second is the reason to set it at the planner's existing ~100k band
  rather than at the median. **Explicitly rejected:** BAGEN's other half, where
  the model forecasts its own remaining budget — measured at 47% interval
  coverage even after training, i.e. wrong more than half the time, and a
  model-estimated stop signal sitting beside a deterministic gate is the parallel
  path ruling 6 forbids.
- **Suggestion:** one clause in the spawn prompt, one terminal state in the
  Stopping list. Adopt once, from both sources, not twice.
- **Path:**
  1. `run-queue/SKILL.md:79-86`: add *exhausted* as a fifth stop condition, and
     recast the four existing ad-hoc conditions as one named terminal-state list
     (this is the only erasure available here — four bullets become one set).
  2. `implement-finding/SKILL.md:98-107` (the verbatim spawn template): add a
     clause telling the worker its output ceiling and that overrunning it means
     stop-and-report, never push on and never commit.
  3. **Measured before/after:** worst single-step output across a full campaign.
     Baseline **180k (2026-07-16)** against a 27k median; target: no step exceeds
     its declared ceiling.

### 4. (user-decides) Post-edit-death policy, and pin the go/stop to a plan-file SHA

- **Evidence:** two holes in an otherwise complete durability story.
  (a) `implement-finding/SKILL.md:62-69` handles a spawn that dies to a 5xx
  **pre-edit** — probe with a 1-turn haiku task, downgrade tier or stop. A spawn
  that dies **after** it has edited files has **no policy anywhere in `.claude/`**;
  the only revert/checkout/stash mention in the whole tree is at
  `implement-finding/SKILL.md:88`, which is the micro-inline path, not a worker
  death. (b) `run-queue/SKILL.md:60-70` has the planner write a plan file, shows
  it, and pauses for one go/stop — but the plan file is **not committed at that
  point**, so the approval is pinned to nothing. Together these are why
  `run-queue/SKILL.md:85-86`'s claim that every stop leaves the workspace green
  is true only for stops the loop *chooses*, not for stops the API imposes.
- **Ideal:** a worker death at any point leaves a defined workspace state, and
  every rework approval names the exact plan text that was approved.
- **Gap:** both are unbounded today. The loop's own precedent for improvised
  recovery is the 45-minute / 8-dead-spawn 529 storm
  (`audit-devloop-2026-07-17.md:194-200`) — call it ~300k of avoided flailing on
  a single recurrence.
- **Tradeoffs:** an automatic discard-to-last-green could **destroy a
  partially-correct expensive diff** — on a 180k-class step that is real money.
  Handing the dirty tree to the user instead costs an attention round-trip and
  leaves the queue note describing a workspace that is not green, which breaks
  the resumable-from-files property finding 1 is strengthening. Both costs are
  real, which is why this was put to the user — and the answer removed the
  choice rather than settling it; see Path step 1.
- **Suggestion:** name the choice explicitly in the skill rather than saying
  "recover" and leaving it to judgment. Research doc 7.3's layer-5 practice
  (deliberate crash drills that kill a worker mid-edit) is **rejected** as
  disproportionate at solo-developer scale.
- **Path:**
  1. **DECIDED 2026-07-25 at loop launch — neither (a) nor (b).** The user ruled
     that a non-critical event must never interrupt them and that the orchestrator
     decides stop-vs-respawn. That answer is only implementable because preserving
     the diff first makes discarding non-destructive, which is what the two
     original options were trading against each other. The rule to encode:
     on a spawn death after edits, `git status --short`, then
     `git stash push -u -m "dead-spawn <item>"`, then discard to the last green
     commit and respawn — no user contact. Stop and hand over **only** when the
     same item dies post-edit a second time in one loop: that is a loop, not an
     incident, and respawning a third time burns tokens against an unchanged
     cause. Stash refs are named per item so a salvage is possible later; nothing
     is ever destroyed, so no judgment call about diff value is required.
  2. `implement-finding/SKILL.md:62-69`: extend probe-and-override with the
     post-edit case, encoding the answer to (1) as a stated rule.
  3. `run-queue/SKILL.md:60-70`, rework substep 2: commit the plan file **before**
     the pause, so the go/stop is pinned to a SHA.
  4. **Measured before/after:** loop stops requiring improvised workspace cleanup.
     Baseline: policy undefined, so every post-edit death is improvised (n=0
     observed to date, but the 529 storm proves the class recurs). Target 0
     improvisations, plus 100% of rework go/stop gates having a plan-file SHA in
     the approving commit.

### 5. The lessons batch: eviction contract, resolution status, note schema, merge rubric

- **Evidence:** `tasks/lessons/` is the one memory tier in this project with no
  eviction contract. **23 files, 0 ever deleted in 44 days, growing ~3/day**, and
  `/tasks/` is gitignored (`.gitignore:39`) so there is no history either. Two
  entries are already absorbed by harness mechanisms and have no retirement path:
  `2026-07-24-never-double-run-a-suite.md` is enforced by `finding-worker.md:95-98`
  ("at most ONCE … never run a full suite twice back-to-back"), and
  `2026-07-22-orchestrate-never-implement.md` by `implement-finding/SKILL.md:6-9`.
  Two more overlap outright — `2026-07-21-metric-must-detect-failure.md` and
  `2026-07-25-metric-cleared-picture-did-not-move.md` both say a metric can pass
  while the thing is broken — and the loop has no dedup step. Meanwhile
  `~/.claude/CLAUDE.md` requires reviewing the index at session start, so this is
  a cost paid **every session, forever**.
- **Ideal:** the index is skimmable, each entry states when it fires and where it
  stops, an entry a mechanism has absorbed can be struck, and two entries saying
  the same thing merge instead of both being carried.
- **Gap:** unpruned growth in exactly the place the global rules say to hunt for
  it. The fix reuses `audit-base.md:24` (carry forward, re-verify, drop resolved)
  and `:97-108` (delete superseded) **verbatim — no new system**.
- **Tradeoffs:** ceremony is the standing objection to a schema on a 500-byte
  note, and it is why this lands at **four fields rather than five**. `∂` written
  during a correction turn is also anecdotal — the source's boundaries came from
  cross-task canonicalization over many traces, ours comes from n=1, so expect it
  too narrow at first and expect at least one lesson to under-fire before the
  boundary is widened. Both failures are visible in the note text and cheap to
  reverse.
- **Suggestion:** four parts, **one batch** — they touch the same file and
  conflict if split (research doc cross-cutting finding B says so explicitly).
  From EvoAgentBench (5.5, arXiv:2607.05202) take the vocabulary and the merge
  rubric, **not** the headline: pass two established the headline is an
  oracle-routed upper bound the paper itself calls "not a deployable method", and
  the schema is never ablated. Drop `ρ` (role): its only validated use is making
  an automatic merge over 170 units decidable, and we merge 23 notes by hand — a
  field with no consumer is the ceremony ruling 7 forbids.
- **Path:**
  1. `~/.claude/CLAUDE.md:14`: name the four note fields — **γ (when) / π (rule) /
     ∂ (does not apply when) / ℰ (what happened)**. π and ℰ already exist in our
     notes as "**Rule:**" and "**What happened:**" blocks, so this is a two-field
     addition, not a five-field imposition.
  2. `tasks/lessons.md`: give each entry a **three-state** status — persistent /
     quiet / absorbed-by-mechanism (naming the enforcing `file:line`). Only
     *absorbed* is strikeable; *quiet* is decay, not death.
  3. `tasks/lessons.md`: **the index line becomes γ + link**, replacing the prose
     one-liner. This is the erasure that pays for the batch — today each rule is
     written twice, once in the note and once in the index, and it drifts. If γ
     were added *alongside* the one-liner, this finding would make the only
     recurring cost larger.
  4. Add the eviction contract to `~/.claude/CLAUDE.md`'s lessons convention,
     reusing `audit-base.md:24`'s wording: re-verify at session start, drop
     resolved, strike absorbed.
  5. Apply the merge rubric once across the 23 notes (same role, compatible
     triggers, equivalent procedures, same correction target, compatible
     boundaries; shared topic or lexical overlap explicitly insufficient) and
     land the merges it finds.
  6. **Measured before/after:** index entries struck as absorbed-by-mechanism —
     today **0**, with **2 immediately strikeable** (named above) plus the metric
     dedup as a third; and merge rate, predicted ≥1 pair (live candidate:
     `2026-07-25-sonnet-implements-never-analyzes.md` against
     `2026-07-22-delegate-cheap-execution.md`, where the first already
     hand-writes a boundary against the second).

### 6. (user-decides) Widen lesson sourcing to loop-recorded failures, capped

- **Evidence:** `~/.claude/CLAUDE.md`'s Workflow bullet 4 triggers on "after ANY
  correction from **me**" — so the only failures that ever become lessons are the
  ones the user personally caught. **0 of 23 lessons originate from a
  loop-recorded failure.** A worker's red gate, a reverted diff, a stop-and-report
  or an audit whose stated mechanism the measurement contradicted produces
  nothing, even when the contradiction is sitting in git:
  `reworks-devloop-2026-07-17.md:103-157` records a prescribed fix measured
  **inert** — "the deadline had >2x headroom in every failing run" — and no lesson
  exists about audits prescribing remedies against unmeasured failure modes.
  `reworks-devloop-2026-07-17.md:312-344` is a second instance. Separately, a
  worker that trips `clippy -D warnings` or a golden diff, fixes it and goes green
  produces a real execution signal that **nothing records**.
- **Ideal:** the loop learns from failures it can see itself, and proposes them
  for the user to accept — never writes standing rules unreviewed.
- **Gap:** the two rework-2/rework-5 cases are loop-quality failures the current
  convention structurally cannot see, because the user never had to correct them.
- **Tradeoffs:** this is the one finding here with a **recurring cost** — one
  analysis-tier spawn per campaign, ~45–53k (35–38k boot + ~10–15k of input, and
  the input is git, not transcripts, because §9 destroys conversation state at
  every phase gate). Against a campaign's 1,993k subagent output that is ~2.5%.
  Break-even is a coin flip, not an expectation: the 180k outlier is a 153k
  excess, so three campaigns of the pass (150k) pay for **one** prevented
  outlier, and nothing measures how often it would prevent one. The second cost
  is **lesson inflation** — a pass that runs every campaign will find *something*
  every campaign. That is why the cap, the dedup, the suppressor and the status
  marker must all land with finding 5 and not after. Third: a self-mined lesson
  has no user correction behind it, so a wrong one becomes a rule nobody vetted;
  and asked to judge causality with no gold reference, the pass will produce a
  plausible narrative regardless — hence the artifact requirement.
- **Suggestion:** widen the trigger only to failures that left a **durable
  artifact**, cap the output, and make the pass propose rather than write.
  SENTINEL's twelve-category taxonomy is **rejected**: pass two found the
  categories are a hand-written list inside a prompt, prefixed "Failure
  categories to consider include", and the authors' own failure analysis
  abandons them for four different labels. Importing it as a checklist is the
  shim ruling 6 forbids.
- **Path:**
  1. **DECIDED 2026-07-25 at loop launch: adopt in full.** The user took the
     recurring ~45–53k end-of-campaign pass together with the trigger widening,
     accepting that break-even rests on a prevention rate nothing yet measures.
     Steps 2–4 all land. Step 5's recurrence metric is what will eventually
     retire or confirm that acceptance.
  2. `~/.claude/CLAUDE.md` Workflow bullet 4: change the trigger to "after ANY
     correction from me, **or after any loop failure that left a durable
     artifact** — a gate that was red and stayed red through a fix round, a commit
     that was reverted, or a queue note struck as inert, no-op or not-reproduced",
     and require that **every lesson proposed from a loop failure cites its
     artifact — a SHA, a gate record, or the struck note line. No artifact, no
     lesson.** Note what this deliberately excludes: "a finding whose measurement
     contradicted its own premise" stays a category the user may accept, but is
     not a trigger, because it has no gold reference and would have the pass
     free-forming.
  3. `run-queue/SKILL.md:88-90` (End of queue, where the campaign aggregate is
     already reported): add one analysis-tier spawn (fable, or opus when fable is
     unavailable — ruling 3) that reads the campaign's commits, reverts, struck
     queue notes and per-step gate results, **including the clean ones**.
  4. Its output contract: **at most 3 proposed lessons, and zero is a valid and
     expected result**; each citing its artifact and naming a near-duplicate among
     the existing entries or explicitly stating there is none; plus a
     **suppression line** naming the areas whose gates were clean, in which the
     pass is forbidden to propose; plus a strike list of entries whose class did
     not recur and which a named harness `file:line` now enforces.
  5. **Measured before/after:** lessons whose origin is a loop-recorded failure —
     today **0 of 23**. The target is a **cap, not a floor**: ≤3 proposals per
     campaign. Secondary metric: recurrence rate — tag each lesson with a failure
     class and measure whether that class recurs in later campaigns.

### 7. Give `audit-devloop` a failure vocabulary — HORIZON's definitions, not its numbers

- **Evidence:** **no file under `.claude/` contains a failure taxonomy** or any
  classification of *why* a run failed. Every counter the loop owns was derived
  one incident at a time, and they cluster in the design-level categories —
  `rework-planner.md:71-74` (repeat context rather than reference it),
  `rework-planner.md:56-64` and CLAUDE.md §9 (context economy),
  `run-queue/SKILL.md:28-33` (durable state). The process-level categories,
  **planning error chief among them**, have zero instrumentation. Worse,
  `audit-devloop/SKILL.md:28` pre-attributes every observed stall to the rule
  ("each is evidence the rule, not the worker, is wrong"), which **forecloses
  plan-attribution before the audit starts**.
- **Ideal:** the audit names failures from a published vocabulary with stated
  boundaries, and can order a finding by failure composition rather than only by
  token weight — which is the loop's first non-token metric.
- **Gap:** plan quality is not measured anywhere, so the ordering axis
  (`audit-devloop/SKILL.md:13`: token spend → user attention → wall time) cannot
  rank a plan-quality fix at all. The falsification baseline is already
  computable at **zero tokens** from committed files: the 2026-07-17 campaign
  shows **at least three premises falsified by execution** — audit finding 8,
  reworks finding 3, reworks finding 4.
- **Tradeoffs:** a taxonomy invites forcing every incident into a box, and pass
  two found the source's validation is a **pilot at n=40 against one annotator
  with the calibration procedure never described**; its documented ceiling is
  κ=0.61 between two experts who wrote the taxonomy. So our labels carry no
  measured agreement and must ship marked unvalidated. Two subtler traps, both
  from pass two: the source's only published decision rule routes ambiguity into
  planning error, so an instrument that inherits it **confirms our own hypothesis
  regardless of the truth**; and planning dominates hardest in the source's
  domains where the agent's only output *is* a plan, which is exactly
  `finding-worker`'s shape. Cost is ~+2–4k output on an audit that already reads
  the transcript corpus — labelling adds output, not reads.
- **Suggestion:** take the definitions and the boundary discriminators; refuse
  the headline. Pass two established the source's **72.5% / 27.5% split does not
  reconcile with its own appendix** (14.75% weighted by model), appears once in a
  figure caption whose denominator is wrong, and swings 20.8% → 6.6% between two
  models on one corpus.
- **Path:**
  1. `audit-devloop/SKILL.md:25-34`, first "What to hunt for" bullet: add a
     labelling clause carrying the seven category definitions (process-level:
     environment error, instruction error, planning error, history error
     accumulation; design-level: catastrophic forgetting, memory limitation,
     false assumption) plus the two boundary discriminators the source actually
     supplies — environment error vs false assumption ("the external world
     changes" vs "the agent's incorrect prior belief about how the environment
     should behave"), and catastrophic forgetting vs memory limitation
     (constraint **still in context but not attended to** vs **exceeded effective
     memory capacity**).
  2. Same clause, and this is the load-bearing part: **invert the residual.**
     Where a proximal cause fits more than one of {planning error, catastrophic
     forgetting, history error accumulation, memory limitation} and no evidence
     discriminates, label it **`unattributed`** and quote the turning point —
     never default to planning error. Report the unattributed count as a
     first-class number.
  3. Same clause: **exactly one primary label per incident plus optional
     contributing labels** (the source advertises multi-label and practises
     single-label and never says which its judge did, so counts that do not sum
     are the default failure); an **evidence anchor** per label (transcript turn
     or `file:line`); **stratification by (audit skill, model routing)**; and an
     explicit ban on reporting a single process/design percentage, with the
     reason stated inline so a future reader knows it was deliberate.
  4. `audit-devloop/SKILL.md:28`: drop or qualify "the rule, not the worker, is
     wrong" so plan-attribution is admissible. Pair it with step 2 — removing one
     bias without the other just installs the opposite one.
  5. **Measured before/after:** lead with **plan-premise falsifications per
     campaign**, which needs no taxonomy, no judge and no calibration, against the
     computable baseline of **≥3** (2026-07-17). Success is at least one finding
     ordered by failure composition rather than by token weight.

### 8. (micro) Wire the error register into `rework-planner`

- **Evidence:** `tasks/lessons.md` *is* an error register, and the planner cannot
  see it. `~/.claude/CLAUDE.md` has the **main session** review the index at
  session start, but `rework-planner.md` never mentions `tasks/lessons.md`, nor
  the reworks file's "Tracked observations" section
  (`reworks-devloop-2026-07-17.md:398-412`) — so a freshly spawned planner designs
  every rework with **no access to the register at all**. Its only inputs are the
  finding's section and the code.
- **Ideal:** the tier that makes design decisions reads the record of past design
  mistakes before making new ones.
- **Gap:** the audit's own record shows **plan premises falsified by execution at
  ≥3 per campaign**, each costing a filed rework and in two cases a full
  attribution campaign (rework 3's 20-run pass, rework 5's 30-run pass). The
  cheapest of those far exceeds the injection cost.
- **Tradeoffs:** the index **grows monotonically** and will eventually stop being
  cheap to inject — which is precisely why finding 5 lands first; without an
  eviction contract this quietly becomes a per-spawn tax. Second risk: a planner
  reading lessons may **over-fit to a past incident** and design defensively
  against a failure that does not apply to its rework.
- **Suggestion:** one sentence, index only — never the individual lesson files.
- **Path:**
  1. `rework-planner.md`, opening block before "Design standard": before
     designing, read `tasks/lessons.md` (**the index only**) and the source
     reworks file's "Tracked observations" section if present.
  2. **Measured before/after:** `tasks/lessons.md` is 26 lines (~1.2k tokens) and
     a Tracked-observations section ~15 lines (~0.5k) ⇒ **≈ +1.7k per planner
     spawn** against a measured 51.1k planner output (2026-07-16), ≈ +3%; ≈ +10k
     across a 6-rework campaign, under 0.03% of the campaign's 1,993k subagent
     output. Direct check: the next planner transcript shows a `tasks/lessons.md`
     read before its first design decision. Outcome metric: plan-premise
     falsifications per campaign against the ≥3 baseline (shared with finding 7).

### 9. (trial) `scripts/lint-findings.sh` — enforce the clauses that are honor-system today

- **Evidence:** two clauses state a bar with nothing checking it.
  `audit-base.md:18-19` requires that "Every finding cites concrete evidence
  (`file:line`, a specific entry, a measured number)", and
  `audit-devloop/SKILL.md:39` states "a devloop finding without a measurable claim
  is an opinion, not a finding". Nothing verifies either. The loop already runs a
  deterministic close-out linter of exactly this shape — `scripts/lint-comments.sh`
  with an allowlist, wired at `settings.json:11-19`.
- **Ideal:** a report cannot reach the queue with an anchor that does not resolve
  or a finding whose Path names no measurable.
- **Gap:** unmeasured. **This finding's own first step is to measure it**, at
  zero cost.
- **Tradeoffs:** the script's ceiling is low and must be stated in the same
  breath as the proposal: it **does not address the 12.5% mechanism-error rate**.
  Pass two settled that question — both measured mechanism errors required
  running the code under load, and the oracle for that class is the worker's
  measurement, which the loop already has. An LLM verifier over findings is
  **rejected outright**, not deferred: DeepVerifier's own transition rates put
  break-even at a **40.2% base error rate** against our measured 12.5%, where the
  same arithmetic gives −8.8 points; its false-green rate is 28.57%, its
  false-red 25%, and its mechanism is up to 12 sub-agent invocations per item
  (~405–540k per audit, +30% to +191% of a loop) which collides with ruling 2.
  The script is **our** idea standing on `lint-comments.sh` as precedent, not a
  derivative of that paper — the paper contains no deterministic checker at all.
- **Suggestion:** measure first, build only if the measurement justifies it.
- **Path:**
  1. **Gate zero, before writing a line and at zero token cost:** count stale or
     unresolvable `file:line` anchors across the existing reports in
     `docs/reviews/`. **If the count is 0, drop this finding** — it would enforce
     a clause nobody violates, and that is an addition ruling 7 argues against.
  2. If non-zero: `scripts/lint-findings.sh`, ~20–40 lines, asserting exactly
     three things — every `path:line` token resolves (path exists, line number in
     range); every numbered finding has an Evidence bullet containing at least one
     anchor; every numbered finding names either a test identifier or a
     before/after number.
  3. One sentence in `audit-base.md` naming it as the audit's close-out gate,
     exiting non-zero on violation.
  4. **Measured before/after:** the gate-zero count from step 1 is the baseline;
     target 0 stale anchors per report thereafter.

### 10. (trial) A mechanical hook for green-but-wrong diffs

- **Evidence:** the loop has no detector for a diff that passes every gate and
  implements the wrong thing. `implement-finding/SKILL.md:120-133` has the
  orchestrator show the worker's report, run `git status --short` and
  `git diff --stat`, and then states explicitly: "no review beyond the two
  commands above unless the user asks". `git diff --stat` is line counts — nothing
  reads diff *content*. The one rule aimed at this class lives in unenforceable
  prose at `finding-worker.md:104-110`: the test "must construct the Path's named
  scenario and call real production code", and "re-implementing logic inline or
  asserting constants proves nothing and does not count".
- **Ideal:** the rule the worker currently grades itself against becomes a gate
  the worker cannot pass by claiming to have passed it — the same move
  `lint-comments.sh` already made for the comment policy.
- **Gap:** the enumerable subset is small but real: a new `#[test]` whose body
  asserts only literals and consts, or which never calls into a workspace crate
  path.
- **Tradeoffs:** **false positives on legitimate table-driven or const-boundary
  tests**, requiring an allowlist that then drifts — and this risk is demonstrated,
  not hypothetical: `audit-devloop-2026-07-17.md:233` records exactly that failure
  for the comment lint. The two alternatives are worse and are **rejected**: a
  reviewer spawn per finding costs 45–48k against a 27k median step, a **~2.7×
  campaign multiplier** that fails ruling 2 by a wider margin than the parallel
  workers already declined; a reviewer on the loop-final step only is ~+17%.
  Option (c), the hook, is **0 tokens per run** with a one-time build cost.
- **Suggestion:** build it in the existing hook slot, not as a new channel.
- **Path:**
  1. New `scripts/hooks/*.mjs` registered alongside `comment_lint_hook.mjs` and
     `wgsl_hook.mjs` in `settings.json:11-19`'s `PostToolUse` `Edit|Write`
     matcher, flagging a new `#[test]` that asserts only literals/consts or never
     calls a workspace crate path.
  2. An allowlist file mirroring `scripts/lint-comments-allowlist.txt`.
  3. **Measured before/after:** the gate is the **false-positive rate**, since the
     run cost is zero. Baseline for the thing it targets: findings filed post-hoc
     against a step previously committed green, derivable from git history plus
     the reworks files over the last three campaigns; target 0 new ones.

### 11. (micro, trial) The planner names the assertion, not just the scenario

- **Evidence:** `rework-planner.md:75-77` (rule 4) requires the Path to name the
  test *scenario* — constructed through real production code, no constants
  asserted — but never its **assertion**. So a worker can land a test that
  constructs the right scenario and asserts something weaker than the plan
  intended, and every downstream check passes: `implement-finding/SKILL.md:122-123`
  sees only `git status --short` and `git diff --stat`, which is file-and-line-count
  partial credit.
- **Ideal:** the plan names the predicate and expected value, so a half-landed
  step is visible rather than green.
- **Gap:** unmeasured — establish it by re-reading the last campaign's step tests
  against their Paths.
- **Tradeoffs:** a planner-specified threshold that is **wrong** makes the worker
  land a confidently wrong check, where today the worker at least sees the real
  behaviour while writing the assertion. `rework-planner.md:78-84` (rule 5)
  already provides the mitigation — the Path must say what to do for each
  plausible outcome — but it can be forgotten. Second-order: plans grow, and plan
  length is itself a tracked cost. The full partial-credit ledger this comes from
  is **rejected**: both 2026-07-17 corrections went through the fresh-haiku path
  at 9.4k combined, so there was no partial-progress state worth preserving, and
  the theoretical win has zero observed instances to price against.
- **Suggestion:** tighten the existing rule; do not add a concept.
- **Path:**
  1. `rework-planner.md:75-77`: the Path names the assertion's predicate and
     expected value, not only the scenario.
  2. **Measured before/after:** cost is ~200–400 tokens per step, ≈ +2–3k on a
     7-step plan against 51.1k measured planner output (≈ +5%). Gate: do
     half-landed Path steps become visible? Measure steps whose landed test
     asserts something weaker than the Path intended (today unmeasured), and
     correction-round count per loop against the 2026-07-17 baseline of 2
     corrections / 9.4k tokens across 65 commits.

## Carried forward from previous report

None. Every finding and rework in `audit-devloop-2026-07-17.md` and
`reworks-devloop-2026-07-17.md` is struck, and that pair is retained as the
telemetry corpus of record rather than deleted (see the header). This report is
sourced from the research evaluation, not from a fresh sweep of the loop.

## Resolved since last report

Not applicable — no prior finding was open to resolve.
