# Campaign vector — rendering 2026-07-16

Emitted by `scripts/campaign_report.py` from `docs/reviews/rendering/audit-rendering-2026-07-16.md`.
Window: 2026-07-16T10:24:31.605Z .. 2026-07-25T16:37:25.287Z
Attribution: spawns whose task names this campaign's audit/reworks/plan files.
Not counted: correction spawns (they name no report) — see
unattributed_spawns_in_window.
## Cost

| field | value |
| --- | --- |
| spawns | 68 |
| spawns_finding_worker | 55 |
| spawns_rework_planner | 13 |
| spawns_other | 0 |
| spawns_by_model | claude-sonnet-5 36, claude-haiku-4-5-20251001 13, <synthetic> 8, claude-fable-5 5, opus 2, sonnet 2, claude-opus-4-8 1, haiku 1 |
| unattributed_spawns_in_window | 339 |
| output_tokens | 1851183 |
| output_max | 180278 |
| output_max_spawn | a3cb2e009c07cb328: Implement finding 5 of docs/reviews/rendering/plan-rendering-rework-6-2026-07-16 |
| output_median_nonzero | 25560 |
| cache_create_tokens | 16029830 |
| cache_read_tokens | 456470099 |
| dead_spawns | 8 |
## Tool time

| field | value |
| --- | --- |
| agent_hours | 8.6152 |
| tool_hours | 1.7685 |
| tool_share_pct | 20.5 |
| tool_seconds_test | 2123.06 |
| tool_seconds_compile | 222.41 |
| tool_seconds_shell | 986.851 |
| tool_seconds_edit | 2911.11 |
| tool_seconds_read | 123.019 |
| tool_seconds_other | 0 |
## Orchestrator (window-attributed, not task-attributed)

| field | value |
| --- | --- |
| orchestrator_output_tokens | 10414343 |
| orchestrator_cache_create_tokens | 30043170 |
| orchestrator_sessions | 22 |
## Outcome

| field | value |
| --- | --- |
| queue_items | 16 |
| queue_items_struck | 16 |
| suite_first | 347 |
| suite_last | 393 |
| gate_mismatches | 0 |
| stops_blocked | 0 |
| stops_stalled | 0 |
| stops_exhausted | 0 |
| stops_malformed | 0 |
| premise_falsifications_recorded | 0 |
| carried_forward_in | 0 |
| commits_workspace | 51 |
| commits_claude | 0 |
| reverts | 0 |
| commit_range | 1aaa4cd0835c..b59e5754b2ad |
## Divergence from the hand census

Compared against the hand census in `docs/reviews/devloop/audit-devloop-2026-07-17.md:3-16`,
which counted the 2026-07-16/17 rendering campaign by hand.

Calibration checks, both passed:

| check | expected | measured |
| --- | --- | --- |
| output_max | 180,278 ± 1,000 | 180,278 |
| tool_share_pct | 19.7 ± 3.0 | 20.5 |

Field-by-field divergence:

| field | script | hand census |
| --- | --- | --- |
| spawns | 68 (55 finding-worker, 13 rework-planner) | 76 |
| models | sonnet 38, haiku 14, synthetic 8, fable 5, opus 3 | sonnet 41, haiku 18, fable 7, opus 1, haiku probe 1, husks 9 |
| output_tokens | 1,851,183 | ~1,993,000 |
| cache_create_tokens | 16,029,830 | ~17,580,000 |
| agent_hours | 8.62 | 9.94 |
| dead_spawns | 8 | 9 |
| output_median_nonzero | 25,560 | ~27,000 |
| commits_workspace | 51 | 65 |

Every divergence above has one cause: the census's denominator is the
2026-07-16/17 *time window*, which also contains the concurrent networking,
game-architecture and rust-tooling loops and the correction spawns, while the
script attributes a spawn to the campaign whose report path its task names.
The script's model counts sum raw model aliases as they appear in the
transcripts (`sonnet` and `claude-sonnet-5` are the same model, counted
separately in the `spawns_by_model` row above).

One census entry differs for a second reason: the 9th "synthetic husk" is a
spawn that died after 8 assistant turns having emitted 463 output tokens, so
the `output_tokens == 0 or model == "<synthetic>"` rule correctly does not
count it dead.

The script's numbers, not the census's, are the series going forward: the
census's denominator is a time window that cannot be reproduced, while
path attribution can be recomputed from the transcripts at any time.
