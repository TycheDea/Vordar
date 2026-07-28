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
# Plain system Python: numpy + Pillow only. PIL 12.3 decodes BC7 and BC5
# .dds directly, so no texconv round-trip. glTF/GLB accessors are read with
# pure struct, and the normal-map Laplacian is hand-rolled numpy --
# deliberately not cv2, which lives inside Blender's bundled interpreter and
# is unavailable here.
import argparse
import json
import re
import struct
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

REPO_ROOT = Path(__file__).resolve().parents[2]
PROPS_DIR = REPO_ROOT / "content/models/props"
ZONES_RON = REPO_ROOT / "content/zones/zones.ron"
ASSETS_JSON = REPO_ROOT / "content/models/assets.json"
HOLES_DIR = REPO_ROOT / "target/prop-coverage"
COVERAGE_JSON = HOLES_DIR / "coverage.json"
REFERENCE_PROP = "rock_face_01"

# manifest slot string -> MaterialData field name (content_lint.rs:259-265),
# so a later promotion into that test can reuse these keys unchanged.
SLOT_NAMES = {"base": "base_color", "normal": "normal", "mr": "metallic_roughness", "ao": "occlusion"}

FIELDS = [
    "roughness_mean", "roughness_std", "metallic_mean",
    "ao_mean", "ao_bound",
    "albedo_luma_p1", "albedo_luma_p50", "albedo_luma_p99", "albedo_blown_frac", "albedo_sat",
    "normal_lap_std", "normal_flat_frac",
    "island_frac",
    "shipped_height_m",
    "atlas_px_per_m", "placed_px_per_m", "world_area_m2", "atlas_w", "atlas_h",
    "roughness_factor", "metallic_factor", "base_color_factor",
    "blend_coverage", "hole_frac",
    "baked_fraction_ts",
]
PRECISION = {
    "roughness_mean": 3, "roughness_std": 3, "metallic_mean": 3, "ao_mean": 3,
    "albedo_luma_p1": 3, "albedo_luma_p50": 3, "albedo_luma_p99": 3, "albedo_blown_frac": 3, "albedo_sat": 3,
    "normal_lap_std": 3, "normal_flat_frac": 3, "island_frac": 3,
    "atlas_px_per_m": 1, "placed_px_per_m": 1, "world_area_m2": 2, "shipped_height_m": 3,
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


def mesh_height_m(gltf, buffers):
    """World-space Y-extent (glTF's up axis via export_yup) across every
    mesh-bearing node, in the glb's own baked units -- the shipped size a
    prop actually renders at, independent of any registry field."""
    ys = np.concatenate([(pos * scale)[:, 1] for pos, _uv, _tris, scale in iter_prims(gltf, buffers)])
    if ys.size == 0:
        return None
    return float(ys.max() - ys.min())


def island_mask(gltf, buffers, w, h):
    """Atlas-resolution boolean mask, True where a TEXCOORD_0 triangle covers
    that texel. Rasterized directly from the glb's own UVs (no Blender), so
    audit stats can be restricted to surface texels instead of the whole
    atlas (F-2: off-island texels are mean-fill/inpaint, not surface data,
    and dilute every whole-atlas statistic toward however much of the atlas
    xatlas actually packed). glTF TEXCOORD_0 and PIL's array both put the
    origin at top-left, so V maps straight across -- no flip."""
    img = Image.new("L", (w, h), 0)
    draw = ImageDraw.Draw(img)
    for _pos, uv, tris, _scale in iter_prims(gltf, buffers):
        px = uv * np.array([w, h], dtype=np.float32)
        for tri in tris:
            draw.polygon([tuple(px[i]) for i in tri], fill=255)
    return np.asarray(img) > 0


def covered_mask(prop_name, island):
    """Albedo mask for a `kind: generated` prop: the rasterized UV island
    intersected with coverage.py's hole map covered (grey) texels -- island
    texels no multiview render reached are Telea-inpainted, and that filler
    runs brighter than genuine content on every shipped prop, so unmasked
    albedo stats flatter exactly the props with the most inpaint to hide.
    The hole map's own island (covered|hole) is not used on its own: it is
    baked through a Blender margin bake and so is a dilation of the true UV
    footprint, and its margin texels are dilated copies of chart-edge
    colour, not surface. Refuses (does not fall back to the island mask) on
    a missing map, a dimension mismatch, or the rasterized island landing
    <98% inside the hole map's island -- the containment relation that
    actually holds between a tight footprint and its dilation, and the
    cheap check that catches a wrong orientation."""
    path = HOLES_DIR / f"holes_{prop_name}.png"
    if not path.exists():
        print(f"error: {path} missing -- run: python scripts/ai-pipeline/prop_coverage_sweep.py "
              f"--asset {prop_name}", file=sys.stderr)
        sys.exit(1)
    holes = np.asarray(Image.open(path).convert("RGB")).astype(np.int16)
    if holes.shape[:2] != island.shape:
        print(f"error: {path} is {holes.shape[1]}x{holes.shape[0]}, atlas is "
              f"{island.shape[1]}x{island.shape[0]}", file=sys.stderr)
        sys.exit(1)
    covered = (np.abs(holes - 64) <= 8).all(axis=-1)
    holed = (holes[..., 0] >= 247) & (holes[..., 1] <= 8) & (holes[..., 2] <= 8)
    outside = float((island & ~(covered | holed)).sum()) / float(island.sum())
    if outside > 0.02:
        print(f"error: {path} island misses {outside:.1%} of the rasterized UV island "
              f"(must be >= 98% contained) -- wrong orientation?", file=sys.stderr)
        sys.exit(1)
    return island & covered


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


# --- baked-lighting instrument (A6.2) ---------------------------------------
#
# baked_fraction: best R^2 of luma ~ a + b*max(N.L, 0) over a hemisphere of
# light directions -- baked lighting is luminance that tracks surface
# orientation. A uniformly dark object with flat albedo scores ~0. Calibrated
# against a hard-sun/flat-white control (~0.33 measured; a pure Lambert 1.0
# is not reachable once shadow and occlusion are real) and against generated
# multiview albedo (0.006-0.018 measured).

def rgb_of(path):
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.float32) / 255.0


def luma_of(path):
    a = rgb_of(path)
    return 0.2126 * a[..., 0] + 0.7152 * a[..., 1] + 0.0722 * a[..., 2]


def _light_dirs(n_az=24, n_el=6):
    out = []
    for el in np.linspace(0.0, np.pi / 2 * 0.95, n_el):
        for az in np.linspace(0, 2 * np.pi, n_az, endpoint=False):
            out.append([np.cos(el) * np.cos(az), np.cos(el) * np.sin(az), np.sin(el)])
    return np.array(out, dtype=np.float32)


def baked_fraction(luma, normals, mask):
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
    rgb = rgb_of(path)[mask]
    m = luma[mask]
    mx, mn = rgb.max(axis=-1), rgb.min(axis=-1)
    sat = np.divide(mx - mn, mx, out=np.zeros_like(mx), where=mx > 0)
    stats = {
        "albedo_luma_p1": float(np.percentile(m, 1)),
        "albedo_luma_p50": float(np.percentile(m, 50)),
        "albedo_luma_p99": float(np.percentile(m, 99)),
        "albedo_blown_frac": float((m > 0.9).mean()),
        "albedo_sat": float(sat.mean()),
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
    """baked_fraction (below) applied in tangent space off the
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


def generation_stats(prop_name, asset):
    """(blend_coverage, hole_frac) for a `kind: generated` prop, from
    prop_coverage_sweep.py's target/prop-coverage/coverage.json -- the same
    coverage computation covered_mask's albedo routing depends on, so this
    table's blend_coverage and its albedo mask cannot silently disagree. A
    `kind: downloaded` prop was never generated and truthfully has no
    coverage."""
    if asset.get("kind") != "generated":
        return None, None
    coverage = json.loads(COVERAGE_JSON.read_text(encoding="utf-8")) if COVERAGE_JSON.exists() else {}
    entry = coverage.get(prop_name)
    if entry is None:
        print(f"error: {COVERAGE_JSON} has no entry for {prop_name!r} -- run: "
              f"python scripts/ai-pipeline/prop_coverage_sweep.py --asset {prop_name}", file=sys.stderr)
        sys.exit(1)
    stats = entry["coverage"]
    return stats["blend_coverage"], stats["hole_texels"] / asset["texture_size"] ** 2


# --- per-prop assembly --------------------------------------------------

def measure_prop(prop_dir, scales, assets):
    asset = assets.get(prop_dir.name, {})
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
    if asset.get("kind") == "generated":
        target_height = asset.get("height_m")
        if target_height is None:
            print(f"error: asset {prop_dir.name!r} is kind=generated but has no height_m in {ASSETS_JSON}", file=sys.stderr)
            sys.exit(1)
        shipped_height = mesh_height_m(gltf, buffers)
        row["shipped_height_m"] = shipped_height
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
        albedo_mask = covered_mask(prop_dir.name, mask) if asset.get("kind") == "generated" else mask
        stats, luma = albedo_stats(slots["base_color"], albedo_mask)
        row.update(stats)

    if "normal" in slots:
        row["normal_lap_std"], row["normal_flat_frac"] = normal_stats(slots["normal"], mask)
        if luma is not None:
            row["baked_fraction_ts"] = baked_fraction_ts(luma, rgb_of(slots["normal"]))

    row["blend_coverage"], row["hole_frac"] = generation_stats(prop_dir.name, asset)
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
    assets = json.loads(ASSETS_JSON.read_text(encoding="utf-8"))
    prop_dirs = {p.name: p for p in sorted(PROPS_DIR.iterdir()) if p.is_dir()}

    targets = dict(prop_dirs)
    if args.asset:
        resolved = resolve_asset(args.asset, prop_dirs)
        targets = {resolved.name: resolved}
        if REFERENCE_PROP in prop_dirs:
            targets.setdefault(REFERENCE_PROP, prop_dirs[REFERENCE_PROP])

    rows = [measure_prop(prop_dir, scales, assets) for prop_dir in targets.values()]
    print_table(rows)
    if args.json:
        print()
        print(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
