#!/usr/bin/env bash
# Checks the anchor-resolution clause audit-base.md states but never verifies:
# every `path:line` anchor whose path contains a `/` must resolve to a real
# file, and its last line number must not exceed that file's line count.
#
#   bash scripts/lint-findings.sh <report.md> [more.md ...]
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [ "$#" -eq 0 ]; then
  echo "usage: bash scripts/lint-findings.sh <report.md> [more.md ...]" >&2
  exit 1
fi
files=("$@")

search_dirs=(
  "$repo_root"
  "$repo_root/.claude/skills"
  "$repo_root/.claude/agents"
  "$repo_root/.claude"
  "$HOME/.claude/skills"
  "$HOME/.claude/agents"
  "$HOME/.claude"
)

hits=0

for f in "${files[@]}"; do
  while IFS= read -r anchor; do
    path="${anchor%%:*}"
    case "$path" in
      */*) : ;;
      *) continue ;;
    esac
    last="${anchor##*-}"; last="${last##*:}"

    resolved=""
    for dir in "${search_dirs[@]}"; do
      if [ -f "$dir/$path" ]; then
        resolved="$dir/$path"
        break
      fi
    done

    if [ -z "$resolved" ]; then
      echo "$f: stale anchor '$anchor' - no such file: $path"
    elif [ "$last" -gt "$(wc -l < "$resolved")" ]; then
      echo "$f: stale anchor '$anchor' - $path has $(wc -l < "$resolved") lines"
    else
      continue
    fi
    hits=$((hits + 1))
  done < <(grep -oE '`[A-Za-z0-9_./-]+\.[A-Za-z0-9]+:[0-9]+(-[0-9]+)?`' "$f" | tr -d '`')
done

if [ "$hits" -gt 0 ]; then
  echo "lint-findings: $hits violation(s)" >&2
  exit 1
fi
echo "lint-findings: 0 violations"
