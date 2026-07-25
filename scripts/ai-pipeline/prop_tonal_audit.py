# Measures where chapel_arch's tonal/chromatic range is lost, stage by
# stage, against the rock_face_01 photoscan control: raw multiview diffusion
# (H1) -> post-MaterialAnything-delight albedo (H3) -> shipped atlas -> control.
# Decision-bearing for the delighting A/B (tasks/ai-pipeline -- MaterialAnything
# keep/drop): the numbers here are what showed delighting lifts albedo luma
# p1 36x and removes 66% of its std per view, before blending, which is why
# the A/B renders the recovered range through the engine instead of trusting
# a flat statistic. Read-only: decodes existing artifacts on disk (this
# candidate's target/prop-batch/ multiview intermediates plus the shipped
# content/models/props/ atlases), runs no generation stage.
import json
import sys
from math import cos, radians, sin
from pathlib import Path

import numpy as np
from PIL import Image

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import prop_audit as pa  # noqa: E402

CAND = REPO / "target" / "prop-batch" / "b3" / "arch" / "cand_0"
MV = CAND / "multiview"  # the 1536-res run confirmed to feed the shipped glb
CHAPEL_GLB = REPO / "content" / "models" / "props" / "chapel_arch" / "chapel_arch.glb"
CHAPEL_TEX = REPO / "content" / "models" / "props" / "chapel_arch" / "chapel_arch.textures"
ROCK_GLTF = REPO / "content" / "models" / "props" / "rock_face_01" / "rock_face_01_1k.gltf"
ROCK_TEX = REPO / "content" / "models" / "props" / "rock_face_01" / "rock_face_01_1k.textures"

MV_ELEVATION_DEG = 15.0


# --- CIELAB (D65) -----------------------------------------------------------

def srgb_to_lab(rgb):
    """rgb: (...,3) float in [0,1], gamma-encoded sRGB. Returns (...,3) Lab."""
    c = rgb.astype(np.float64)
    lin = np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)
    r, g, b = lin[..., 0], lin[..., 1], lin[..., 2]
    x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b
    y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b
    z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b
    xn, yn, zn = 0.95047, 1.0, 1.08883
    eps, kappa = 216 / 24389, 24389 / 27

    def f(t):
        return np.where(t > eps, np.cbrt(t), (kappa * t + 16) / 116)

    fx, fy, fz = f(x / xn), f(y / yn), f(z / zn)
    L = 116 * fy - 16
    a = 500 * (fx - fy)
    bb = 200 * (fy - fz)
    return np.stack([L, a, bb], axis=-1)


def luma_bt709(rgb):
    return 0.2126 * rgb[..., 0] + 0.7152 * rgb[..., 1] + 0.0722 * rgb[..., 2]


def stats_block(rgb, mask, label):
    """rgb: (H,W,3) float [0,1]; mask: (H,W) bool. Returns dict of stats."""
    if mask.sum() == 0:
        return {"label": label, "n": 0}
    px = rgb[mask]
    luma = luma_bt709(px)
    lab = srgb_to_lab(px[None, :, :])[0]
    p1, p5, p50, p95, p99 = np.percentile(luma, [1, 5, 50, 95, 99])
    return {
        "label": label,
        "n": int(mask.sum()),
        "luma_p1": float(p1), "luma_p5": float(p5), "luma_p50": float(p50),
        "luma_p95": float(p95), "luma_p99": float(p99),
        "luma_range_p99_p1": float(p99 - p1),
        "luma_std": float(luma.std()),
        "lab_a_std": float(lab[:, 1].std()),
        "lab_b_std": float(lab[:, 2].std()),
        "chroma_std": float(np.sqrt(lab[:, 1] ** 2 + lab[:, 2] ** 2).std()),
    }


def print_stats(rows):
    fields = ["n", "luma_p1", "luma_p5", "luma_p50", "luma_p95", "luma_p99",
              "luma_range_p99_p1", "luma_std", "lab_a_std", "lab_b_std", "chroma_std"]
    name_w = max(len(r["label"]) for r in rows) + 2
    print(f"{'':<{name_w}}" + "".join(f"{f:>15}" for f in fields))
    for r in rows:
        if r.get("n", 0) == 0:
            print(f"{r['label']:<{name_w}}" + f"{'(no pixels)':>15}")
            continue
        print(f"{r['label']:<{name_w}}" + "".join(
            f"{r[f]:>15.4f}" if f != "n" else f"{r[f]:>15d}" for f in fields))
    print()


# --- 1. Per-view: gen.png (H1, raw diffusion) vs albedo.png (H3, delit) ----

def view_masked_rgb(view_dir, name, mask):
    img = np.asarray(Image.open(view_dir / name).convert("RGB"), dtype=np.float32) / 255.0
    return img


print("=" * 100)
print("STAGE A -- per-view camera-space stats (H1 raw diffusion vs H3 delit albedo)")
print("=" * 100)
n_views = len(list(MV.glob("mask_*.png")))
print(f"multiview dir: {MV}  ({n_views} views, confirmed by gen.png sha256 match "
      f"against content/models/props/chapel_arch/generation_manifest.json)\n")

rows = []
for i in range(n_views):
    mask = np.asarray(Image.open(MV / f"mask_{i}.png").convert("L")) > 127
    gen_dir = MV / f"view_{i}"
    gen = view_masked_rgb(gen_dir, "gen.png", mask)
    alb = view_masked_rgb(gen_dir, "albedo.png", mask)
    rows.append(stats_block(gen, mask, f"view_{i} gen.png (H1)"))
    rows.append(stats_block(alb, mask, f"view_{i} albedo.png (H3)"))
print_stats(rows)

# aggregate across all views (mask-weighted, pooled pixels)
all_gen, all_alb, all_mask_px = [], [], 0
for i in range(n_views):
    mask = np.asarray(Image.open(MV / f"mask_{i}.png").convert("L")) > 127
    gen_dir = MV / f"view_{i}"
    gen = view_masked_rgb(gen_dir, "gen.png", mask)
    alb = view_masked_rgb(gen_dir, "albedo.png", mask)
    all_gen.append(gen[mask])
    all_alb.append(alb[mask])
all_gen = np.concatenate(all_gen)[None, :, :]
all_alb = np.concatenate(all_alb)[None, :, :]
pooled_mask = np.ones(all_gen.shape[:2], dtype=bool)
print_stats([
    stats_block(all_gen, pooled_mask, "ALL VIEWS pooled gen.png (H1)"),
    stats_block(all_alb, pooled_mask, "ALL VIEWS pooled albedo.png (H3)"),
])


# --- 2. Atlas: shipped chapel_arch base color vs rock_face_01 control ------

print("=" * 100)
print("STAGE B -- atlas-space stats: shipped chapel_arch vs rock_face_01 (control)")
print("=" * 100)

gltf, buffers = pa.load_gltf(CHAPEL_GLB)
chapel_base = np.asarray(Image.open(CHAPEL_TEX / "img2.dds").convert("RGB"), dtype=np.float32) / 255.0
w, h = chapel_base.shape[1], chapel_base.shape[0]
island = pa.island_mask(gltf, buffers, w, h)
print(f"chapel_arch atlas: {w}x{h}, island_frac={island.mean():.4f} "
      f"({int(island.sum())} texels)")

# island-masked, same methodology as chapel_arch (not the raw full-frame
# jpg -- that carries ~35% off-island padding, per prop_audit's own
# island_frac=0.645 for this prop, which would dilute the comparison)
rock_gltf, rock_buffers = pa.load_gltf(ROCK_GLTF)
rock = np.asarray(Image.open(ROCK_TEX / "img1.dds").convert("RGB"), dtype=np.float32) / 255.0
rw, rh = rock.shape[1], rock.shape[0]
rock_mask = pa.island_mask(rock_gltf, rock_buffers, rw, rh)
print(f"rock_face_01 control (base slot, island-masked): {rw}x{rh}, "
      f"island_frac={rock_mask.mean():.4f} ({int(rock_mask.sum())} texels)\n")

atlas_rows = [
    stats_block(chapel_base, island, "chapel_arch atlas (island only)"),
    stats_block(rock, rock_mask, "rock_face_01 diffuse (control) [REF]"),
]
print_stats(atlas_rows)

# recorded (authoritative) hole fraction from generation_manifest.json
gen_manifest = json.loads((REPO / "content" / "models" / "props" / "chapel_arch" /
                            "generation_manifest.json").read_text(encoding="utf-8"))
tex = gen_manifest["texture"]
hole_texels, texture_size = tex["hole_texels"], tex["texture_size"]
blend_coverage = tex["blend_coverage"]
print(f"recorded (authoritative, from generation_manifest.json):")
print(f"  blend_coverage (of island) = {blend_coverage}")
print(f"  hole_texels = {hole_texels} / texture_size^2 ({texture_size}^2={texture_size**2}) "
      f"= {hole_texels/texture_size**2:.4f} of full atlas")
print(f"  hole_texels / island_texels(measured) = {hole_texels/int(island.sum()):.4f} "
      f"(cross-check against 1-blend_coverage={1-blend_coverage:.4f})\n")


# --- 3. Facing+frustum coverage proxy (H4 spatial isolation) ---------------
# NOTE: this is a PROXY. The real "covered" mask blend_views() computed also
# tests occlusion against each view's float depth (multiview/depth_i.exr);
# no EXR-capable library is available in this system Python (cv2/OpenEXR/
# imageio/tifffile all absent -- checked), and re-deriving it exactly needs
# Blender's bpy. This proxy is facing+frustum ONLY (no occlusion test), which
# is a strict superset of the true covered set (occlusion can only shrink
# coverage) -- so this proxy's "hole" bucket is a STRICT SUBSET of the true
# Telea-filled texels; the "covered" bucket is contaminated with some texels
# that are actually true holes (self-occluded facing texels: undercuts,
# interior arch faces, drum-joint crevices). This biases the two buckets
# TOWARD each other, i.e. understates whatever contrast this comparison finds.
#
# glTF stores Y-up (prop_texture.py exports with export_yup=True) but every
# formula below (mv_view's cross products, the s/u screen axes) was written
# and verified in Blender's native Z-up space -- POSITION/NORMAL read from
# the glb are rotated back (inverse of the exporter's +90 deg X rotation)
# before this math runs, or "up" silently becomes the glb's 1.4 m Z-depth
# axis instead of its actual 5.5 m Y-height axis and the projected coverage
# is off by tens of points (found while building the delight A/B variant:
# target/delight-ab/build_variant_atlas.py's self-check against the recorded
# blend_coverage went from +0.27 wrong to -0.03 once this was applied, see
# target/delight-ab/variant_build_stats.json). u is also negated versus
# prop_texture.py's own view_weight formula: that code indexes Blender's
# bottom-up (row 0 = image bottom) pixel arrays, while this script decodes
# mask_i.png top-down (PIL, row 0 = image top).
print("=" * 100)
print("STAGE C -- H4 isolation via facing+frustum coverage proxy (see caveat above)")
print("=" * 100)


def mesh_attrs(gltf, buffers):
    """First mesh-bearing node's POSITION/NORMAL/TEXCOORD_0/indices, world
    scale applied to POSITION (matches pa.iter_prims's node-scale handling)."""
    for pos, uv, tris, scale in pa.iter_prims(gltf, buffers):
        return pos * scale, uv, tris
    raise RuntimeError("no mesh-bearing node")


def read_normal(gltf, buffers):
    for node in gltf.get("nodes", []):
        if "mesh" not in node:
            continue
        for prim in gltf["meshes"][node["mesh"]]["primitives"]:
            return pa.accessor_array(gltf, buffers, prim["attributes"]["NORMAL"])
    raise RuntimeError("no NORMAL attribute")


def gltf_to_blender_zup(v):
    return np.stack([v[:, 0], -v[:, 2], v[:, 1]], axis=1)


pos, uv, tris = mesh_attrs(gltf, buffers)
pos = gltf_to_blender_zup(pos)
nrm = gltf_to_blender_zup(read_normal(gltf, buffers))
lo, hi = pos.min(axis=0), pos.max(axis=0)
half = float(np.linalg.norm(hi - lo) / 2) * 1.05
center = (lo + hi) / 2

print(f"mesh bounds (chapel_arch.glb, {pos.shape[0]} verts, {tris.shape[0]} tris): "
      f"half={half:.3f} m, center={center.round(3)}")


def mv_view(az_deg, el_deg):
    az, el = radians(az_deg), radians(el_deg)
    d = np.array([sin(az) * cos(el), -cos(az) * cos(el), sin(el)])
    f = -d
    s = np.cross(f, [0.0, 0.0, 1.0])
    s /= np.linalg.norm(s)
    u = np.cross(s, f)
    dist = 2.0 * half
    return {"cam": center + d * dist, "f": f, "s": s, "u": u}


base_azs = [0.0, 90.0, 180.0, 270.0]
views = [mv_view(a, MV_ELEVATION_DEG) for a in base_azs]
for extra in tex.get("extra_views", []):
    views.append(mv_view(extra["azimuth_deg"], extra["elevation_deg"]))
print(f"reconstructed {len(views)} view cameras (4 base + "
      f"{len(tex.get('extra_views', []))} extra, matching generation_manifest.json)")

# --- rasterize per-atlas-texel world position + normal via barycentric
# interpolation, same V-flip pixel convention as pa.island_mask() ----------
w_atlas, h_atlas = w, h
pos_map = np.zeros((h_atlas, w_atlas, 3), dtype=np.float64)
nrm_map = np.zeros((h_atlas, w_atlas, 3), dtype=np.float64)
hit_map = np.zeros((h_atlas, w_atlas), dtype=bool)

px_all = np.stack([uv[:, 0], 1.0 - uv[:, 1]], axis=1) * np.array([w_atlas, h_atlas], dtype=np.float64)

for tri in tris:
    p = px_all[tri]  # (3,2) pixel-space
    x0 = max(int(np.floor(p[:, 0].min())), 0)
    x1 = min(int(np.ceil(p[:, 0].max())), w_atlas - 1)
    y0 = max(int(np.floor(p[:, 1].min())), 0)
    y1 = min(int(np.ceil(p[:, 1].max())), h_atlas - 1)
    if x1 < x0 or y1 < y0:
        continue
    xs, ys = np.meshgrid(np.arange(x0, x1 + 1) + 0.5, np.arange(y0, y1 + 1) + 0.5)
    v0 = p[1] - p[0]
    v1 = p[2] - p[0]
    v2x = xs - p[0, 0]
    v2y = ys - p[0, 1]
    d00 = v0 @ v0
    d01 = v0 @ v1
    d11 = v1 @ v1
    denom = d00 * d11 - d01 * d01
    if abs(denom) < 1e-12:
        continue
    d20 = v2x * v0[0] + v2y * v0[1]
    d21 = v2x * v1[0] + v2y * v1[1]
    vv = (d11 * d20 - d01 * d21) / denom
    ww = (d00 * d21 - d01 * d20) / denom
    uu = 1.0 - vv - ww
    inside = (uu >= -1e-6) & (vv >= -1e-6) & (ww >= -1e-6)
    if not inside.any():
        continue
    pw = pos[tri]
    nw = nrm[tri]
    interp_pos = (uu[..., None] * pw[0] + vv[..., None] * pw[1] + ww[..., None] * pw[2])
    interp_nrm = (uu[..., None] * nw[0] + vv[..., None] * nw[1] + ww[..., None] * nw[2])
    sub_y, sub_x = np.where(inside)
    pos_map[y0 + sub_y, x0 + sub_x] = interp_pos[sub_y, sub_x]
    nrm_map[y0 + sub_y, x0 + sub_x] = interp_nrm[sub_y, sub_x]
    hit_map[y0 + sub_y, x0 + sub_x] = True

norm_len = np.linalg.norm(nrm_map, axis=-1, keepdims=True)
nrm_map = nrm_map / np.maximum(norm_len, 1e-9)
print(f"rasterized {hit_map.sum()} texels ({hit_map.sum()/max(island.sum(),1):.4f} of measured island) "
      f"vs island_mask's {int(island.sum())} texels -- discrepancy is UV-rasterizer edge/AA "
      f"differences between this script's fill-if-barycentric-inside test and PIL's polygon fill")

covered_proxy = np.zeros((h_atlas, w_atlas), dtype=bool)
facing_count = np.zeros((h_atlas, w_atlas), dtype=np.int32)
FACING_EPS = 1e-2  # sqrt(MV_COVERAGE_EPS=1e-4), matching view_weight's threshold on (n.-f)^2
for v in views:
    rel = pos_map - v["cam"]
    s_half = half  # ortho_scale/2 == half, matching prop_texture.py's view_weight
    pxc = ((rel @ v["s"]) / s_half * 0.5 + 0.5) * w_atlas - 0.5
    pyc = ((rel @ -v["u"]) / s_half * 0.5 + 0.5) * h_atlas - 0.5
    inside = (pxc >= 0) & (pxc <= w_atlas - 1) & (pyc >= 0) & (pyc <= h_atlas - 1)
    facing = (nrm_map @ -v["f"]) > FACING_EPS
    this_view = inside & facing
    covered_proxy |= this_view
    facing_count += this_view.astype(np.int32)

hole_proxy = island & hit_map & ~covered_proxy
covered_island_proxy = island & hit_map & covered_proxy
print(f"\nproxy coverage (facing+frustum, NO occlusion test):")
print(f"  covered_proxy (of island & rasterized) = {covered_island_proxy.sum()/max((island & hit_map).sum(),1):.4f}")
print(f"  hole_proxy (of island & rasterized)     = {hole_proxy.sum()/max((island & hit_map).sum(),1):.4f}")
print(f"  authoritative blend_coverage (WITH occlusion) = {blend_coverage}")
print(f"  gap ({1-blend_coverage:.4f} true hole frac vs {hole_proxy.sum()/max((island & hit_map).sum(),1):.4f} "
      f"proxy hole frac) attributed to occlusion-only holes (undercuts/crevices this proxy cannot see)\n")

hole_rows = [
    stats_block(chapel_base, covered_island_proxy, "atlas texels: proxy-covered (never-inpainted, best case)"),
    stats_block(chapel_base, hole_proxy, "atlas texels: proxy-hole (definitely Telea-inpainted)"),
]
print_stats(hole_rows)
print("Caveat: proxy-covered bucket still contains an unknown number of TRUE holes\n"
      "(occlusion-only holes this proxy cannot detect), so the delta between these\n"
      "two rows is a LOWER BOUND on how much smoother/flatter inpainted texels are\n"
      "than genuinely covered ones.")


# --- 4. H2 isolation: singly-facing vs multiply-facing atlas texels --------
# Direct test of "does blend_views's weighted-sum average kill contrast where
# views overlap": within the SAME shipped atlas (same material stage, same
# spatial region set), compare texels only one view's facing test reaches
# (blend_views degenerates to a single-view copy there, weight from one
# source only) against texels 2+ views reach (a true weighted average).
# facing_count is still the occlusion-blind proxy above -- a texel counted
# "multi" here may in fact have been single- or zero-covered once occlusion
# is applied, which would mix true single-source texels into the "multi"
# bucket and bias this comparison DOWN (toward finding no difference).
print("=" * 100)
print("STAGE D -- H2 isolation: single- vs multi-view-facing atlas texels")
print("=" * 100)

valid = island & hit_map
single = valid & (facing_count == 1)
multi = valid & (facing_count >= 2)
print(f"single-facing texels: {int(single.sum())} ({single.sum()/max(valid.sum(),1):.4f} of valid island)")
print(f"multi-facing texels:  {int(multi.sum())} ({multi.sum()/max(valid.sum(),1):.4f} of valid island)\n")

print_stats([
    stats_block(chapel_base, single, "atlas texels: single-view-facing (proxy)"),
    stats_block(chapel_base, multi, "atlas texels: multi-view-facing (proxy)"),
])
