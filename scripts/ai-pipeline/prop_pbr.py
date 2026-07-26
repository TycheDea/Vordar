# Per-view PBR decomposition via MaterialAnything's material estimator
# (xanderhuang/material_estimator, Apache-2.0). Runs in the MaterialAnything
# venv (NOT Blender's Python) — proptex/albedo.py invokes it as a subprocess
# between view generation and blending, while no other process holds the GPU.
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
# Usage: python prop_pbr.py <gen.png> <normal.png> <mask.png> <albedo.png> --seed S

import argparse
import sys
from pathlib import Path

import numpy as np
from PIL import Image

MA_DIR = Path(r"C:\tools\MaterialAnything")
MODEL_DIR = MA_DIR / "pretrained_models" / "material_estimator"
EST_RES = 768  # the estimator's native (training) resolution
EST_STEPS = 50  # upstream's estimation setting; cfg 1.0 disables CFG


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


def estimate_view(pipe, gen_png, normal_png, mask_png, albedo_png, seed):
    import torch

    gen = Image.open(gen_png).convert("RGB")
    full_res = gen.size
    mask = np.asarray(Image.open(mask_png).convert("L")) > 127

    rgb = np.array(gen)
    rgb[~mask] = 255
    rgb = Image.fromarray(rgb).resize((EST_RES, EST_RES), Image.LANCZOS)
    normal = Image.open(normal_png).convert("RGB").resize(
        (EST_RES, EST_RES), Image.LANCZOS)
    mask768 = np.asarray(Image.fromarray(mask.astype(np.uint8) * 255).resize(
        (EST_RES, EST_RES), Image.NEAREST)) > 127
    keep = torch.from_numpy((~mask768).astype(np.float32)).unsqueeze(0)

    white = torch.ones(1, EST_RES, EST_RES, 3)
    init_materials = {"albedo": white, "roughness_metallic": white, "bump": white}

    generator = torch.Generator("cuda").manual_seed(seed)
    albedo, _rm, _ = pipe(
        prompt=[""], cond_image=[rgb], normal_image=[normal],
        init_materials=init_materials, masks=keep,
        num_inference_steps=EST_STEPS, guidance_scale=1.0,
        generator=generator, height=EST_RES, width=EST_RES).images
    albedo.resize(full_res, Image.LANCZOS).save(albedo_png)


def main():
    parser = argparse.ArgumentParser(prog="prop_pbr.py")
    parser.add_argument("gen_png", help="The view's lit multiview generation")
    parser.add_argument("normal_png", help="Camera-space normal conditioning image")
    parser.add_argument("mask_png", help="Object mask")
    parser.add_argument("albedo_png", help="Delit albedo to write")
    parser.add_argument("--seed", type=int, required=True)
    args = parser.parse_args()

    estimate_view(load_pipeline(), args.gen_png, args.normal_png, args.mask_png,
                  args.albedo_png, args.seed)


if __name__ == "__main__":
    main()
