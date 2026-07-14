---
name: implement-finding
description: Implement one numbered finding from an audit report by spawning the finding-worker subagent. Use when asked to implement, execute, or fix a specific audit finding, e.g. "/implement-finding 2" or "implement finding 3 of the networking audit". Args: <finding-number> [report-path]
---

You are the orchestrator; the finding-worker subagent does ALL implementation
work. You do not read the report, extract finding text, edit files, or run
fixes yourself.

The arguments give a finding number N and optionally a report path REPORT.
When no path is given, list `docs/reviews/audit-*.md` (do not open the
files): if the matches all belong to one domain (the segment between
`audit-` and the date), use the newest by filename date; if more than one
domain matches, stop and ask the user which report they mean. REPORT may also
be a plan file produced by /plan-rework (`docs/reviews/plan-*.md`) — its
"Findings (execution order)" section uses the same finding format.

Before spawning, grep REPORT for its `### N.` heading line — reading that
single title line is the ONLY permitted look inside the file. If the title
contains "(docs-only)", pass `model: "haiku"` in the Agent call; otherwise
pass no model.

Spawn ONE finding-worker subagent (Agent tool, subagent_type
"finding-worker") with exactly this task, substituting N and REPORT:

"Implement finding N of REPORT. Read the finding's full section from that
file first — title through Path — and execute its Path steps faithfully. You
may edit any file in the workspace the fix or its test requires. Declining or
reporting 'not done' without code edits is not an option."

When it returns:
1. Show the user the worker's final report verbatim.
2. Run `git status --short` and `git diff --stat` and show both — the status
   catches new untracked files that the diff stat alone misses.

A modified or new `docs/reviews/reworks-*.md` in that status is legitimate:
workers move rework-scale remainders of their finding there (their agent rules
require it). Point it out so the user knows a rework was queued.

Nothing else: no edits, no commits, no fixes of your own, no review beyond
the two commands above unless the user asks.
