#!/usr/bin/env python3
"""Verify the Hi3DGen weight files on disk against models.sha256.

Offline, no GPU, no ComfyUI server: re-hashes every `Hi3DGen/...` line in
the manifest against the real load-bearing location for that file. Hi3DGen
itself only manages its own `weights/` directory (`prop_hi3dgen.py`'s
GEOMETRY_WEIGHTS); BiRefNet and DINOv2 are dependencies it never populates
there -- they land in the HuggingFace hub cache and the torch hub checkpoint
cache respectively (README.md's Hi3DGen weights table), so the manifest
prefix->root mapping below has one entry per real root, most-specific
prefix first.
"""
import argparse
import hashlib
import sys
from pathlib import Path

MODELS_SHA256 = Path(__file__).resolve().parent / "models.sha256"

PREFIX_ROOTS = [
    ("Hi3DGen/BiRefNet/",
     Path.home() / ".cache" / "huggingface" / "hub" /
     "models--ZhengPeng7--BiRefNet" / "snapshots" /
     "e2bf8e4460fc8fa32bba5ea4d94b3233d367b0e4"),
    ("Hi3DGen/DINOv2/",
     Path.home() / ".cache" / "torch" / "hub" / "checkpoints"),
    ("Hi3DGen/",
     Path(r"C:\tools\Hi3DGen\Hi3DGen\weights")),
]


def resolve(manifest_path: str):
    """Map a `Hi3DGen/...` manifest path to its real on-disk location, or
    None if no configured root claims its prefix."""
    for prefix, root in PREFIX_ROOTS:
        if manifest_path.startswith(prefix):
            return root / manifest_path[len(prefix):]
    return None


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def load_hi3dgen_entries(manifest: Path):
    entries = []
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        expected_hash, manifest_path = line.split(None, 1)
        if manifest_path.startswith("Hi3DGen/"):
            entries.append((expected_hash, manifest_path))
    return entries


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=MODELS_SHA256,
                         help="models.sha256 to read (default: the real one)")
    args = parser.parse_args()

    problems = []
    checked = 0
    for expected_hash, manifest_path in load_hi3dgen_entries(args.manifest):
        real_path = resolve(manifest_path)
        if real_path is None:
            problems.append(f"{manifest_path}: no root mapping for this prefix")
            continue
        if not real_path.is_file():
            problems.append(f"{manifest_path}: missing on disk ({real_path})")
            continue
        actual_hash = sha256_of(real_path)
        checked += 1
        if actual_hash != expected_hash:
            problems.append(
                f"{manifest_path}: hash mismatch ({real_path})\n"
                f"    expected {expected_hash}\n"
                f"    actual   {actual_hash}"
            )

    if problems:
        print("Hi3DGen weight verify FAILED:")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(f"Hi3DGen weight verify OK: {checked} files match models.sha256.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
