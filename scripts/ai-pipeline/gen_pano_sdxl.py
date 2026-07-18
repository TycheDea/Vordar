#!/usr/bin/env python3
"""Generate a wrap-tileable 360 equirect pano with vanilla SDXL: x-only
circular convolution padding makes the left/right edges continuous, txt2img
runs at the SDXL-native-area 1536x768, and one whole-canvas img2img hop
reaches 2048x1024. Gated by an inline wrap-seam check; writes a provenance
manifest.

Run under the StableMaterials venv:
C:\\tools\\StableMaterials\\venv\\Scripts\\python.exe scripts/ai-pipeline/gen_pano_sdxl.py "<prompt>" --out <dir> [--seed N]
"""
import argparse
import contextlib
import json
import random
import sys
import time
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F
from diffusers import StableDiffusionXLImg2ImgPipeline, StableDiffusionXLPipeline
from PIL import Image

SDXL_CHECKPOINT = r"C:\tools\ComfyUI\ComfyUI\models\checkpoints\sd_xl_base_1.0.safetensors"

TRIGGER_PREFIX = "equirectangular 360 panorama, "
NEGATIVE_PROMPT = (
    "blurry, low quality, jpeg artifacts, watermark, text, seams, stretched, "
    "warped, duplicated horizon, oversaturated"
)

NATIVE_SIZE = (1536, 768)  # (width, height)
NATIVE_STEPS = 40
NATIVE_GUIDANCE = 7.0

HOP_SIZE = (2048, 1024)  # (width, height)
HOP_STRENGTH = 0.35
HOP_STEPS = 40
HOP_GUIDANCE = 7.0

CIRC_PAD = 4

SEAM_STRIP_PX = 8
SEAM_THRESHOLD = 20.0


@contextlib.contextmanager
def circular_x_conv():
    # An equirect pano wraps in x only -- the left edge meets the right edge,
    # while top/bottom are zenith/nadir, not neighbors -- so every Conv2d gets
    # circular padding in x while y keeps the conv's native zero padding:
    # circularly pad CIRC_PAD columns, run the original forward (its own
    # padding still applies on both axes), then crop CIRC_PAD scaled by the
    # conv's width change. The random even x-roll keeps residual pad/crop
    # asymmetry from settling at one fixed column across layers. Patching
    # Conv2d.forward globally is deliberate: while the context is active it
    # covers the UNet and the VAE encode/decode alike, so latent and pixel
    # space both wrap. (Adapted from C:\tools\StableMaterials\weights\
    # pipeline.py rolled_conv, which pads both axes for square tileables.)
    # Crop and unroll must round(), not truncate: diffusers' VAE-encoder
    # Downsample2D pre-pads one column before its padding-0 stride-2 conv,
    # putting the width factor just under 1/2 -- truncation then under-crops
    # 1 px per level and a 2048 canvas comes back 2072 wide.
    orig_forward = torch.nn.Conv2d.forward

    def forward(self, x, *args, **kwargs):
        roll_w = torch.randint(0, 256, (1,)).item() // 2 * 2
        x = torch.roll(x, shifts=roll_w, dims=3)
        x = F.pad(x, (CIRC_PAD, CIRC_PAD, 0, 0), mode="circular")
        w_in = x.shape[-1]
        x = orig_forward(self, x, *args, **kwargs)
        factor = x.shape[-1] / w_in
        crop = round(CIRC_PAD * factor)
        x = x[..., crop:-crop]
        return torch.roll(x, shifts=-round(roll_w * factor), dims=3)

    torch.nn.Conv2d.forward = forward
    try:
        yield
    finally:
        torch.nn.Conv2d.forward = orig_forward


def seam_check(img: Image.Image) -> tuple[float, bool]:
    arr = np.asarray(img)
    left = arr[:, :SEAM_STRIP_PX].astype(np.int16)
    right = arr[:, -SEAM_STRIP_PX:].astype(np.int16)
    mad = float(np.abs(left - right).mean())
    ok = mad <= SEAM_THRESHOLD
    print(f"seam_check left_right: {'PASS' if ok else 'FAIL'} (metric={mad:.2f}, threshold={SEAM_THRESHOLD})")
    return mad, ok


def main():
    parser = argparse.ArgumentParser(description="Generate a wrap-tileable 360 equirect pano with vanilla SDXL + x-only circular padding.")
    parser.add_argument("prompt")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=None)
    args = parser.parse_args()

    seed = args.seed if args.seed is not None else random.randint(0, 2**32 - 1)
    t_start = time.monotonic()

    full_prompt = TRIGGER_PREFIX + args.prompt

    # Global seed, not a standalone Generator: circular_x_conv's per-layer
    # x-rolls draw from the global CPU RNG, so only seeding it makes a run
    # reproducible end to end; the returned generator object is reused for
    # both stages' sampling.
    generator = torch.manual_seed(seed)

    # enable_model_cpu_offload, not .to("cuda"): whole-canvas SDXL at 2048
    # overruns this 12GB card into WDDM shared memory and thrashes (A1
    # lesson). No vae.enable_tiling(): tiles decode as independent zero-edged
    # patches, which would cut the x-wrap the conv patch builds; the whole
    # canvas fits under offload at this size.
    pipe = StableDiffusionXLPipeline.from_single_file(SDXL_CHECKPOINT, torch_dtype=torch.float16)
    pipe.enable_attention_slicing("auto")
    pipe.enable_model_cpu_offload()

    with circular_x_conv():
        native_image = pipe(
            prompt=full_prompt,
            negative_prompt=NEGATIVE_PROMPT,
            width=NATIVE_SIZE[0],
            height=NATIVE_SIZE[1],
            num_inference_steps=NATIVE_STEPS,
            guidance_scale=NATIVE_GUIDANCE,
            generator=generator,
        ).images[0]

    del pipe
    torch.cuda.empty_cache()

    sdxl = StableDiffusionXLImg2ImgPipeline.from_single_file(SDXL_CHECKPOINT, torch_dtype=torch.float16)
    sdxl.enable_attention_slicing("auto")
    sdxl.enable_model_cpu_offload()

    init_image = native_image.resize(HOP_SIZE, Image.Resampling.LANCZOS)
    with circular_x_conv():
        pano = sdxl(
            prompt=full_prompt,
            negative_prompt=NEGATIVE_PROMPT,
            image=init_image,
            strength=HOP_STRENGTH,
            num_inference_steps=HOP_STEPS,
            guidance_scale=HOP_GUIDANCE,
            generator=generator,
        ).images[0]

    del sdxl
    torch.cuda.empty_cache()

    elapsed = time.monotonic() - t_start
    peak_vram_allocated_gb = torch.cuda.max_memory_allocated() / 1e9
    peak_vram_reserved_gb = torch.cuda.max_memory_reserved() / 1e9
    print(f"elapsed_seconds={elapsed:.1f}")
    print(f"peak_vram_allocated_gb={peak_vram_allocated_gb:.2f}")
    print(f"peak_vram_reserved_gb={peak_vram_reserved_gb:.2f}")

    seam_mad, seam_ok = seam_check(pano)
    if not seam_ok:
        print(f"Wrap-seam check FAILED: left_right metric={seam_mad:.2f} > {SEAM_THRESHOLD}")
        sys.exit(1)

    args.out.mkdir(parents=True, exist_ok=True)
    out_png = args.out / f"pano_{HOP_SIZE[0]}x{HOP_SIZE[1]}.png"
    pano.save(out_png)

    manifest = {
        "model": "stabilityai/stable-diffusion-xl-base-1.0",
        "checkpoint": SDXL_CHECKPOINT,
        "mechanism": "x-only circular Conv2d padding (roll/pad/crop), active in txt2img, img2img, and their VAE passes",
        "circular_pad_px": CIRC_PAD,
        "prompt": args.prompt,
        "full_prompt": full_prompt,
        "negative_prompt": NEGATIVE_PROMPT,
        "seed": seed,
        "native": {
            "width": NATIVE_SIZE[0],
            "height": NATIVE_SIZE[1],
            "num_inference_steps": NATIVE_STEPS,
            "guidance_scale": NATIVE_GUIDANCE,
        },
        "hop": {
            "width": HOP_SIZE[0],
            "height": HOP_SIZE[1],
            "strength": HOP_STRENGTH,
            "num_inference_steps": HOP_STEPS,
            "guidance_scale": HOP_GUIDANCE,
            "init_resize": "Lanczos",
        },
        "seam_check": {"strip_px": SEAM_STRIP_PX, "threshold": SEAM_THRESHOLD, "left_right_mad": seam_mad},
        "output_size": list(HOP_SIZE),
        "elapsed_seconds": elapsed,
        "peak_vram_allocated_gb": peak_vram_allocated_gb,
        "peak_vram_reserved_gb": peak_vram_reserved_gb,
        "versions": {"torch": torch.__version__, "diffusers": __import__("diffusers").__version__},
    }
    (args.out / "generation_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    print(f"OK: wrote {out_png} at {HOP_SIZE[0]}x{HOP_SIZE[1]} (seed={seed}, elapsed={elapsed:.1f}s)")


if __name__ == "__main__":
    main()
