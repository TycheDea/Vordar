#!/usr/bin/env bash
# Flags comments that violate CLAUDE.md's comment policy across the
# workspace crates audit-hygiene sweeps (docs/reviews and content/ are out
# of scope: reports cite findings by design, and content/ file contents are
# audit-content-pipeline's territory).
#
# Finding/rework citations, WEAKPOINTS-style doc tags, used-to-be/before-
# the-fix/now-we narration, and Phase N roadmap tags are always forbidden.
# VQ-* roadmap tags are only forbidden when used as provenance, not when
# they anchor a stated constraint (CLAUDE.md's spec-clause exception) -
# telling those apart needs a reader, so known-good VQ-* lines are pinned
# in lint-comments-allowlist.txt, keyed by path:normalized-line-content so
# a pin tracks its line through drift instead of aliasing whatever line
# number it used to sit at; anything else is flagged for review.
# game/vordar-game/tests/content_lint.rs is exempt from the VQ-* scan
# wholesale: every VQ tag there anchors an assert enforcing that clause, so
# the spec-clause exception is a property of the whole file, not a
# per-line pin.
#
#   bash scripts/lint-comments.sh   # scan; prints hits and exits 1 if any
#
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
allowlist="$repo_root/scripts/lint-comments-allowlist.txt"
scope=(smirk game client server benchmarks testing)
vq_scan_exclude=':!game/vordar-game/tests/content_lint.rs'

normalize_line() {
  printf '%s' "$1" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//; s/[[:space:]]+/ /g'
}

hard_patterns=(
  '\b(finding|rework)[[:space:]]+[0-9]+\b'
  '\bWEAKPOINTS\b'
  '\b(used to be|before the fix|now we)\b'
  '\bPhase[[:space:]]+[0-9]+\b'
)

hits=0
cd "$repo_root"

for pattern in "${hard_patterns[@]}"; do
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    echo "$line"
    hits=$((hits + 1))
  done < <(git grep -n -i -E --untracked --exclude-standard -- "$pattern" -- "${scope[@]}" 2>/dev/null || true)
done

while IFS= read -r line; do
  [ -z "$line" ] && continue
  path="${line%%:*}"
  rest="${line#*:}"
  content="${rest#*:}"
  key="$path:$(normalize_line "$content")"
  if grep -qxF -- "$key" "$allowlist" 2>/dev/null; then
    continue
  fi
  echo "$line"
  hits=$((hits + 1))
done < <(git grep -n -E --untracked --exclude-standard -- '\bVQ-[A-Z0-9]+\b' -- "${scope[@]}" "$vq_scan_exclude" 2>/dev/null || true)

if [ "$hits" -gt 0 ]; then
  echo "lint-comments: $hits forbidden pattern hit(s)" >&2
  exit 1
fi
echo "lint-comments: 0 hits"
