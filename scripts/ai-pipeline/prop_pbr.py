# Per-view PBR decomposition via MaterialAnything's material estimator
# (xanderhuang/material_estimator, Apache-2.0). Runs in the MaterialAnything
# venv (NOT Blender's Python) — prop_texture.py invokes it as a subprocess
# between view generation and blending, while no other process holds the GPU.
#
# For each multiview view_<i>/gen.png it writes view_<i>/albedo.png and
# view_<i>/bump.png at the gen resolution, plus a work-dir-level
# pbr_meta.json covering every view. Views whose outputs already exist are
# skipped, so a killed run resumes without respending GPU;
# the meta file is rewritten in full every run (shas from files, seeds from
# the deterministic per-view formula), so a resume yields identical meta.
#
# The estimator is a triple-head SD2.1 UNet conditioned per head on the VAE
# latents of the RGB view and a camera-space normal render plus a raw
# RePaint keep-mask channel (1 = pin to init_materials, 0 = estimate).
# Standalone single-view use therefore reproduces upstream's first-view
# call: all-white init materials, keep-mask 1 on the background (pinned
# white) and 0 over the object (estimated). Inputs must be exactly 768x768
# (the cond image's size sets the latent size — the pipeline never resizes),
# with the object composited onto white.
#
# Usage: python prop_pbr.py <work_dir> --views N --seed S

import argparse
import hashlib
import json
import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image

MA_DIR = Path(r"C:\tools\MaterialAnything")
MODEL_DIR = MA_DIR / "pretrained_models" / "material_estimator"
HF_MODEL = "xanderhuang/material_estimator"
EST_RES = 768  # the estimator's native (training) resolution
EST_STEPS = 50  # upstream's estimation setting; cfg 1.0 disables CFG


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load_pipeline():
    import torch
    sys.path.insert(0, str(MA_DIR))
    # upstream's lib/diffusion_helper drags in gradio/ControlNet at import
    # time — replicate its ~10-line loader against the custom classes instead
    from models.scheduling_ddpm import DDPMScheduler
    from pipelines.pipeline_stable_diffusion_switcher import StableDiffusionPipeline

    pipe = StableDiffusionPipeline.from_pretrained(
        str(MODEL_DIR), torch_dtype=torch.float16).to("cuda")
    pipe.scheduler = DDPMScheduler.from_pretrained(
        str(MODEL_DIR), subfolder="scheduler")
    pipe.set_progress_bar_config(disable=True)
    return pipe


def estimate_view(pipe, work, i, seed):
    import torch

    vdir = work / f"view_{i}"
    gen = Image.open(vdir / "gen.png").convert("RGB")
    full_res = gen.size
    mask = np.asarray(Image.open(work / f"mask_{i}.png").convert("L")) > 127

    rgb = np.array(gen)
    rgb[~mask] = 255
    rgb = Image.fromarray(rgb).resize((EST_RES, EST_RES), Image.LANCZOS)
    normal = Image.open(work / f"normal_{i}.png").convert("RGB").resize(
        (EST_RES, EST_RES), Image.LANCZOS)
    mask768 = np.asarray(Image.fromarray(mask.astype(np.uint8) * 255).resize(
        (EST_RES, EST_RES), Image.NEAREST)) > 127
    keep = torch.from_numpy((~mask768).astype(np.float32)).unsqueeze(0)

    white = torch.ones(1, EST_RES, EST_RES, 3)
    init_materials = {"albedo": white, "roughness_metallic": white, "bump": white}

    generator = torch.Generator("cuda").manual_seed(seed)
    albedo, _rm, bump = pipe(
        prompt=[""], cond_image=[rgb], normal_image=[normal],
        init_materials=init_materials, masks=keep,
        num_inference_steps=EST_STEPS, guidance_scale=1.0,
        generator=generator, height=EST_RES, width=EST_RES).images
    albedo.resize(full_res, Image.LANCZOS).save(vdir / "albedo.png")
    bump.resize(full_res, Image.LANCZOS).save(vdir / "bump.png")


def main():
    parser = argparse.ArgumentParser(prog="prop_pbr.py")
    parser.add_argument("work_dir")
    parser.add_argument("--views", type=int, required=True)
    parser.add_argument("--seed", type=int, required=True)
    args = parser.parse_args()
    work = Path(args.work_dir)

    t0 = time.time()
    todo = [i for i in range(args.views)
            if not (work / f"view_{i}" / "albedo.png").exists()
            or not (work / f"view_{i}" / "bump.png").exists()]
    if todo:
        pipe = load_pipeline()
        for i in todo:
            estimate_view(pipe, work, i, args.seed * 1000 + i)

    meta = {
        "model": HF_MODEL,
        "steps": EST_STEPS,
        "resolution": EST_RES,
        "estimated_views": todo,
        "elapsed_s": round(time.time() - t0, 1),
        "views": [{
            "seed": args.seed * 1000 + i,
            "albedo_sha256": sha256_file(work / f"view_{i}" / "albedo.png"),
            "bump_sha256": sha256_file(work / f"view_{i}" / "bump.png"),
        } for i in range(args.views)],
    }
    (work / "pbr_meta.json").write_text(json.dumps(meta, indent=1), encoding="utf-8")


if __name__ == "__main__":
    main()
