#!/usr/bin/env python3
"""Generate a 360 equirect pano with Diffusion360 (text -> 512x1024 base ->
diffusion super-resolution -> 2048x1024), then Lanczos-downsample to the
project's target size and write a provenance manifest.

Run under the Diffusion360 venv:
C:\\tools\\Diffusion360\\venv\\Scripts\\python.exe scripts/ai-pipeline/gen_pano_d360.py "<prompt>" --out <dir> [--seed N]
"""
import argparse
import importlib.util
import json
import random
import sys
import time
from pathlib import Path

import torch
from diffusers import ControlNetModel, DiffusionPipeline, EulerAncestralDiscreteScheduler, UniPCMultistepScheduler
from PIL import Image

# The vendored fork's get_weighted_text_embeddings places token tensors via
# pipe.device, which reports cpu under enable_model_cpu_offload (modules only
# move to the GPU for the duration of their own forward call). This mirrors
# _execution_device's hook scan but falls back to the ORIGINAL device property
# (not itself): pipe.device must resolve to the accelerate execution device
# once offload hooks exist, while enable_model_cpu_offload's pre-hook path
# needs the unaliased original -- aliasing straight to _execution_device
# recurses forever through its own no-hook fallback. Process-local, applied
# before either pipeline loads -- no vendored file edited.
_orig_device = DiffusionPipeline.device.fget


def _hook_aware_device(self):
    for model in self.components.values():
        if not isinstance(model, torch.nn.Module):
            continue
        for module in model.modules():
            hook = getattr(module, "_hf_hook", None)
            if hook is not None and getattr(hook, "execution_device", None) is not None:
                return torch.device(hook.execution_device)
    return _orig_device(self)


DiffusionPipeline.device = property(_hook_aware_device)

REPO_DIR = Path(r"C:\tools\Diffusion360\SD-T2I-360PanoImage")
REPO_COMMIT = "3e980d23198666f5364bedc63ebfdfd9004ee162"
WEIGHTS_DIR = Path(r"C:\tools\Diffusion360\weights")

# importlib-load the two pipeline modules directly, bypassing
# txt2panoimg/__init__.py: that package __init__ transitively imports
# realesrgan, which this script's venv deliberately doesn't install (the
# RealESRGAN upscale stage is dropped -- the diffusion SR stage below already
# clears the 2048x1024 target). Neither module has intra-package relative
# imports, so loading them standalone is safe.
def _load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


_pipeline_base = _load_module("d360_pipeline_base", REPO_DIR / "txt2panoimg" / "pipeline_base.py")
_pipeline_sr = _load_module("d360_pipeline_sr", REPO_DIR / "txt2panoimg" / "pipeline_sr.py")
StableDiffusionBlendExtendPipeline = _pipeline_base.StableDiffusionBlendExtendPipeline
StableDiffusionControlNetImg2ImgPanoPipeline = _pipeline_sr.StableDiffusionControlNetImg2ImgPanoPipeline

TRIGGER_PREFIX = "<360panorama>, "
# Upstream's (Text2360PanoramaImagePipeline) own preset suffix/negative --
# reused verbatim rather than re-tuned, so the bake-off measures the model,
# not a prompt rewrite.
PRESET_QUALITY_SUFFIX = "photorealistic, trend on artstation, ((best quality)), ((ultra high res))"
PRESET_NEGATIVE = (
    "persons, complex texture, small objects, sheltered, blur, worst quality, "
    "low quality, zombie, logo, text, watermark, username, monochrome, "
    "complex lighting"
)

BASE_SIZE = (1024, 512)  # (width, height)
BASE_STEPS = 20
BASE_GUIDANCE = 7.5

SR_SIZE = (3072, 1536)  # (width, height)
SR_STEPS = 7
SR_STRENGTH = 0.8
SR_GUIDANCE = 15.0
SR_CONTROLNET_SCALE = 1.0

OUTPUT_SIZE = (2048, 1024)  # (width, height)


def main():
    parser = argparse.ArgumentParser(description="Generate a 360 equirect pano with Diffusion360.")
    parser.add_argument("prompt")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=None)
    args = parser.parse_args()

    seed = args.seed if args.seed is not None else random.randint(0, 2**32 - 1)
    t_start = time.monotonic()

    prompt_base = TRIGGER_PREFIX + args.prompt + ", " + PRESET_QUALITY_SUFFIX
    prompt_sr = args.prompt + ", " + PRESET_QUALITY_SUFFIX

    # Global seed, not torch.Generator("cuda"): upstream's own recipe, and
    # the one generator object is reused across both stages below (matching
    # upstream) so the SR stage's sampling is also seed-determined.
    generator = torch.manual_seed(seed)

    base_pipe = StableDiffusionBlendExtendPipeline.from_pretrained(
        str(WEIGHTS_DIR / "sd-base"), torch_dtype=torch.float16, safety_checker=None, requires_safety_checker=False
    )
    base_pipe.vae.enable_tiling()
    base_pipe.scheduler = EulerAncestralDiscreteScheduler.from_config(base_pipe.scheduler.config)
    base_pipe.enable_model_cpu_offload()

    base_image = base_pipe(
        prompt_base,
        negative_prompt=PRESET_NEGATIVE,
        num_inference_steps=BASE_STEPS,
        height=BASE_SIZE[1],
        width=BASE_SIZE[0],
        guidance_scale=BASE_GUIDANCE,
        generator=generator,
    ).images[0]

    del base_pipe
    torch.cuda.empty_cache()

    controlnet = ControlNetModel.from_pretrained(str(WEIGHTS_DIR / "sr-control"), torch_dtype=torch.float16)
    sr_pipe = StableDiffusionControlNetImg2ImgPanoPipeline.from_pretrained(
        str(WEIGHTS_DIR / "sr-base"),
        controlnet=controlnet,
        torch_dtype=torch.float16,
        safety_checker=None,
        requires_safety_checker=False,
    )
    sr_pipe.vae.enable_tiling()
    sr_pipe.scheduler = UniPCMultistepScheduler.from_config(sr_pipe.scheduler.config)
    sr_pipe.enable_model_cpu_offload()

    sr_input = base_image.resize(SR_SIZE, Image.Resampling.LANCZOS)
    sr_image = sr_pipe(
        prompt_sr,
        negative_prompt=PRESET_NEGATIVE,
        image=sr_input,
        control_image=sr_input,
        num_inference_steps=SR_STEPS,
        strength=SR_STRENGTH,
        controlnet_conditioning_scale=SR_CONTROLNET_SCALE,
        guidance_scale=SR_GUIDANCE,
        generator=generator,
    ).images[0]

    del sr_pipe, controlnet
    torch.cuda.empty_cache()

    output_image = sr_image.resize(OUTPUT_SIZE, Image.Resampling.LANCZOS)

    elapsed = time.monotonic() - t_start
    peak_vram_allocated_gb = torch.cuda.max_memory_allocated() / 1e9
    peak_vram_reserved_gb = torch.cuda.max_memory_reserved() / 1e9
    print(f"elapsed_seconds={elapsed:.1f}")
    print(f"peak_vram_allocated_gb={peak_vram_allocated_gb:.2f}")
    print(f"peak_vram_reserved_gb={peak_vram_reserved_gb:.2f}")

    args.out.mkdir(parents=True, exist_ok=True)
    out_png = args.out / f"pano_{OUTPUT_SIZE[0]}x{OUTPUT_SIZE[1]}.png"
    output_image.save(out_png)

    manifest = {
        "model": "archerfmy0831/sd-t2i-360panoimage",
        "repo": "ArcherFMY/SD-T2I-360PanoImage",
        "repo_commit": REPO_COMMIT,
        "weights_dir": str(WEIGHTS_DIR),
        "prompt": args.prompt,
        "negative_prompt": PRESET_NEGATIVE,
        "seed": seed,
        "base": {
            "model_dir": "sd-base",
            "prompt": prompt_base,
            "width": BASE_SIZE[0],
            "height": BASE_SIZE[1],
            "num_inference_steps": BASE_STEPS,
            "guidance_scale": BASE_GUIDANCE,
            "scheduler": "EulerAncestralDiscreteScheduler",
        },
        "sr": {
            "model_dir": "sr-base",
            "controlnet_dir": "sr-control",
            "prompt": prompt_sr,
            "width": SR_SIZE[0],
            "height": SR_SIZE[1],
            "num_inference_steps": SR_STEPS,
            "strength": SR_STRENGTH,
            "guidance_scale": SR_GUIDANCE,
            "controlnet_conditioning_scale": SR_CONTROLNET_SCALE,
            "scheduler": "UniPCMultistepScheduler",
        },
        "realesrgan_stage": "dropped -- diffusion SR stage already exceeds the 2048x1024 target",
        "output_size": list(OUTPUT_SIZE),
        "downsample": "Lanczos",
        "elapsed_seconds": elapsed,
        "peak_vram_allocated_gb": peak_vram_allocated_gb,
        "peak_vram_reserved_gb": peak_vram_reserved_gb,
        "versions": {"torch": torch.__version__, "diffusers": __import__("diffusers").__version__},
    }
    (args.out / "generation_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    print(f"OK: wrote {out_png} at {OUTPUT_SIZE[0]}x{OUTPUT_SIZE[1]} (seed={seed}, elapsed={elapsed:.1f}s)")


if __name__ == "__main__":
    main()
