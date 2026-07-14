# Shared audit contract

Every `audit-*` skill runs under this contract. The invoking skill supplies
the persona and these parameters: the **domain** slug (also its report folder
name under `docs/reviews/`), the **report title**, the **ordering impact
axis**, the **ideal-end-state hint**, the **sweep** instruction, plus its
Scope, its "What to hunt for" list, and any extra requirements. Extras add to
this contract, never replace it.

## Mission

Find improvements and suggestions — of any kind, at any scale — within the
skill's Scope. You implement nothing. Your sole deliverable is a written
report.

## Non-negotiables

1. **No laziness.** You read the actual code and files, not just their names. Every finding cites concrete evidence (`file:line`, a specific entry, a measured number). Generic advice that could apply to any repo is forbidden — if a finding doesn't reference something specific you saw here, delete it. Do not stop early because the sweep is long; incomplete coverage is a failed audit.
2. **The bar is the best possible final state.** Judge everything against the top of the top — the ideal end state this project could reach in the skill's domain. Never write "this is enough", "good enough for now", "sufficient for the current state", or any equivalent middle-ground framing. If something falls short of the ideal, it is a finding, no matter how many steps lie between here and there. Distance to the ideal is recorded, never used as an excuse to lower the bar.
3. **Report only. No implementations.** The only files you may create are the report files. You must not modify source, configs, docs, diagrams, scripts, schemas, or assets — not even "trivial" fixes you notice along the way.

## Method

1. Check `docs/reviews/<domain>/` for the most recent `audit-<domain>-*.md` and `reworks-<domain>-*.md` reports. Carry forward every unresolved finding (re-verify each; drop resolved ones and say so).
2. Sweep the full Scope, the way the skill's **sweep** instruction describes.
3. For each finding, define the ideal end state first, then measure the gap.
4. Weigh findings by the skill's **ordering impact axis** — but ORDER them in the report by implementation order: a finding goes before another when implementing it first makes the other easier, safer, or properly testable (test/tooling infrastructure and prerequisite mechanisms first, dependents after). Among findings with no dependency between them, higher impact goes first. Never order by ease of fixing. State the reason inline (e.g. "before finding 5: provides the impairment knob its test needs") whenever a dependency, not impact, decided the position.
5. Headless verification only — never launch the game. Reason from code and files; where a claim needs runtime confirmation, say exactly what test or measurement would confirm it.

## Report

Split findings into two categories and two files under `docs/reviews/<domain>/`
(create the folder if it doesn't exist; today's date):

- `docs/reviews/<domain>/audit-<domain>-YYYY-MM-DD.md` - **fixes and small changes**:
  findings a worker can land surgically in one run - a bounded diff plus a regression
  test, no new subsystem, no schema/protocol redesign, no cross-crate architecture
  shift.
- `docs/reviews/<domain>/reworks-<domain>-YYYY-MM-DD.md` - **reworks and big new
  features**: findings that need a design pass before anyone should write code (new
  subsystem, schema/protocol change, auth, architecture shift). These are consumed by
  /plan-rework, which turns one rework into a plan of fix-sized steps that
  /implement-finding can then execute one by one.

When one finding contains both (a surgical step plus rework-scale follow-ons), put the
surgical step in the fixes file and the follow-ons in the reworks file, each referencing
the other. Number findings independently within each file. The implementation-order
note is ONE cross-type sequence spanning BOTH files - dependencies cross the
fix/rework boundary (a rework can be the prerequisite of a fix and vice versa) - so
write a single ordered queue mixing `finding N` (fixes file) and `rework N` (reworks
file) entries, placed under the fixes file's "## Findings (implementation order)"
heading and mirrored verbatim in the reworks file. A rework whose own gate is unmet
(e.g. gated on a measurement not yet taken) is listed as parked with its gate stated,
not given a position. Both files use this structure (the skill may add finding
bullets — e.g. a Tradeoffs bullet — but never remove these):

```
# <Report title> — YYYY-MM-DD

## Ideal end state
<2–5 sentences: what "top of the top" looks like — see the skill's ideal-end-state hint>

## Findings (implementation order)
### 1. <title>
- **Evidence:** file:line references and what you observed
- **Ideal:** what the best possible version looks like
- **Gap:** why the current state falls short
- **Suggestion:** concrete direction (no changes made — this is a recommendation)
- **Path:** the steps from here to the ideal, however many there are

## Carried forward from previous report
<unresolved prior findings, re-verified>

## Resolved since last report
<prior findings that no longer apply>
```

Every finding must be actionable by a developer who reads only the report.
