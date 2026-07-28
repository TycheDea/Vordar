#!/usr/bin/env python3
"""Hi3DGen image -> untextured raw glb geometry runner: BiRefNet background
removal -> StableNormal-turbo normal bridge -> Hi3DGenPipeline geometry ->
to_trimesh export. Transcribes C:\\tools\\Hi3DGen\\Hi3DGen\\app.py's working
recipe (generate_3d), minus gradio. Texturing is a later pipeline stage --
this script's output is bare geometry.

Run under the Hi3DGen venv; cwd-independent (all weight/output paths
resolve against REPO_DIR or the parsed args, not the working directory):
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe <path-to-this-repo>\\scripts\\ai-pipeline\\prop_hi3dgen.py <image.png> --out <dir> [--seed N] [--ss-steps N] [--slat-steps N] [--ss-cfg F] [--slat-cfg F]
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
# Required by torch.use_deterministic_algorithms for CUDA >= 10.2's
# deterministic cuBLAS GEMM/matmul kernels; must be set before the CUDA
# context is created.
os.environ["CUBLAS_WORKSPACE_CONFIG"] = ":4096:8"

import random
import subprocess
import sys
import time
from contextlib import contextmanager
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
from hi3dgen.modules import sparse as hi3dgen_sparse  # noqa: E402
from hi3dgen.modules.sparse import conv as hi3dgen_sparse_conv  # noqa: E402
from hi3dgen.pipelines import Hi3DGenPipeline  # noqa: E402

# flash_attn has no Windows wheel, so xformers is a hard single-wheel
# dependency for sparse attention; a resolved value other than "xformers"
# means the torch/xformers pairing didn't come up the way ATTN_BACKEND asked,
# and letting that ride would only surface ~90s into the geometry stage.
assert hi3dgen_sparse.ATTN == "xformers", (
    f"hi3dgen.modules.sparse.ATTN resolved to {hi3dgen_sparse.ATTN!r}, expected 'xformers' "
    "(ATTN_BACKEND env var / torch-xformers wheel mismatch?)"
)

GEOMETRY_WEIGHTS = REPO_DIR / "weights" / "trellis-normal-v0-1"
NORMAL_WEIGHTS_REPO = "Stable-X/yoso-normal-v1-8-1"
YOSO_VERSION = "yoso-normal-v1-8-1"
# The full pipeline warm-starts its multi-step SD refinement from the YOSO
# estimate above, so both weight sets load together (hub:hubconf.py StableNormal()).
STABLE_NORMAL_FULL_REPO = "Stable-X/stable-normal-v0-1"
STABLE_NORMAL_DIFFUSION_VERSION = "stable-normal-v0-1"
NORMAL_ENTRYPOINTS = {"turbo": "StableNormal_turbo", "full": "StableNormal"}
BIREFNET_REPO = "ZhengPeng7/BiRefNet"
BIREFNET_REVISION = "e2bf8e4460fc8fa32bba5ea4d94b3233d367b0e4"
STABLE_NORMAL_HUB_SNAPSHOT = "hugoycj_StableNormal_main"

# app.py's Advanced Settings slider defaults (Stage 1: Sparse Structure /
# Stage 2: Structured Latent) -- the two stages have different native
# defaults, so an omitted --ss-steps/--slat-steps applies each rather than
# one shared number.
SS_SAMPLING_STEPS_DEFAULT = 50
SLAT_SAMPLING_STEPS_DEFAULT = 6
# CFG guidance strength from weights/trellis-normal-v0-1/pipeline.json's
# checkpoint defaults (5.0), not from the fork's Gradio demo (3.0).
SS_CFG_DEFAULT = 5.0
SLAT_CFG_DEFAULT = 5.0
# StableNormal's signature default (1024) resolves fine detail that 768 smears.
NORMAL_RESOLUTION_DEFAULT = 1024

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


def resident_gib() -> float:
    """Live (not cached) device allocation right now, the figure a free has to
    move for the freed weights to actually be gone."""
    return torch.cuda.memory_allocated() / GIB


@contextmanager
def staged(pipeline: Hi3DGenPipeline, *names: str):
    """Hold only the named models on pipeline.device for the block, then park
    them back on the CPU and drop the freed blocks. The stages read their
    weights in disjoint windows, so nothing is gained by having them resident
    together and a card this size cannot hold the sum."""
    for name in names:
        pipeline.models[name].to(pipeline.device)
    try:
        yield
    finally:
        for name in names:
            pipeline.models[name].to("cpu")
        torch.cuda.empty_cache()


@torch.no_grad()
def staged_run(pipeline: Hi3DGenPipeline, image: Image.Image, seed: int,
               ss_params: dict, slat_params: dict):
    """Hi3DGenPipeline.run(preprocess_image=False) with every stage's weights
    resident only while that stage runs. Call order and seeding point match
    run()'s, and the sampled stages must stay inside no_grad -- the mesh
    extractor calls .numpy() on the decoded field."""
    with staged(pipeline, "image_cond_model"):
        cond = pipeline.get_cond([image])
    torch.manual_seed(seed)
    with staged(pipeline, "sparse_structure_flow_model", "sparse_structure_decoder"):
        coords = pipeline.sample_sparse_structure(cond, 1, ss_params)
    with staged(pipeline, "slat_flow_model"):
        slat = pipeline.sample_slat(cond, coords, slat_params)
    with staged(pipeline, "slat_decoder_mesh"):
        return pipeline.decode_slat(slat, ["mesh"])


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


class DegenerateMeshError(Exception):
    """Raised when extracted geometry fails the mesh sanity gate."""


def check_matte(rgba: Image.Image) -> float:
    """Refuse a degenerate BiRefNet matte: opaque fraction >= 0.995 (the
    matte did nothing -- a raw RGB image, alpha == 255) or no opaque pixels
    at all. preprocess_image() is this matte's only surviving consumer, and
    a degenerate one there reconstructs the background as geometry -- silent
    degeneration, not a fit. Returns the measured opaque fraction.

    The threshold is preprocess_image()'s own bbox test (alpha > 0.8 * 255):
    pixels below it are outside the crop the pipeline derives, so a matte
    this gate accepts on softer pixels is one preprocess_image would reject."""
    alpha = np.asarray(rgba.convert("RGBA"))[:, :, 3]
    opaque = alpha > 0.8 * 255
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
    resolution/framing. Mirrors preprocess_image()'s internal resize-if-large
    + mask steps but stops short of bbox crop/pad/resize, so the alpha lines
    up pixel-for-pixel with the untouched concept image. Used as the
    conditioning source for preprocess_image(), which accepts RGBA and reuses
    the matte directly instead of re-running BiRefNet."""
    rgb = image.convert("RGB")
    max_size = max(rgb.size)
    scale = min(1, 1024 / max_size)
    if scale < 1:
        rgb = rgb.resize((int(rgb.width * scale), int(rgb.height * scale)), Image.Resampling.LANCZOS)
    mask = pipeline._get_birefnet_mask(rgb)
    rgba = np.array(rgb.convert("RGBA"))
    rgba[:, :, 3] = mask * 255
    return Image.fromarray(rgba)


def full_res_conditioning_source(image: Image.Image, concept_rgba: Image.Image) -> Image.Image:
    """The matte's alpha carried back onto the untouched full-resolution RGB,
    so preprocess_image()'s bbox crop is taken from original pixels instead of
    the <=1024 copy BiRefNet had to run on. Identity when the concept image is
    already within 1024 on its longest side, which is where the whole pipeline
    sits today."""
    rgba = image.convert("RGBA")
    alpha = concept_rgba.getchannel("A").resize(rgba.size, Image.Resampling.LANCZOS)
    rgba.putalpha(alpha)
    return rgba


def check_mesh(mesh_result, trimesh_mesh: trimesh.Trimesh) -> dict:
    """Refuse degenerate raw geometry before it reaches decimation/xatlas
    three stages downstream, where it currently surfaces as a confusing
    Blender abort (prop_cleanup.py). Mirrors check_matte's refusal at the
    input side. Returns the measured stats on success."""
    if not mesh_result.success:
        raise DegenerateMeshError(
            "Hi3DGen mesh extraction reported success=False (empty vertices or faces)")
    vertices = trimesh_mesh.vertices
    n_nonfinite = int((~np.isfinite(vertices)).any(axis=1).sum())
    if n_nonfinite:
        raise DegenerateMeshError(f"{n_nonfinite} non-finite vertices in raw mesh")
    extents = trimesh_mesh.bounding_box.extents
    if not (extents > 0).all():
        raise DegenerateMeshError(f"degenerate bounding box extents {extents.tolist()}")
    areas = trimesh_mesh.area_faces
    n_degenerate = int((areas <= 0).sum())
    if n_degenerate:
        raise DegenerateMeshError(
            f"{n_degenerate}/{len(areas)} zero-area (degenerate) faces in raw mesh")
    return {
        "vertex_count": int(vertices.shape[0]),
        "face_count": int(trimesh_mesh.faces.shape[0]),
        "degenerate_face_count": n_degenerate,
        "bbox_extents": extents.tolist(),
    }


def main():
    parser = argparse.ArgumentParser(description="Hi3DGen image -> raw untextured glb geometry.")
    parser.add_argument("image", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=None)
    parser.add_argument("--ss-steps", type=int, default=SS_SAMPLING_STEPS_DEFAULT, help="Sparse structure stage sampling steps.")
    parser.add_argument("--slat-steps", type=int, default=SLAT_SAMPLING_STEPS_DEFAULT, help="Structured latent stage sampling steps.")
    parser.add_argument("--ss-cfg", type=float, default=SS_CFG_DEFAULT, help="Sparse structure stage CFG guidance strength.")
    parser.add_argument("--slat-cfg", type=float, default=SLAT_CFG_DEFAULT, help="Structured latent stage CFG guidance strength.")
    parser.add_argument("--normal-resolution", type=int, default=NORMAL_RESOLUTION_DEFAULT, help="StableNormal processing resolution.")
    parser.add_argument("--normal-model", choices=sorted(NORMAL_ENTRYPOINTS), default="turbo", help="StableNormal predictor: single-step turbo (fast) or the full two-stage SD-based refinement (slower, sharper high-frequency detail).")
    parser.add_argument("--normal-steps", type=int, default=None, help="Override the normal predictor's denoising steps (turbo is a fixed single step regardless of this value).")
    parser.add_argument("--crop-from-original", action="store_true", help="Take the object crop from full-resolution pixels instead of the <=1024 matte copy.")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.image = args.image.resolve()

    seed = args.seed if args.seed is not None else random.randint(0, 2**32 - 1)

    args.out.mkdir(parents=True, exist_ok=True)

    t_start = time.perf_counter()
    hi3dgen_pipeline = Hi3DGenPipeline.from_pretrained(GEOMETRY_WEIGHTS)
    # The device the geometry stages run on; their weights stay on the CPU
    # until staged_run() brings each one over for its own stage.
    hi3dgen_pipeline.device = torch.device("cuda")
    preload_birefnet(hi3dgen_pipeline)

    normal_entrypoint = NORMAL_ENTRYPOINTS[args.normal_model]
    normal_load_kwargs = {"yoso_version": YOSO_VERSION}
    if args.normal_model == "full":
        normal_load_kwargs["diffusion_version"] = STABLE_NORMAL_DIFFUSION_VERSION
    normal_predictor = torch.hub.load(
        os.path.join(torch.hub.get_dir(), STABLE_NORMAL_HUB_SNAPSHOT),
        normal_entrypoint,
        source="local",
        local_cache_dir=str(REPO_DIR / "weights"),
        **normal_load_kwargs,
    )

    t_loaded = time.perf_counter()
    resident = {"after_load": resident_gib()}

    image = Image.open(args.image).convert("RGBA")
    concept_rgba = matte_concept(hi3dgen_pipeline, image)
    try:
        check_matte(concept_rgba)
    except DegenerateMatteError as e:
        sys.exit(f"prop_hi3dgen: {e}")
    concept_rgba_path = args.out / "concept_rgba.png"
    concept_rgba.save(concept_rgba_path)
    # concept_rgba already carries the real matte, so preprocess_image()'s
    # has_alpha branch reuses it directly instead of running BiRefNet again:
    # the matte above is the model's last consumer in this process, and its
    # weights must not ride the geometry stage's peak.
    del hi3dgen_pipeline.birefnet_model
    torch.cuda.empty_cache()
    resident["after_birefnet_free"] = resident_gib()
    conditioning_source = (
        full_res_conditioning_source(image, concept_rgba) if args.crop_from_original else concept_rgba
    )
    image = hi3dgen_pipeline.preprocess_image(conditioning_source, resolution=1024)
    t_preprocessed = time.perf_counter()
    # app.py never seeds this stage (YOSONormalsPipeline draws from the
    # ambient RNG, no generator= plumbed through hubconf's Predictor), so a
    # same-seed re-run doesn't reproduce the same normal map without this --
    # hi3dgen_pipeline.run() below reseeds again for its own stages, the same
    # global-RNG convention it already uses internally.
    torch.manual_seed(seed)
    normal_image = normal_predictor(
        image, resolution=args.normal_resolution, match_input_resolution=True,
        data_type="object", num_inference_steps=args.normal_steps,
    )
    # The normal map is the geometry stage's only input: keeping it splits a
    # bad mesh into "the normal predictor saw it wrong" vs "the sampler built
    # it wrong", which the mesh alone cannot distinguish.
    normal_path = args.out / "normal.png"
    normal_image.save(normal_path)
    t_normal = time.perf_counter()
    # normal_image is a PIL image, so the predictor's weights have no consumer
    # past this point and must not ride the geometry stage's peak.
    del normal_predictor
    torch.cuda.empty_cache()
    resident["after_normal_free"] = resident_gib()

    # Both samplers merge their checkpoint params (cfg_interval, rescale_t,
    # from weights/*/pipeline.json) under whatever run() is handed. Merging
    # here first makes the recorded dicts the ones the samplers actually ran
    # with, not just the steps/cfg this script asked for.
    ss_params = {**hi3dgen_pipeline.sparse_structure_sampler_params, "steps": args.ss_steps, "cfg_strength": args.ss_cfg}
    slat_params = {**hi3dgen_pipeline.slat_sampler_params, "steps": args.slat_steps, "cfg_strength": args.slat_cfg}
    outputs = staged_run(hi3dgen_pipeline, normal_image, seed, ss_params, slat_params)
    t_geometry = time.perf_counter()
    mesh_result = outputs["mesh"][0]
    trimesh_mesh = mesh_result.to_trimesh(transform_pose=True)
    try:
        mesh_stats = check_mesh(mesh_result, trimesh_mesh)
    except DegenerateMeshError as e:
        sys.exit(f"prop_hi3dgen: {e}")

    raw_glb_path = args.out / "raw.glb"
    trimesh_mesh.export(str(raw_glb_path))
    t_end = time.perf_counter()

    vram = vram_peaks()
    vram["resident_gib"] = resident
    manifest = {
        "model": "Stable-X/Hi3DGen",
        "hi3dgen": hi3dgen_id(),
        "weights": {
            "geometry": str(GEOMETRY_WEIGHTS),
            "normal": NORMAL_WEIGHTS_REPO,
            "normal_diffusion": STABLE_NORMAL_FULL_REPO if args.normal_model == "full" else None,
            "birefnet": BIREFNET_REPO,
        },
        "input_image": str(args.image),
        "input_image_sha256": hashlib.sha256(args.image.read_bytes()).hexdigest(),
        "concept_rgba": str(concept_rgba_path),
        "concept_rgba_sha256": hashlib.sha256(concept_rgba_path.read_bytes()).hexdigest(),
        "normal": str(normal_path),
        "normal_sha256": hashlib.sha256(normal_path.read_bytes()).hexdigest(),
        "normal_resolution": args.normal_resolution,
        "normal_model": args.normal_model,
        "normal_steps": args.normal_steps,
        "crop_from_original": args.crop_from_original,
        "seed": seed,
        "sampler_params": {"sparse_structure": ss_params, "slat": slat_params},
        "backends": {
            "attn": hi3dgen_attention.BACKEND,
            "sparse_attn": hi3dgen_sparse.ATTN,
            "spconv_algo": hi3dgen_sparse_conv.SPCONV_ALGO,
        },
        "versions": {
            "torch": torch.__version__,
            "cuda": torch.version.cuda,
            "xformers": xformers.__version__,
            "spconv": spconv.__version__,
            "trimesh": trimesh.__version__,
            "skimage": skimage.__version__,
            "numpy": np.__version__,
        },
        "elapsed_s": {
            "load": t_loaded - t_start,
            "preprocess": t_preprocessed - t_loaded,
            "normal": t_normal - t_preprocessed,
            "geometry": t_geometry - t_normal,
            "export": t_end - t_geometry,
            "total": t_end - t_start,
        },
        "vertex_count": mesh_stats["vertex_count"],
        "face_count": mesh_stats["face_count"],
        "degenerate_face_count": mesh_stats["degenerate_face_count"],
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
