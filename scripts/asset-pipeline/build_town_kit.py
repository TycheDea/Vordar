# Blender-headless: generate the Rocalba town-kit buildings procedurally.
#
# One glTF-separate export (.gltf + .bin) per building type (campaign
# decision D7), all types referencing one shared <out>/textures/ set -- the
# repo carries a single copy of each material family's maps instead of nine
# embedded ones (VQ-B5). Cites docs/town-premise.md
# S3 (six-material closed vocabulary) and S5/S6 (building register, chapel
# spec). Geometry lives in townkit/{geo,buildings}.py; townkit/materials.py
# derives the tiling UV scale from VQ-A3's texel-density figure and loads
# baked materials from --materials-dir when present, else flat placeholder
# colors from the premise palette. townkit/verify.py re-imports each export
# into a fresh scene to check material names, the vordar_detail extra, UVs
# and loose/boundary geometry; townkit/render.py renders one silhouette
# preview PNG per type.
#
# Usage: blender --background --python build_town_kit.py -- \
#            --types all --materials-dir <dir> --out <dir>

import argparse
import hashlib
import json
import sys
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import bpy  # noqa: E402

from townkit import buildings  # noqa: E402
from townkit import materials as matlib  # noqa: E402
from townkit import render as renderlib  # noqa: E402
from townkit import verify as verifylib  # noqa: E402

ALL_TYPES = list(buildings.BUILDERS.keys())


def parse_args(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--types", default="all")
    parser.add_argument("--materials-dir", default=None)
    parser.add_argument("--out", required=True)
    return parser.parse_args(argv)


def clear_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def export_selected(objs, path):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    result = bpy.ops.export_scene.gltf(filepath=str(path), export_format="GLTF_SEPARATE",
                                        export_texture_dir="textures",
                                        export_yup=True, export_image_format="AUTO",
                                        export_extras=True, use_selection=True)
    if result != {"FINISHED"}:
        raise RuntimeError(f"export_scene.gltf({path}) returned {result}")


def check_texture_uris(gltf_path):
    """Every image must be an external file under the shared textures/ dir --
    an embedded (data:) or stray-path image would silently defeat the
    one-copy-on-disk layout."""
    doc = json.loads(gltf_path.read_text())
    uris = [img.get("uri", "") for img in doc.get("images", [])]
    bad = [u for u in uris if not u.startswith("textures/")]
    if bad:
        raise RuntimeError(f"{gltf_path.name}: non-shared image URIs {bad}")
    missing = [u for u in uris if not (gltf_path.parent / u).is_file()]
    if missing:
        raise RuntimeError(f"{gltf_path.name}: dangling image URIs {missing}")
    return sorted(uris)


def assert_chapel_dims(dims):
    checks = []

    def check(label, val, lo, hi):
        ok = lo <= val <= hi
        checks.append({"label": label, "value": val, "lo": lo, "hi": hi, "ok": ok})
        if not ok:
            raise AssertionError(f"chapel {label}={val} outside [{lo}, {hi}]")

    check("nave_width", dims["nave_width"], 7.0 - 0.2, 7.0 + 0.2)
    check("nave_length", dims["nave_length"], 16.0 - 0.2, 16.0 + 0.2)
    check("vault_peak", dims["vault_peak"], 10.0, 12.0)
    check("door_width", dims["door_width"], 2.4 - 0.1, 2.4 + 0.1)
    check("door_height", dims["door_height"], 3.2 - 0.1, 3.2 + 0.1)
    # C1 — footprint guard: converts §5's invisible footprints.ron coupling
    # into a hard build-time failure. Any future feature that projects past
    # the east wall or the side walls trips this before it ever reaches the
    # 0.02 m D5 margin.
    check("footprint_x", dims["footprint_x"], 20.21, 20.25)
    check("footprint_y", dims["footprint_y"], 8.18, 8.22)
    check("espadana_apex", dims["espadana_apex"], 12.30, 12.50)
    check("overall_height", dims["overall_height"], 13.05, 13.25)
    return checks


def _json_default(o):
    if isinstance(o, set):
        return sorted(o)
    return str(o)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    args = parse_args(argv)
    # Resolved once: after a scene reset (read_factory_settings), Blender's
    # own relative-path base drifts from the process cwd, so every path
    # handed to a bpy.ops.* call downstream must already be absolute.
    out_dir = Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    if args.materials_dir is not None:
        args.materials_dir = str(Path(args.materials_dir).resolve())
    preview_dir = out_dir / "previews"
    preview_dir.mkdir(parents=True, exist_ok=True)

    types = ALL_TYPES if args.types == "all" else args.types.split(",")
    unknown = [t for t in types if t not in buildings.BUILDERS]
    if unknown:
        print(f"build_town_kit: unknown types {unknown}, choices are {ALL_TYPES}", file=sys.stderr)
        sys.exit(1)

    results = []
    tex_dir = out_dir / "textures"
    shared_hashes = {}
    # Staged canonical texture copies must outlive every export in the run
    # (the exporter reads source bytes at export time), then vanish -- the
    # shared set the repo keeps is only what the exporter writes to tex_dir.
    staging = tempfile.TemporaryDirectory(prefix="townkit-texsrc-")
    for t in types:
        clear_scene()
        mats, sources = matlib.build_materials(args.materials_dir, staging.name)
        objs, dims = buildings.BUILDERS[t](mats)
        for o in objs:
            # Roof deck panels (geo._roof_deck_panel) and any shell already
            # finalized pre-union (buildings.build_casa_shell / _casa_corner)
            # carry their own correct UV -- a blanket re-project here would
            # blow away the deck's per-slope planar mapping with the generic
            # box projection it was built specifically to avoid (G2 D1/D2/D5).
            if not o.get("vordar_uv_final"):
                matlib.project_uv(o)

        gltf_path = out_dir / f"{t}.gltf"
        export_selected(objs, gltf_path)
        size_bytes = gltf_path.stat().st_size + (out_dir / f"{t}.bin").stat().st_size
        texture_uris = check_texture_uris(gltf_path)
        # Exports overwrite into the shared textures/ dir; a rewrite with
        # different bytes would silently corrupt what earlier .gltf files
        # reference, so any already-seen URI must hash identically.
        for uri in texture_uris:
            digest = hashlib.sha256((out_dir / uri).read_bytes()).hexdigest()
            if shared_hashes.setdefault(uri, digest) != digest:
                raise RuntimeError(f"[{t}] rewrote shared texture {uri} with different bytes")

        vreport = verifylib.verify_export(gltf_path)
        preview_path = preview_dir / f"{t}.png"
        mean_px = renderlib.render_preview(preview_path)
        # Street-level angle: lower and closer, so it looks into gable ends
        # and along wall tops rather than down onto roofs -- open gables and
        # thin exposed rims can't hide behind the three-quarter view alone.
        preview_path_street = preview_dir / f"{t}_street.png"
        mean_px_street = renderlib.render_preview(preview_path_street, az_deg=110.0, el_deg=8.0,
                                                   dist_scale=2.0)

        chapel_checks = assert_chapel_dims(dims) if t == "chapel" else None

        entry = {
            "type": t,
            "tris": vreport["total_tris"],
            "size_bytes": size_bytes,
            "texture_uris": texture_uris,
            "material_sources": sources,
            "verify": vreport,
            "preview_mean": mean_px,
            "preview_path": str(preview_path),
            "preview_mean_street": mean_px_street,
            "preview_path_street": str(preview_path_street),
            "dims": dims,
            "chapel_checks": chapel_checks,
        }
        results.append(entry)
        print(f"[{t}] tris={vreport['total_tris']} size={size_bytes}B "
              f"materials_ok={not vreport['bad_material_names']} "
              f"uv_ok={not vreport['missing_uv']} loose_v={vreport['loose_verts']} "
              f"loose_e={vreport['loose_edges']} boundary_e={vreport['boundary_edges']} "
              f"normals_faults={len(vreport['normals_faults'])} "
              f"joint_gaps_bad={sum(1 for g in vreport['joint_gaps'] if not g['ok'])} "
              f"preview_mean={mean_px:.4f} preview_mean_street={mean_px_street:.4f}")

    staging.cleanup()
    if types == ALL_TYPES and tex_dir.is_dir():
        # A full rebuild owns the shared dir: an unreferenced file is a
        # stale leftover whose bytes would ride into installs as duplicates.
        referenced = {Path(u).name for u in shared_hashes}
        for p in tex_dir.iterdir():
            if p.is_file() and p.name not in referenced:
                p.unlink()
    summary_path = out_dir / "build_report.json"
    # A partial --types run rebuilds some .gltf files and leaves the rest on
    # disk untouched, so their entries stay true and must survive: overwriting
    # the report wholesale would report the kit as smaller than it is.
    merged = {}
    if summary_path.is_file():
        merged = {e["type"]: e for e in json.loads(summary_path.read_text())}
    merged.update({e["type"]: e for e in results})
    report = [merged[t] for t in ALL_TYPES if t in merged]
    with open(summary_path, "w") as f:
        json.dump(report, f, indent=2, default=_json_default)
    shared_bytes = sum((out_dir / u).stat().st_size for u in shared_hashes)
    print(json.dumps({"summary": str(summary_path), "texel_scale_m": matlib.TEXEL_SCALE_M,
                      "shared_texture_files": len(shared_hashes),
                      "shared_texture_bytes": shared_bytes}))


if __name__ == "__main__":
    main()
