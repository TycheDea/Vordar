#!/usr/bin/env python3
"""Hi3DGen image -> untextured raw glb geometry runner: BiRefNet background
removal -> StableNormal-turbo normal bridge -> Hi3DGenPipeline geometry ->
to_trimesh export. Transcribes C:\\tools\\Hi3DGen\\Hi3DGen\\app.py's working
recipe (generate_3d), minus gradio. Texturing is a later pipeline stage --
this script's output is bare geometry.

Run under the Hi3DGen venv, cwd=C:\\tools\\Hi3DGen\\Hi3DGen (weights/,
torch.hub's local snapshots, and hi3dgen's internal relative-path lookups
all resolve from there -- this script's own path may be absolute, it lives
outside that tree):
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe <path-to-this-repo>\\scripts\\ai-pipeline\\prop_hi3dgen.py <image.png> --out <dir> [--seed N] [--steps N]
"""
import argparse
import hashlib
import json
import os

# Must be set before importing hi3dgen's modules (attention/sparse backends
# read these at import time) -- same in-process pattern app.py uses for
# SPCONV_ALGO, extended to ATTN_BACKEND so the script needs no shell setup.
os.environ["ATTN_BACKEND"] = "xformers"
os.environ["SPCONV_ALGO"] = "native"
# GEOMETRY_WEIGHTS below is a fully-populated local snapshot, so this is a
# hard guard against silently falling back to a network fetch (e.g. a repo
# id typo) rather than failing loudly.
os.environ["HF_HUB_OFFLINE"] = "1"

import random
import sys
from pathlib import Path

import numpy as np
import torch
import xformers
from PIL import Image

# hi3dgen is a plain repo checkout (not pip-installed) with real intra-package
# relative imports (hi3dgen.pipelines.hi3dgen does `from ..modules import
# sparse`), so it must be importable as a top-level package -- cwd alone does
# not land on sys.path for a `python script.py` invocation from elsewhere
# (Diffusion360 runner precedent: gen_pano_d360.py's REPO_DIR).
REPO_DIR = Path(r"C:\tools\Hi3DGen\Hi3DGen")
sys.path.insert(0, str(REPO_DIR))
from hi3dgen.pipelines import Hi3DGenPipeline  # noqa: E402

GEOMETRY_WEIGHTS = REPO_DIR / "weights" / "trellis-normal-v0-1"
NORMAL_WEIGHTS_REPO = "Stable-X/yoso-normal-v1-8-1"
YOSO_VERSION = "yoso-normal-v1-8-1"
BIREFNET_REPO = "ZhengPeng7/BiRefNet"
STABLE_NORMAL_HUB_SNAPSHOT = "hugoycj_StableNormal_main"

# app.py's Advanced Settings slider defaults (Stage 1: Sparse Structure /
# Stage 2: Structured Latent) -- the two stages have different native
# defaults, so an omitted --steps applies each rather than one shared number.
SS_SAMPLING_STEPS_DEFAULT = 50
SLAT_SAMPLING_STEPS_DEFAULT = 6


def preload_birefnet(pipeline: Hi3DGenPipeline) -> None:
    """hi3dgen.pipelines.hi3dgen.Hi3DGenPipeline._lazy_load_birefnet() hardcodes
    the local path 'weights/BiRefNet', which A3.2 never populated (it cached
    ZhengPeng7/BiRefNet under the shared HF cache instead). Pre-set the same
    attribute the lazy loader would, from the real repo id with
    local_files_only=True (already fully cached -- no network), so
    preprocess_image's lazy-load branch is skipped."""
    from transformers import AutoModelForImageSegmentation

    birefnet_model = AutoModelForImageSegmentation.from_pretrained(
        BIREFNET_REPO, trust_remote_code=True, local_files_only=True
    ).to(pipeline.device)
    birefnet_model.eval()
    pipeline.birefnet_model = birefnet_model


class DegenerateMatteError(Exception):
    """Raised when a concept matte fails the opaque-fraction gate."""


def check_matte(rgba: Image.Image) -> float:
    """Refuse a degenerate BiRefNet matte: opaque fraction >= 0.995 (the
    matte did nothing -- a raw RGB image, alpha == 255) or no opaque pixels
    at all. preprocess_image() is this matte's only surviving consumer, and
    a degenerate one there reconstructs the background as geometry -- silent
    degeneration, not a fit. Returns the measured opaque fraction."""
    alpha = np.asarray(rgba.convert("RGBA"))[:, :, 3]
    opaque = alpha > 0.1 * 255
    opaque_fraction = float(opaque.mean())
    if opaque_fraction >= 0.995:
        raise DegenerateMatteError(
            f"concept matte has no usable alpha ({opaque_fraction:.1%} opaque) -- "
            "BiRefNet produced a full-frame matte, not a fit")
    if opaque_fraction == 0.0:
        raise DegenerateMatteError(
            f"concept matte has no opaque pixels ({opaque_fraction:.1%} opaque)")
    return opaque_fraction


def matte_concept(pipeline: Hi3DGenPipeline, image: Image.Image) -> Image.Image:
    """BiRefNet background-removal matte at the concept image's own
    resolution/framing -- mirrors preprocess_image()'s internal
    resize-if-large + mask steps but stops short of its bbox crop/pad/resize/
    premultiply, so the alpha lines up pixel-for-pixel with the untouched
    concept image. This is what the texturing stage (prop_texture.py)
    projects: it needs the object's silhouette and the sampled pixels to
    come from the same frame, not Hi3DGen's cropped/centered working copy."""
    rgb = image.convert("RGB")
    max_size = max(rgb.size)
    scale = min(1, 1024 / max_size)
    if scale < 1:
        rgb = rgb.resize((int(rgb.width * scale), int(rgb.height * scale)), Image.Resampling.LANCZOS)
    mask = pipeline._get_birefnet_mask(rgb)
    rgba = np.array(rgb.convert("RGBA"))
    rgba[:, :, 3] = mask * 255
    return Image.fromarray(rgba)


def main():
    parser = argparse.ArgumentParser(description="Hi3DGen image -> raw untextured glb geometry.")
    parser.add_argument("image", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=None)
    parser.add_argument("--steps", type=int, default=None, help="Overrides both sampler stages uniformly; omit for app.py's per-stage defaults (50/6).")
    args = parser.parse_args()

    seed = args.seed if args.seed is not None else random.randint(0, 2**32 - 1)
    ss_steps = args.steps if args.steps is not None else SS_SAMPLING_STEPS_DEFAULT
    slat_steps = args.steps if args.steps is not None else SLAT_SAMPLING_STEPS_DEFAULT

    args.out.mkdir(parents=True, exist_ok=True)

    hi3dgen_pipeline = Hi3DGenPipeline.from_pretrained(GEOMETRY_WEIGHTS)
    hi3dgen_pipeline.cuda()
    preload_birefnet(hi3dgen_pipeline)

    try:
        normal_predictor = torch.hub.load(
            os.path.join(torch.hub.get_dir(), STABLE_NORMAL_HUB_SNAPSHOT),
            "StableNormal_turbo",
            yoso_version=YOSO_VERSION,
            source="local",
            local_cache_dir="./weights",
            pretrained=True,
        )
    except Exception:
        normal_predictor = torch.hub.load(
            "hugoycj/StableNormal",
            "StableNormal_turbo",
            trust_repo=True,
            yoso_version=YOSO_VERSION,
            local_cache_dir="./weights",
        )

    image = Image.open(args.image).convert("RGBA")
    concept_rgba = matte_concept(hi3dgen_pipeline, image)
    try:
        check_matte(concept_rgba)
    except DegenerateMatteError as e:
        sys.exit(f"prop_hi3dgen: {e}")
    concept_rgba_path = args.out / "concept_rgba.png"
    concept_rgba.save(concept_rgba_path)
    # concept_rgba already carries the real matte, so preprocess_image()'s
    # has_alpha branch reuses it directly instead of running BiRefNet again.
    image = hi3dgen_pipeline.preprocess_image(concept_rgba, resolution=1024)
    # app.py never seeds this stage (YOSONormalsPipeline draws from the
    # ambient RNG, no generator= plumbed through hubconf's Predictor), so a
    # same-seed re-run doesn't reproduce the same normal map without this --
    # hi3dgen_pipeline.run() below reseeds again for its own stages, the same
    # global-RNG convention it already uses internally.
    torch.manual_seed(seed)
    normal_image = normal_predictor(image, resolution=768, match_input_resolution=True, data_type="object")

    outputs = hi3dgen_pipeline.run(
        normal_image,
        seed=seed,
        formats=["mesh"],
        preprocess_image=False,
        sparse_structure_sampler_params={"steps": ss_steps},
        slat_sampler_params={"steps": slat_steps},
    )
    trimesh_mesh = outputs["mesh"][0].to_trimesh(transform_pose=True)

    raw_glb_path = args.out / "raw.glb"
    trimesh_mesh.export(str(raw_glb_path))

    manifest = {
        "model": "Stable-X/Hi3DGen",
        "weights": {
            "geometry": str(GEOMETRY_WEIGHTS),
            "normal": NORMAL_WEIGHTS_REPO,
            "birefnet": BIREFNET_REPO,
        },
        "input_image": str(args.image),
        "input_image_sha256": hashlib.sha256(args.image.read_bytes()).hexdigest(),
        "concept_rgba": str(concept_rgba_path),
        "concept_rgba_sha256": hashlib.sha256(concept_rgba_path.read_bytes()).hexdigest(),
        "seed": seed,
        "steps": {"sparse_structure_sampler": ss_steps, "slat_sampler": slat_steps},
        "torch_version": torch.__version__,
        "xformers_version": xformers.__version__,
        "vertex_count": int(trimesh_mesh.vertices.shape[0]),
        "face_count": int(trimesh_mesh.faces.shape[0]),
        "peak_vram_allocated_gb": torch.cuda.max_memory_allocated() / 1e9,
    }
    (args.out / "generation_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    print(
        f"OK: wrote {raw_glb_path} ({manifest['vertex_count']} verts, {manifest['face_count']} faces, "
        f"peak_vram={manifest['peak_vram_allocated_gb']:.2f} GB)"
    )


if __name__ == "__main__":
    main()
