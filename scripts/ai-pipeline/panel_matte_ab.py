#!/usr/bin/env python3
"""Measures whether a Z-Image concept sheet's three panels (front/side/back)
are genuinely three different viewpoints or the same view drawn twice.

A background-subtraction silhouette was tried first and rejected: its
threshold is a free parameter, and the sheets with real viewpoint change also
carry cast shadows, which subtraction counts as object -- biasing the
instrument against exactly the case it must detect. This script instead
mattes each panel through the same BiRefNet pass the production pipeline
uses (hi3dgen.headless.Session.matte, gated by prop_hi3dgen.check_matte) and
builds each panel's silhouette from that matte's own alpha cut
(mv_ab_metrics.ALPHA_THRESHOLD, the same 0.8*255 bbox test preprocess_image
applies downstream) -- no threshold of this script's own choosing. Two
panels that are the same view redrawn line up almost exactly under that
silhouette (high pairwise IoU); two panels that are real, different
viewpoints do not.

One Session load for the whole run -- the model load dominates wall time,
not the 18 BiRefNet passes over 512x512 panels.

Run under the Hi3DGen venv (same dependency set as prop_hi3dgen.py):
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe scripts/ai-pipeline/panel_matte_ab.py <subject_dir> [<subject_dir> ...] [--out target/mv-ab/panel_distinctness.json]
"""
import argparse
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

from hi3dgen import headless  # noqa: E402
from prop_hi3dgen import DegenerateMatteError, check_matte  # noqa: E402
import mv_ab_metrics as mvab  # noqa: E402

PANELS = ("front", "side", "back")
PAIRS = (("front", "side"), ("front", "back"), ("side", "back"))


def matte_panel(session, seed_dir, panel):
    """Mattes one panel, saves the RGBA result beside it, and returns
    (opaque_fraction, refusal_message_or_None, normalized_silhouette)."""
    image = Image.open(seed_dir / f"view_{panel}.png").convert("RGBA")
    rgba = session.matte(image)
    rgba.save(seed_dir / f"view_{panel}_rgba.png")

    alpha = np.asarray(rgba)[:, :, 3]
    mask = alpha > mvab.ALPHA_THRESHOLD
    opaque_fraction = float(mask.mean())

    refused = None
    try:
        check_matte(rgba)
    except DegenerateMatteError as e:
        refused = str(e)

    return opaque_fraction, refused, mvab.normalize_mask(mask)


def process_subject(session, subject_dir):
    result = {}
    for seed_dir in sorted(subject_dir.glob("seed*")):
        fractions = {}
        refusals = {}
        norms = {}
        for panel in PANELS:
            fraction, refused, norm = matte_panel(session, seed_dir, panel)
            fractions[panel] = fraction
            refusals[panel] = refused
            norms[panel] = norm
        result[seed_dir.name] = {
            "opaque_fraction": fractions,
            "refused": refusals,
            "iou": {
                f"{a}-{b}": mvab.iou(norms[a], norms[b])
                for a, b in PAIRS
            },
        }
    return result


def main():
    parser = argparse.ArgumentParser(
        description="Matte-based panel-distinctness A/B for multi-view concept sheets.")
    parser.add_argument("subjects", nargs="+", help="subject directories, each holding seed*/view_{front,side,back}.png")
    parser.add_argument("--out", default="target/mv-ab/panel_distinctness.json", help="output JSON path")
    args = parser.parse_args()

    session = headless.Session()

    result = {}
    for subject in args.subjects:
        subject_dir = Path(subject)
        result[subject_dir.name] = process_subject(session, subject_dir)

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(result, indent=2))
    print(f"wrote {out_path}")


if __name__ == "__main__":
    main()
