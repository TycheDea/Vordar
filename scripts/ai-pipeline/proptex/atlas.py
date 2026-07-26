"""Geometry atlas bake and per-texel view projection for the multiview
retexture atlas (prop_texture.py's basecolor path); consumed by albedo.py's
blend loop.

Every array this module touches -- the atlas pos/nrm/island and view depths
-- passes through Blender's own image API (baked directly), which converts
any file's on-disk row order to Blender's row-0-is-bottom pixel buffer on
load and back again on save. Nothing here reads a raw file decoder, so
nothing here needs its own flip; views.py's render_normal_view is the one
place that writes pixels through a non-Blender path (cv2), and states its
own flip where that happens.
"""

from math import isqrt

import bpy
import numpy as np

from proptex.scene import emission_graph, emit_bake, img_array, new_image

MV_WEIGHT_EXPONENT = 2.0
MV_OCCLUSION_EPS = 0.02  # meters; props are ~1.8 m tall (prop_cleanup)
MV_EDGE_PAD_PX = 8  # the base bleeds background across the depth edge; padding
# the object's colors outward keeps edge texels on-material without
# shrinking thin members (erosion would erase a ~6 px scroll arm entirely)


def bake_geometry_atlas(clean, atlas, rig, texture_size, out_dir):
    """Bake per-texel world position (normalized to mesh bounds), world
    normal (mapped to [0,1]) and an island mask into the atlas, written to
    out_dir as pos.npy, nrm.npy and island.npy."""
    lo = rig["lo"]
    extent = np.maximum(rig["hi"] - lo, 1e-6)

    tree, n_emit = emission_graph(clean)
    n_geo = tree.nodes.new("ShaderNodeNewGeometry")

    n_sub = tree.nodes.new("ShaderNodeVectorMath")
    n_sub.operation = "SUBTRACT"
    n_sub.inputs[1].default_value = tuple(lo)
    n_div = tree.nodes.new("ShaderNodeVectorMath")
    n_div.operation = "DIVIDE"
    n_div.inputs[1].default_value = tuple(extent)
    tree.links.new(n_geo.outputs["Position"], n_sub.inputs[0])
    tree.links.new(n_sub.outputs["Vector"], n_div.inputs[0])
    tree.links.new(n_div.outputs["Vector"], n_emit.inputs["Color"])
    pos_img = new_image("mv_pos", srgb=False, fill=(0, 0, 0), size=texture_size, float_buffer=True)
    emit_bake(clean, tree, pos_img, atlas)

    n_madd = tree.nodes.new("ShaderNodeVectorMath")
    n_madd.operation = "MULTIPLY_ADD"
    n_madd.inputs[1].default_value = (0.5, 0.5, 0.5)
    n_madd.inputs[2].default_value = (0.5, 0.5, 0.5)
    tree.links.new(n_geo.outputs["Normal"], n_madd.inputs[0])
    tree.links.new(n_madd.outputs["Vector"], n_emit.inputs["Color"])
    nrm_img = new_image("mv_nrm", srgb=False, fill=(0.5, 0.5, 0.5), size=texture_size, float_buffer=True)
    emit_bake(clean, tree, nrm_img, atlas)

    n_emit.inputs["Color"].default_value = (1.0, 1.0, 1.0, 1.0)
    for link in list(n_emit.inputs["Color"].links):
        tree.links.remove(link)
    mask_img = new_image("mv_mask", srgb=False, fill=(0, 0, 0), size=texture_size)
    emit_bake(clean, tree, mask_img, atlas)

    pos = lo + img_array(pos_img).reshape(-1, 4)[:, :3].astype(np.float64) * extent
    nrm = img_array(nrm_img).reshape(-1, 4)[:, :3].astype(np.float64) * 2.0 - 1.0
    nrm /= np.maximum(np.linalg.norm(nrm, axis=1, keepdims=True), 1e-9)
    island = img_array(mask_img).reshape(-1, 4)[:, 0] > 0.5
    for img in (pos_img, nrm_img, mask_img):
        bpy.data.images.remove(img)
    np.save(out_dir / "pos.npy", pos)
    np.save(out_dir / "nrm.npy", nrm)
    np.save(out_dir / "island.npy", island)


def atlas_size(texels):
    """Side of the square atlas a flat per-texel array came from. Every
    consumer of the atlas arrays derives the dimension here instead of being
    handed one, so a caller's texture_size cannot disagree with the arrays
    it was handed."""
    return isqrt(texels.shape[0])


def bilinear(arr, px, py):
    """Sample a (h, w) or (h, w, c) array at continuous pixel coords."""
    h, w = arr.shape[:2]
    x = np.clip(px, 0.0, w - 1.0)
    y = np.clip(py, 0.0, h - 1.0)
    x0 = np.clip(np.floor(x).astype(np.int64), 0, w - 2)
    y0 = np.clip(np.floor(y).astype(np.int64), 0, h - 2)
    fx, fy = x - x0, y - y0
    if arr.ndim == 3:
        fx, fy = fx[:, None], fy[:, None]
    a = arr[y0, x0] * (1 - fx) + arr[y0, x0 + 1] * fx
    b = arr[y0 + 1, x0] * (1 - fx) + arr[y0 + 1, x0 + 1] * fx
    return a * (1 - fy) + b * fy


def pad_edges(colors, mask, iterations):
    """Dilate on-mask colors outward so samples just past the silhouette
    read the object's material instead of the generated background."""
    for _ in range(iterations):
        grown = mask.copy()
        for src, dst in (((slice(1, None),), (slice(None, -1),)),
                         ((slice(None, -1),), (slice(1, None),)),
                         ((slice(None), slice(1, None)), (slice(None), slice(None, -1))),
                         ((slice(None), slice(None, -1)), (slice(None), slice(1, None)))):
            fill = mask[src] & ~grown[dst]
            colors[dst][fill] = colors[src][fill]
            grown[dst] |= mask[src]
        mask = grown
    return colors


def view_weight(v, depth, rig, pos, nrm):
    """Per-texel geometric blend weight of one view: squared facing, zeroed
    outside the ortho frustum or occluded by nearer geometry in the view's
    float depth render. A texel on the mesh projects onto its own surface,
    so no silhouette test: only occlusion disqualifies it. Returns
    (weight, px, py); px/py let blend_views sample the generated colors."""
    depth = depth.astype(np.float64)
    h, w = depth.shape
    rel = pos - v["cam"]
    px = ((rel @ v["s"]) / rig["half"] * 0.5 + 0.5) * w - 0.5
    py = ((rel @ v["u"]) / rig["half"] * 0.5 + 0.5) * h - 0.5
    zc = rel @ v["f"]
    inside = (px >= 0) & (px <= w - 1) & (py >= 0) & (py <= h - 1)
    z_rend = v["far"] - bilinear(depth, px, py) * (v["far"] - v["near"])
    visible = zc <= z_rend + MV_OCCLUSION_EPS
    weight = np.maximum(0.0, nrm @ -v["f"]) ** MV_WEIGHT_EXPONENT
    weight *= (inside & visible).astype(np.float64)
    return weight, px, py
