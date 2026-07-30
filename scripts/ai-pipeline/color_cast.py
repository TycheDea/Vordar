"""Object-masked color-cast metrics for a textured prop candidate.

Measures mean R-B (0-255) and CIELAB a*/L* over the base-color atlas,
restricted to the UV-island footprint (prop_audit.island_mask) so empty
atlas texels never dilute the read. Usage:

    python color_cast.py <cand_dir>

where <cand_dir> contains final.textures/manifest.json as produced by the
prop chain's bake stage.
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image

import prop_audit as pa


def srgb_to_lab(rgb):
    # rgb: float array in [0,1], sRGB
    def inv_gamma(c):
        return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)
    lin = inv_gamma(rgb)
    r, g, b = lin[..., 0], lin[..., 1], lin[..., 2]
    x = r * 0.4124564 + g * 0.3575761 + b * 0.1804375
    y = r * 0.2126729 + g * 0.7151522 + b * 0.0721750
    z = r * 0.0193339 + g * 0.1191920 + b * 0.9503041
    xn, yn, zn = 0.95047, 1.0, 1.08883
    x, y, z = x / xn, y / yn, z / zn

    def f(t):
        d = 6.0 / 29.0
        return np.where(t > d ** 3, np.cbrt(t), t / (3 * d * d) + 4.0 / 29.0)
    fx, fy, fz = f(x), f(y), f(z)
    L = 116.0 * fy - 16.0
    a = 500.0 * (fx - fy)
    bb = 200.0 * (fy - fz)
    return L, a, bb


def measure(cand_dir):
    cand_dir = Path(cand_dir)
    manifest = pa.json.loads((cand_dir / "final.textures" / "manifest.json").read_text(encoding="utf-8"))
    textures_dir = cand_dir / "final.textures"
    slots = {pa.SLOT_NAMES.get(img["slot"], img["slot"]): textures_dir / img["file"] for img in manifest["images"]}
    gltf, buffers = pa.load_gltf(cand_dir / manifest["source"])
    img = Image.open(slots["base_color"])
    w, h = img.size
    mask = pa.island_mask(gltf, buffers, w, h)
    rgb = np.asarray(img.convert("RGB"), dtype=np.float32) / 255.0
    m = rgb[mask]
    L, a, b = srgb_to_lab(m[np.newaxis, :, :])
    r255 = m[:, 0].mean() * 255.0
    b255 = m[:, 2].mean() * 255.0
    print(f"{cand_dir.name}: island_frac={mask.mean():.4f} n={mask.sum()}")
    print(f"  R-B (0-255) = {r255 - b255:.2f}")
    print(f"  Lab a* mean = {a.mean():.3f}")
    print(f"  Lab L* mean = {L.mean():.3f}")


if __name__ == "__main__":
    measure(sys.argv[1])
