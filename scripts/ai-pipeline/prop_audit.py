# Measurement-only prop metric audit (AI content pipeline, Phase 0 inspection
# instrument). Prints one row per shipped prop under content/models/props/,
# with the Poly Haven photoscan rock_face_01 as the reference row.
#
# No thresholds, no pass/fail, no non-zero exit on a bad number. Nobody yet
# knows what quality this pipeline can reach -- every generated prop shipped
# so far read as AI slop under an instrument that was itself broken (gamma-
# wrong offscreen renders, a density "control" that is actually a material
# control, a shader that never reads AO). Writing a gate now would encode a
# guess from evidence already known to be bad. Stable thresholds get promoted
# verbatim into game/vordar-game/tests/content_lint.rs in a later phase, once
# that phase has established what is achievable; the metric names and slot
# keys below are chosen to match content_lint.rs's MaterialData vocabulary
# (base_color / normal / metallic_roughness / occlusion) so that promotion is
# a rename-free port, not a rewrite.
#
# Plain system Python: numpy + Pillow only, both already used by
# bakeoff/metrics.py. PIL 12.3 decodes BC7 and BC5 .dds directly, so no
# texconv round-trip. glTF/GLB accessors are read with pure struct, and the
# normal-map Laplacian is hand-rolled numpy -- deliberately not cv2, which
# lives inside Blender's bundled interpreter and is unavailable here.
import argparse
import json
import re
import struct
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

from bakeoff.metrics import baked_fraction, luma_of, rgb_of

REPO_ROOT = Path(__file__).resolve().parents[2]
PROPS_DIR = REPO_ROOT / "content/models/props"
ZONES_RON = REPO_ROOT / "content/zones/zones.ron"
REFERENCE_PROP = "rock_face_01"

# manifest slot string -> MaterialData field name (content_lint.rs:259-265),
# so a later promotion into that test can reuse these keys unchanged.
SLOT_NAMES = {"base": "base_color", "normal": "normal", "mr": "metallic_roughness", "ao": "occlusion"}

FIELDS = [
    "roughness_mean", "roughness_std", "metallic_mean",
    "ao_mean", "ao_bound",
    "albedo_luma_p1", "albedo_luma_p50", "albedo_luma_p99", "albedo_blown_frac",
    "normal_lap_std", "normal_flat_frac",
    "island_frac",
    "atlas_px_per_m", "placed_px_per_m", "world_area_m2", "atlas_w", "atlas_h",
    "roughness_factor", "metallic_factor", "base_color_factor",
    "blend_coverage", "hole_frac",
    "baked_fraction_ts",
]
PRECISION = {
    "roughness_mean": 3, "roughness_std": 3, "metallic_mean": 3, "ao_mean": 3,
    "albedo_luma_p1": 3, "albedo_luma_p50": 3, "albedo_luma_p99": 3, "albedo_blown_frac": 3,
    "normal_lap_std": 3, "normal_flat_frac": 3, "island_frac": 3,
    "atlas_px_per_m": 1, "placed_px_per_m": 1, "world_area_m2": 2,
    "roughness_factor": 2, "metallic_factor": 2,
    "blend_coverage": 4, "hole_frac": 4, "baked_fraction_ts": 4,
}

# --- glTF/GLB reading (pure struct; no pygltflib/trimesh dependency) -------

_COMPONENT = {5120: ("b", 1), 5121: ("B", 1), 5122: ("h", 2), 5123: ("H", 2), 5125: ("I", 4), 5126: ("f", 4)}
_TYPE_COUNT = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def load_gltf(path):
    """(json_dict, [buffer_bytes, ...]) for a .glb, or a .gltf + external .bin."""
    data = path.read_bytes()
    if path.suffix == ".glb":
        magic, _version, length = struct.unpack_from("<4sII", data, 0)
        if magic != b"glTF":
            raise ValueError(f"{path}: not a GLB (bad magic)")
        offset, gltf, bin_chunk = 12, None, None
        while offset < length:
            chunk_len, chunk_type = struct.unpack_from("<II", data, offset)
            offset += 8
            chunk = data[offset:offset + chunk_len]
            offset += chunk_len
            if chunk_type == 0x4E4F534A:  # 'JSON'
                gltf = json.loads(chunk)
            elif chunk_type == 0x004E4942:  # 'BIN\0'
                bin_chunk = chunk
        return gltf, [bin_chunk]
    gltf = json.loads(data)
    buffers = [(path.parent / b["uri"]).read_bytes() for b in gltf.get("buffers", [])]
    return gltf, buffers


def accessor_array(gltf, buffers, index):
    """Decode one accessor into an (count, n_components) float32 array."""
    acc = gltf["accessors"][index]
    fmt, comp_size = _COMPONENT[acc["componentType"]]
    n_comp = _TYPE_COUNT[acc["type"]]
    bv = gltf["bufferViews"][acc["bufferView"]]
    buf = buffers[bv["buffer"]]
    base = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
    stride = bv.get("byteStride", n_comp * comp_size)
    out = np.empty((acc["count"], n_comp), dtype=np.float32)
    for i in range(acc["count"]):
        out[i] = struct.unpack_from(f"<{n_comp}{fmt}", buf, base + i * stride)
    return out


def first_material(gltf):
    for mesh in gltf.get("meshes", []):
        for prim in mesh["primitives"]:
            if "material" in prim:
                return gltf["materials"][prim["material"]]
    return {}


def iter_prims(gltf, buffers):
    """(pos, uv, tris, node_scale) for every mesh-bearing node primitive."""
    for node in gltf.get("nodes", []):
        if "mesh" not in node:
            continue
        scale = np.array(node.get("scale", [1.0, 1.0, 1.0]), dtype=np.float32)
        for prim in gltf["meshes"][node["mesh"]]["primitives"]:
            pos = accessor_array(gltf, buffers, prim["attributes"]["POSITION"])
            uv = accessor_array(gltf, buffers, prim["attributes"]["TEXCOORD_0"])
            indices = accessor_array(gltf, buffers, prim["indices"])[:, 0].astype(np.int64)
            yield pos, uv, indices.reshape(-1, 3), scale


def mesh_areas(gltf, buffers):
    """Sum world-space and UV-space triangle area over every mesh-bearing
    node. Node scale is applied component-wise to positions before the area
    cross product (correct under anisotropic scale); placement scale from
    zones.ron is a separate, later division (placed_px_per_m)."""
    world_total = uv_total = 0.0
    for pos, uv, tris, scale in iter_prims(gltf, buffers):
        pos = pos * scale
        v0, v1, v2 = pos[tris[:, 0]], pos[tris[:, 1]], pos[tris[:, 2]]
        world_total += 0.5 * float(np.linalg.norm(np.cross(v1 - v0, v2 - v0), axis=1).sum())
        u0, u1, u2 = uv[tris[:, 0]], uv[tris[:, 1]], uv[tris[:, 2]]
        uv_total += 0.5 * float(np.abs((u1[:, 0] - u0[:, 0]) * (u2[:, 1] - u0[:, 1])
                                        - (u2[:, 0] - u0[:, 0]) * (u1[:, 1] - u0[:, 1])).sum())
    return world_total, uv_total


def island_mask(gltf, buffers, w, h):
    """Atlas-resolution boolean mask, True where a TEXCOORD_0 triangle covers
    that texel. Rasterized directly from the glb's own UVs (no Blender), so
    audit stats can be restricted to surface texels instead of the whole
    atlas (F-2: off-island texels are mean-fill/inpaint, not surface data,
    and dilute every whole-atlas statistic toward however much of the atlas
    xatlas actually packed)."""
    img = Image.new("L", (w, h), 0)
    draw = ImageDraw.Draw(img)
    for _pos, uv, tris, _scale in iter_prims(gltf, buffers):
        # V flipped: the bake wrote texels in Blender's V-up UV space, but
        # glTF TEXCOORD_0 stores V-down (top-left origin) -- confirmed
        # empirically against the shipped normal atlas's flat-texel geometry.
        px = np.stack([uv[:, 0], 1.0 - uv[:, 1]], axis=1) * np.array([w, h], dtype=np.float32)
        for tri in tris:
            draw.polygon([tuple(px[i]) for i in tri], fill=255)
    return np.asarray(img) > 0


# --- zones.ron placement scale ---------------------------------------------

# One prop tuple per line in zones.ron, model immediately followed by its own
# scale field -- a regex scan is enough; this is not a general RON parser.
_SCALE_RE = re.compile(r'model:\s*"([^"]+)".*?scale:\s*([0-9.]+)', re.DOTALL)


def placement_scales():
    """prop dir name -> max placement scale used anywhere in zones.ron."""
    text = ZONES_RON.read_text(encoding="utf-8")
    scales = {}
    for model, scale in _SCALE_RE.findall(text):
        name = Path(model).parent.name
        scales[name] = max(scales.get(name, 0.0), float(scale))
    return scales


# --- per-texture metrics -----------------------------------------------------

def arm_stats(path, mask):
    arm = rgb_of(path)  # MR packing: G=roughness, B=metallic (R is unused padding)
    return {
        "roughness_mean": float(arm[..., 1][mask].mean()),
        "roughness_std": float(arm[..., 1][mask].std()),
        "metallic_mean": float(arm[..., 2][mask].mean()),
    }


def ao_stats(path, mask):
    occ = rgb_of(path)  # glTF occlusionTexture reads the R channel
    return {"ao_mean": float(occ[..., 0][mask].mean())}


def albedo_stats(path, mask):
    luma = luma_of(path)
    m = luma[mask]
    stats = {
        "albedo_luma_p1": float(np.percentile(m, 1)),
        "albedo_luma_p50": float(np.percentile(m, 50)),
        "albedo_luma_p99": float(np.percentile(m, 99)),
        "albedo_blown_frac": float((m > 0.9).mean()),
    }
    return stats, luma


def _laplacian(a):
    return a[:-2, 1:-1] + a[2:, 1:-1] + a[1:-1, :-2] + a[1:-1, 2:] - 4 * a[1:-1, 1:-1]


def normal_stats(path, mask):
    rgb8 = np.asarray(Image.open(path).convert("RGB"), dtype=np.int16)
    flat = (np.abs(rgb8[..., 0] - 128) <= 2) & (np.abs(rgb8[..., 1] - 128) <= 2)
    flat_frac = float(flat[mask].mean())
    rgb = rgb8.astype(np.float32) / 255.0
    nx, ny = 2.0 * rgb[..., 0] - 1.0, 2.0 * rgb[..., 1] - 1.0
    interior_mask = mask[1:-1, 1:-1]
    pooled = np.concatenate([_laplacian(nx)[interior_mask], _laplacian(ny)[interior_mask]])
    return float(pooled.std()), flat_frac


def baked_fraction_ts(luma, normal_rgb):
    """baked_fraction (bakeoff/metrics.py) applied in tangent space off the
    normal atlas, instead of the world-space normals the prior art derives
    from generation-stage depth renders -- those work dirs are not shipped,
    only .glb + .textures + generation_manifest.json survive. This is a
    relative comparator against the reference row, not a physical light fit:
    assets with 26-30% dead-flat normal texels (normal_flat_frac) feed the
    regressor a near-constant N, which suppresses the score on exactly the
    props under suspicion. albedo_luma_p1 is the discriminator that actually
    fires; this metric rides along for corroboration, not as the primary
    signal. BC5 normal atlases carry no blue channel (PIL decodes it as 0),
    so Z is reconstructed from the unit-length constraint.
    """
    if luma.shape != normal_rgb.shape[:2]:
        return None
    nx, ny = 2.0 * normal_rgb[..., 0] - 1.0, 2.0 * normal_rgb[..., 1] - 1.0
    nz = np.sqrt(np.clip(1.0 - nx * nx - ny * ny, 0.0, None))
    normals = np.dstack([nx, ny, nz])
    mask = np.ones(luma.shape, dtype=bool)
    return float(baked_fraction(luma, normals, mask))


def generation_stats(prop_dir):
    path = prop_dir / "generation_manifest.json"
    if not path.exists():
        return None, None
    tex = json.loads(path.read_text(encoding="utf-8")).get("texture", {})
    coverage = tex.get("blend_coverage")
    holes, size = tex.get("hole_texels"), tex.get("texture_size")
    hole_frac = holes / size ** 2 if holes is not None and size else None
    return coverage, hole_frac


# --- per-prop assembly --------------------------------------------------

def measure_prop(prop_dir, scales):
    row = {f: None for f in FIELDS}
    row["prop"] = prop_dir.name
    row["reference"] = prop_dir.name == REFERENCE_PROP
    row["note"] = None

    manifests = sorted(prop_dir.glob("*.textures/manifest.json"))
    if not manifests:
        row["note"] = "no manifest.json -- textures undiscoverable"
        return row
    manifest = json.loads(manifests[0].read_text(encoding="utf-8"))
    textures_dir = manifests[0].parent
    # manifest keys by image index (matches img<N>.dds); do NOT resolve slots
    # via the glTF's own texture->image graph, which can disagree (a prop's
    # baseColorTexture.index is a *texture* index, not the img<N>.dds index).
    slots = {SLOT_NAMES.get(img["slot"], img["slot"]): textures_dir / img["file"]
             for img in manifest["images"]}

    gltf, buffers = load_gltf(prop_dir / manifest["source"])
    material = first_material(gltf)
    pbr = material.get("pbrMetallicRoughness", {})
    row["roughness_factor"] = pbr.get("roughnessFactor", 1.0)
    row["metallic_factor"] = pbr.get("metallicFactor", 1.0)
    row["base_color_factor"] = tuple(pbr.get("baseColorFactor", [1.0, 1.0, 1.0, 1.0]))
    row["ao_bound"] = "occlusionTexture" in material

    world_area, uv_area = mesh_areas(gltf, buffers)
    row["world_area_m2"] = world_area
    atlas_slot = next((s for s in ("base_color", "normal", "metallic_roughness", "occlusion") if s in slots), None)
    mask = None
    if atlas_slot:
        w, h = Image.open(slots[atlas_slot]).size
        row["atlas_w"], row["atlas_h"] = w, h
        mask = island_mask(gltf, buffers, w, h)
        row["island_frac"] = float(mask.mean())
        if world_area:
            row["atlas_px_per_m"] = (uv_area * w * h / world_area) ** 0.5
            scale = scales.get(prop_dir.name)
            if scale:
                row["placed_px_per_m"] = row["atlas_px_per_m"] / scale

    if "metallic_roughness" in slots:
        row.update(arm_stats(slots["metallic_roughness"], mask))
    else:
        row["roughness_mean"] = row["roughness_factor"]
        row["metallic_mean"] = row["metallic_factor"]
        row["roughness_std"] = 0.0

    if "occlusion" in slots:
        row.update(ao_stats(slots["occlusion"], mask))

    luma = None
    if "base_color" in slots:
        stats, luma = albedo_stats(slots["base_color"], mask)
        row.update(stats)

    if "normal" in slots:
        row["normal_lap_std"], row["normal_flat_frac"] = normal_stats(slots["normal"], mask)
        if luma is not None:
            row["baked_fraction_ts"] = baked_fraction_ts(luma, rgb_of(slots["normal"]))

    row["blend_coverage"], row["hole_frac"] = generation_stats(prop_dir)
    return row


# --- table / CLI -------------------------------------------------------

def format_cell(field, value):
    if value is None:
        return "-"  # ASCII, not an em dash -- Windows stdout defaults to cp1252
    if field == "ao_bound":
        return "true" if value else "false"
    if field in ("atlas_w", "atlas_h"):
        return str(int(value))
    if field == "base_color_factor":
        return ",".join(f"{v:.2f}" for v in value)
    return f"{value:.{PRECISION[field]}f}"


def print_table(rows):
    ordered = sorted(rows, key=lambda r: (not r["reference"], r["prop"]))
    labels = [r["prop"] + (" [REF]" if r["reference"] else "") for r in ordered]
    name_width = max([20] + [len(l) for l in labels])
    formatted = [{f: format_cell(f, r[f]) for f in FIELDS} for r in ordered]
    widths = {f: max([len(f)] + [len(row[f]) for row in formatted]) for f in FIELDS}

    print(f"{'prop':<{name_width}} " + " ".join(f"{f:>{widths[f]}}" for f in FIELDS))
    for label, row, raw in zip(labels, formatted, ordered):
        print(f"{label:<{name_width}} " + " ".join(f"{row[f]:>{widths[f]}}" for f in FIELDS))
        if raw["note"]:
            print(f"  ! {raw['prop']}: {raw['note']}")


def resolve_asset(arg, prop_dirs):
    candidate = Path(arg)
    if candidate.is_dir():
        return candidate.resolve()
    if arg in prop_dirs:
        return prop_dirs[arg]
    print(f"error: --asset '{arg}' is not a prop name or directory under {PROPS_DIR}", file=sys.stderr)
    sys.exit(1)


def main():
    parser = argparse.ArgumentParser(description="Measurement-only prop metric audit (no thresholds).")
    parser.add_argument("--asset", help="measure a single prop (name under content/models/props, or a path)")
    parser.add_argument("--json", action="store_true", help="also print a machine-readable JSON dump")
    args = parser.parse_args()

    scales = placement_scales()
    prop_dirs = {p.name: p for p in sorted(PROPS_DIR.iterdir()) if p.is_dir()}

    targets = dict(prop_dirs)
    if args.asset:
        resolved = resolve_asset(args.asset, prop_dirs)
        targets = {resolved.name: resolved}
        if REFERENCE_PROP in prop_dirs:
            targets.setdefault(REFERENCE_PROP, prop_dirs[REFERENCE_PROP])

    rows = [measure_prop(prop_dir, scales) for prop_dir in targets.values()]
    print_table(rows)
    if args.json:
        print()
        print(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
