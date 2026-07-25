"""Per-campaign token/cost attribution report.

Scans Claude Code subagent transcripts under a project's `.claude/projects/<mangled>`
directory, attributes each spawn to a campaign by matching its first user message
against that campaign's audit/reworks/plan-rework report paths, and writes a
`## Cost` breakdown to docs/campaigns/<domain>-<date>.md.
"""

import argparse
import json
import re
import statistics
import sys
from collections import Counter
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path

REPORT_FILENAME_RE = re.compile(r"^audit-(?P<domain>.+)-(?P<date>\d{4}-\d{2}-\d{2})\.md$")
SYNTHETIC_MODEL = "<synthetic>"

TOOL_CATEGORIES = ("test", "compile", "shell", "edit", "read", "other")
_TEST_CMD_RE = re.compile(r"cargo\s+(nextest|test)")
_COMPILE_CMD_RE = re.compile(r"cargo\s+(build|check|clippy)")


def categorize_tool(name, command):
    if name in ("Bash", "PowerShell"):
        cmd = command or ""
        if _TEST_CMD_RE.search(cmd):
            return "test"
        if _COMPILE_CMD_RE.search(cmd):
            return "compile"
        return "shell"
    if name in ("Edit", "Write", "NotebookEdit"):
        return "edit"
    if name in ("Read", "Glob", "Grep"):
        return "read"
    return "other"


def _parse_ts(ts):
    return datetime.fromisoformat(ts.replace("Z", "+00:00"))


@dataclass
class Attribution:
    domain: str
    date: str


@dataclass
class TranscriptScan:
    first_user_text: str
    assistant_records: list
    tool_seconds: dict = field(default_factory=dict)


@dataclass
class Spawn:
    agent_id: str
    agent_type: str
    model: str
    output_tokens: int
    cache_creation_input_tokens: int
    cache_read_input_tokens: int
    first_ts: str
    last_ts: str
    task_text: str
    dead: bool
    attributed: bool
    tool_seconds: dict = field(default_factory=dict)


def attribution_patterns(report_path):
    name = Path(report_path).name
    m = REPORT_FILENAME_RE.match(name)
    if not m:
        raise ValueError(f"report filename does not match audit-<domain>-<date>.md: {name}")
    return Attribution(domain=m.group("domain"), date=m.group("date"))


def is_attributed(text, domain, date):
    if not text:
        return False
    audit_lit = f"docs/reviews/{domain}/audit-{domain}-{date}.md"
    reworks_lit = f"docs/reviews/{domain}/reworks-{domain}-{date}.md"
    if audit_lit in text or reworks_lit in text:
        return True
    plan_re = re.compile(
        re.escape(f"docs/reviews/{domain}/plan-{domain}-rework-")
        + r"\d+-(\d{4}-\d{2}-\d{2})"
        + re.escape(".md")
    )
    for match in plan_re.finditer(text):
        if match.group(1) >= date:
            return True
    return False


def _extract_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for block in content:
            if isinstance(block, dict) and block.get("type") == "text":
                parts.append(block.get("text", ""))
        return "".join(parts)
    return ""


def scan_transcript(jsonl_path):
    first_user_text = None
    assistant_records = []
    open_tool_uses = {}
    tool_seconds = {category: 0.0 for category in TOOL_CATEGORIES}
    with open(jsonl_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            record = json.loads(line)
            rtype = record.get("type")
            message = record.get("message", {})
            if rtype == "user" and first_user_text is None:
                first_user_text = _extract_text(message.get("content"))
            elif rtype == "assistant":
                usage = message.get("usage", {})
                assistant_records.append({
                    "timestamp": record.get("timestamp"),
                    "model": message.get("model"),
                    "output_tokens": usage.get("output_tokens", 0),
                    "cache_creation_input_tokens": usage.get("cache_creation_input_tokens", 0),
                    "cache_read_input_tokens": usage.get("cache_read_input_tokens", 0),
                })

            content = message.get("content")
            if not isinstance(content, list):
                continue
            for block in content:
                if not isinstance(block, dict):
                    continue
                btype = block.get("type")
                if btype == "tool_use":
                    tool_id = block.get("id")
                    ts = record.get("timestamp")
                    if not tool_id or not ts:
                        continue
                    input_ = block.get("input")
                    command = input_.get("command") if isinstance(input_, dict) else None
                    open_tool_uses[tool_id] = (block.get("name"), command, ts)
                elif btype == "tool_result":
                    tool_id = block.get("tool_use_id")
                    result_ts = record.get("timestamp")
                    if tool_id not in open_tool_uses or not result_ts:
                        continue
                    name, command, use_ts = open_tool_uses.pop(tool_id)
                    duration = (_parse_ts(result_ts) - _parse_ts(use_ts)).total_seconds()
                    if 0 <= duration < 3600:
                        tool_seconds[categorize_tool(name, command)] += duration
    return TranscriptScan(
        first_user_text=first_user_text or "",
        assistant_records=assistant_records,
        tool_seconds=tool_seconds,
    )


def collect_spawns(transcripts_dir, patterns):
    spawns = []
    for jsonl_path in sorted(Path(transcripts_dir).glob("*/subagents/agent-*.jsonl")):
        meta = {}
        meta_path = jsonl_path.with_name(jsonl_path.stem + ".meta.json")
        if meta_path.is_file():
            try:
                meta = json.loads(meta_path.read_text(encoding="utf-8"))
            except (json.JSONDecodeError, OSError):
                meta = {}

        scan = scan_transcript(jsonl_path)
        agent_id = jsonl_path.stem
        if agent_id.startswith("agent-"):
            agent_id = agent_id[len("agent-"):]

        output_tokens = sum(r["output_tokens"] for r in scan.assistant_records)
        cache_create = sum(r["cache_creation_input_tokens"] for r in scan.assistant_records)
        cache_read = sum(r["cache_read_input_tokens"] for r in scan.assistant_records)
        timestamps = [r["timestamp"] for r in scan.assistant_records if r["timestamp"]]
        dead = output_tokens == 0 or any(r["model"] == SYNTHETIC_MODEL for r in scan.assistant_records)

        model = meta.get("model")
        if not model:
            seen = sorted({r["model"] for r in scan.assistant_records if r["model"]})
            model = ", ".join(seen) if seen else "unknown"

        spawns.append(Spawn(
            agent_id=agent_id,
            agent_type=meta.get("agentType"),
            model=model,
            output_tokens=output_tokens,
            cache_creation_input_tokens=cache_create,
            cache_read_input_tokens=cache_read,
            first_ts=min(timestamps) if timestamps else None,
            last_ts=max(timestamps) if timestamps else None,
            task_text=scan.first_user_text[:80],
            dead=dead,
            attributed=is_attributed(scan.first_user_text, patterns.domain, patterns.date),
            tool_seconds=scan.tool_seconds,
        ))
    return spawns


def _window_bounds(spawns):
    starts = [s.first_ts for s in spawns if s.first_ts]
    ends = [s.last_ts for s in spawns if s.last_ts]
    if not starts or not ends:
        return None
    return (min(starts), max(ends))


def render_cost_section(spawns):
    attributed = [s for s in spawns if s.attributed]
    window = _window_bounds(attributed)

    total = len(attributed)
    finding_worker = sum(1 for s in attributed if s.agent_type == "finding-worker")
    rework_planner = sum(1 for s in attributed if s.agent_type == "rework-planner")
    other = total - finding_worker - rework_planner

    model_counts = Counter(s.model for s in attributed)
    by_model = ", ".join(
        f"{name} {count}" for name, count in sorted(model_counts.items(), key=lambda kv: (-kv[1], kv[0]))
    )

    unattributed_in_window = 0
    if window:
        start, end = window
        for s in spawns:
            if not s.attributed and s.first_ts and start <= s.first_ts <= end:
                unattributed_in_window += 1

    output_totals = [s.output_tokens for s in attributed]
    output_sum = sum(output_totals)
    nonzero = [v for v in output_totals if v > 0]
    output_max = max(output_totals) if output_totals else 0
    output_max_spawn_obj = max(attributed, key=lambda s: s.output_tokens) if attributed else None
    output_max_spawn = (
        f"{output_max_spawn_obj.agent_id}: {output_max_spawn_obj.task_text}" if output_max_spawn_obj else ""
    )
    output_median_nonzero = statistics.median_low(nonzero) if nonzero else 0

    cache_create = sum(s.cache_creation_input_tokens for s in attributed)
    cache_read = sum(s.cache_read_input_tokens for s in attributed)
    dead = sum(1 for s in attributed if s.dead)

    rows = [
        ("spawns", total),
        ("spawns_finding_worker", finding_worker),
        ("spawns_rework_planner", rework_planner),
        ("spawns_other", other),
        ("spawns_by_model", by_model),
        ("unattributed_spawns_in_window", unattributed_in_window),
        ("output_tokens", output_sum),
        ("output_max", output_max),
        ("output_max_spawn", output_max_spawn),
        ("output_median_nonzero", output_median_nonzero),
        ("cache_create_tokens", cache_create),
        ("cache_read_tokens", cache_read),
        ("dead_spawns", dead),
    ]

    lines = ["## Cost", "", "| field | value |", "| --- | --- |"]
    lines.extend(f"| {name} | {value} |" for name, value in rows)
    return "\n".join(lines) + "\n"


def _span_hours(spawn):
    if not spawn.first_ts or not spawn.last_ts:
        return 0.0
    return (_parse_ts(spawn.last_ts) - _parse_ts(spawn.first_ts)).total_seconds() / 3600.0


def _fmt_seconds(value):
    return f"{value:g}"


def render_tool_time_section(spawns):
    attributed = [s for s in spawns if s.attributed]

    agent_hours = sum(_span_hours(s) for s in attributed)
    totals = {category: 0.0 for category in TOOL_CATEGORIES}
    for s in attributed:
        for category in TOOL_CATEGORIES:
            totals[category] += s.tool_seconds.get(category, 0.0)
    tool_hours = sum(totals.values()) / 3600.0
    tool_share_pct = f"{100 * tool_hours / agent_hours:.1f}" if agent_hours else "n/a"

    rows = [
        ("agent_hours", f"{agent_hours:.4f}"),
        ("tool_hours", f"{tool_hours:.4f}"),
        ("tool_share_pct", tool_share_pct),
        ("tool_seconds_test", _fmt_seconds(totals["test"])),
        ("tool_seconds_compile", _fmt_seconds(totals["compile"])),
        ("tool_seconds_shell", _fmt_seconds(totals["shell"])),
        ("tool_seconds_edit", _fmt_seconds(totals["edit"])),
        ("tool_seconds_read", _fmt_seconds(totals["read"])),
        ("tool_seconds_other", _fmt_seconds(totals["other"])),
    ]

    lines = ["## Tool time", "", "| field | value |", "| --- | --- |"]
    lines.extend(f"| {name} | {value} |" for name, value in rows)
    return "\n".join(lines) + "\n"


def collect_orchestrator_totals(transcripts_dir, window):
    if not window:
        return 0, 0, 0
    start, end = window
    output_sum = 0
    cache_create_sum = 0
    session_count = 0
    for jsonl_path in sorted(Path(transcripts_dir).glob("*.jsonl")):
        scan = scan_transcript(jsonl_path)
        in_window = [
            r for r in scan.assistant_records
            if r["timestamp"] and start <= r["timestamp"] <= end
        ]
        if in_window:
            session_count += 1
            output_sum += sum(r["output_tokens"] for r in in_window)
            cache_create_sum += sum(r["cache_creation_input_tokens"] for r in in_window)
    return output_sum, cache_create_sum, session_count


def render_orchestrator_section(transcripts_dir, window):
    output_sum, cache_create_sum, session_count = collect_orchestrator_totals(transcripts_dir, window)
    rows = [
        ("orchestrator_output_tokens", output_sum),
        ("orchestrator_cache_create_tokens", cache_create_sum),
        ("orchestrator_sessions", session_count),
    ]
    lines = ["## Orchestrator (window-attributed, not task-attributed)", "", "| field | value |", "| --- | --- |"]
    lines.extend(f"| {name} | {value} |" for name, value in rows)
    return "\n".join(lines) + "\n"


def _mangle_repo_root(repo_root):
    s = str(repo_root)
    for ch in [":", "\\", "/", "_", "."]:
        s = s.replace(ch, "-")
    return s


def _render_header(domain, date, report_path, attributed_spawns):
    window = _window_bounds(attributed_spawns)
    window_text = f"{window[0]} .. {window[1]}" if window else "(no timestamps)"
    lines = [
        f"# Campaign vector — {domain} {date}",
        "",
        f"Emitted by `scripts/campaign_report.py` from `{report_path}`.",
        f"Window: {window_text}",
        "Attribution: spawns whose task names this campaign's audit/reworks/plan files.",
        "Not counted: correction spawns (they name no report) — see",
        "unattributed_spawns_in_window.",
    ]
    return "\n".join(lines)


def main(argv):
    parser = argparse.ArgumentParser(prog="campaign_report.py")
    parser.add_argument("report")
    parser.add_argument("--transcripts", default=None)
    parser.add_argument("--out", default=None)
    args = parser.parse_args(argv)

    repo_root = Path(__file__).resolve().parent.parent
    report_path = Path(args.report)
    if not report_path.is_file():
        print(f"error: report file not found: {report_path}", file=sys.stderr)
        return 1

    try:
        attribution = attribution_patterns(str(report_path))
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.transcripts:
        transcripts_dir = Path(args.transcripts)
    else:
        transcripts_dir = Path.home() / ".claude" / "projects" / _mangle_repo_root(repo_root)

    if not transcripts_dir.is_dir():
        print(f"error: transcripts directory not found: {transcripts_dir}", file=sys.stderr)
        return 1

    spawns = collect_spawns(transcripts_dir, attribution)
    attributed = [s for s in spawns if s.attributed]
    if not attributed:
        print(
            f"error: zero spawns attributed to {attribution.domain} {attribution.date} "
            f"under {transcripts_dir}",
            file=sys.stderr,
        )
        return 1

    out_dir = Path(args.out) if args.out else repo_root / "docs" / "campaigns"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / f"{attribution.domain}-{attribution.date}.md"

    header = _render_header(attribution.domain, attribution.date, args.report, attributed)
    cost_section = render_cost_section(spawns)
    tool_time_section = render_tool_time_section(spawns)
    window = _window_bounds(attributed)
    orchestrator_section = render_orchestrator_section(transcripts_dir, window)
    out_path.write_text(
        header + "\n" + cost_section + tool_time_section + orchestrator_section, encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
