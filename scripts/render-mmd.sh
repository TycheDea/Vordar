#!/usr/bin/env bash
# Render every .mmd in the repo to an .svg beside it, using mermaid-cli.
#
#   bash scripts/render-mmd.sh            # render all *.mmd -> *.svg
#   bash scripts/render-mmd.sh path.mmd   # render just the given file(s)
#
# SVG is the source of truth: it scales without reflow, so open the .svg in a
# browser (Ctrl+scroll to zoom) rather than trusting an IDE Mermaid preview.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Collect targets: explicit args, or every tracked-ish .mmd under the repo.
if [ "$#" -gt 0 ]; then
  files=("$@")
else
  mapfile -t files < <(find "$repo_root" -name '*.mmd' \
    -not -path '*/node_modules/*' -not -path '*/target/*')
fi

if [ "${#files[@]}" -eq 0 ]; then
  echo "no .mmd files found"
  exit 0
fi

for mmd in "${files[@]}"; do
  svg="${mmd%.mmd}.svg"
  echo "rendering $mmd -> $svg"
  npx -y @mermaid-js/mermaid-cli -i "$mmd" -o "$svg" -b white
done

echo "done: ${#files[@]} file(s)"
