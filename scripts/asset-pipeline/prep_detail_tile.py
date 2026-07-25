#!/usr/bin/env python3
"""DC-neutralise a CC0 photoscan tile for use as a multiplicative world-space
detail layer (see `tasks/aa-visual-upgrade-plan.md` micro-detail phase, T1).

Albedo: per-channel high-pass (subtract a wide gaussian blur, re-add 127.5) so
the tile's mean luminance lands at 0.5 -- it is multiplied onto the atlas
around 0.5, so any DC offset would tint every stone prop, and the low-frequency
blotch this removes is exactly what would make the 0.45 m tiling period visible.

Normal: passthrough at full native amplitude. The only real prerequisite is
that the mean normal not lean (X/Y within 128 +/- 3) -- this raw scan's mean
already does, so no correction is applied. There is no mean-Z requirement:
for a normal map carrying real grain, mean(sqrt(1-x^2-y^2)) sits well under
1.0 by Jensen's inequality alone, and forcing it up would mean discarding the
grain amplitude this tile exists to carry.

Run: python scripts/asset-pipeline/prep_detail_tile.py <color.png> <normalgl.png> <rough.png> <out_dir>
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

HIGHPASS_RADIUS = 96  # px, gaussian blur sigma removed from albedo (~1/21 of a 2048 tile)
LUMA = np.array([0.2126, 0.7152, 0.0722])


def dc_neutralize_albedo(img: Image.Image) -> tuple[Image.Image, float, float]:
    arr = np.asarray(img.convert("RGB")).astype(np.float64)
    pre_luma = float((arr @ LUMA).mean() / 255.0)

    blurred = np.asarray(img.filter(ImageFilter.GaussianBlur(HIGHPASS_RADIUS))).astype(np.float64)
    hp = np.clip(arr - blurred + 127.5, 0, 255)

    post_luma = float((hp @ LUMA).mean() / 255.0)
    return Image.fromarray(hp.round().astype(np.uint8), "RGB"), pre_luma, post_luma


def normal_stats(img: Image.Image) -> tuple[Image.Image, np.ndarray, np.ndarray]:
    rgb = img.convert("RGB")
    arr = np.asarray(rgb).astype(np.float64)
    mean_rgb = arr.reshape(-1, 3).mean(axis=0)
    std_rgb = arr.reshape(-1, 3).std(axis=0)
    return rgb, mean_rgb, std_rgb


def main():
    color_path, normal_path, rough_path, out_dir = sys.argv[1:5]
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    color = Image.open(color_path)
    albedo_out, pre_luma, post_luma = dc_neutralize_albedo(color)
    albedo_out.save(out_dir / "diff_2048.png")
    print(f"albedo mean luminance: pre={pre_luma:.4f} post={post_luma:.4f} (target 0.5 +/- 0.02)")

    normal = Image.open(normal_path)
    normal_out, mean_rgb, std_rgb = normal_stats(normal)
    normal_out.save(out_dir / "nor_gl_2048.png")
    print(f"normal mean RGB: {mean_rgb.round(2)} (X/Y target 128 +/- 3, no Z constraint)")
    print(f"normal std RGB: {std_rgb.round(2)} (full native amplitude, no scale applied)")

    rough = Image.open(rough_path).convert("L")
    rough.save(out_dir / "rough_2048.png")
    print(f"roughness: passthrough, size={rough.size}")


if __name__ == "__main__":
    main()
