#!/usr/bin/env python3
"""Assert a running ComfyUI server sees the expected downloaded models.

Queries GET /models/<folder> (server.py's raw folder-listing endpoint,
distinct from /object_info's per-node dropdown lists) for each folder in
EXPECTED and diffs the response against the exact filename set below.
"""
import json
import sys
import urllib.error
import urllib.request

COMFY_URL = "http://127.0.0.1:8188"

EXPECTED = {
    "checkpoints": {
        "sd_xl_base_1.0.safetensors",
        "sdxl_360_diffusion.safetensors",
        "flux1-schnell-fp8.safetensors",
    },
    "controlnet": {
        "controlnet-openpose-sdxl-1.0.safetensors",
        "controlnet-depth-sdxl-1.0.safetensors",
    },
    "text_encoders": {
        "clip_l.safetensors",
        "t5xxl_fp8_e4m3fn.safetensors",
    },
    "vae": set(),
    "diffusion_models": set(),
}


def list_models(folder: str) -> set[str]:
    with urllib.request.urlopen(f"{COMFY_URL}/models/{folder}", timeout=10) as resp:
        return set(json.load(resp))


def main() -> int:
    problems = []
    for folder, expected_files in EXPECTED.items():
        try:
            actual_files = list_models(folder)
        except (urllib.error.URLError, urllib.error.HTTPError) as e:
            problems.append(f"{folder}: request failed ({e})")
            continue
        missing = expected_files - actual_files
        extra = actual_files - expected_files
        if missing:
            problems.append(f"{folder}: missing {sorted(missing)}")
        if extra:
            problems.append(f"{folder}: extra {sorted(extra)}")

    if problems:
        print("Model inventory check FAILED:")
        for p in problems:
            print(f"  - {p}")
        return 1

    print("Model inventory check OK: all expected files present, no extras.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
