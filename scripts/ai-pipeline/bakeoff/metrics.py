# Baked-lighting measurement for generated multiview albedo (A6.2).
#
# The texturing stage emit-bakes generated views straight into basecolor, so
# any lighting in them is a defect. Mean luma and R-B were both tried as
# proxies and both are confounded -- a dark palette reads as "lit", a warm
# albedo reads as "warm light".
#
# This measures the thing directly: baked lighting is luminance that tracks
# surface orientation. Normals come from the depth map the view was
# conditioned on; the score is the variance explained by the best-fitting
# single directional light. A uniformly dark object with flat albedo scores ~0.
#
# Calibrate with lit_control.py before trusting a number: it renders the same
# rig with a hard sun and a flat white material, which is what "strongly lit"
# actually measures on this geometry (~0.33 -- real renders include shadow and
# occlusion, so a pure Lambert 1.0 is not reachable). Generated views sit at
# 0.006-0.018. Without that control the scores are unfalsifiable.
#
# Caveat: albedo can itself correlate with orientation (candle tops are wax,
# bases are stone), so the floor is not zero. Geometry and subject are fixed
# across configs, so DIFFERENCES are attributable to lighting; absolutes are not.
#
# Usage: python metrics.py <bakeoff_dir> [model ...]
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def normals_from_depth(depth):
    """Depth is an emission ramp; absolute scale is irrelevant to direction."""
    d = depth.astype(np.float32) / 255.0
    gy, gx = np.gradient(d)
    n = np.dstack([-gx, -gy, np.full_like(d, 0.05)])
    return n / np.maximum(np.linalg.norm(n, axis=2, keepdims=True), 1e-6)


def _light_dirs(n_az=24, n_el=6):
    out = []
    for el in np.linspace(0.0, np.pi / 2 * 0.95, n_el):
        for az in np.linspace(0, 2 * np.pi, n_az, endpoint=False):
            out.append([np.cos(el) * np.cos(az), np.cos(el) * np.sin(az), np.sin(el)])
    return np.array(out, dtype=np.float32)


def baked_fraction(luma, normals, mask):
    """Best R^2 of luma ~ a + b*max(N.L, 0) over a hemisphere of light dirs."""
    y = luma[mask]
    y = y - y.mean()
    denom = float((y * y).sum())
    if denom < 1e-9:
        return 0.0
    nm = normals[mask]
    best = 0.0
    for L in _light_dirs():
        x = np.maximum(nm @ L, 0.0)
        x = x - x.mean()
        sx = float((x * x).sum())
        if sx < 1e-9:
            continue
        b = float((x * y).sum()) / sx
        best = max(best, (b * b * sx) / denom)
    return best


def rgb_of(path):
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0


def luma_of(path):
    a = rgb_of(path)
    return 0.2126 * a[..., 0] + 0.7152 * a[..., 1] + 0.0722 * a[..., 2]


def main():
    root = Path(sys.argv[1])
    models = sys.argv[2:] or sorted(p.name for p in root.iterdir()
                                    if p.is_dir() and (p / "view_0.png").exists())

    masks, nrms = [], []
    for depth in sorted(root.glob("depth_*.png")):
        d = np.asarray(Image.open(depth).convert("L")).astype(np.float32)
        masks.append(d > 2)
        nrms.append(normals_from_depth(d))

    # drift = sigma of per-view mean colour: blend_views() reprojects all views
    # into one atlas, so per-view colour disagreement lands directly as seams.
    print(f"{'model':<20} {'baked':>6} {'drift':>7} {'lumRng':>7} {'meanLuma':>9}   per-view baked")
    for m in models:
        vals, lums, means = [], [], []
        for i in range(len(masks)):
            p = root / m / f"view_{i}.png"
            if not p.exists():
                continue
            rgb = rgb_of(p)
            lum = 0.2126 * rgb[..., 0] + 0.7152 * rgb[..., 1] + 0.0722 * rgb[..., 2]
            vals.append(baked_fraction(lum, nrms[i], masks[i]))
            lums.append(lum[masks[i]].mean())
            means.append(rgb[masks[i]].mean(axis=0))
        if vals:
            drift = np.array(means).std(axis=0).mean()
            print(f"{m:<20} {np.mean(vals):>6.3f} {drift:>7.4f} "
                  f"{max(lums) - min(lums):>7.4f} {np.mean(lums):>9.3f}   "
                  + " ".join(f"{v:.3f}" for v in vals))


if __name__ == "__main__":
    main()
