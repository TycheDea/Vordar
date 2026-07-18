#!/usr/bin/env python3
"""Generate a tileable ground PBR material (albedo/normal/roughness) with
StableMaterials, optionally extended past its 1024 native ceiling via a
whole-canvas SDXL img2img pass on albedo only.

Run under the StableMaterials venv:
C:\\tools\\StableMaterials\\venv\\Scripts\\python.exe scripts/ai-pipeline/gen_material.py "<prompt>" --out <dir> [--size N] [--seed N]
"""
import argparse
import json
import random
import sys
from pathlib import Path

import numpy as np
import torch
from diffusers import DiffusionPipeline, StableDiffusionXLImg2ImgPipeline
from PIL import Image

WEIGHTS_DIR = r"C:\tools\StableMaterials\weights"
SDXL_CHECKPOINT = r"C:\tools\ComfyUI\ComfyUI\models\checkpoints\sd_xl_base_1.0.safetensors"

# tileable=True's monkeypatched circular pad=4 collides with the UNet
# bottleneck below this -- generation never runs smaller than this, target
# sizes below it are reached by downscaling after.
NATIVE_MIN = 512
# StableMaterials' safe "hires" ceiling (2x its native training resolution);
# targets beyond this go through the SDXL img2img pass instead of generating
# directly.
NATIVE_MAX = 1024

GUIDANCE_SCALE = 10.0
NUM_INFERENCE_STEPS = 50
UPSCALE_STRENGTH = 0.35
UPSCALE_STEPS = 40
UPSCALE_GUIDANCE = 7.0
SEAM_STRIP_PX = 8
SEAM_THRESHOLD = 20.0

MAP_ATTR = {"diff": "basecolor", "nor_gl": "normal", "rough": "roughness"}


def seam_metric(a: np.ndarray, b: np.ndarray) -> float:
    return float(np.abs(a.astype(np.int16) - b.astype(np.int16)).mean())


def tiling_check(maps: dict) -> tuple[dict, list]:
    metrics = {}
    failures = []
    for tag, img in maps.items():
        arr = np.asarray(img)
        edge_pairs = {
            "left_right": (arr[:, :SEAM_STRIP_PX], arr[:, -SEAM_STRIP_PX:]),
            "top_bottom": (arr[:SEAM_STRIP_PX, :], arr[-SEAM_STRIP_PX:, :]),
        }
        metrics[tag] = {}
        for pair_name, (strip_a, strip_b) in edge_pairs.items():
            metric = seam_metric(strip_a, strip_b)
            metrics[tag][pair_name] = metric
            ok = metric <= SEAM_THRESHOLD
            print(f"tiling_check {tag} {pair_name}: {'PASS' if ok else 'FAIL'} (metric={metric:.2f}, threshold={SEAM_THRESHOLD})")
            if not ok:
                failures.append(f"{tag} {pair_name} metric={metric:.2f} > {SEAM_THRESHOLD}")
    return metrics, failures


def main():
    parser = argparse.ArgumentParser(description="Generate a tileable ground PBR material with StableMaterials.")
    parser.add_argument("prompt")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--size", type=int, default=2048)
    parser.add_argument("--seed", type=int, default=None)
    args = parser.parse_args()

    seed = args.seed if args.seed is not None else random.randint(0, 2**32 - 1)
    native_size = max(NATIVE_MIN, min(args.size, NATIVE_MAX))

    pipe = DiffusionPipeline.from_pretrained(WEIGHTS_DIR, trust_remote_code=True, torch_dtype=torch.float16).to("cuda")
    generator = torch.Generator("cuda").manual_seed(seed)
    result = pipe(
        prompt=args.prompt,
        height=native_size,
        width=native_size,
        guidance_scale=GUIDANCE_SCALE,
        num_inference_steps=NUM_INFERENCE_STEPS,
        tileable=True,
        generator=generator,
    )
    material = result.images[0]
    maps = {tag: getattr(material, attr) for tag, attr in MAP_ATTR.items()}

    del pipe
    torch.cuda.empty_cache()

    upscaled = args.size > NATIVE_MAX
    if upscaled:
        # Only albedo goes through the diffusion pass: normal-map pixels are
        # a tangent-space unit vector and roughness a physical scalar tied to
        # the surface's actual variation, so hallucinated per-pixel detail
        # there would be geometrically/physically wrong, not just noisier.
        init_image = maps["diff"].resize((args.size, args.size), Image.Resampling.LANCZOS)
        sdxl = StableDiffusionXLImg2ImgPipeline.from_single_file(SDXL_CHECKPOINT, torch_dtype=torch.float16).to("cuda")
        sdxl.vae.enable_tiling()
        sdxl.enable_attention_slicing("auto")
        upscale_generator = torch.Generator("cuda").manual_seed(seed)
        upscale_result = sdxl(
            prompt=args.prompt,
            image=init_image,
            strength=UPSCALE_STRENGTH,
            num_inference_steps=UPSCALE_STEPS,
            guidance_scale=UPSCALE_GUIDANCE,
            generator=upscale_generator,
        )
        maps["diff"] = upscale_result.images[0]
        maps["nor_gl"] = maps["nor_gl"].resize((args.size, args.size), Image.Resampling.LANCZOS)
        maps["rough"] = maps["rough"].resize((args.size, args.size), Image.Resampling.LANCZOS)
        del sdxl
        torch.cuda.empty_cache()
    elif args.size != native_size:
        maps = {tag: img.resize((args.size, args.size), Image.Resampling.LANCZOS) for tag, img in maps.items()}

    tiling_metrics, failures = tiling_check(maps)
    print(f"peak_vram_allocated_gb={torch.cuda.max_memory_allocated() / 1e9:.2f}")
    print(f"peak_vram_reserved_gb={torch.cuda.max_memory_reserved() / 1e9:.2f}")
    if failures:
        print("Tileability check FAILED: " + "; ".join(failures))
        sys.exit(1)

    args.out.mkdir(parents=True, exist_ok=True)
    for tag, img in maps.items():
        img.save(args.out / f"{tag}_{args.size}.png")

    # generation_manifest.json, not manifest.json: bake_textures.mjs writes
    # its own manifest.json (different {source, images} shape) into this
    # same directory -- same name would let the bake step clobber this file.
    manifest = {
        "model": "gvecchio/StableMaterials",
        "prompt": args.prompt,
        "seed": seed,
        "size": args.size,
        "native_generation_size": native_size,
        "upscaled": upscaled,
        "upscale_model": "stabilityai/stable-diffusion-xl-base-1.0" if upscaled else None,
        "guidance_scale": GUIDANCE_SCALE,
        "num_inference_steps": NUM_INFERENCE_STEPS,
        "tiling_check": {"threshold": SEAM_THRESHOLD, "strip_px": SEAM_STRIP_PX, "metrics": tiling_metrics},
    }
    (args.out / "generation_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    print(f"OK: wrote {args.out} at {args.size}x{args.size} (native={native_size}, upscaled={upscaled})")


if __name__ == "__main__":
    main()
