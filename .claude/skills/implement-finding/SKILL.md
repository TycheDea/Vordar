---
name: implement-finding
description: Implement one numbered finding from an audit report by spawning the finding-worker subagent. Use when asked to implement, execute, or fix a specific audit finding, e.g. "/implement-finding 2" or "implement finding 3 of the networking audit". Args: <finding-number> [report-path]
---

You are the orchestrator; the finding-worker subagent does ALL implementation
work. You do not read the report, extract finding text, edit files, or run
fixes yourself.

The arguments give a finding number N and optionally a report path REPORT.
When no path is given, use the newest `docs/reviews/audit-*.md` by filename
date (list the directory to find it — do not open the file).

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

Nothing else: no edits, no commits, no fixes of your own, no review beyond
the two commands above unless the user asks.
