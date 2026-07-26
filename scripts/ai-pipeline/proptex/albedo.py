"""Albedo-source policy, the per-view delight, and the multiview blend for
the retexture atlas (prop_texture.py's basecolor path).

`needs_estimator` is the one place `albedo_source` decides anything: the
surface class declares whether a view's albedo is the lit multiview
generation itself (`direct`) or MaterialAnything's delit decomposition of it
(`delit`), and no other module reads that field.
"""

import json
import subprocess
from pathlib import Path

import bpy
import cv2
import numpy as np

import comfy_run
from proptex.atlas import (
    MV_EDGE_PAD_PX, atlas_size, bilinear, pad_edges, view_weight,
)
from proptex.coverage import covered_mask, coverage_stats
from proptex.registry import RegistryError
from proptex.scene import img_array, new_image, save_png

# The estimator runs in its own venv, not Blender's Python: its diffusers
# pin (0.28.2) predates everything else in the pipeline.
MA_PYTHON = Path(r"C:\tools\MaterialAnything") / "venv" / "Scripts" / "python.exe"
PROP_PBR = Path(__file__).resolve().parents[1] / "prop_pbr.py"

ESTIMATOR_WEIGHTS = (
    "MaterialAnything/material_estimator/text_encoder/model.safetensors",
    "MaterialAnything/material_estimator/unet/diffusion_pytorch_model.safetensors",
    "MaterialAnything/material_estimator/vae/diffusion_pytorch_model.safetensors",
)


class EstimateError(Exception):
    pass


def needs_estimator(albedo_source):
    """Whether the declared surface class routes its albedo through the
    delighting estimator (`delit`: foliage, character_skin) rather than
    painting it from the lit multiview generation (`direct`: limestone,
    wood, painted_metal)."""
    if albedo_source not in ("direct", "delit"):
        raise RegistryError(f"unknown albedo_source {albedo_source!r}")
    return albedo_source == "delit"


def torch_id():
    """The estimator venv's torch build, the toolchain string the estimate
    stage carries in its cache params: fp16 sampling is not reproducible
    across a torch or CUDA swap, and neither is a file the stage can hash."""
    torch = subprocess.run(
        [str(MA_PYTHON), "-c",
         "import torch; print(torch.__version__, torch.version.cuda)"],
        capture_output=True, text=True, check=True).stdout.strip()
    return f"torch {torch}"


def estimator_models():
    """{input key -> sha256} of the estimator's weight files, read from the
    same models.sha256 manifest the ComfyUI graphs' models are read from.
    Hashing them here would re-read 4.3 GiB on every run, hit or miss, and
    would be a second source of truth for the same fact."""
    hashes = comfy_run.load_model_hashes(comfy_run.MODELS_SHA256)
    inputs = {}
    for name in ESTIMATOR_WEIGHTS:
        if name not in hashes:
            raise EstimateError(f"no sha256 for estimator weight {name!r} "
                                f"in {comfy_run.MODELS_SHA256}")
        inputs[f"model:{name}"] = hashes[name]
    return inputs


def estimate_albedo(gen_png, normal_png, mask_png, seed, out_dir):
    """One view's delit albedo, decomposed from its lit generation by the
    MaterialAnything estimator (prop_pbr.py, subprocess in its own venv).
    The child's stdout is captured, not inherited: gen_prop.py parses this
    stage's single '{'-prefixed stdout line, which a streamed child JSON
    line would break."""
    proc = subprocess.run(
        [str(MA_PYTHON), str(PROP_PBR), str(gen_png), str(normal_png), str(mask_png),
         str(out_dir / "albedo.png"), "--seed", str(seed)],
        capture_output=True, text=True)
    if proc.returncode != 0:
        raise EstimateError(f"material estimator failed:\n{proc.stdout}{proc.stderr}")


def numpy_cv2_id():
    """numpy and OpenCV build identity, the toolchain string the blend stage
    carries in its cache params: the reprojection arithmetic and the Telea
    fill are theirs, and neither is a file the stage can hash."""
    return f"numpy {np.__version__} cv2 {cv2.__version__}"


def blend_views(views, sources, depths, rig, pos, nrm, island, out_dir):
    """Facing-weighted, occlusion-tested blend of every view's albedo source
    into the atlas, written to out_dir as base.png plus coverage.json;
    `sources` is each view's albedo source image, parallel to `views`.
    Island texels no view covered are Telea-inpainted from their
    surroundings, off-island texels keep a mean-color fill. Returns the
    coverage measurements."""
    accum = np.zeros((pos.shape[0], 3))
    wsum = np.zeros(pos.shape[0])
    for i, v in enumerate(views):
        gen_img = bpy.data.images.load(str(sources[i]))
        gen = img_array(gen_img)[:, :, :3].astype(np.float64)
        bpy.data.images.remove(gen_img)
        gen = pad_edges(gen, depths[i] > 0.01, MV_EDGE_PAD_PX)
        weight, px, py = view_weight(v, depths[i], rig, pos, nrm)
        accum += bilinear(gen, px, py) * weight[:, None]
        wsum += weight

    covered = covered_mask(wsum, island)
    out = np.empty((pos.shape[0], 4), dtype=np.float32)
    out[:, 3] = 1.0
    blended = accum[covered] / wsum[covered, None]
    fill = blended.mean(axis=0) if covered.any() else np.full(3, 0.5)
    out[:, :3] = fill
    out[covered, :3] = blended

    size = atlas_size(island)
    holes = island & ~covered
    if holes.any() and covered.any():
        img8 = (np.clip(out[:, :3], 0.0, 1.0) * 255.0).round().astype(np.uint8)
        img8 = img8.reshape(size, size, 3)
        mask8 = holes.reshape(size, size).astype(np.uint8)
        filled = cv2.inpaint(img8, mask8, 3, cv2.INPAINT_TELEA)
        # only hole texels take the 8-bit inpaint; covered texels keep
        # their float-precision blend
        out[holes, :3] = filled.reshape(-1, 3)[holes] / 255.0

    base_img = new_image("prop_base", srgb=True, fill=(0, 0, 0), size=size)
    base_img.pixels.foreach_set(out.ravel())
    save_png(base_img, out_dir / "base.png")
    bpy.data.images.remove(base_img)

    stats = coverage_stats(covered, island)
    (out_dir / "coverage.json").write_text(json.dumps(stats, indent=2), encoding="utf-8")
    return stats
