#!/usr/bin/env python3
"""Driver for proptex/coverage.py over every generated prop's shipped glb.
Plain system Python -- Blender does the geometry work, this script only
loops the subprocess call and collects results.

Run:
  python scripts/ai-pipeline/prop_coverage_sweep.py [--asset NAME]

Writes target/prop-coverage/holes_<name>.png per asset and a combined
target/prop-coverage/coverage.json keyed by asset name. The shipped glb (not
an archived pre-unwrap candidate) is deliberate: it carries the UV atlas the
hole map must align with.
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PROPS_DIR = REPO_ROOT / "content/models/props"
ASSETS_JSON = REPO_ROOT / "content/models/assets.json"
OUT_DIR = REPO_ROOT / "target/prop-coverage"
COVERAGE_PY = REPO_ROOT / "scripts/ai-pipeline/proptex/coverage.py"
BLENDER = Path(r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe")


def generated_assets():
    assets = json.loads(ASSETS_JSON.read_text(encoding="utf-8"))
    return [name for name, entry in assets.items()
            if entry.get("kind") == "generated" and (PROPS_DIR / name).is_dir()]


def run_one(name):
    glb = PROPS_DIR / name / f"{name}.glb"
    if not glb.exists():
        sys.exit(f"prop_coverage_sweep: {name}: missing glb at {glb}")

    map_path = OUT_DIR / f"holes_{name}.png"
    cmd = [str(BLENDER), "--background", "--python", str(COVERAGE_PY), "--",
           str(glb), "--asset", name, "--map", str(map_path)]
    proc = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    sys.stdout.write(proc.stdout)
    sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        sys.exit(f"prop_coverage_sweep: {name}: blender failed (exit {proc.returncode})")

    for line in proc.stdout.splitlines():
        line = line.strip()
        if line.startswith("{"):
            return json.loads(line)
    sys.exit(f"prop_coverage_sweep: {name}: expected a JSON stats line in coverage.py output, found none")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--asset", help="restrict the sweep to one asset")
    args = parser.parse_args()

    names = [args.asset] if args.asset else generated_assets()
    OUT_DIR.mkdir(parents=True, exist_ok=True)

    coverage_path = OUT_DIR / "coverage.json"
    results = json.loads(coverage_path.read_text(encoding="utf-8")) if coverage_path.exists() else {}
    for name in names:
        results[name] = run_one(name)
    coverage_path.write_text(json.dumps(results, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
