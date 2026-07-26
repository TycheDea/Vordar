# Blender-headless: texture a cleaned Hi3DGen prop mesh.
#
# Every bake targets the clean mesh's UV atlas, unwrapped once by
# prop_cleanup.py (xatlas) — this stage fails if the mesh arrives without
# UVs, so the atlas is identical across texture re-runs of a candidate.
#
# Channels:
#   - basecolor (multiview retexture): ortho depth renders of the clean mesh
#     from the resolved per-view camera rig feed a ControlNet-depth
#     text-to-image base via a ComfyUI server whose lifecycle lives entirely
#     inside this stage.
#     Opposite views are tiled side by side into ONE conditioning canvas per
#     sampling pass (Paint3D's front+back grid: the model attends to both
#     views at once, so they come out consistent), with silhouettes
#     dilated a few px so it paints material past the true edge; the
#     decoded canvas is split back into per-view images, reprojected into
#     the atlas and blended with facing weights and depth-occlusion
#     tests; island texels no view covered are Telea-inpainted. Coverage
#     is purely geometric, so before generating anything the stage
#     greedily adds up to MV_EXTRA_MAX extra views (azimuth/elevation
#     candidate grid, Text2Tex-style next-best-view) whenever one would
#     newly cover >= MV_EXTRA_MIN_GAIN of the island.
#   - normal map = real high-to-low Cycles bake from <hires.glb> onto the
#     atlas UVs (prop_cleanup.py keeps both meshes rigidly aligned).
#   - occlusion map = Cycles AO bake, same selected-to-active rig as the
#     normal bake, authored as the glTF occlusionTexture via a "glTF
#     Material Output" node group (the exporter's Occlusion-socket
#     contract) so it ships separately from basecolor/normal/MR.
#   - MR: two contract-declared constants (metallic/roughness) carried by
#     the glTF scalar factors, never classified from the basecolor: luma
#     conflates albedo, shading and material.
#   - prints one JSON stats line (the only '{'-prefixed stdout line) for
#     gen_prop.py's chained manifest.
#
# Usage: blender --background --python prop_texture.py -- \
#            <clean.glb> <hires.glb> <textured.glb> \
#            --asset NAME --seed N

import argparse
import json
import shutil
import sys
import traceback
from contextlib import ExitStack
from pathlib import Path

import bpy
import numpy as np

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
# The stage modules are imported as modules as well as by name: a cache unit
# names its stage module as the entry point of the source set whose sha256 is
# that stage's version, and only this file — never a stage module — reaches
# for the cache, so cache.py stays out of every stage's import closure.
import comfy_run  # noqa: E402
import proptex.albedo  # noqa: E402
import proptex.atlas  # noqa: E402
import proptex.coverage  # noqa: E402
import proptex.export  # noqa: E402
import proptex.generate  # noqa: E402
import proptex.views  # noqa: E402
from proptex.albedo import (  # noqa: E402
    PROP_PBR, blend_views, estimate_albedo, estimator_models, needs_estimator,
    numpy_cv2_id, torch_id,
)
from proptex.atlas import bake_geometry_atlas  # noqa: E402
from proptex.cache import cached, hits, outputs_of, sha256_file  # noqa: E402
from proptex.coverage import extra_candidates, pick_extra_views, pick_indices  # noqa: E402
from proptex.export import bake_ao_map, bake_normal_map, export_prop  # noqa: E402
from proptex.generate import (  # noqa: E402
    canvas_outputs, render_canvas, view_pairs, workflow_inputs,
)
from proptex.provenance import chain  # noqa: E402
from proptex.registry import resolve  # noqa: E402  (stdlib-only, safe under Blender's Python)
from proptex.scene import blender_id, import_glb  # noqa: E402
from proptex.views import (  # noqa: E402
    MV_ELEVATION_DEG, depth_setup, mv_camera_rig, normal_setup, read_depth,
    render_depth_view, render_normal_view, view_hint,
)

def fail(msg):
    print(f"prop_texture: {msg}", file=sys.stderr)
    sys.exit(1)


def direction_unit(v):
    """Cache unit name of a per-view stage: the direction, never the view's
    index — the base set, the next-best-view candidate grid and the picked
    extras must land on the same entry whenever they name the same
    direction, which is what makes a picked candidate a hit."""
    return f"az{v['azimuth_deg']:g}_el{v['elevation_deg']:g}"


def direction_params(v, view_res, blender):
    """Cache params of a per-view stage. The angles are floats because the
    key is the canonical JSON of this dict: a declared azimuth of 0 and a
    candidate's 0.0 would otherwise hash to different keys."""
    return {"azimuth_deg": float(v["azimuth_deg"]),
            "elevation_deg": float(v["elevation_deg"]),
            "view_res": view_res, "blender": blender}


def depth_units(clean, views, rig, view_res, mesh_input, blender):
    """One `depth` cache unit per view direction. The shared Blender setup
    is entered unconditionally: a few shader nodes and a camera cost
    milliseconds, and the render is what a hit skips."""
    with depth_setup(clean, rig, view_res) as cam_obj:
        return [cached("depth", direction_unit(v), proptex.views,
                       direction_params(v, view_res, blender), mesh_input,
                       ("depth.exr", "depth.png"),
                       lambda out_dir, v=v: render_depth_view(cam_obj, v, out_dir))
                for v in views]


def normal_units(clean, views, depths, units, rig, view_res, mesh_input, blender):
    """One `normal_view` cache unit per view direction: the estimator's
    camera-space normal and object mask, which only `albedo_source: delit`
    has a consumer for."""
    with normal_setup(clean, rig, view_res) as cam_obj:
        return [cached("normal_view", direction_unit(v), proptex.views,
                       direction_params(v, view_res, blender),
                       {**mesh_input, **outputs_of(unit, "depth.exr")},
                       ("normal.png", "mask.png"),
                       lambda out_dir, v=v, d=depth: render_normal_view(cam_obj, v, d, out_dir))
                for v, depth, unit in zip(views, depths, units)]


def geometry_stages(clean, atlas, rig, views, mesh_input, contract):
    """The content-addressed geometry chain: the atlas bake, one depth
    render per base direction, one per surviving next-best-view candidate,
    the pick itself, and the picked extras appended to the view list — their
    depths already rendered as candidates, so a pick costs no render.
    Returns the grown view list, its depth arrays and depth cache units, the
    atlas unit and its arrays, and every unit in chain order."""
    view_res = contract.view_res
    blender = blender_id()

    atlas_unit = cached(
        "atlas", None, proptex.atlas,
        {"texture_size": contract.texture_size, "blender": blender},
        mesh_input, ("pos.npy", "nrm.npy", "island.npy"),
        lambda out_dir: bake_geometry_atlas(clean, atlas, rig,
                                            contract.texture_size, out_dir))
    pos, nrm, island = (np.load(atlas_unit.dir / name)
                        for name in ("pos.npy", "nrm.npy", "island.npy"))

    units = depth_units(clean, views, rig, view_res, mesh_input, blender)
    depths = [read_depth(u.dir / "depth.exr") for u in units]
    cands = extra_candidates(views, rig)
    cand_units = depth_units(clean, [cand for _, cand in cands], rig, view_res,
                             mesh_input, blender)

    def pick(out_dir):
        meta = pick_extra_views(
            views, depths, cands,
            (read_depth(u.dir / "depth.exr") for u in cand_units),
            rig, pos, nrm, island)
        (out_dir / "extras.json").write_text(json.dumps(meta, indent=2), encoding="utf-8")

    nbv_inputs = {**mesh_input, **atlas_unit.record["outputs"]}
    for u in units + cand_units:
        nbv_inputs.update(outputs_of(u, "depth.exr"))
    nbv_unit = cached(
        "nbv", None, proptex.coverage,
        {"azimuths": contract.azimuths, "elevation_deg": MV_ELEVATION_DEG,
         "blender": blender},
        nbv_inputs, ("extras.json",), pick)

    extra_meta = json.loads((nbv_unit.dir / "extras.json").read_text(encoding="utf-8"))
    picked = pick_indices(cands, extra_meta)
    return (views + [cands[j][1] for j in picked],
            depths + [read_depth(cand_units[j].dir / "depth.exr") for j in picked],
            units + [cand_units[j] for j in picked],
            atlas_unit, (pos, nrm, island),
            [atlas_unit] + units + cand_units + [nbv_unit])


def generate_units(views, units, subject, seed, view_res, n_extra, comfy):
    """One `generate` cache unit per conditioning canvas, plus each view's
    (unit, filename) albedo source in view order. The ComfyUI server is
    opened only when some canvas misses: it costs a minute and ~10 GiB of
    VRAM, unlike the millisecond Blender setups the per-view stages enter
    unconditionally."""
    pairs = view_pairs(len(views), n_extra)
    graph = workflow_inputs()
    calls = []
    for k, pair in enumerate(pairs):
        params = {"subject": subject,
                  "view_hints": [views[i]["hint"] for i in pair],
                  "canvas_seed": seed * 100 + k,
                  "width": view_res * len(pair), "height": view_res,
                  "comfy": comfy}
        inputs = dict(graph)
        for i in pair:
            inputs.update(outputs_of(units[i], "depth.png"))
        calls.append((k, pair, params, inputs))

    with ExitStack() as stack:
        if any(not hits("generate", proptex.generate, params, inputs)
               for _, _, params, inputs in calls):
            stack.enter_context(comfy_run.server())
        canvases = [
            cached("generate", f"canvas_{k}", proptex.generate, params, inputs,
                   canvas_outputs(pair),
                   lambda out_dir, pair=pair, params=params: render_canvas(
                       views, pair, [units[i].dir / "depth.png" for i in pair],
                       params["canvas_seed"], view_res, subject, out_dir))
            for k, pair, params, inputs in calls]

    sources = [None] * len(views)
    for unit, (_, pair, _, _) in zip(canvases, calls):
        for slot, i in enumerate(pair):
            sources[i] = (unit, f"gen_{slot}.png")
    return canvases, sources


def estimate_units(sources, normals, seed, torch):
    """One `estimate` cache unit per view: MaterialAnything's delit
    decomposition of that view's lit generation. Its source set names
    prop_pbr.py explicitly — the subprocess launch is the one edge in the
    pipeline that no import expresses."""
    models = estimator_models()
    return [cached("estimate", f"view_{i}", proptex.albedo,
                   {"view_seed": seed * 1000 + i, "torch": torch},
                   {**outputs_of(gen_unit, gen_name),
                    **outputs_of(nrm, "normal.png", "mask.png"), **models},
                   ("albedo.png",),
                   lambda out_dir, g=gen_unit.dir / gen_name, n=nrm.dir,
                   s=seed * 1000 + i: estimate_albedo(
                       g, n / "normal.png", n / "mask.png", s, out_dir),
                   extra_files=(PROP_PBR,))
            for i, ((gen_unit, gen_name), nrm) in enumerate(zip(sources, normals))]


def basecolor_stages(clean, clean_glb_path, atlas, seed, contract):
    """The content-addressed basecolor chain: geometry, the delit
    conditioning renders and their per-view estimates when the surface class
    needs them, one ComfyUI canvas per view pair, and the facing-weighted
    blend into the atlas. Returns the blend unit and every unit in chain
    order."""
    mesh_input = {"clean.glb": sha256_file(clean_glb_path)}
    mv_specs = [(view_hint(a), a, MV_ELEVATION_DEG) for a in contract.azimuths]
    views, rig = mv_camera_rig(clean, mv_specs)
    views, depths, dunits, atlas_unit, (pos, nrm, island), units = geometry_stages(
        clean, atlas, rig, views, mesh_input, contract)

    delit = needs_estimator(contract.albedo_source)
    normals = (normal_units(clean, views, depths, dunits, rig, contract.view_res,
                            mesh_input, blender_id()) if delit else [])
    units += normals
    canvases, sources = generate_units(views, dunits, contract.subject, seed,
                                       contract.view_res, len(views) - len(mv_specs),
                                       comfy_run.comfy_id())
    units += canvases
    if delit:
        estimates = estimate_units(sources, normals, seed, torch_id())
        units += estimates
        sources = [(u, "albedo.png") for u in estimates]

    blend_inputs = {**mesh_input, **atlas_unit.record["outputs"]}
    for u in dunits:
        blend_inputs.update(outputs_of(u, "depth.exr"))
    for unit, name in sources:
        blend_inputs.update(outputs_of(unit, name))
    blend_unit = cached(
        "blend", None, proptex.albedo,
        {"albedo_source": contract.albedo_source,
         "views": [{"azimuth_deg": float(v["azimuth_deg"]),
                    "elevation_deg": float(v["elevation_deg"])} for v in views],
         "numpy_cv2": numpy_cv2_id()},
        blend_inputs, ("base.png", "coverage.json"),
        lambda out_dir: blend_views(views, [u.dir / name for u, name in sources],
                                    depths, rig, pos, nrm, island, out_dir))
    return blend_unit, units + [blend_unit]


# ---------------------------------------------------------------------------

def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="prop_texture.py")
    parser.add_argument("clean_glb")
    parser.add_argument("hires_glb")
    parser.add_argument("textured_glb")
    parser.add_argument("--asset", required=True,
                        help="Registered asset name (content/models/assets.json); resolves "
                             "the surface-class contract (proptex.registry) that decides "
                             "whether the material estimator runs")
    parser.add_argument("--seed", type=int, required=True, help="Base seed for the multiview passes")
    args = parser.parse_args(argv)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    clean = import_glb(args.clean_glb)
    hires = import_glb(args.hires_glb)

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 1
    scene.cycles.use_denoising = False

    me = clean.data
    if not me.uv_layers:
        fail("clean mesh carries no UV atlas — re-run prop_cleanup")
    atlas = me.uv_layers[0].name

    contract = resolve(args.asset)
    blend_unit, units = basecolor_stages(clean, args.clean_glb, atlas, args.seed, contract)

    blender = blender_id()
    bake_params = {"texture_size": contract.texture_size, "blender": blender}
    bake_inputs = {"clean.glb": sha256_file(args.clean_glb),
                   "clean_hires.glb": sha256_file(args.hires_glb)}
    normal_unit = cached(
        "bake_normal", None, proptex.export, bake_params, bake_inputs, ("normal.png",),
        lambda out_dir: bake_normal_map(clean, hires, atlas,
                                        contract.texture_size, out_dir))
    ao_unit = cached(
        "bake_ao", None, proptex.export, bake_params, bake_inputs, ("occlusion.png",),
        lambda out_dir: bake_ao_map(clean, hires, atlas,
                                    contract.texture_size, out_dir))
    export_unit = cached(
        "export", None, proptex.export,
        {"metallic": contract.metallic, "roughness": contract.roughness,
         "detail": contract.detail, "blender": blender},
        {"clean.glb": sha256_file(args.clean_glb),
         **outputs_of(blend_unit, "base.png"),
         **outputs_of(normal_unit, "normal.png"),
         **outputs_of(ao_unit, "occlusion.png")},
        ("textured.glb",),
        lambda out_dir: export_prop(clean, blend_unit.dir / "base.png",
                                    normal_unit.dir / "normal.png",
                                    ao_unit.dir / "occlusion.png", contract, out_dir))
    shutil.copyfile(export_unit.dir / "textured.glb", args.textured_glb)

    print(json.dumps(chain(units + [normal_unit, ao_unit, export_unit])))


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:
        # without --python-exit-code Blender exits 0 on an uncaught script
        # exception -- route every failure through an explicit non-zero exit
        traceback.print_exc()
        sys.exit(1)
