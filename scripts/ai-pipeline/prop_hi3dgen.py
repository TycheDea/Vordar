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
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import skimage
import spconv
import torch
import trimesh
import xformers
from PIL import Image

# hi3dgen is a plain repo checkout (not pip-installed) with real intra-package
# relative imports (hi3dgen.pipelines.hi3dgen does `from ..modules import
# sparse`), so it must be importable as a top-level package -- cwd alone does
# not land on sys.path for a `python script.py` invocation from elsewhere
# (Diffusion360 runner precedent: gen_pano_d360.py's REPO_DIR).
REPO_DIR = Path(r"C:\tools\Hi3DGen\Hi3DGen")
sys.path.insert(0, str(REPO_DIR))
from hi3dgen.modules import attention as hi3dgen_attention  # noqa: E402
from hi3dgen.modules.sparse import conv as hi3dgen_sparse_conv  # noqa: E402
from hi3dgen.pipelines import Hi3DGenPipeline  # noqa: E402

GEOMETRY_WEIGHTS = REPO_DIR / "weights" / "trellis-normal-v0-1"
NORMAL_WEIGHTS_REPO = "Stable-X/yoso-normal-v1-8-1"
YOSO_VERSION = "yoso-normal-v1-8-1"
BIREFNET_REPO = "ZhengPeng7/BiRefNet"
BIREFNET_REVISION = "e2bf8e4460fc8fa32bba5ea4d94b3233d367b0e4"
STABLE_NORMAL_HUB_SNAPSHOT = "hugoycj_StableNormal_main"

# app.py's Advanced Settings slider defaults (Stage 1: Sparse Structure /
# Stage 2: Structured Latent) -- the two stages have different native
# defaults, so an omitted --steps applies each rather than one shared number.
SS_SAMPLING_STEPS_DEFAULT = 50
SLAT_SAMPLING_STEPS_DEFAULT = 6

GIB = 1024 ** 3


def hi3dgen_id():
    """The Hi3DGen checkout's identity, the toolchain string this stage
    carries in its manifest. REPO_DIR is a working git checkout on a topic
    branch, not a pinned package: checking out another branch changes every
    mesh it produces while every file the manifest hashes stays identical."""
    def out(*args):
        return subprocess.run(["git", "-C", str(REPO_DIR), *args],
                              capture_output=True, text=True, check=True).stdout.strip()
    return {
        "rev": out("rev-parse", "HEAD"),
        "branch": out("rev-parse", "--abbrev-ref", "HEAD"),
        "dirty": bool(out("status", "--porcelain")),
    }


def vram_peaks():
    """GiB, the unit nvidia-smi and the card's spec sheet both use -- a
    decimal-GB figure reads ~7% under the card's own accounting, which is
    how a 12.3 GiB run on a 12 GiB card was recorded as '13.2' and never
    compared against anything."""
    return {
        "device": torch.cuda.get_device_name(0),
        "total_gib": torch.cuda.get_device_properties(0).total_memory / GIB,
        "peak_allocated_gib": torch.cuda.max_memory_allocated() / GIB,
        "peak_reserved_gib": torch.cuda.max_memory_reserved() / GIB,
    }


def preload_birefnet(pipeline: Hi3DGenPipeline) -> None:
    """hi3dgen.pipelines.hi3dgen.Hi3DGenPipeline._lazy_load_birefnet() hardcodes
    the local path 'weights/BiRefNet', which A3.2 never populated (it cached
    ZhengPeng7/BiRefNet under the shared HF cache instead). Pre-set the same
    attribute the lazy loader would, from the real repo id with
    local_files_only=True (already fully cached -- no network), so
    preprocess_image's lazy-load branch is skipped."""
    from transformers import AutoModelForImageSegmentation

    birefnet_model = AutoModelForImageSegmentation.from_pretrained(
        BIREFNET_REPO,
        revision=BIREFNET_REVISION,
        trust_remote_code=True,
        local_files_only=True,
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

    t_start = time.perf_counter()
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

    t_loaded = time.perf_counter()

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
    t_preprocessed = time.perf_counter()
    # app.py never seeds this stage (YOSONormalsPipeline draws from the
    # ambient RNG, no generator= plumbed through hubconf's Predictor), so a
    # same-seed re-run doesn't reproduce the same normal map without this --
    # hi3dgen_pipeline.run() below reseeds again for its own stages, the same
    # global-RNG convention it already uses internally.
    torch.manual_seed(seed)
    normal_image = normal_predictor(image, resolution=768, match_input_resolution=True, data_type="object")
    # The normal map is the geometry stage's only input: keeping it splits a
    # bad mesh into "the normal predictor saw it wrong" vs "the sampler built
    # it wrong", which the mesh alone cannot distinguish.
    normal_path = args.out / "normal.png"
    normal_image.save(normal_path)
    t_normal = time.perf_counter()

    # Both samplers merge their checkpoint params (cfg_strength, cfg_interval,
    # rescale_t, from weights/*/pipeline.json) under whatever run() is handed.
    # Merging here first makes the recorded dicts the ones the samplers
    # actually ran with, not the two steps counts this script asked for.
    ss_params = {**hi3dgen_pipeline.sparse_structure_sampler_params, "steps": ss_steps}
    slat_params = {**hi3dgen_pipeline.slat_sampler_params, "steps": slat_steps}
    outputs = hi3dgen_pipeline.run(
        normal_image,
        seed=seed,
        formats=["mesh"],
        preprocess_image=False,
        sparse_structure_sampler_params=ss_params,
        slat_sampler_params=slat_params,
    )
    t_geometry = time.perf_counter()
    trimesh_mesh = outputs["mesh"][0].to_trimesh(transform_pose=True)

    raw_glb_path = args.out / "raw.glb"
    trimesh_mesh.export(str(raw_glb_path))
    t_end = time.perf_counter()

    vram = vram_peaks()
    manifest = {
        "model": "Stable-X/Hi3DGen",
        "hi3dgen": hi3dgen_id(),
        "weights": {
            "geometry": str(GEOMETRY_WEIGHTS),
            "normal": NORMAL_WEIGHTS_REPO,
            "birefnet": BIREFNET_REPO,
        },
        "input_image": str(args.image),
        "input_image_sha256": hashlib.sha256(args.image.read_bytes()).hexdigest(),
        "concept_rgba": str(concept_rgba_path),
        "concept_rgba_sha256": hashlib.sha256(concept_rgba_path.read_bytes()).hexdigest(),
        "normal": str(normal_path),
        "normal_sha256": hashlib.sha256(normal_path.read_bytes()).hexdigest(),
        "seed": seed,
        "sampler_params": {"sparse_structure": ss_params, "slat": slat_params},
        "backends": {
            "attn": hi3dgen_attention.BACKEND,
            "spconv_algo": hi3dgen_sparse_conv.SPCONV_ALGO,
        },
        "versions": {
            "torch": torch.__version__,
            "cuda": torch.version.cuda,
            "xformers": xformers.__version__,
            "spconv": spconv.__version__,
            "trimesh": trimesh.__version__,
            "skimage": skimage.__version__,
        },
        "elapsed_s": {
            "load": t_loaded - t_start,
            "preprocess": t_preprocessed - t_loaded,
            "normal": t_normal - t_preprocessed,
            "geometry": t_geometry - t_normal,
            "export": t_end - t_geometry,
            "total": t_end - t_start,
        },
        "vertex_count": int(trimesh_mesh.vertices.shape[0]),
        "face_count": int(trimesh_mesh.faces.shape[0]),
        "vram": vram,
    }
    (args.out / "hi3dgen_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    if vram["peak_reserved_gib"] > 0.9 * vram["total_gib"]:
        print(
            f"WARNING: peak VRAM {vram['peak_reserved_gib']:.2f} GiB reserved of "
            f"{vram['total_gib']:.2f} GiB on {vram['device']} -- at this fill the driver "
            "starts spilling to system memory, which slows the run without failing it",
            file=sys.stderr,
        )
    print(
        f"OK: wrote {raw_glb_path} ({manifest['vertex_count']} verts, {manifest['face_count']} faces, "
        f"peak_vram={vram['peak_allocated_gib']:.2f} GiB allocated / "
        f"{vram['peak_reserved_gib']:.2f} GiB reserved, {manifest['elapsed_s']['total']:.1f} s)"
    )


if __name__ == "__main__":
    main()
