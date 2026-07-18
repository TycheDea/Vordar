#!/usr/bin/env python3
"""LDR equirect PNG -> game-ready Radiance .hdr: linearize, expand highlights
into HDR, blend the wrap seam, optionally inject a parameterized sun disc,
self-check against the committed CC0 reference stats, write .hdr + provenance
manifest.

Run under the StableMaterials venv:
C:\\tools\\StableMaterials\\venv\\Scripts\\python.exe scripts/ai-pipeline/hdr_post.py <ldr.png> --out <file.hdr> [--sun auto|AZ,EL|none] [--sun-intensity N] [--seed-manifest <generation_manifest.json>]
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

TARGET_W, TARGET_H = 2048, 1024

# Expansion curve out = GAIN*L + (PEAK_WHITE-GAIN)*L**POWER: strictly
# monotonic (all-positive terms), calibrated so pure white lands at
# evening_road_01's measured cloud/glow peak (~20) and a lit dusk sky's
# diffuse register (linear ~0.3-0.5) stays near its median (~0.55).
CURVE_GAIN = 1.4
CURVE_POWER = 6
PEAK_WHITE = 20.0

BLEND_COLS = 32

SUN_CORE_RADIUS_DEG = 0.5
SUN_LIMB_OUTER_DEG = 0.75
SUN_GLOW_SIGMA_DEG = 2.0
SUN_GLOW_FRACTION = 0.015
SUN_TINT = (1.0, 0.92, 0.80)
# EquirectImage::decode_hdr converts f32->f16 on upload; values > 65504
# become Inf in the IBL bake. 30000 keeps headroom under that ceiling
# while exceeding anything the engine can visibly use.
INTENSITY_CLAMP = 30000.0

SEAM_STRIP_PX = 8
SEAM_MAD_MAX = 0.02
MEDIAN_BAND = (0.02, 2.0)

LUMA = np.array([0.2126, 0.7152, 0.0722], dtype=np.float32)


def srgb_to_linear(c: np.ndarray) -> np.ndarray:
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4).astype(np.float32)


def seam_mad(img: np.ndarray) -> float:
    return float(np.abs(img[:, :SEAM_STRIP_PX] - img[:, -SEAM_STRIP_PX:]).mean())


def blend_wrap_seam(img: np.ndarray) -> np.ndarray:
    # A circular image has no independent signal to stitch in: the "content
    # past the right edge" IS the left edge. So blend mirror-pairs across the
    # seam (col W-1-m with col m), weight 0.5 at the seam tapering to 0 by
    # BLEND_COLS out -- the two seam-adjacent columns land on the same value,
    # making the wrap exactly continuous without duplicating content.
    out = img.copy()
    m = np.arange(BLEND_COLS, dtype=np.float32)
    w = 0.25 * (1.0 + np.cos(np.pi * m / BLEND_COLS))
    left = img[:, :BLEND_COLS]
    right = img[:, -1:-BLEND_COLS - 1:-1]
    wc = w[None, :, None]
    out[:, :BLEND_COLS] = (1.0 - wc) * left + wc * right
    out[:, -1:-BLEND_COLS - 1:-1] = (1.0 - wc) * right + wc * left
    return out


def pixel_angles(w: int, h: int) -> tuple:
    az = (np.arange(w, dtype=np.float32) + 0.5) / w * 360.0
    el = 90.0 - (np.arange(h, dtype=np.float32) + 0.5) / h * 180.0
    return az, el


def detect_sun(img: np.ndarray) -> tuple:
    h, w = img.shape[:2]
    lum = img @ LUMA
    top = cv2.GaussianBlur(lum[: h // 2], (0, 0), sigmaX=4)
    thr = np.percentile(top, 99.9)
    ys, xs = np.nonzero(top >= thr)
    wts = top[ys, xs]
    az, el = pixel_angles(w, h)
    # Circular mean over azimuth: a bright region straddling the wrap seam
    # would otherwise average to the opposite side of the sky.
    ang = np.radians(az[xs])
    az_c = float(np.degrees(np.arctan2((wts * np.sin(ang)).sum(), (wts * np.cos(ang)).sum()))) % 360.0
    el_c = float((wts * el[ys]).sum() / wts.sum())
    return az_c, el_c


def inject_sun(img: np.ndarray, az_s: float, el_s: float, intensity: float) -> np.ndarray:
    h, w = img.shape[:2]
    az, el = pixel_angles(w, h)
    # Angular distance on the sphere, not pixel distance: equirect pixels
    # compress toward the poles, so a pixel-space disc would be an ellipse
    # anywhere off the equator (and the glow wraps the seam for free).
    el_r, els_r = np.radians(el), np.radians(el_s)
    az_r, azs_r = np.radians(az), np.radians(az_s)
    cosd = np.sin(el_r)[:, None] * np.sin(els_r) + np.cos(el_r)[:, None] * np.cos(els_r) * np.cos(az_r - azs_r)[None, :]
    theta = np.degrees(np.arccos(np.clip(cosd, -1.0, 1.0)))
    t = np.clip((SUN_LIMB_OUTER_DEG - theta) / (SUN_LIMB_OUTER_DEG - SUN_CORE_RADIUS_DEG), 0.0, 1.0)
    core = t * t * (3.0 - 2.0 * t)
    glow = SUN_GLOW_FRACTION * np.exp(-((theta / SUN_GLOW_SIGMA_DEG) ** 2))
    shape = (intensity * (core + glow)).astype(np.float32)
    tint = np.array(SUN_TINT, dtype=np.float32)
    return img + shape[:, :, None] * tint[None, None, :]


def parse_sun(spec: str) -> tuple:
    if spec in ("auto", "none"):
        return spec, None
    try:
        az, el = (float(p) for p in spec.split(","))
    except ValueError:
        sys.exit(f"--sun must be 'auto', 'none', or 'AZ,EL' degrees; got {spec!r}")
    return "explicit", (az % 360.0, el)


def main():
    parser = argparse.ArgumentParser(description="LDR equirect PNG -> game-ready Radiance .hdr")
    parser.add_argument("input", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--sun", default="auto")
    parser.add_argument("--sun-intensity", type=float, default=2000.0)
    parser.add_argument("--seed-manifest", type=Path, default=None)
    args = parser.parse_args()
    sun_mode, sun_pos = parse_sun(args.sun)

    src_bytes = args.input.read_bytes()
    src_sha256 = hashlib.sha256(src_bytes).hexdigest()
    img = Image.open(args.input).convert("RGB")
    src_size = img.size
    if img.width != 2 * img.height:
        sys.exit(f"input must be exactly 2:1 equirect; got {img.width}x{img.height}")
    if src_size != (TARGET_W, TARGET_H):
        img = img.resize((TARGET_W, TARGET_H), Image.Resampling.LANCZOS)

    # Float32 end to end with a C-infinity curve: the pipeline itself
    # quantizes nothing, so it adds no banding beyond the 8-bit source's own
    # steps (which diffusion per-pixel texture already dithers). 1-LSB input
    # dither was rejected at design time: the curve's ~45x top-end slope
    # turns it into linear-domain noise that alone eats the seam-MAD budget.
    lin = srgb_to_linear(np.asarray(img, dtype=np.float32) / 255.0)

    hdr = CURVE_GAIN * lin + (PEAK_WHITE - CURVE_GAIN) * lin**CURVE_POWER
    hdr = blend_wrap_seam(hdr)

    sun_az = sun_el = None
    if sun_mode != "none":
        sun_az, sun_el = sun_pos if sun_mode == "explicit" else detect_sun(hdr)
        hdr = inject_sun(hdr, sun_az, sun_el, args.sun_intensity)
    hdr = np.minimum(hdr, INTENSITY_CLAMP)

    lum = hdr @ LUMA
    stats = {
        "peak": float(hdr.max()),
        "p99_9": float(np.percentile(lum, 99.9)),
        "median": float(np.median(lum)),
        "mean": float(lum.mean()),
        "seam_mad": seam_mad(hdr),
    }

    checks = {
        "dims_2to1_2048x1024": hdr.shape[:2] == (TARGET_H, TARGET_W),
        "finite_nonnegative": bool(np.isfinite(hdr).all() and (hdr >= 0.0).all()),
        f"peak_le_{INTENSITY_CLAMP:.0f}": stats["peak"] <= INTENSITY_CLAMP,
        f"median_in_{MEDIAN_BAND}": MEDIAN_BAND[0] <= stats["median"] <= MEDIAN_BAND[1],
        f"seam_mad_le_{SEAM_MAD_MAX}": stats["seam_mad"] <= SEAM_MAD_MAX,
    }
    for name, ok in checks.items():
        print(f"self-check {name}: {'PASS' if ok else 'FAIL'}")
    print(f"stats: peak={stats['peak']:.2f} p99.9={stats['p99_9']:.3f} median={stats['median']:.4f} mean={stats['mean']:.4f} seam_mad={stats['seam_mad']:.5f}")
    if not all(checks.values()):
        sys.exit(1)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    if not cv2.imwrite(str(args.out), np.ascontiguousarray(hdr[:, :, ::-1])):
        sys.exit(f"cv2.imwrite failed for {args.out}")

    seed_manifest = json.loads(args.seed_manifest.read_text(encoding="utf-8")) if args.seed_manifest else None
    manifest = {
        "source": str(args.input),
        "source_sha256": src_sha256,
        "source_size": list(src_size),
        "generation_manifest": seed_manifest,
        "linearize": "exact sRGB EOTF (IEC 61966-2-1)",
        "expansion_curve": {
            "formula": "gain*L + (peak_white-gain)*L**power",
            "gain": CURVE_GAIN,
            "power": CURVE_POWER,
            "peak_white": PEAK_WHITE,
        },
        "seam_blend": {"type": "mirror-pair cosine cross-blend", "columns": BLEND_COLS},
        "sun": {
            "mode": sun_mode,
            "azimuth_deg": sun_az,
            "elevation_deg": sun_el,
            "intensity": args.sun_intensity if sun_mode != "none" else None,
            "core_radius_deg": SUN_CORE_RADIUS_DEG,
            "limb_outer_deg": SUN_LIMB_OUTER_DEG,
            "glow_sigma_deg": SUN_GLOW_SIGMA_DEG,
            "glow_fraction": SUN_GLOW_FRACTION,
            "tint_rgb": list(SUN_TINT),
            "clamp": INTENSITY_CLAMP,
            "mapping": "azimuth 0..360 -> x 0..W left-to-right, elevation +90 -> y=0 (zenith)",
        },
        "stats": stats,
        "versions": {"opencv": cv2.__version__, "numpy": np.__version__},
    }
    manifest_path = args.out.with_name(args.out.stem + ".manifest.json")
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"OK: wrote {args.out} + {manifest_path.name}")


if __name__ == "__main__":
    main()
