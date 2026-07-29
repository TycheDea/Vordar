#!/usr/bin/env python3
"""Hi3DGen image -> untextured raw glb geometry runner: drives
hi3dgen.headless.Session (BiRefNet matte -> StableNormal normal bridge ->
Hi3DGen geometry -> trimesh export) and adds this project's CLI, sanity
gates and per-candidate manifest. Texturing is a later pipeline stage --
this script's output is bare geometry.

--seed is repeatable: each seed is one candidate under <out>/cand_<seed>/,
and every candidate in a run shares the model load, the matte and the normal
prediction, none of which depend on the seed. A candidate whose outputs are
already on disk is skipped, so an interrupted run resumes.

Run under the Hi3DGen venv; cwd-independent (weight paths resolve against the
installed hi3dgen package, outputs against the parsed args):
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe <path-to-this-repo>\\scripts\\ai-pipeline\\prop_hi3dgen.py <image.png> --out <dir> [--seed N ...] [--ss-steps N] [--slat-steps N] [--ss-cfg F] [--slat-cfg F]
"""
import argparse
import hashlib
import json
import random
import sys
import time
from pathlib import Path

import numpy as np
import trimesh
from PIL import Image

from hi3dgen import headless

# The artefacts a finished candidate directory holds; the manifest is written
# last, so its presence is what makes a skip safe.
CANDIDATE_OUTPUTS = ("raw.glb", "concept_rgba.png", "normal.png", "hi3dgen_manifest.json")


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


def check_mesh(mesh_result, trimesh_mesh: trimesh.Trimesh) -> dict:
    """Refuse degenerate raw geometry before it reaches decimation/xatlas
    three stages downstream, where it currently surfaces as a confusing
    Blender abort (prop_cleanup.py). Mirrors check_matte's refusal at the
    input side.

    Zero-area faces are the one degeneracy that does not condemn a
    candidate: the GPU extractor's float reduction order varies between
    runs, so a handful of exactly-coincident vertices in a mesh of
    three-quarters of a million faces is an expected artifact. They are
    dropped from trimesh_mesh in place and counted, so the count still
    reaches the manifest and a rising trend stays visible. A mesh left with
    no faces at all by that drop is still refused.

    Returns the measured stats -- counts describe the mesh after the drop."""
    if not mesh_result.success:
        raise DegenerateMeshError(
            "Hi3DGen mesh extraction reported success=False (empty vertices or faces)")
    n_nonfinite = int((~np.isfinite(trimesh_mesh.vertices)).any(axis=1).sum())
    if n_nonfinite:
        raise DegenerateMeshError(f"{n_nonfinite} non-finite vertices in raw mesh")
    degenerate = trimesh_mesh.area_faces <= 0
    n_degenerate = int(degenerate.sum())
    if n_degenerate:
        trimesh_mesh.update_faces(~degenerate)
        trimesh_mesh.remove_unreferenced_vertices()
    if not len(trimesh_mesh.faces):
        raise DegenerateMeshError(
            f"all {n_degenerate} faces in raw mesh are zero-area (degenerate)")
    extents = trimesh_mesh.bounding_box.extents
    if not (extents > 0).all():
        raise DegenerateMeshError(f"degenerate bounding box extents {extents.tolist()}")
    return {
        "vertex_count": int(trimesh_mesh.vertices.shape[0]),
        "face_count": int(trimesh_mesh.faces.shape[0]),
        "degenerate_face_count": n_degenerate,
        "bbox_extents": extents.tolist(),
    }


def main():
    parser = argparse.ArgumentParser(description="Hi3DGen image -> raw untextured glb geometry.")
    parser.add_argument("image", type=Path)
    parser.add_argument("--out", type=Path, required=True, help="Batch directory; each candidate lands in <out>/cand_<seed>/")
    parser.add_argument("--seed", type=int, action="append", dest="seeds", metavar="N",
                        help="Repeatable: one candidate per seed, all sharing this run's model load and normal prediction.")
    parser.add_argument("--ss-steps", type=int, default=headless.SS_SAMPLING_STEPS_DEFAULT, help="Sparse structure stage sampling steps.")
    parser.add_argument("--slat-steps", type=int, default=headless.SLAT_SAMPLING_STEPS_DEFAULT, help="Structured latent stage sampling steps.")
    parser.add_argument("--ss-cfg", type=float, default=headless.SS_CFG_DEFAULT, help="Sparse structure stage CFG guidance strength.")
    parser.add_argument("--slat-cfg", type=float, default=headless.SLAT_CFG_DEFAULT, help="Structured latent stage CFG guidance strength.")
    parser.add_argument("--normal-resolution", type=int, default=headless.NORMAL_RESOLUTION_DEFAULT, help="StableNormal processing resolution.")
    parser.add_argument("--normal-model", choices=sorted(headless.NORMAL_ENTRYPOINTS), default="turbo", help="StableNormal predictor: single-step turbo (fast) or the full two-stage SD-based refinement (slower, sharper high-frequency detail).")
    parser.add_argument("--normal-steps", type=int, default=None, help="Override the normal predictor's denoising steps (turbo is a fixed single step regardless of this value).")
    parser.add_argument("--crop-from-original", action="store_true", help="Take the object crop from full-resolution pixels instead of the <=1024 matte copy.")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.image = args.image.resolve()

    seeds = args.seeds if args.seeds else [random.randint(0, 2**32 - 1)]
    if len(set(seeds)) != len(seeds):
        parser.error(f"repeated --seed value in {seeds}: two candidates would write the same cand_<seed>/ directory")
    if len(seeds) > 1 and args.normal_model == "full":
        # The full predictor initialises its prediction latent from noise
        # (stablenormal.pipeline_stablenormal's prepare_latents), so its
        # normal map is a function of the seed. Only the turbo predictor's
        # latent is the deterministic image latent, which is what lets one
        # prediction serve every candidate.
        parser.error("--normal-model full takes a single --seed: its normal map is seed-dependent, "
                     "so it cannot be shared across candidates")

    cand_dirs = {seed: args.out / f"cand_{seed}" for seed in seeds}
    pending = [seed for seed in seeds
               if not all((cand_dirs[seed] / name).exists() for name in CANDIDATE_OUTPUTS)]
    for seed in seeds:
        if seed not in pending:
            print(f"cand_{seed}: skip (exists) -> {cand_dirs[seed]}")
    if not pending:
        print(f"OK: all {len(seeds)} candidate(s) already complete under {args.out}")
        return
    for seed in pending:
        cand_dirs[seed].mkdir(parents=True, exist_ok=True)

    t_start = time.perf_counter()
    session = headless.Session(normal_model=args.normal_model)
    t_loaded = time.perf_counter()

    image = Image.open(args.image).convert("RGBA")
    concept_rgba = session.matte(image)
    # Gated before prepare(), so a degenerate matte costs no normal prediction.
    try:
        check_matte(concept_rgba)
    except DegenerateMatteError as e:
        sys.exit(f"prop_hi3dgen: {e}")
    for seed in pending:
        concept_rgba.save(cand_dirs[seed] / "concept_rgba.png")
    t_matted = time.perf_counter()

    prepared = session.prepare(
        image, concept_rgba,
        normal_resolution=args.normal_resolution,
        normal_steps=args.normal_steps,
        crop_from_original=args.crop_from_original,
        seed=seeds[0],
    )
    # The normal map is the geometry stage's only input: keeping it splits a
    # bad mesh into "the normal predictor saw it wrong" vs "the sampler built
    # it wrong", which the mesh alone cannot distinguish.
    for seed in pending:
        prepared.normal_image.save(cand_dirs[seed] / "normal.png")

    # Paid once for the whole run, and repeated into every candidate's
    # manifest so each record stands alone.
    shared_elapsed_s = {
        "load": t_loaded - t_start,
        "preprocess": (t_matted - t_loaded) + prepared.elapsed_s["preprocess"],
        "normal": prepared.elapsed_s["normal"],
        "cond": prepared.elapsed_s["cond"],
    }

    for position, seed in enumerate(pending):
        cand_dir = cand_dirs[seed]
        t_cand = time.perf_counter()
        cand = session.sample(
            seed,
            ss_steps=args.ss_steps, slat_steps=args.slat_steps,
            ss_cfg=args.ss_cfg, slat_cfg=args.slat_cfg,
        )
        t_geometry = time.perf_counter()
        try:
            mesh_stats = check_mesh(cand.mesh_result, cand.mesh)
        except DegenerateMeshError as e:
            sys.exit(f"prop_hi3dgen: cand_{seed}: {e}")

        raw_glb_path = cand_dir / "raw.glb"
        cand.mesh.export(str(raw_glb_path))
        t_cand_end = time.perf_counter()

        concept_rgba_path = cand_dir / "concept_rgba.png"
        normal_path = cand_dir / "normal.png"
        vram = headless.vram_peaks()
        vram["resident_gib"] = session.resident
        manifest = {
            "model": "Stable-X/Hi3DGen",
            **session.identity(),
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
            # Which candidate of which run produced this mesh, and the RNG
            # state its samplers started from: identical to the state a run
            # of this seed alone reaches, whatever the batch position.
            "batch": {"seeds": seeds, "generated": pending, "position": position},
            "sampler_rng_state_sha256": cand.rng_state_sha256,
            "sampler_params": cand.sampler_params,
            "extraction": cand.extraction,
            "elapsed_s": {
                **shared_elapsed_s,
                "geometry": t_geometry - t_cand,
                # A sub-interval of "geometry" above, not a sibling of it:
                # both samplers run before extraction starts, so these two
                # must never be summed.
                "extraction": cand.extract_s,
                "export": t_cand_end - t_geometry,
                "candidate": t_cand_end - t_cand,
            },
            "vertex_count": mesh_stats["vertex_count"],
            "face_count": mesh_stats["face_count"],
            "degenerate_face_count": mesh_stats["degenerate_face_count"],
            "vram": vram,
        }
        (cand_dir / "hi3dgen_manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")

        print(
            f"OK: wrote {raw_glb_path} ({manifest['vertex_count']} verts, {manifest['face_count']} faces, "
            f"peak_vram={vram['peak_allocated_gib']:.2f} GiB allocated / "
            f"{vram['peak_reserved_gib']:.2f} GiB reserved, {manifest['elapsed_s']['candidate']:.1f} s)"
        )

    t_end = time.perf_counter()
    if vram["peak_reserved_gib"] > 0.9 * vram["total_gib"]:
        print(
            f"WARNING: peak VRAM {vram['peak_reserved_gib']:.2f} GiB reserved of "
            f"{vram['total_gib']:.2f} GiB on {vram['device']} -- at this fill the driver "
            "starts spilling to system memory, which slows the run without failing it",
            file=sys.stderr,
        )
    print(
        f"OK: {len(pending)} candidate(s) under {args.out} in {t_end - t_start:.1f} s "
        f"(shared setup {sum(shared_elapsed_s.values()):.1f} s, of which model load {shared_elapsed_s['load']:.1f} s)"
    )


if __name__ == "__main__":
    main()
