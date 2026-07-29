#!/usr/bin/env python3
"""Extraction-stage runner over a saved cubefeats.pt latent: replays
SparseFeatures2Mesh (structured-latent -> mesh) without re-running the
GPU diffusion pipeline that produced it. CPU is the default device because
its float order is deterministic where the GPU scatter_reduce path
decode_slat() runs through is not, making this the A/B instrument for
extraction-stage changes.

Run under the Hi3DGen venv; cwd-independent:
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe <path-to-this-repo>\\scripts\\ai-pipeline\\prop_extract.py <latents_dir> --out <dir> [--device cpu|cuda]
"""
import argparse
import hashlib
import json
import types
import time
from pathlib import Path

import torch
import trimesh

from hi3dgen.representations.mesh import SparseFeatures2Mesh

# decoder_mesh.py's SLatMeshDecoder constructs its extractor with
# res=resolution*4 where resolution=64, so 256 is the checkpoint's real
# marching-cubes grid size, not a tunable of this runner.
MESH_RESOLUTION = 256


def main():
    parser = argparse.ArgumentParser(
        description="Replay SparseFeatures2Mesh extraction over a saved cubefeats.pt latent."
    )
    parser.add_argument("latents_dir", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--device", choices=["cpu", "cuda"], default="cpu")
    args = parser.parse_args()
    latents_dir = args.latents_dir.resolve()
    out_dir = args.out.resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    cubefeats_path = latents_dir / "cubefeats.pt"
    cubefeats_sha256 = hashlib.sha256(cubefeats_path.read_bytes()).hexdigest()
    saved = torch.load(cubefeats_path, map_location=args.device)
    # SparseFeatures2Mesh.__call__ reads only .coords[:, 1:] and .feats off
    # its argument, so a real SparseTensor is not needed to drive it.
    cubefeats = types.SimpleNamespace(
        coords=saved["coords"].to(args.device), feats=saved["feats"].to(args.device)
    )

    extractor = SparseFeatures2Mesh(
        device=args.device, res=MESH_RESOLUTION
    )

    t_start = time.perf_counter()
    with torch.no_grad():
        mesh_result = extractor(cubefeats, training=False)
    trimesh_mesh = mesh_result.to_trimesh(transform_pose=True)
    elapsed_s = time.perf_counter() - t_start

    out_path = out_dir / "raw.glb"
    trimesh_mesh.export(str(out_path))

    stats = {
        "vertex_count": int(trimesh_mesh.vertices.shape[0]),
        "face_count": int(trimesh_mesh.faces.shape[0]),
        "volume": float(trimesh_mesh.volume),
        "body_count": int(trimesh_mesh.body_count),
        "is_watertight": bool(trimesh_mesh.is_watertight),
        "device": args.device,
        "elapsed_s": elapsed_s,
        "cubefeats_sha256": cubefeats_sha256,
        "out_glb": str(out_path),
    }
    print(json.dumps(stats))


if __name__ == "__main__":
    main()
