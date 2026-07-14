---
name: plan-rework
description: Design an implementation plan for one rework-scale finding by spawning the rework-planner subagent. Use when asked to plan, design, or break down a rework or big feature from a reworks report, e.g. "/plan-rework 1" or "plan rework 2 of the networking reworks". Args: <rework-number> [reworks-path]
---

You are the orchestrator; the rework-planner subagent does ALL design work.
You do not read the reworks report, extract finding text, or design anything
yourself.

The arguments give a rework finding number N and optionally a reworks report
path REPORT. When no path is given, list `docs/reviews/reworks-*.md` (do not
open the files): if the matches all belong to one domain (the segment between
`reworks-` and the date), use the newest by filename date; if more than one
domain matches, stop and ask the user which report they mean.

Spawn ONE rework-planner subagent (Agent tool, subagent_type "rework-planner")
with exactly this task, substituting N and REPORT:

"Design the implementation plan for rework finding N of REPORT. Read the
finding's full section from that file first, study every part of the codebase
the design touches, and write the plan document as your agent instructions
specify. You write no code. Reporting 'not done' without a written plan file
is not an option."

When it returns:
1. Show the user the planner's final report verbatim.
2. Run `git status --short` and show it — the plan file should be the ONLY
   new artifact; anything else is out of bounds and must be flagged.

The plan file's "Findings (execution order)" section uses the audit fix
format, so each step is executed afterwards with
`/implement-finding <k> <plan-file-path>`.

Nothing else: no edits, no design opinions of your own, no review beyond the
command above unless the user asks.
