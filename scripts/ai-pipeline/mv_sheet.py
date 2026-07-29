#!/usr/bin/env python3
"""Generate one three-panel concept sheet (front/side/back) per seed for a
subject, via the Z-Image-Turbo mv_sheet.json workflow, and split the sheet
into matting-ready panels.

Plain system Python (imports comfy_run, not torch); owns the ComfyUI server
lifecycle for the duration of the run.

Run:
  python scripts/ai-pipeline/mv_sheet.py --subject "<prompt text>" \
      --name <output_dir_name> --seed N [--seed M ...] --out target/mv-ab
"""
import argparse
import json
import sys
from pathlib import Path

import cv2
import numpy as np

import comfy_run

SCRIPT_DIR = Path(__file__).resolve().parent
MV_SHEET_WORKFLOW = SCRIPT_DIR / "workflows" / "mv_sheet.json"
PANEL_NAMES = ("view_front.png", "view_side.png", "view_back.png")


def split_thirds_array(img: np.ndarray) -> tuple:
    """Pure split of an image array into three equal-width panels (front,
    side, back), left to right. Raises if the width doesn't divide evenly."""
    width = img.shape[1]
    if width % 3 != 0:
        raise ValueError(f"sheet width {width} does not divide evenly by 3")
    third = width // 3
    return tuple(img[:, i * third:(i + 1) * third] for i in range(3))


def split_thirds(sheet_path: Path, out_dir: Path) -> dict:
    img = cv2.imread(str(sheet_path), cv2.IMREAD_UNCHANGED)
    if img is None:
        raise SystemExit(f"mv_sheet: could not read {sheet_path}")
    try:
        panels = split_thirds_array(img)
    except ValueError as e:
        raise SystemExit(f"mv_sheet: {e}")
    saved = {}
    for name, panel in zip(PANEL_NAMES, panels):
        panel_path = out_dir / name
        cv2.imwrite(str(panel_path), panel)
        saved[name] = str(panel_path)
    return saved


def run_seed(subject_prompt: str, seed: int, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    # A3.7 convention: mv_sheet.json ships a {subject} placeholder.
    workflow = json.loads(MV_SHEET_WORKFLOW.read_text(encoding="utf-8").replace("{subject}", subject_prompt))
    for node in workflow.values():
        inputs = node.get("inputs", {})
        for key in inputs:
            if key in ("seed", "noise_seed"):
                inputs[key] = seed

    sheet_raw = out_dir / "sheet_raw"
    manifest = comfy_run.run_workflow(workflow, sheet_raw)
    pngs = sorted(sheet_raw.glob("*.png"))
    if len(pngs) != 1:
        sys.exit(f"mv_sheet: sheet stage produced {len(pngs)} PNG(s) in {sheet_raw}, expected exactly 1")
    sheet_path = out_dir / "sheet.png"
    sheet_path.write_bytes(pngs[0].read_bytes())

    panels = split_thirds(sheet_path, out_dir)

    meta = {
        "subject_prompt": subject_prompt,
        "seed": seed,
        "comfy_manifest": manifest,
        "panels": panels,
    }
    (out_dir / "sheet_meta.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")
    print(f"mv_sheet: seed {seed} -> {sheet_path}")


def main():
    parser = argparse.ArgumentParser(description="Generate concept sheets (front/side/back) for one subject.")
    parser.add_argument("--subject", required=True, help="Subject text substituted for {subject} in the sheet prompt")
    parser.add_argument("--name", required=True, help="Output directory name for this subject (e.g. olive_stump)")
    parser.add_argument("--seed", type=int, action="append", dest="seeds", required=True, metavar="N",
                        help="Repeatable: one sheet per seed")
    parser.add_argument("--out", type=Path, default=Path("target/mv-ab"), help="Base output directory")
    args = parser.parse_args()

    subject_dir = args.out / args.name
    with comfy_run.server():
        for seed in args.seeds:
            run_seed(args.subject, seed, subject_dir / f"seed{seed}")


if __name__ == "__main__":
    main()
