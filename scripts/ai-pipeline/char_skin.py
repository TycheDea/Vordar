#!/usr/bin/env python3
"""SkinTokens skin-only runner: predict skin weights for fit.glb's mesh
over the canonical skeleton it carries, transferred onto that same
textured mesh (demo.py's use_skeleton + use_transfer path) -> skinned.glb.
The output re-emits generic bone names and drops non-deforming leaf
bones; char_rig.py finish maps it back onto the canonical rig.

Run under the SkinTokens venv, cwd=C:\\tools\\SkinTokens\\SkinTokens (the
checkpoint path and bpy_server.py resolve from there):
C:\\tools\\SkinTokens\\venv\\Scripts\\python.exe <path-to-this-repo>\\scripts\\ai-pipeline\\char_skin.py <fit.glb> --out <skinned.glb> [--seed N]

SkinTokens ships no seed control and samples (do_sample=True), so
reproducibility is external: torch/numpy/random are seeded here and the
seed is recorded in the stats line.
"""
import argparse
import json
import random
import sys
import time
from pathlib import Path

import numpy as np
import torch

REPO_DIR = Path(r"C:\tools\SkinTokens\SkinTokens")
sys.path.insert(0, str(REPO_DIR))
import demo  # noqa: E402

# Their default is 10 at the stated >=14 GB VRAM; 4 is the A1b-validated
# 12 GB setting (3.7 GiB peak, user feel-check passed 2026-07-22).
NUM_BEAMS = 4


def main():
    parser = argparse.ArgumentParser(description="SkinTokens skin-only: fit.glb -> skinned.glb over the supplied skeleton.")
    parser.add_argument("fit_glb", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=None)
    args = parser.parse_args()

    seed = args.seed if args.seed is not None else random.randint(0, 2**31 - 1)
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)

    # A stale output would defeat the exists() check below -- run_rig
    # reports export failures by printing, not raising.
    if args.out.exists():
        args.out.unlink()

    demo.start_bpy_server()
    demo.wait_for_bpy_server()
    demo.load_model(demo.MODEL_CKPTS[0], None)

    t0 = time.time()
    demo.run_rig(
        [args.fit_glb.resolve()],
        top_k=5,
        top_p=0.95,
        temperature=1.0,
        repetition_penalty=2.0,
        num_beams=NUM_BEAMS,
        use_skeleton=True,
        use_transfer=True,
        use_postprocess=False,
        output_paths=[args.out.resolve()],
        model_ckpt=demo.MODEL_CKPTS[0],
        hf_path=None,
    )
    gen_time = time.time() - t0

    if not args.out.exists():
        sys.exit(f"char_skin: SkinTokens produced no output at {args.out}")

    print(json.dumps({
        "model_ckpt": demo.MODEL_CKPTS[0],
        "seed": seed,
        "num_beams": NUM_BEAMS,
        "gen_time_s": round(gen_time, 1),
        "peak_vram_gib": round(torch.cuda.max_memory_allocated() / 2**30, 2),
        "skinned_glb": str(args.out),
    }))


if __name__ == "__main__":
    main()
