# Plan: A queue-runner convention — one launch decision per report instead of one prompt per rework — 2026-07-17

Source: `docs/reviews/devloop/reworks-devloop-2026-07-17.md` finding 1.
Gate status at plan time: cleared — fixes finding 5 (probe-and-override
playbook) landed 2026-07-17 in `plan-rework/SKILL.md` and
`implement-finding/SKILL.md`; fixes finding 1 decided (implement loops run
under an opus session; audits and plan reviews stay on fable). The rework's
own decision landed 2026-07-17: **pause-on-plan is THE behavior** — the
fully-autonomous variant was explicitly not chosen and is not planned as a
mode flag.

## Ideal end state

"Run the queue" is a single instruction: the orchestrator walks a report's
cross-type queue note in order — fixes findings loop automatically (their
user-decides questions batched once at launch), each rework is planned,
shown, and waits for exactly one go/stop word before its steps loop — and
everything after each approval (loop, per-step commit, queue-note strike in
both report files, per-loop token report, advance) is automatic, stopping at
any blocker, unanswered plan question, or user interrupt at a commit
boundary. The queue note itself is the campaign's durable state, so the run
survives compaction and resumes from the file alone. Measured target from
the finding: the next multi-rework campaign completes with one plan-approval
word per rework and zero other content-free prompts (vs 6 full prompts in
the rendering campaign) at unchanged commit/gate discipline.

## Design decisions

1. **Skill shape: a new `run-queue` skill that chains the existing skills by
   invocation — zero convention text duplicated.** The runner's body says
   "run the /plan-rework procedure" and "run the /implement-finding
   procedure" (Skill-tool invocations; a skill already loaded this session
   is followed from its loaded text). Every convention those skills carry —
   model routing, the micro inline path, user-decides batching,
   probe-and-override API recovery, worker spawn templates, the corrections
   path — therefore applies by construction, satisfying the finding's hard
   requirement that the runner *embed* the planner-fallback convention
   rather than restate or fork it. Rejected: extending
   `implement-finding/SKILL.md` with a queue mode (it is a per-finding skill;
   campaign chaining and plan-rework invocation do not belong in it, and its
   "Nothing else: no commits" tail would need surgery); a script or hook
   (the sequence needs judgment at checkpoints — it is orchestrator
   behavior, not automation).

2. **Pause-on-plan is the only behavior** (user decision 2026-07-17,
   recorded in the finding). The runner plans a rework, shows the plan plus
   the previous loop's token report, and waits for one go/stop. No
   autonomy flag, no skip option. Fixes findings have no plan and therefore
   no pause — the single launch checkpoint is their approval.

3. **User-decides questions split by type.** Fixes-level "(user-decides)"
   tags anywhere in the remaining queue are batched at the launch
   checkpoint, per implement-finding's existing convention (launch = queue
   launch). Rework-level user-decides are NOT asked at launch: a rework's
   real questions do not exist until its plan is written (rework-planner
   puts them in Design decisions), and asking earlier forces a blind
   decision — they surface at that rework's own pause, which is already a
   checkpoint. Rejected: batching everything at launch (blind rework
   decisions), asking nothing (mid-loop stalls the convention exists to
   prevent).

4. **Tier: the queue session runs opus; plan depth is protected by pinning
   `model: fable` in `rework-planner.md` frontmatter.** Finding 1's decision
   assumed the user switches `/model` between loop sessions (opus) and
   plan/audit sessions (fable). A queue run chains both phases in ONE
   session, and `rework-planner.md` pins no model today — under an opus
   session the planner would *inherit opus*, silently downgrading every plan
   in the campaign. The one-line frontmatter pin (same mechanism
   `finding-worker.md` already uses for sonnet) makes the tier decision
   hold in any session. This deliberately diverges from fixes finding 5's
   wording ("the planner keeps inheriting fable as its default"): inheritance
   was only correct while every orchestrator session WAS fable; opus-for-loops
   broke that premise, so the pin re-derives the intent. Probe-and-override
   is unaffected — a spawn-time `model: "opus"` override takes precedence
   over agent frontmatter, so the documented fallback still works verbatim.
   Rejected: having the runner pass `model: "fable"` on planner spawns (forks
   plan-rework's spawn template — exactly the restatement the finding bans);
   telling the user to switch models mid-queue (reintroduces the manual step
   the runner exists to remove).

5. **Durable state is the queue note; the runner is stateless and
   resumable.** Position = first unstruck entry in the cross-type queue
   blockquote; strikes and done-notes (the reworks-queue-mark-done
   convention, applied to BOTH report files since the note is mirrored) are
   the only bookkeeping. This makes compaction — the finding's named risk
   for long unattended runs — safe: after any compaction the runner re-reads
   the queue note before the next spawn instead of trusting memory, and a
   fresh session's "run the queue" resumes from the file alone. Rejected: a
   separate campaign-state file (a second source of truth that can drift
   from the queue note the user already treats as canonical).

6. **Stop conditions, enumerated.** (a) User "stop" at a pause-on-plan
   checkpoint. (b) Any user message during the run, honored at the next
   commit boundary — finish the in-flight worker, commit if green, report
   position (this is the mid-campaign stop order the rendering campaign
   exercised). (c) A worker failure that survives one fresh-spawn correction
   round. (d) A gate that stays red after one rerun. (e) The probe-red
   API-down case from the embedded playbook. (f) A plan question the user
   has not answered. Every stop leaves the workspace green and the queue
   note accurate.

7. **Token aggregation: per-loop reports ride the checkpoint messages, plus
   one campaign aggregate at queue end.** Each rework's pause message
   carries the *previous* loop's token report (the user is already reading
   that message, so the report costs no extra attention round); the end of
   the queue reports per-loop figures, total, commit count, and suite count
   first→last. This preserves the existing per-loop reporting convention
   (283k–1.33M per loop, reported per loop, rendering campaign) unchanged.

8. **Commit discipline codified, not changed.** Each green worker return is
   committed by the orchestrator (short descriptive message, no attribution
   trailers — the user's standing commit style); the last step of each loop
   is named loop-final in its spawn task so finding-worker's existing
   full-gate rule fires and its X/X lands in the done-note. The runner's
   launch counts as the user asking, which is what implement-finding's
   "Nothing else … unless the user asks" tail requires; the runner text says
   so explicitly so a lower-tier session never sees a conflict.

No open user questions: the two product choices this rework carried
(adopt/decline, pause-on-plan vs autonomous) were both decided 2026-07-17
and are recorded in the finding's DECIDED bullet.

## Findings (execution order)

### 1. Pin the rework-planner agent to fable so an opus queue session cannot silently downgrade plan depth (docs-only)

- **Evidence:** `.claude/agents/rework-planner.md:1-4` — frontmatter has
  `name:` and `description:` but no `model:` line, so the planner inherits
  the session's model. `.claude/agents/finding-worker.md:1-5` shows the pin
  mechanism in use (`model: sonnet`). The loops-on-opus decision
  (`implement-finding/SKILL.md`, "Loop behavior": "Loops of pre-planned
  findings may run under an opus session; audits and plan reviews stay on
  fable") means a queue-runner session runs opus — and an unpinned planner
  spawned from it would inherit opus, violating "plan reviews stay on
  fable" without anyone noticing.
- **Ideal:** `rework-planner.md` frontmatter carries `model: fable`, so the
  planner runs fable regardless of the invoking session's tier. The
  probe-and-override fallback in `plan-rework/SKILL.md` ("respawn the
  planner with `model: "opus"`") keeps working unchanged, because a
  spawn-time model parameter takes precedence over agent frontmatter.
- **Gap:** one missing frontmatter line; the tier decision currently holds
  only by the accident that plans have so far been requested from fable
  sessions.
- **Suggestion:** add the single line `model: fable` to the frontmatter of
  `.claude/agents/rework-planner.md`, directly after the `description:`
  line. Change nothing else in the file.
- **Path:** (1) Edit `.claude/agents/rework-planner.md`: in the frontmatter
  block (between the `---` markers), insert `model: fable` as a new line
  immediately after the `description:` line. (2) Gate: none — the file is
  gitignored config (`.claude/` is its own nested repo, pushed to
  TycheDea/ClaudeConfig by the user afterward; do NOT run git in `.claude/`
  and do not commit anything yourself). (3) Show the diff of the frontmatter
  in your final report. No test exists or is needed for agent config; the
  behavioral proof is the next queue run's planner spawning on fable, which
  the campaign ledger records.

### 2. Author the run-queue skill — the written sequence that chains plan-rework and implement-finding over a report's cross-type queue (docs-only)

- **Evidence:** no queue-level skill exists —
  `Glob .claude/skills/**/SKILL.md` lists only the eight audit skills plus
  `plan-rework` and `implement-finding`. The chaining the rendering campaign
  ran six times by hand (plan → show → go → loop → commit each → strike both
  queue notes → report tokens; see
  `docs/reviews/devloop/reworks-devloop-2026-07-17.md` finding 1 Evidence)
  lives only in orchestrator memory. The queue-note format the runner reads
  is defined in `.claude/skills/audit-base.md` ("Report" section: one
  cross-type blockquote under "## Findings (implementation order)" in the
  fixes file, mirrored verbatim in the reworks file; parked entries listed
  with their gate, no position) and exercised in
  `docs/reviews/rendering/reworks-rendering-2026-07-16.md:17-60` (strikes +
  done-notes).
- **Ideal:** `.claude/skills/run-queue/SKILL.md` exists and contains the
  complete runner convention: launch checkpoint with batched fixes-level
  user-decides, automatic fixes loops, pause-on-plan per rework, per-step
  commits, strikes in both files, per-loop token reports, enumerated stop
  conditions, compaction-safe resumability — chaining /plan-rework and
  /implement-finding by invocation and duplicating none of their text.
- **Gap:** the entire skill file; the conventions it packages are all
  individually codified but the sequence is unwritten.
- **Suggestion:** create the file with exactly the content below — it was
  designed in this plan's design pass; write it verbatim, changing nothing.
- **Path:** (1) Create the directory `.claude/skills/run-queue/` and write
  `.claude/skills/run-queue/SKILL.md` with exactly this content, byte for
  byte:

  ````markdown
  ---
  name: run-queue
  description: Run a report's cross-type findings queue end to end — fixes findings loop automatically, each rework is planned and shown and waits for one go/stop before its steps loop, every green step commits, queue notes strike, tokens report per loop. Use when asked to run a whole queue or campaign, e.g. "/run-queue" or "run the rendering queue". Args: [report-path]
  ---

  You are the orchestrator running an entire findings queue. This skill only
  chains procedures that live elsewhere: /plan-rework and /implement-finding
  do all per-item work, and every convention they carry — model routing, the
  micro inline path, user-decides batching, probe-and-override API-failure
  recovery, spawn templates, the corrections path — applies here by INVOKING
  those skills (Skill tool; a skill already loaded this session is followed
  from its loaded text), never by restating them. If an instruction here
  seems to conflict with one of theirs, theirs wins for the step it governs.

  Your launch is the user's standing request for the whole run: the commits,
  queue-note strikes, and token reports below are user-asked, which is what
  implement-finding's closing "unless the user asks" requires.

  **Session tier.** Queue runs happen under an opus session (loops-on-opus,
  decided 2026-07-17). If the session is not opus at launch, ask the user
  to switch (`/model opus`) and wait. Plan depth is safe either way: the
  rework-planner agent is pinned to fable in its own definition.

  **Locate the queue.** Resolve REPORT the way /implement-finding does when
  no path is given (list `docs/reviews/*/audit-*.md` without opening them;
  newest if all matches share one domain folder; ask the user if several
  domains match). Read ONLY the cross-type queue note — the blockquote under
  "## Findings (implementation order)" — never the finding bodies (those are
  read per item by the procedures you chain). The queue note is the
  campaign's durable state: your position is the first unstruck entry;
  entries listed as parked (gate stated, no position) are skipped and named
  at launch. After any compaction, re-read the queue note before the next
  spawn instead of trusting memory.

  **Launch checkpoint (once per campaign).** Show the user: the remaining
  queue, any parked entries you will skip, and ALL "(user-decides)"
  questions from fixes findings anywhere in the remaining queue, batched
  per implement-finding's convention. Do NOT ask rework-level user-decides
  here — a rework's real questions do not exist until its plan is written,
  and they surface at that rework's own pause. Collect the decisions, then
  run.

  **Walk the queue in order.**

  - **`finding N`** (fixes file): run the /implement-finding procedure —
    route the model, apply inline if "(micro)", spawn otherwise, review its
    status/diff. When green, commit it (short descriptive message, no
    attribution trailers). Consecutive fixes findings are one loop: carry
    the last full-suite count as the next spawn's baseline, and name the
    loop's final finding as loop-final in its spawn task so it runs the
    full gate. When a contiguous fixes segment completes, strike those
    entries in the queue note in BOTH report files (fixes and reworks —
    the note is mirrored) with a one-line done-note, commit the strike,
    then advance.

  - **`rework N`** (reworks file):
    1. Run the /plan-rework procedure. Its API-failure recovery — probe
       and override — lives there; follow it there.
    2. Show the planner's final report verbatim plus the previous loop's
       token report, and surface any user question the plan's Design
       decisions raise. **Pause: wait for one go/stop from the user.** This
       checkpoint always happens — it is the behavior, not a mode.
    3. On go: loop the /implement-finding procedure over the plan's steps
       in order (`/implement-finding <k> <plan-path>`), committing each
       green step; name the last step loop-final in its spawn task. On
       stop: end the run and report position.
    4. After the last step's commit: strike `rework N` in the queue note
       in BOTH report files and append "done YYYY-MM-DD (<plan-file>, K
       steps, loop-final gate X/X)" per the mark-done convention; commit
       the strike. (Skip whatever the plan's own close-out step already
       struck.)
    5. Report the loop's tokens (loop total; name any outlier spawn), then
       advance.

  **Stopping.** Any user message during the run is honored at the next
  commit boundary: finish the in-flight worker, commit if green, report
  position (the struck queue note plus the in-flight item). Stop the queue
  yourself — never push on — when: a worker failure survives one
  fresh-spawn correction round; a gate stays red after one rerun; the probe
  says the API path is down; or a plan raises a question the user has not
  answered. Every stop leaves the workspace green and the queue note
  accurate, so a later "run the queue" resumes from the files alone.

  **End of queue.** Strike anything completed but still unstruck, then
  report the campaign aggregate: tokens per loop and total, commit count,
  and the suite count first→last.
  ````

  (2) Gate: none — the file is gitignored config (`.claude/` is its own
  nested repo, pushed to TycheDea/ClaudeConfig by the user afterward; do NOT
  run git in `.claude/` and do not commit anything yourself). (3) Mechanical
  verification, then done: confirm with Glob that
  `.claude/skills/plan-rework/SKILL.md`, `.claude/skills/implement-finding/SKILL.md`,
  and `.claude/agents/rework-planner.md` all exist (the files the new skill
  references), and paste the new file's frontmatter block in your final
  report. No behavioral test is possible for a skill file; the finding's
  measured proof (one approval word per rework, zero content-free prompts,
  unchanged commit/gate discipline) is read from the next campaign's ledger.

### 3. Strike rework 1 done in both devloop report files' queue notes (docs-only)

- **Evidence:** the mirrored cross-type queue note appears in
  `docs/reviews/devloop/audit-devloop-2026-07-17.md` (blockquote under
  "## Findings (implementation order)", lines 34-45) and verbatim in
  `docs/reviews/devloop/reworks-devloop-2026-07-17.md` (lines 16-27). Both
  copies end with the entry `rework 1 (user-decides; after finding 5: a
  queue runner must embed the planner-fallback convention finding 5
  writes).` — unstruck. The mark-done convention (rendering precedent:
  `docs/reviews/rendering/reworks-rendering-2026-07-16.md:19-50`) strikes
  the entry and appends a done line naming date, plan file, and step count.
- **Ideal:** both queue-note copies show rework 1 struck with an identical
  done-note, so the queue note remains the accurate durable state the new
  runner reads.
- **Gap:** this step is the plan's close-out — once steps 1-2 are committed
  the rework is complete, and an unstruck queue entry would misreport it as
  pending.
- **Suggestion:** in each of the two files, wrap the `rework 1 (…)` queue
  entry in `~~` strikethrough and append one done sentence inside the same
  blockquote. Do not touch any other queue entries (some `finding N`
  entries may or may not already be struck — leave them exactly as found).
- **Path:** (1) In `docs/reviews/devloop/audit-devloop-2026-07-17.md`,
  inside the queue blockquote, change the fragment
  `rework 1 (user-decides; after finding 5: a queue`
  (through) `runner must embed the planner-fallback convention finding 5 writes).`
  to the same text wrapped as
  `~~rework 1 (user-decides; after finding 5: a queue runner must embed the planner-fallback convention finding 5 writes)~~.`
  and append, as a new `>`-prefixed sentence at the end of that first
  blockquote paragraph (before the explanatory second paragraph):
  `> Rework 1 done 2026-07-17 (plan-devloop-rework-1-2026-07-17.md, 3
  steps; pause-on-plan queue runner: a run-queue skill chains plan-rework +
  implement-finding by invocation, rework-planner pinned to fable so an
  opus queue session cannot downgrade plans).`
  Preserve the blockquote's `> ` prefixes and line-wrap style. (2) Apply
  the identical edit to the identical text in
  `docs/reviews/devloop/reworks-devloop-2026-07-17.md`. (3) Gate: none —
  markdown under `docs/reviews/` only, no source touched; show the diff of
  both files in your final report. These two files ARE tracked by the
  vordar repo (unlike `.claude/`), but you never commit — the orchestrator
  commits this step.
