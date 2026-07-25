# Plan: The loop meters cost but never quality — a per-campaign outcome vector — 2026-07-25

Source: `docs/reviews/devloop/reworks-devloop-2026-07-25.md` finding 1.

## Ideal end state

One committed script, `scripts/campaign_report.py`, takes a campaign's audit
report path and emits `docs/campaigns/<domain>-<audit-date>.md`: a fixed,
ordered field table covering both halves of the campaign — what it cost (spawn
count by agent and model, output/cache tokens, the outlier spawn, dead spawns,
agent-hours and tool-time by category) and what it produced (queue items struck,
suite first→last, commits and reverts in both repos, stop lines by terminal
state, recorded plan-premise falsifications, findings carried in). Every number
is either derived mechanically from artifacts the loop already writes, or is a
tally of a token a human already wrote down — the script never judges. Two
campaign files are compared by reading the same rows in the same order, so a
proposal to change the loop is argued against a movement rather than a story.
`audit-devloop` stops re-deriving the transcript hotspot census by hand and
stops asking every finding to invent a bespoke measurable; `run-queue` stops
hand-aggregating tokens per loop and per campaign.

## Design decisions

### 1. The (user-decides) branch is one appended step, and nothing before it changes

The rework's Path step 1 is unanswered and is asked at this rework's own pause,
after this plan is shown. Three answers, and the plan is built so the answer is
a late, isolated branch:

- **(A) No vector wanted.** Nothing here lands. Strike `rework 1` in the queue
  note of both report files as declined, with the reason. Zero steps execute.
- **(B) Inform the user's judgement only — recommended.** Steps 1–8 land exactly
  as written. The script exits 0 whenever it successfully emits a vector and
  non-zero *only* when it cannot produce one (missing transcript directory,
  missing report, zero spawns attributed). No field value ever changes the exit
  code.
- **(C) May gate automatically.** Steps 1–8 land **byte-identically to (B)**,
  plus step 9 — the only step that exists under (C) — which adds a
  **consistency** exit code and wires it into `run-queue`'s end of queue.

The recommendation is (B). Under (C) the gate must still refuse to gate on
*worth*: no threshold on tokens, on output, on struck-item ratio, or on any
field that expresses whether the campaign was good. Ruling 5 (`the user decides
worth`) and the source document's cluster 5 (a self-improving loop optimizing a
gameable objective) both point the same way, and `tasks/lessons.md` carries two
live entries that say a number can clear while the thing is broken
(`metric-cleared-picture-did-not-move`, persistent). What step 9 may gate on is
**internal inconsistency of the emission** — a `gate X/Y` done-note where X≠Y, a
struck queue entry with no commit in the campaign range, a dead spawn with no
stop line recorded — because those are contradictions in the record, not
judgements about quality. That distinction is the whole content of step 9 and it
is why (C) costs one step rather than a redesign.

### 2. The spec's "reject anything needing a transcript" premise is false, and rejecting it is the largest win here

The rework's Path step 2 says to reject outcome dimensions that need a
transcript "since `.claude/CLAUDE.md` §9 destroys conversation state at every
phase gate". §9 destroys the *live session context*; it does not touch the
on-disk record. Subagent transcripts persist as files under
`~/.claude/projects/<mangled-repo-path>/<session>/subagents/agent-*.jsonl`, each
with an `agent-*.meta.json` sibling — `audit-devloop/SKILL.md:21` already lists
them in Scope and the 2026-07-17 audit measured 76 of them. Verified while
designing this plan: **609 subagent transcripts, 730 MB, unbroken daily coverage
2026-07-03 → 2026-07-25**.

This matters because the finding's own Evidence (a) is that
`scripts/token-report.ps1` has "no per-campaign, per-loop or per-spawn
attribution" — and per-spawn attribution is exactly what these files carry, in
structured form: every `type: "assistant"` record has
`message.usage.{output_tokens, cache_creation_input_tokens,
cache_read_input_tokens}`, `message.model`, and an ISO `timestamp`. Honouring
the spec's step 2 literally would have left the vector with no denominator at
all and would have deleted nothing from `audit-devloop`. Recorded here as a
divergence from the spec, per the "a wall in the spec is information" rule.

Prototyped during this design pass against the real corpus, the mechanical
derivation reproduces the 2026-07-17 hand census closely enough to be its
replacement: worst single spawn **180.3k output** against the hand-recorded
**180k**; tool share **19.5%** against the hand-recorded **19.7%**; nine
API-failure husks located exactly. See step 6 for the full comparison and for
which divergences are expected.

### 3. Campaign membership is keyed on the report path in the spawn task, not on a time window

`implement-finding/SKILL.md` makes every worker spawn start with
`Implement finding N of <report-or-plan path>.` and `plan-rework` makes every
planner spawn start with `Design the implementation plan for rework finding N of
<reworks path>.` So the campaign a spawn belongs to is written in its first user
message. A spawn is in the campaign iff that message contains any of:

- `docs/reviews/<domain>/audit-<domain>-<date>.md`
- `docs/reviews/<domain>/reworks-<domain>-<date>.md`
- `docs/reviews/<domain>/plan-<domain>-rework-<k>-<date2>.md` with `date2 >= date`

Rejected alternative: a pure time window. Measured on 2026-07-16/17, the window
holds 121 transcripts against 64 that name a rendering path — the window is
polluted by concurrent networking, game-architecture and rust-tooling loops.
Rejected alternative: `.meta.json`'s `description` field — it is the
orchestrator's free-text label and is not stable.

Corrections spawns (`implement-finding`'s fresh-haiku correction path) name no
report and are therefore **not** attributed. They are counted and reported as
`unattributed_spawns_in_window`, deliberately mirroring the `unattributed`
residual that `audit-devloop/SKILL.md:27` now requires of failure labelling: an
honest residual beats a guessed attribution.

### 4. The script counts; it never judges. Exactly one new writing obligation

Every outcome field is one of two kinds:

- **Mechanically derived** — token sums, spawn counts, commit counts, queue-note
  strikes, `gate X/Y` numbers, tool timings. No human input.
- **Tallied from a token a human already wrote** — stop lines (the five-field
  line `run-queue/SKILL.md` requires on every self-initiated stop, landed today
  by fixes finding 1) and the done-note.

The single addition is a `premise-falsified:` clause on the existing mark-done
convention. Without it the vector cannot carry the metric that fixes findings 7
and 8 both nominate as the loop's lead non-token number (plan-premise
falsifications per campaign, computable baseline **≥3** for 2026-07-17), and the
rework's own Path step 5 ("the 12.5% mechanism-error rate becomes a tracked
series rather than a number someone counted once") is unreachable. It costs the
orchestrator ~10 tokens on the campaigns where it applies and zero where it does
not. It is an extension of a line already being written, not a new artifact.

The stop line needs a **prescribed syntax**. Today's clause names five fields
but no separator, and an unparseable line cannot become a series. Step 8 pins
it to `**STOP** <item> · <tier> · <category> · <attempted> · <gate>` — a
tightening of a clause landed hours ago, not a new concept. No stop line exists
yet, so nothing breaks.

### 5. One script, Python 3, stdlib only, in the vordar repo

`scripts/ai-pipeline/` and `scripts/asset-pipeline/` are already Python; the
work here is per-line JSON aggregation over a 730 MB corpus, which is Python's
job and not PowerShell's. `python3` is on PATH (3.12.12 verified). No
third-party dependency is introduced, so no new install gate.

The script lands at `scripts/campaign_report.py` — **vordar repo, tracked**.
Its tests land at `scripts/tests/test_campaign_report.py` with fixtures under
`scripts/tests/fixtures/` — **vordar repo, tracked**, run with
`python3 -m unittest discover -s scripts/tests -t .` (stdlib, no pytest).
Its output lands in `docs/campaigns/` — **vordar repo, tracked**, which is what
makes the series exist (`docs/tokens/` holds one file and no series). Steps 7–8
edit `.claude/skills/audit-devloop/SKILL.md`, `.claude/skills/run-queue/SKILL.md`
and `.claude/skills/audit-base.md` — the **nested `.claude` repo**, gitignored by
vordar, pushed to TycheDea/ClaudeConfig.

Rejected: extending `scripts/token-report.ps1`. It is a weekly Task Scheduler
job over `ccusage`'s account-wide totals; a per-campaign, per-spawn instrument
on a different cadence with a different unit would be two tools wearing one
name.

Rejected: emitting both markdown and JSON. Two formats for one fact is the
duplication the erasure rule forbids. The vector is markdown whose field
sections are strict `| field | value |` tables, which a regex parses in three
lines if step 9 ever needs it.

### 6. The output file's fixed shape

Every step below adds exactly one section; the header, section order and field
order never change, because comparison across campaigns is "read the same rows".

```
# Campaign vector — <domain> <audit-date>

Emitted by `scripts/campaign_report.py` from `<report path>`.
Window: <first spawn ISO> .. <last spawn ISO>
Attribution: spawns whose task names this campaign's audit/reworks/plan files.
Not counted: correction spawns (they name no report) — see
unattributed_spawns_in_window.

## Cost            <- step 1
## Tool time       <- step 2
## Orchestrator    <- step 3
## Outcome         <- steps 4 and 5
## Divergence from the hand census   <- step 6, only when one exists
```

### 7. Orchestrator tokens are window-attributed and labelled as such

The main session's own spend (`646.6k` output on 2026-07-17, roughly a quarter
of that campaign) lives in `<session>.jsonl`, not in `subagents/`. A session
spans several campaigns, so it cannot be split by task text. Excluding it would
drop a quarter of the cost block; inventing a task-level split would be a guess.
The resolution is to attribute session records **by timestamp inside the
campaign window** and to name the section `## Orchestrator (window-attributed,
not task-attributed)` so no reader mistakes its precision for the subagent
block's.

### 8. Lessons index entries that bear on this design

Read before designing, per this agent's contract. Three apply:
`probe-must-fail-when-broken` (persistent) — the script must exit non-zero when
it cannot produce a vector, never emit an empty one and exit 0;
`keep-verification-artifacts` (persistent) — the validation run's output is
committed, not quoted; `metric-cleared-picture-did-not-move` (persistent) — a
field clearing is not evidence the campaign was good, which is why step 9 is
gated on consistency and never on worth. No other entry applies and this design
does not defend against ones that do not.

---

## Findings (execution order)

### 1. `scripts/campaign_report.py`: campaign attribution and the subagent cost block

- **Evidence:** no per-campaign or per-spawn attribution exists anywhere.
  `scripts/token-report.ps1:1-7` is a weekly Task Scheduler job over
  `ccusage weekly --json`, account-wide and week-granular, writing
  `docs/tokens/<date>.md` (one file exists: `2026-07-23.md`). The per-spawn data
  it lacks is on disk: `~/.claude/projects/<mangled-repo-path>/<session>/subagents/agent-*.jsonl`,
  each with an `agent-*.meta.json` sibling containing `{"agentType": "...",
  "description": "...", "model": "..."}` (`model` is absent on transcripts older
  than 2026-07-17). Each `.jsonl` line is one JSON record; records with
  `"type":"assistant"` carry `message.model`, an ISO-8601 `timestamp`, and
  `message.usage.{output_tokens, cache_creation_input_tokens,
  cache_read_input_tokens}`. Records with `"type":"user"` carry
  `message.content` (string or content-block list); the **first** such record is
  the spawn task, which `implement-finding/SKILL.md:118-130` guarantees begins
  `Implement finding N of <path>.` and `/plan-rework` guarantees begins
  `Design the implementation plan for rework finding N of <path>.`
- **Ideal:** `python3 scripts/campaign_report.py docs/reviews/rendering/audit-rendering-2026-07-16.md`
  writes `docs/campaigns/rendering-2026-07-16.md` containing a header and a
  `## Cost` section, and exits 0; it exits 1 with a message on stderr when the
  transcript directory does not exist, the report file does not exist, or zero
  spawns are attributed.
- **Gap:** the file, the attribution rule and the emission do not exist.
- **Suggestion:** one module, stdlib only, no third-party imports. Public
  functions the tests call directly: `attribution_patterns(report_path)`,
  `scan_transcript(jsonl_path)`, `collect_spawns(transcripts_dir, patterns)`,
  `render_cost_section(spawns)`, `main(argv)`.
- **Path:**
  1. Create `scripts/campaign_report.py`. CLI: one positional `report` path;
     `--transcripts DIR` (default: `~/.claude/projects/<mangled>` where
     `<mangled>` is the absolute repo root with each of `: \ / _ .` replaced by
     `-`, e.g. `C:\Users\egm_8\IdeaProjects\vordar` →
     `C--Users-egm-8-IdeaProjects-vordar`); `--out DIR` (default
     `<repo>/docs/campaigns`). Derive `<domain>` and `<audit-date>` from the
     report filename `audit-<domain>-<date>.md`; exit 1 if it does not match.
  2. Attribution: a spawn is in the campaign iff its first user message contains
     `docs/reviews/<domain>/audit-<domain>-<date>.md`,
     `docs/reviews/<domain>/reworks-<domain>-<date>.md`, or
     `docs/reviews/<domain>/plan-<domain>-rework-<k>-<date2>.md` with
     `date2 >= date` (string compare on ISO dates is sufficient). Scan
     `<transcripts>/*/subagents/agent-*.jsonl`; read `agent-*.meta.json` beside
     each for `agentType` and `model`.
  3. Per spawn accumulate: summed `output_tokens`, `cache_creation_input_tokens`,
     `cache_read_input_tokens`; `agentType`; model = `meta.model` when present,
     else the set of `message.model` values seen (older transcripts). A spawn is
     **dead** iff its summed `output_tokens == 0` or any assistant record has
     `message.model == "<synthetic>"`.
  4. Emit `docs/campaigns/<domain>-<date>.md`: the header block from the plan's
     Design decision 6 (title, emitting command, `Window:` first→last assistant
     timestamp across attributed spawns, the two attribution notes), then a
     `## Cost` section as a `| field | value |` table in exactly this order:
     `spawns`, `spawns_finding_worker`, `spawns_rework_planner`, `spawns_other`,
     `spawns_by_model` (comma-joined `name count`, sorted by count desc),
     `unattributed_spawns_in_window` (transcripts whose first assistant
     timestamp falls inside the window but which are not attributed),
     `output_tokens`, `output_max`, `output_max_spawn` (agent id + the first 80
     characters of its task), `output_median_nonzero`, `cache_create_tokens`,
     `cache_read_tokens`, `dead_spawns`.
  5. Exit 1 with a stderr message when: `--transcripts` dir missing (print the
     path tried), report file missing, filename unparseable, or zero spawns
     attributed. Never emit a file in those cases.
  6. **Test (fail-first).** Create `scripts/tests/test_campaign_report.py` and
     fixtures under `scripts/tests/fixtures/`. Build a fixture transcript tree
     `scripts/tests/fixtures/transcripts/sess1/subagents/` with four agents,
     hand-written and exact:
     - `agent-a1`: meta `{"agentType":"finding-worker","model":"sonnet"}`; first
       user record `Implement finding 1 of docs/reviews/demo/plan-demo-rework-1-2026-07-20.md.`;
       two assistant records at `2026-07-20T10:00:00Z` and `2026-07-20T10:05:00Z`
       with `output_tokens` 100 and 50, `cache_creation_input_tokens` 1000 and 0,
       `cache_read_input_tokens` 5000 and 5000, `model` `claude-sonnet-5`.
     - `agent-a2`: meta `{"agentType":"rework-planner","model":"fable"}`; first
       user record `Design the implementation plan for rework finding 1 of docs/reviews/demo/reworks-demo-2026-07-20.md.`;
       one assistant record at `2026-07-20T10:02:00Z`, `output_tokens` 300.
     - `agent-a3`: meta `{"agentType":"rework-planner","model":"fable"}`; same
       task text as `agent-a2`; one assistant record at `2026-07-20T10:03:00Z`
       with `model` `<synthetic>` and `output_tokens` 0.
     - `agent-a4`: meta `{"agentType":"finding-worker","model":"haiku"}`; first
       user record `Implement finding 2 of docs/reviews/other/audit-other-2026-07-20.md.`;
       one assistant record at `2026-07-20T10:04:00Z`, `output_tokens` 999.
     Also create the fixture report `scripts/tests/fixtures/reviews/demo/audit-demo-2026-07-20.md`
     (any body; the filename is what step 1 reads).
     Assertions, all through `main(argv)` and the emitted file — construct the
     scenario, do not re-implement the arithmetic inline: `spawns` is 3;
     `spawns_finding_worker` 1; `spawns_rework_planner` 2;
     `unattributed_spawns_in_window` 1; `output_tokens` 450; `output_max` 300;
     `output_median_nonzero` 150; `cache_create_tokens` 1000;
     `cache_read_tokens` 10000; `dead_spawns` 1; and a second test asserting
     `main` returns non-zero and writes no file when `--transcripts` points at a
     non-existent directory.
  7. Gate: `python3 -m unittest discover -s scripts/tests -t .` green. This step
     touches no Rust, no WGSL and no `content/`, so the cargo suite is not
     re-run (CLAUDE.md §7).

### 2. Tool-time attribution by category — the section that retires the hand hotspot census

- **Evidence:** `.claude/skills/audit-devloop/SKILL.md:27` orders the auditor to
  do this by hand, every audit: "for each `tool_use`, pair its timestamp with its
  `tool_result`'s to get tool wall time; the remainder of the gap between turns
  is model time. Attribute cost to categories (test runs, file reads, edits,
  exploration) and name the top offenders with numbers. Re-derive this every
  audit". The 2026-07-17 audit did exactly that over 76 transcripts and published
  `9.94 agent-hours wall, tool share 19.7%` (`audit-devloop-2026-07-17.md:6-8`).
  In the transcript records, an assistant record's `message.content` is a list of
  blocks; a `{"type":"tool_use","id":...,"name":...,"input":{...}}` block is
  closed by a later user record whose `message.content` list contains
  `{"type":"tool_result","tool_use_id": <same id>}`. `input.command` carries the
  command string for `Bash` and `PowerShell`.
- **Ideal:** `docs/campaigns/<domain>-<date>.md` gains a `## Tool time` section
  with agent-hours, tool-hours, tool share, and seconds by category, so an
  auditor reads it instead of pairing timestamps by hand.
- **Gap:** `scripts/campaign_report.py` (created by the previous step) reads
  usage totals only; it does not pair tool calls.
- **Suggestion:** extend `scan_transcript` to also return per-tool paired
  durations, and add `render_tool_time_section(spawns)`. Discard any pairing
  whose duration is negative or ≥ 3600 s (an unclosed call at a transcript
  boundary) rather than clamping it.
- **Path:**
  1. In `scripts/campaign_report.py`, while scanning a transcript, record each
     `tool_use` block's `id`, `name`, `input.command` (when present) and its
     record timestamp; on a later `tool_result` with a matching `tool_use_id`,
     add `(result_ts - use_ts).total_seconds()` to that tool's bucket, skipping
     durations outside `[0, 3600)`.
  2. Category mapping, applied in this order: `Bash`/`PowerShell` whose command
     matches `cargo\s+(nextest|test)` → `test`; `cargo\s+(build|check|clippy)` →
     `compile`; any other `Bash`/`PowerShell` → `shell`; `Edit`/`Write`/
     `NotebookEdit` → `edit`; `Read`/`Glob`/`Grep` → `read`; everything else →
     `other`.
  3. `agent_hours` = sum over attributed spawns of (last timestamp − first
     timestamp) in that transcript, in hours. `tool_hours` = summed paired
     durations. `tool_share_pct` = `100 * tool_hours / agent_hours`, printed to
     one decimal; print `n/a` when `agent_hours` is 0.
  4. Append a `## Tool time` section after `## Cost`, as a `| field | value |`
     table in this order: `agent_hours`, `tool_hours`, `tool_share_pct`,
     `tool_seconds_test`, `tool_seconds_compile`, `tool_seconds_shell`,
     `tool_seconds_edit`, `tool_seconds_read`, `tool_seconds_other`.
  5. **Test (fail-first).** Extend the fixture `agent-a1` from the previous step
     (at `scripts/tests/fixtures/transcripts/sess1/subagents/`) so its first
     assistant record contains two `tool_use` blocks — one `Bash` with
     `input.command` = `cargo nextest run --workspace` (id `t1`), one `Read`
     (id `t2`) — and add a user record whose content list holds a `tool_result`
     for `t1` at `+60s` and one for `t2` at `+2s` from their respective
     `tool_use` timestamps, plus a third `tool_use` (`Bash`, command
     `cargo build`, id `t3`) that is **never** closed. Assert through the emitted
     file: `tool_seconds_test` is 60, `tool_seconds_read` is 2,
     `tool_seconds_compile` is 0 (the unclosed call contributes nothing), and
     `tool_hours` is 62/3600 to 4 decimal places. `agent_hours` follows from the
     fixture timestamps — assert it is the sum of per-spawn spans, not a
     hard-coded literal reproduced by hand arithmetic in the test.
  6. Gate: `python3 -m unittest discover -s scripts/tests -t .` green. No Rust,
     WGSL or `content/` changes, so the cargo suite is not re-run.

### 3. Orchestrator spend, window-attributed

- **Evidence:** the main session's own tokens are a large share of a campaign and
  are absent from `subagents/`. `audit-devloop-2026-07-17.md:9-10` records
  "orchestrator (main context, fable): 646.6k output tokens across the two
  campaign days, peak context 734k, 2 compactions" against a subagent total of
  1,993k — roughly a quarter of the campaign, derived by hand. Session records
  live in `~/.claude/projects/<mangled-repo-path>/<session-uuid>.jsonl`, siblings
  of the `<session-uuid>/` directories that hold `subagents/`, and use the same
  record schema (`type`, `timestamp`, `message.usage`, `message.model`).
- **Ideal:** the emitted vector carries orchestrator output and cache-create
  tokens for the campaign window, in a section whose title states that the
  attribution is by time window, not by task.
- **Gap:** `scripts/campaign_report.py` scans only `*/subagents/agent-*.jsonl`.
- **Suggestion:** reuse `scan_transcript` unchanged; the only new logic is
  selecting session files and filtering their assistant records by timestamp.
- **Path:**
  1. In `scripts/campaign_report.py`, after the campaign window is known (first
     and last assistant timestamp across attributed spawns), scan
     `<transcripts>/*.jsonl` (top level only — not the `subagents/`
     subdirectories) and sum `output_tokens` and `cache_creation_input_tokens`
     over assistant records whose `timestamp` falls inside `[window_start,
     window_end]` inclusive.
  2. Append a section titled exactly
     `## Orchestrator (window-attributed, not task-attributed)` after
     `## Tool time`, as a `| field | value |` table in this order:
     `orchestrator_output_tokens`, `orchestrator_cache_create_tokens`,
     `orchestrator_sessions` (count of session files contributing at least one
     record inside the window).
  3. **Test (fail-first).** Add `scripts/tests/fixtures/transcripts/sess1.jsonl`
     (top level, beside the existing `sess1/` directory) with three assistant
     records: `2026-07-20T09:00:00Z` `output_tokens` 700 (before the window),
     `2026-07-20T10:01:00Z` `output_tokens` 400 with
     `cache_creation_input_tokens` 2000 (inside), `2026-07-20T11:00:00Z`
     `output_tokens` 900 (after). The window from the step-1/2 fixtures is
     `10:00:00Z .. 10:05:00Z`. Assert through the emitted file:
     `orchestrator_output_tokens` is 400, `orchestrator_cache_create_tokens` is
     2000, `orchestrator_sessions` is 1.
  4. Gate: `python3 -m unittest discover -s scripts/tests -t .` green. No Rust,
     WGSL or `content/` changes, so the cargo suite is not re-run.

### 4. The outcome block read out of the report files

- **Evidence:** the campaign's outcome is already written down, in prose the
  loop re-reads by hand. `audit-base.md:47-58` defines the cross-type queue note
  — a blockquote under `## Findings (implementation order)`, mirrored in both
  report files, whose entries are `finding N` / `rework N` and whose completed
  entries are wrapped in `~~ ~~`. `run-queue/SKILL.md:75-79` defines the
  mark-done convention `done YYYY-MM-DD (<plan-file>, K steps, loop-final gate
  X/X)`. `run-queue/SKILL.md:90-97` requires a five-field stop line under the
  note on every self-initiated stop (item, model tier, failure category from
  {blocked, stalled, exhausted}, what was attempted, the gate to re-verify) —
  but prescribes **no syntax**, so nothing can count them. Real shapes to parse:
  `docs/reviews/rendering/audit-rendering-2026-07-16.md:23-51` (16 entries, all
  struck, several `loop-final gate 340/340`-style tokens),
  `docs/reviews/devloop/audit-devloop-2026-07-25.md:37-58` (12 entries, 8
  struck, no gate tokens at all — a config-only campaign). Both reports'
  `## Carried forward from previous report` sections read `None.`
- **Ideal:** the emitted vector carries queue items and struck count, suite
  first→last, gate mismatches, stops by terminal state, recorded premise
  falsifications, and findings carried in — all tallied, never inferred.
- **Gap:** `scripts/campaign_report.py` does not open the report file except to
  parse its filename.
- **Suggestion:** one function `parse_report(report_path)` returning a dict; the
  fixture report drives every assertion. Two of the tokens it counts do not
  exist in any report yet — the stop line's prescribed syntax and the
  `premise-falsified:` clause — so this step defines what it parses and the next
  docs step writes the same syntax into the conventions. The exact strings, so
  both steps agree:
  - stop line: a line beginning `**STOP**` with five ` · `-separated fields,
    the third being one of `blocked`, `stalled`, `exhausted`.
  - falsification marker: the substring `premise-falsified:` anywhere in the
    blockquote note; each occurrence counts one.
- **Path:**
  1. In `scripts/campaign_report.py`, add `parse_report(report_path)`. Locate
     the `## Findings (implementation order)` heading, then take the first
     contiguous run of lines beginning with `>` as the **note**.
  2. Queue entries: within the note, take the span from its start through the
     first line ending in `.**` inclusive; find every match of
     `\b(finding|rework)\s+(\d+)\b` in that span; an entry is **struck** iff the
     count of `~~` occurrences strictly before its match offset is odd. Emit
     `queue_items` (distinct `kind number` pairs) and `queue_items_struck`.
  3. Suite counts: find every `gate\s+(\d+)/(\d+)` in the whole note, in order.
     `suite_first` is the first match's first number, `suite_last` the last
     match's first number; `gate_mismatches` is the count of matches where the
     two numbers differ. When there are no matches, print `n/a`, `n/a`, `0` —
     `audit-devloop-2026-07-25.md` is exactly this case and it must not fail.
  4. Stops: count lines in the whole report that begin with `**STOP**` and hold
     five ` · `-separated fields; tally by the third field. Emit `stops_blocked`,
     `stops_stalled`, `stops_exhausted`. A `**STOP**` line whose third field is
     not one of the three, or which has the wrong field count, increments
     `stops_malformed`.
  5. Falsifications: `premise_falsifications_recorded` = number of occurrences of
     `premise-falsified:` in the note.
  6. Carried in: `carried_forward_in` = number of lines matching `^### ` between
     the `## Carried forward from previous report` heading and the next `## `
     heading; 0 when the section is absent or contains none.
  7. Append a `## Outcome` section after the orchestrator section, as a
     `| field | value |` table in this order: `queue_items`,
     `queue_items_struck`, `suite_first`, `suite_last`, `gate_mismatches`,
     `stops_blocked`, `stops_stalled`, `stops_exhausted`, `stops_malformed`,
     `premise_falsifications_recorded`, `carried_forward_in`.
  8. **Test (fail-first).** Replace the body of the fixture report
     `scripts/tests/fixtures/reviews/demo/audit-demo-2026-07-20.md` with a
     realistic one: a `## Findings (implementation order)` heading; a blockquote
     note whose bold line is
     `> **~~finding 1 → finding 2~~ → rework 1 → finding 3.**`; a following note
     line `> findings 1–2 done 2026-07-20 (plan-demo-rework-1-2026-07-20.md, 3
     steps, loop-final gate 400/400; premise-falsified: finding 2).`; another
     `> rework 1 done 2026-07-20 (loop-final gate 402/402).`; then, below the
     note, `**STOP** finding 3 · sonnet · stalled · one fresh-spawn round · cargo
     nextest run --workspace green`; then a
     `## Carried forward from previous report` section holding two `### `
     headings; then a `## Resolved since last report` heading. Assert through
     the emitted file: `queue_items` 4, `queue_items_struck` 2, `suite_first`
     400, `suite_last` 402, `gate_mismatches` 0, `stops_stalled` 1,
     `stops_blocked` 0, `stops_exhausted` 0, `stops_malformed` 0,
     `premise_falsifications_recorded` 1, `carried_forward_in` 2. Add a second
     test over a copy of the fixture with the gate tokens removed, asserting
     `suite_first` and `suite_last` render as `n/a` and the run still exits 0.
  9. Gate: `python3 -m unittest discover -s scripts/tests -t .` green. No Rust,
     WGSL or `content/` changes, so the cargo suite is not re-run.

### 5. The outcome block read out of git — both repositories

- **Evidence:** a campaign's commits are its most durable artifact and nothing
  counts them per campaign. `run-queue/SKILL.md:104-106` has the orchestrator
  report "commit count" by hand at end of queue. The report file itself pins the
  range: `git log --diff-filter=A --format=%H -- docs/reviews/rendering/audit-rendering-2026-07-16.md`
  yields exactly one commit, `1aaa4cd0835c71a3078339694fee943bb710d3df` (dated
  2026-07-16), and `git log -1 --format=%H -- <same path>` yields the campaign's
  last touch of that report, `b59e5754b2adacc43a775e3b3352d4c55160f262` (the
  rework-6 close-out, 2026-07-17). `git rev-list --count 1aaa4cd..b59e575`
  measures 51 in the workspace repo, and `.claude` (a separate nested repository
  — `implement-finding/SKILL.md:74-81` states this explicitly) measures 0 over
  the same time span. Devloop campaigns invert that ratio: the 2026-07-25
  campaign is `.claude` commits `c3884c5`..`8eee577` with three workspace
  commits.
- **Ideal:** the vector carries commits in both repositories and the revert count
  for the campaign's commit range.
- **Gap:** `scripts/campaign_report.py` never invokes git.
- **Suggestion:** `subprocess.run` with explicit argv lists (never `shell=True`),
  `cwd` set to the repo root for the workspace repo and to `<repo>/.claude` for
  the nested one. If `<repo>/.claude/.git` does not exist, print `n/a` for the
  `.claude` fields rather than failing — the script must run on a checkout
  without the private config repo.
- **Path:**
  1. In `scripts/campaign_report.py`, resolve the range: `start` = the **last**
     line of `git log --diff-filter=A --format=%H -- <report>` (the add commit);
     `end` = `git log -1 --format=%H -- <report>`. If `start` is empty (report
     not yet committed), print `n/a` for every git field and continue with exit
     0. Add `--until SHA` to override `end`.
  2. `commits_workspace` = `git rev-list --count <start>..<end>`.
     `reverts` = count of lines from `git log --format=%s <start>..<end>` that
     match `^Revert`.
  3. `.claude` range by time: `since` = `git log -1 --format=%cI <start>`,
     `until` = `git log -1 --format=%cI <end>`; `commits_claude` =
     `git -C <repo>/.claude log --since=<since> --until=<until> --format=%H`
     line count. `n/a` when `<repo>/.claude/.git` is absent.
  4. Append these fields to the existing `## Outcome` table, after
     `carried_forward_in`, in this order: `commits_workspace`, `commits_claude`,
     `reverts`, `commit_range` (rendered `<start12>..<end12>`).
  5. **Test (fail-first).** Two tests. (a) A unit test over the real repository
     that calls the range-resolution function directly with
     `docs/reviews/rendering/audit-rendering-2026-07-16.md` and asserts `start`
     is `1aaa4cd0835c71a3078339694fee943bb710d3df` and
     `commits_workspace` is 51 — git history is append-only, so this is stable;
     skip the test with `unittest.SkipTest` when `git rev-parse --verify
     1aaa4cd0` fails, so a shallow clone does not turn into a red gate. (b) A
     test asserting that a report path that exists on disk but is not committed
     (write one into a `tempfile.TemporaryDirectory` and point `--out` there)
     renders `commits_workspace` as `n/a` and still exits 0.
  6. Gate: `python3 -m unittest discover -s scripts/tests -t .` green. No Rust,
     WGSL or `content/` changes, so the cargo suite is not re-run.

### 6. Validation run: emit the two historical campaigns and record the divergence from the hand census

- **Evidence:** the loop has exactly one hand census to check the instrument
  against, and it is precise. `audit-devloop-2026-07-17.md:3-16` reports, for the
  2026-07-16/17 rendering campaign: "76 subagent transcripts (41 sonnet workers,
  18 haiku workers, 7 fable planners, 1 opus planner, 1 haiku probe, 9 synthetic
  husks from API failures) — a full audit→10-fixes→6-reworks cycle, 65 commits,
  suite 328→400 … 9.94 agent-hours wall, tool share 19.7% … subagent output
  1,993k tokens, cache-create 17.58M". `rework-planner.md` rule 1 and the source
  research doc carry two more anchors: worst measured step **180k output
  (2026-07-16)** against a **27k** campaign median. The rework's own Path step 5
  makes "the campaign census currently recomputed by hand each audit is emitted
  by a script instead" the measured before/after of this whole rework.
- **Ideal:** `docs/campaigns/rendering-2026-07-16.md` and
  `docs/campaigns/devloop-2026-07-17.md` exist, are committed, and the rendering
  one carries a `## Divergence from the hand census` section explaining every
  field that differs from `audit-devloop-2026-07-17.md:3-16` — so the next
  auditor reads the file instead of re-deriving it.
- **Gap:** the script has never been run against a real corpus, and the
  divergences between task-based attribution and the 2026-07-17 window-based
  hand count are not written down anywhere.
- **Suggestion:** run it, commit the output, and write the divergence section
  **into the emitted file by hand as a final `## Divergence from the hand
  census` section** — it is a one-time annotation of one historical campaign, not
  a script feature. Do not add divergence logic to `scripts/campaign_report.py`.
- **Path:**
  1. Run `python3 scripts/campaign_report.py docs/reviews/rendering/audit-rendering-2026-07-16.md`
     and `python3 scripts/campaign_report.py docs/reviews/devloop/audit-devloop-2026-07-17.md`.
     Both must exit 0 and write into `docs/campaigns/`.
  2. **Hard checks — these two must hold, and a miss means the instrument is
     wrong, not the census.** In `docs/campaigns/rendering-2026-07-16.md`:
     `output_max` must be within 1,000 of **180,278** (the hand-recorded worst
     step is 180k), and `tool_share_pct` must be within 3.0 points of **19.7**.
     If either fails, **stop and report as blocked** — do not adjust the
     attribution rule or the pairing rule to make a number land. Report which
     check failed and the value obtained.
  3. **Expected divergences — record them, do not tune the script to remove
     them.** Measured during this plan's design pass with the same rules the
     script implements, the rendering campaign yields: `spawns` **64** (51
     finding-worker, 13 rework-planner) against the census's 76; models sonnet
     36 / haiku 14 / fable 5 / opus 1 / synthetic 8 against 41/18/7/1 + 1 probe +
     9 husks; `output_tokens` **≈1,769.6k** against 1,993k; `cache_create_tokens`
     **≈15.60M** against 17.58M; `agent_hours` **≈8.87** against 9.94;
     `dead_spawns` **8** against 9; `output_median_nonzero` **≈23.5k** against
     the 27k figure; `commits_workspace` **51** against 65. Every one of these
     has the same single cause: the census counted every transcript in the
     2026-07-16/17 *time window*, which also contains the concurrent networking,
     game-architecture and rust-tooling loops and the correction spawns, while
     the script attributes by the report path named in the spawn task. The 9th
     husk is a spawn that died after 8 assistant turns with 463 output tokens,
     which the `output_tokens == 0 or model == "<synthetic>"` rule correctly does
     not call dead. Numbers within ~10% of these are a pass; a number off by more
     than 25% from the value listed here means something other than attribution
     changed — stop and report it rather than editing the expectation.
  4. Append to `docs/campaigns/rendering-2026-07-16.md` a final section
     `## Divergence from the hand census` stating: the census it is compared
     against (`docs/reviews/devloop/audit-devloop-2026-07-17.md:3-16`), the two
     hard checks and their measured values, the divergence list from step 3 with
     the one-line cause, and one sentence recording that the script's numbers —
     not the census's — are the series going forward, because the census's
     denominator is a time window that cannot be reproduced.
  5. `docs/campaigns/devloop-2026-07-17.md` gets no divergence section; there is
     no hand census for it. Expect roughly `spawns` 34, `output_tokens` ≈708k,
     `output_max` ≈59k, `suite_first`/`suite_last` from that report's queue note.
     If it exits non-zero, that is a real defect — report it.
  6. Gate: `python3 -m unittest discover -s scripts/tests -t .` green (nothing in
     this step changes the module, so the suite must be unchanged), plus both
     script runs exiting 0 with the files present. This step's diff is two new
     files under `docs/campaigns/` — no Rust, WGSL or `content/` changes, so the
     cargo suite is not re-run.

### 7. `audit-devloop` reads the vector instead of re-deriving the census (docs-only)

- **Evidence:** this is the erasure that pays for the rework, and the rework
  names it ("The last question is the one that makes this worth doing — if
  nothing gets deleted, this is an addition"). Three clauses in
  `.claude/skills/audit-devloop/SKILL.md` are made redundant by
  `docs/campaigns/<domain>-<date>.md`:
  `:21` lists raw subagent transcripts in Scope as "the time/token telemetry of
  real pipeline runs"; `:27` orders the timestamp-pairing, category-attribution
  hotspot census with "Re-derive this every audit — the hotspots move as rules
  change"; `:39` requires every finding without a code test to invent its own
  before/after measurable ("a devloop finding without a measurable claim is an
  opinion, not a finding"). Note the file lives in the nested `.claude`
  repository (gitignored by vordar, pushed to TycheDea/ClaudeConfig), so this
  step's commit is a `.claude` commit.
- **Ideal:** the audit reads the emitted vector for the cost census, spends its
  transcript reads only on what the vector does not emit, and expresses a
  finding's measurable as a named vector field wherever one covers it.
- **Gap:** all three clauses still describe hand derivation.
- **Suggestion:** shorten, do not append. If the diff does not remove lines from
  `:27`, the step has not done its job.
- **Path:**
  1. `.claude/skills/audit-devloop/SKILL.md:21` (Scope): retarget the bullet to
     name `docs/campaigns/<domain>-<date>.md` as the first-class telemetry
     artifact, keeping the raw `~/.claude/projects/.../subagents/agent-*.jsonl`
     path as the fallback for what the vector does not emit.
  2. `:27` (first "What to hunt for" bullet): **delete** the hand-derivation
     sentences — the `tool_use`/`tool_result` timestamp pairing, the category
     attribution, and "Re-derive this every audit" — and replace them with a
     single sentence directing the auditor to read the campaign vector's
     `## Cost` and `## Tool time` sections (running
     `python3 scripts/campaign_report.py <report>` when no file exists yet for
     the campaign) and to re-derive only what those sections do not carry. Leave
     the entire failure-labelling clause in that bullet untouched — it is a
     separate mechanism landed today and the vector does not replace it.
  3. `:39`: change the requirement from "its Path names the measured
     before/after" to "its Path names the campaign-vector field it moves
     (`docs/campaigns/<domain>-<date>.md`); a bespoke measurable only where no
     field covers the claim".
  4. Verification, since there is no test for a skill file: after editing, show
     `git -C .claude diff --stat` and confirm the diff **removes** at least three
     lines from the `:27` bullet; and confirm by reading the file that
     `docs/campaigns/` is now named in Scope, in the hotspot bullet, and in the
     measurable clause. Nothing in the workspace repo changes in this step, so
     no workspace gate applies.

### 8. `run-queue` runs the script instead of hand-aggregating; the queue note gains a stop-line syntax and a falsification marker (docs-only)

- **Evidence:** the orchestrator derives the numbers by hand, every loop and
  every campaign. `.claude/skills/run-queue/SKILL.md:80-81` says "Report the
  loop's tokens (loop total; name any outlier spawn), then advance", and
  `:104-106` says "report the campaign aggregate: tokens per loop and total,
  commit count, and the suite count first→last" — with no script behind either,
  which is the rework's Evidence (b). Two conventions in the same file are the
  inputs the vector parses and currently have no machine-readable form:
  `:75-79`'s mark-done clause `done YYYY-MM-DD (<plan-file>, K steps, loop-final
  gate X/X)`, and `:90-97`'s five-field stop line, which names its fields (item,
  model tier, failure category, what was attempted, the gate to re-verify) but
  prescribes no separator. `audit-base.md:47-58` carries the queue-note contract
  that a fresh audit must preserve, including the stop line. All three files are
  in the nested `.claude` repository.
- **Ideal:** the orchestrator runs one command and pastes its result; the stop
  line and the falsification marker have exactly one syntax, identical in the
  convention and in the parser.
- **Gap:** the hand aggregation is still specified, and the two markers have no
  syntax.
- **Suggestion:** the script call replaces the hand list — do not keep both. The
  syntax below must match `scripts/campaign_report.py` character for character;
  it is repeated here rather than referenced because the worker executing this
  step sees only this section.
- **Path:**
  1. `.claude/skills/run-queue/SKILL.md:104-106` (End of queue): replace the
     hand aggregate with running
     `python3 scripts/campaign_report.py <report-path>`, showing the emitted
     `docs/campaigns/<domain>-<date>.md`, and committing it as part of the
     campaign's close-out. Delete the enumeration of fields to report by hand —
     the file carries them.
  2. `:80-81` (per-rework loop token report): replace the hand report with the
     same command, noting that the vector's `output_max` / `output_max_spawn`
     fields are the outlier the clause asks for.
  3. `:75-79` (mark-done convention): extend the parenthetical to
     `done YYYY-MM-DD (<plan-file>, K steps, loop-final gate X/X[, premise-falsified: <item>[, <item>]])`,
     with one sentence saying the optional clause is appended when a step's
     execution contradicted a premise the plan or the finding stated, naming the
     item, and that it is the loop's only record of that class.
  4. `:90-97` (Stopping): pin the five-field stop line to the exact form
     `**STOP** <item> · <tier> · <category> · <attempted> · <gate>` — five
     fields separated by ` · ` (space, U+00B7, space), the third being exactly
     one of `blocked`, `stalled`, `exhausted`. Keep every existing sentence about
     re-verifying the gate on resume and clearing the line.
  5. `.claude/skills/audit-base.md:47-58` (queue-note contract): add the same
     stop-line form and the `premise-falsified:` clause to the note's contract,
     so an audit rewriting the queue note carries both forward verbatim. One
     sentence each; do not restate the surrounding contract.
  6. Verification, since there is no test for a skill file: after editing, read
     back the four edited locations and confirm the stop-line form and the
     `premise-falsified:` string are byte-identical to the strings this section
     names and to the ones `scripts/campaign_report.py` parses (`**STOP**`, the
     ` · ` separator, the three category words, `premise-falsified:`). Show
     `git -C .claude diff --stat` and confirm `run-queue/SKILL.md` loses lines at
     the two aggregate sites. Nothing in the workspace repo changes.
  7. If the orchestrator names this step loop-final, run the full gate once
     (`cargo nextest run --workspace` then `cargo test --doc --workspace`) and
     report the count: no step in this plan touches Rust, WGSL or `content/`, so
     the count must equal the one HEAD was green at when the loop started. A
     changed count means something outside this plan moved — report it, do not
     chase it.

### 9. (only if the user's answer to the rework's Path step 1 is "may gate automatically") Consistency exit code

- **Evidence:** under answer (B) the script's exit code means only "a vector was
  produced" (steps 1–5). Under answer (C) the user has said the vector may gate
  something automatically. `tasks/lessons.md` carries
  `metric-cleared-picture-did-not-move` and the rework's own Tradeoff (c) —
  "a vector that cannot detect a bad campaign is worse than no vector, because it
  launders a bad campaign as a good one" — so the gate must fire on
  contradictions in the record, never on whether the campaign was good.
  `run-queue/SKILL.md:104-106` (as rewritten by step 8) is where a campaign
  close-out check would sit; `.claude/skills/run-queue/SKILL.md` is in the nested
  `.claude` repository, `scripts/campaign_report.py` in the workspace repo.
- **Ideal:** `python3 scripts/campaign_report.py --check <report>` exits 2 when
  the emitted vector contradicts itself, 1 when no vector could be produced, 0
  otherwise; without `--check` the exit code is unchanged from step 1.
- **Gap:** no consistency logic and no wiring exist.
- **Suggestion:** three predicates, all internal contradictions, no thresholds on
  any quantity that expresses worth. Do not add a token budget, an output
  ceiling, a struck-ratio floor, or any field-value threshold — if a future
  reader wants one, that is a new user decision, not an extension of this one.
- **Path:**
  1. In `scripts/campaign_report.py`, add a `--check` flag. When set, after
     emitting the file, evaluate: (a) `gate_mismatches > 0`; (b)
     `stops_malformed > 0`; (c) `queue_items_struck > 0` and
     `commits_workspace == 0` and `commits_claude` is `0` or `n/a`. Any true
     predicate prints one line per violation to stderr and returns exit code 2.
     The file is still written in every case.
  2. `.claude/skills/run-queue/SKILL.md`, End of queue: add `--check` to the
     command the step-8 edit introduced, and one sentence stating that exit 2
     means the campaign record contradicts itself and the orchestrator reports it
     to the user before striking the last entry — it never blocks a commit that
     is already green, and it never judges the campaign's worth.
  3. **Test (fail-first).** In `scripts/tests/test_campaign_report.py`, add three
     tests driving `main(argv)` with `--check` over copies of the fixture report
     `scripts/tests/fixtures/reviews/demo/audit-demo-2026-07-20.md`: one whose
     note carries `loop-final gate 400/402` (asserts exit 2, and that the emitted
     file still exists); one whose stop line has four ` · `-separated fields
     instead of five (asserts exit 2); and the unmodified fixture (asserts exit
     0 with `--check`). Assert the violation text appears on stderr for the first
     two.
  4. Gate: `python3 -m unittest discover -s scripts/tests -t .` green. The
     workspace-repo diff is `scripts/campaign_report.py` and the test file only;
     no Rust, WGSL or `content/` changes, so the cargo suite is not re-run.
