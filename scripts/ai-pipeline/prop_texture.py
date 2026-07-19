# Blender-headless: texture a cleaned Hi3DGen prop mesh (Phase A3.6,
# strategy: Blender projection bake — ruling in tasks/ai-pipeline/a3.md).
#
#   - Smart-UV unwraps the decimated mesh into an atlas, then bakes the
#     concept image onto basecolor by planar projection along the concept
#     view axis (EMIT bake, deterministic, CPU). The projection passes
#     through the mesh, so back faces receive the mirrored front read —
#     the near-symmetric prop classes this pipeline targets survive that.
#   - normal map = real high-to-low Cycles bake from <hires.glb> onto the
#     atlas UVs (prop_cleanup.py keeps both meshes rigidly aligned).
#   - MR map = per-material-zone constants (A3.6 channels contract): zones
#     are classified per texel from baked basecolor value — dark texels =
#     metal (weathered iron), light = dielectric (wax/stone). No real MR
#     capture exists in this strategy; the constants are the contract.
#   - prints one JSON stats line (the only '{'-prefixed stdout line) for
#     gen_prop.py's chained manifest.
#
# Usage: blender --background --python prop_texture.py -- \
#            <clean.glb> <hires.glb> <concept.png> <textured.glb>

import argparse
import json
import sys
import tempfile
import time
import traceback
from math import radians
from pathlib import Path

import bpy
import numpy as np

TEXTURE_SIZE = 1024
# Blender-world direction the concept camera looks FROM. Hi3DGen +
# prop_cleanup emit the concept view on the glTF +Z face, which the
# Blender glTF importer maps to -Y (verified on the A3.4 smoke mesh:
# the +Y projection renders mirrored against the concept).
FRONT_AXIS = "-Y"
# Material-zone constants (A3.6 channels contract, candelabra fixture):
# texels darker than the band are weathered iron, lighter are wax/stone.
# sRGB-value smoothstep band avoids hard speckle at the zone boundary.
METAL_VALUE_BAND = (0.24, 0.34)
METAL_ROUGHNESS = 0.4
DIELECTRIC_ROUGHNESS = 0.7


def fail(msg):
    print(f"prop_texture: {msg}", file=sys.stderr)
    sys.exit(1)


def import_glb(path):
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.gltf(filepath=str(path))
    new = [o for o in set(bpy.context.scene.objects) - before if o.type == "MESH"]
    if not new:
        fail(f"no mesh in {path}")
    for o in bpy.context.scene.objects:
        o.select_set(o in new)
    bpy.context.view_layer.objects.active = new[0]
    if len(new) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    world = obj.matrix_world.copy()
    obj.parent = None
    obj.matrix_world = world
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    return obj


def select_only(objs, active):
    for o in bpy.context.scene.objects:
        o.select_set(o in objs)
    bpy.context.view_layer.objects.active = active


def concept_stats(img):
    """Alpha bbox (uv space) + mean opaque color of the concept image."""
    w, h = img.size
    px = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(px)
    px = px.reshape(h, w, 4)  # row 0 = image bottom in Blender
    opaque = px[:, :, 3] > 0.1
    if not opaque.any():
        fail("concept image has no opaque pixels (alpha everywhere)")
    ys, xs = np.where(opaque)
    u0, u1 = xs.min() / w, (xs.max() + 1) / w
    v0, v1 = ys.min() / h, (ys.max() + 1) / h
    mean = px[opaque][:, :3].mean(axis=0)
    return (u0, u1, v0, v1), tuple(float(c) for c in mean)


def project_uvs(me, bbox_uv):
    """Planar-project loop UVs along FRONT_AXIS, fitting the mesh's world
    XZ extent to the concept's alpha bbox."""
    u0, u1, v0, v1 = bbox_uv
    nv, nl = len(me.vertices), len(me.loops)
    co = np.empty(nv * 3, dtype=np.float32)
    me.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    vidx = np.zeros(nl, dtype=np.int32)
    me.loops.foreach_get("vertex_index", vidx)
    x, z = co[vidx, 0], co[vidx, 2]
    tx = (x - x.min()) / max(x.max() - x.min(), 1e-9)
    tz = (z - z.min()) / max(z.max() - z.min(), 1e-9)
    if FRONT_AXIS == "+Y":  # screen right = -X when viewed from +Y
        tx = 1.0 - tx
    layer = me.uv_layers.new(name="proj")
    uv = np.empty((nl, 2), dtype=np.float32)
    uv[:, 0] = u0 + tx * (u1 - u0)
    uv[:, 1] = v0 + tz * (v1 - v0)
    layer.uv.foreach_set("vector", uv.ravel())
    return layer


def new_image(name, srgb, fill):
    img = bpy.data.images.new(name, TEXTURE_SIZE, TEXTURE_SIZE, alpha=False)
    img.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
    img.generated_color = (*fill, 1.0)
    return img


def bake_material(obj):
    """Fresh single-material node tree on obj; returns the tree."""
    mat = bpy.data.materials.new("bake")
    mat.use_nodes = True
    mat.node_tree.nodes.clear()
    obj.data.materials.clear()
    obj.data.materials.append(mat)
    return mat.node_tree


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="prop_texture.py")
    parser.add_argument("clean_glb")
    parser.add_argument("hires_glb")
    parser.add_argument("concept_png")
    parser.add_argument("textured_glb")
    args = parser.parse_args(argv)

    t0 = time.time()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    clean = import_glb(args.clean_glb)
    hires = import_glb(args.hires_glb)

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 1

    # ---- atlas unwrap (the mesh arrives without UVs) ----
    select_only([clean], clean)
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=radians(66), island_margin=0.003)
    bpy.ops.object.mode_set(mode="OBJECT")
    me = clean.data
    atlas = me.uv_layers[0].name

    concept = bpy.data.images.load(str(Path(args.concept_png).resolve()))
    bbox_uv, mean_color = concept_stats(concept)
    project_uvs(me, bbox_uv)

    # ---- basecolor: EMIT-bake the projected concept into the atlas ----
    # (emission bake copies colors exactly — no lighting, 1 sample)
    base_img = new_image("prop_base", srgb=True, fill=mean_color)
    tree = bake_material(clean)
    n_uv = tree.nodes.new("ShaderNodeUVMap")
    n_uv.uv_map = "proj"
    n_tex = tree.nodes.new("ShaderNodeTexImage")
    n_tex.image = concept
    n_tex.extension = "CLIP"
    # transparent concept pixels fall back to the mean opaque color so
    # occluded texels read as material, not black
    n_mix = tree.nodes.new("ShaderNodeMixRGB")
    n_mix.inputs["Color1"].default_value = (*mean_color, 1.0)
    n_emit = tree.nodes.new("ShaderNodeEmission")
    n_out = tree.nodes.new("ShaderNodeOutputMaterial")
    n_bake = tree.nodes.new("ShaderNodeTexImage")
    n_bake.image = base_img
    tree.links.new(n_uv.outputs["UV"], n_tex.inputs["Vector"])
    tree.links.new(n_tex.outputs["Color"], n_mix.inputs["Color2"])
    tree.links.new(n_tex.outputs["Alpha"], n_mix.inputs["Fac"])
    tree.links.new(n_mix.outputs["Color"], n_emit.inputs["Color"])
    tree.links.new(n_emit.outputs["Emission"], n_out.inputs["Surface"])
    tree.nodes.active = n_bake
    select_only([clean], clean)
    bpy.ops.object.bake(type="EMIT", margin=8, use_clear=True, uv_layer=atlas)
    t_base = time.time()

    me.uv_layers.remove(me.uv_layers["proj"])

    # ---- normal: real high-to-low bake from the hires mesh ----
    normal_img = new_image("prop_normal", srgb=False, fill=(0.5, 0.5, 1.0))
    n_bake.image = normal_img
    select_only([clean, hires], clean)
    bpy.ops.object.bake(type="NORMAL", normal_space="TANGENT",
                        use_selected_to_active=True, cage_extrusion=0.01,
                        max_ray_distance=0.03, margin=8, use_clear=True,
                        uv_layer=atlas)
    t_normal = time.time()

    # ---- MR: zone constants keyed off the baked basecolor's value ----
    px = np.empty(TEXTURE_SIZE * TEXTURE_SIZE * 4, dtype=np.float32)
    base_img.pixels.foreach_get(px)
    px = px.reshape(-1, 4)
    # float buffers are scene-linear; threshold in sRGB-ish value space
    lum = (0.2126 * px[:, 0] + 0.7152 * px[:, 1] + 0.0722 * px[:, 2])
    value = np.clip(lum, 0.0, 1.0) ** (1.0 / 2.2)
    lo, hi = METAL_VALUE_BAND
    t = np.clip((value - lo) / (hi - lo), 0.0, 1.0)
    metallic = 1.0 - t * t * (3.0 - 2.0 * t)  # smoothstep: dark = metal
    rough = METAL_ROUGHNESS * metallic + DIELECTRIC_ROUGHNESS * (1 - metallic)
    mr = np.zeros((TEXTURE_SIZE * TEXTURE_SIZE, 4), dtype=np.float32)
    mr[:, 1], mr[:, 2], mr[:, 3] = rough, metallic, 1.0
    mr_img = new_image("prop_mr", srgb=False, fill=(0, 0, 0))
    mr_img.pixels.foreach_set(mr.ravel())
    metal_fraction = float((metallic > 0.5).mean())

    # ---- final material + export (hires dropped, images saved so the
    #      glTF exporter embeds them) ----
    tmpdir = tempfile.TemporaryDirectory()
    for img, name in ((base_img, "base.png"), (mr_img, "mr.png"),
                      (normal_img, "normal.png")):
        img.filepath_raw = str(Path(tmpdir.name) / name)
        img.file_format = "PNG"
        img.save()

    mat = bpy.data.materials.new("prop")
    mat.use_nodes = True
    tree = mat.node_tree
    bsdf = tree.nodes["Principled BSDF"]
    n_base = tree.nodes.new("ShaderNodeTexImage")
    n_base.image = base_img
    tree.links.new(n_base.outputs["Color"], bsdf.inputs["Base Color"])
    n_mr = tree.nodes.new("ShaderNodeTexImage")
    n_mr.image = mr_img
    n_sep = tree.nodes.new("ShaderNodeSeparateColor")
    tree.links.new(n_mr.outputs["Color"], n_sep.inputs["Color"])
    tree.links.new(n_sep.outputs["Green"], bsdf.inputs["Roughness"])
    tree.links.new(n_sep.outputs["Blue"], bsdf.inputs["Metallic"])
    n_nrm = tree.nodes.new("ShaderNodeTexImage")
    n_nrm.image = normal_img
    n_nmap = tree.nodes.new("ShaderNodeNormalMap")
    tree.links.new(n_nrm.outputs["Color"], n_nmap.inputs["Color"])
    tree.links.new(n_nmap.outputs["Normal"], bsdf.inputs["Normal"])
    me.materials.clear()
    me.materials.append(mat)

    bpy.data.objects.remove(hires, do_unlink=True)
    select_only([clean], clean)
    bpy.ops.export_scene.gltf(filepath=str(Path(args.textured_glb).resolve()),
                              export_format="GLB", export_yup=True,
                              export_image_format="AUTO")
    tmpdir.cleanup()

    stats = {
        "strategy": "blender_projection_bake",
        "front_axis": FRONT_AXIS,
        "texture_size": TEXTURE_SIZE,
        "concept_alpha_bbox_uv": [round(v, 4) for v in bbox_uv],
        "fill_color": [round(c, 4) for c in mean_color],
        "metal_value_band": list(METAL_VALUE_BAND),
        "metal_roughness": METAL_ROUGHNESS,
        "dielectric_roughness": DIELECTRIC_ROUGHNESS,
        "metal_fraction": round(metal_fraction, 4),
        "base_bake_s": round(t_base - t0, 1),
        "normal_bake_s": round(t_normal - t_base, 1),
        "textured_glb": args.textured_glb,
    }
    print(json.dumps(stats))


try:
    main()
except SystemExit:
    raise
except Exception:
    # without --python-exit-code Blender exits 0 on an uncaught script
    # exception — route every failure through an explicit non-zero exit
    traceback.print_exc()
    sys.exit(1)
