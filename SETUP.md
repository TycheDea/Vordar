# AI setup

How this repo is wired for Claude Code, and how to get the most out of it.
Configuration lives in `.claude/`, which is gitignored here and tracked
separately at `TycheDea/ClaudeConfig` — push there after editing a skill or
agent, or the change exists only on this machine.

## The pieces

### Standing instructions

| File | Scope | Carries |
|---|---|---|
| `~/.claude/CLAUDE.md` | every project | approximate-requests/exact-intent, root-cause-always, no-gambiarra, workflow (plan mode, lessons, subagents), chat style, the erasure mandate |
| `.claude/CLAUDE.md` | this repo | think-before-coding, simplicity, surgical changes, goal-driven execution, the comment policy, escalation rules, batched test cadence, heavy-compute go-ahead, compact-at-phase-gates, `reference/` exclusion |
| `.claude/DESIGN.md` | this repo | the design doc source clauses cite as `DESIGN.md §N` |
| `~/.claude/projects/<repo>/memory/` | every session | durable rulings (art direction, licensing stance, orchestration model), indexed by `MEMORY.md` |

The two CLAUDE.md files load automatically. Memory loads as background context —
it records what was true when written, so a memory naming a file or flag is a
lead, not a fact.

### Hooks (`.claude/settings.json` → `scripts/hooks/`)

Non-negotiable, run by the harness rather than by the model:

- **PreToolUse** on `Bash`/`PowerShell` — `deny_dangerous.mjs` blocks force-pushes
  and recursive force-deletes outside the scratchpad.
- **PostToolUse** on `Edit`/`Write` — three linters fire on the file just touched:
  `comment_lint_hook.mjs` (the §5 comment policy, workspace crates only),
  `wgsl_hook.mjs` (`naga` on standalone shaders), `test_shape_hook.mjs`
  (vacuous-test scan, reporting only what your diff introduces).

All four degrade to exit 0 on any internal failure — they block bad edits, never
the session.

### Skills (`.claude/skills/`)

**Audits** — `audit-rendering`, `audit-networking`, `audit-game-architecture`,
`audit-rust-tooling`, `audit-hygiene`, `audit-content-pipeline`,
`audit-project-meta`, `audit-devloop`. Each is a domain persona over one shared
contract (`audit-base.md`): report-only, evidence-cited, judged against the best
possible end state, never "good enough". Output is two files under
`docs/reviews/<domain>/` — `audit-*.md` (fix-sized) and `reworks-*.md`
(needs a design pass first) — ordered by implementation order, with a queue note
mirrored in both.

**Execution** — `/implement-finding N [report]` spawns the `finding-worker`
agent on exactly one finding. `/plan-rework N [reworks]` spawns `rework-planner`
to turn one rework into fix-sized steps. `/run-queue [report]` walks a whole
queue: fixes loop automatically, each rework gates on one go/stop, green steps
commit, queue notes strike, tokens reported per loop.

### Agents (`.claude/agents/`)

- `finding-worker` (sonnet) — faithful execution of one finding's Path, test
  first, never commits.
- `rework-planner` (fable) — reads the finding and the lessons index, writes a
  plan document, writes no code.

Model routing is a standing rule, not a preference: **fable orchestrates and
analyses, sonnet implements, opus judges anything visual** — and opus is the
backup orchestrator, taken up only when fable is out of credits. The
orchestrator never implements, whichever tier is holding the seat.

### Working files

- `tasks/todo.md` — the live campaign log. Checkable items, decisions recorded
  with their evidence, "decided while unsure" batched for the next checkpoint.
- `tasks/lessons.md` + `tasks/lessons/` — the error register. The index carries
  **γ (when it fires) + link + status**; the rule itself lives only in the note,
  or the two drift. Review it at session start; entries are persistent, quiet,
  or absorbed by a named mechanism.
- `docs/reviews/<domain>/` — audit output and gate records.
- `docs/visual-quality.md` — the `VQ-*` clauses lints and asserts cite.
- `tasks/town/`, `tasks/ai-pipeline/` — per-campaign specs and research.

### Tooling

`rustrover-index` MCP for code navigation (find usages, definitions, hierarchies)
— preferred over shell grep on source. `scripts/` holds the linters the hooks
call plus `bench-gate.ps1`, `token-report.ps1`, `campaign_report.py`,
`stress-suite.ps1`, and the asset pipeline under `scripts/asset-pipeline/`.

## Driving it

### Starting a session

`/model fable` — fable holds the orchestrator seat. Fall back to `/model opus`
only when fable is out of credits; opus costs more per turn and is otherwise
better spent as a subagent on visual gates and hard analysis. Then say what you
want; the session bootstraps itself from `tasks/lessons.md` and `tasks/todo.md`,
so you never have to re-explain where the last one stopped.

Open a session per phase, not per day. `/clear` when the next task is unrelated
to the finished one, `/compact` when it continues it.

### Saying what you want

**Be approximate on purpose.** The standing instruction is that your intent
outranks your literal words — a pointer at the outcome beats a specification of
the steps, and over-specifying is how you get a worse design executed faithfully.
Say what "done" looks like, not how to get there.

**Name a goal, not a task list, for anything multi-step.** "The chapel doesn't
read as a chapel" produced a better pass than a numbered list of features would
have. It will plan, and you approve or redirect the plan.

**For bugs, just paste it.** Error text, failing test, a screenshot path. No
framing needed — bug reports are handled autonomously through to the fix.

**Say when you're low on credits or in a hurry.** That changes what gets picked
up: small closable items instead of a campaign step that would strand half-done.

### The pipeline

The cheapest path through real work is three moves:

1. `audit the rendering` (or networking, hygiene, dev loop, …) — produces
   `docs/reviews/<domain>/audit-*.md` and `reworks-*.md`, ordered, with a queue
   note. Costs a lot of reading; do it once per domain, not per question.
2. `/run-queue` — walks that queue end to end. Fixes land and commit
   automatically; each rework stops once for your go/stop; you get a token
   report per loop.
3. `/implement-finding N` or `/plan-rework N` when you want one item rather
   than the whole queue.

Ad-hoc "fix this" works, but it skips the evidence and the ordering, and you
pay for that later in rework.

### Questions it will put to you

**Answer the batch at the checkpoint.** Decisions are deliberately bundled
rather than asked one at a time — scope, licensing, anything irreversible, and
any genuine fork where both options end well. If it presents options with
outcome / confidence / cost, the confidence line is the one to read: a
high-outcome, low-confidence option usually means "let me run the cheap probe
first" is the better answer.

**Approving a plan that lists GPU runs is the go-ahead for exactly those runs.**
Generation work — textures, HDRIs, meshes, seed sweeps — otherwise waits for you
with a wall-time estimate. Compiles, test suites and seconds-scale smoke renders
never ask.

**Correct once.** A correction becomes a lesson note under `tasks/lessons/`, and
the index is re-read at the start of every session, so you should not have to
give the same correction twice. If you do, the lesson's trigger was written too
narrowly — say so and it gets widened.

### What not to ask for

- **A GUI run.** Verification is headless by ruling: offscreen renders through
  `zone_review` / `asset_inspect`, judged by opus. Ask for frames, not a window.
- **A test run after every change.** The cadence is one suite run per batch plus
  one to confirm; asking per-change just burns tokens.
- **A quick patch around a wall.** Flags, shims, parallel paths and
  dodge-the-rule test rewrites are refused by standing instruction. If something
  is blocked you will be told it is blocked, which is the useful answer.

### Housekeeping

- Say **"save the state"** to end cleanly — notes get written, nothing new gets
  started.
- Ask for the **token report** (`scripts/token-report.ps1`) when you want to see
  where a campaign's budget went.
- After any skill or agent edit, have it **push `.claude/` to
  `TycheDea/ClaudeConfig`** — that directory is gitignored here, so an unpushed
  change exists only on this machine.
