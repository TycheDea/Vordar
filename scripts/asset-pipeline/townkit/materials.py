"""Rocalba town-kit materials: the six-material closed vocabulary from
docs/town-premise.md S3, as Principled BSDF Blender materials.

Texel-scale derivation (docs/visual-quality.md VQ-A3): installed props carry
a ~6.4 mm/texel macro atlas. A tiling wall material at 2048x2048 lands in the
same perceived band when one full UV tile covers
    2048 px * 0.0064 m/px = 13.1072 m
of wall, so TEXEL_SCALE_M is the `cube_size` passed to bpy.ops.uv.cube_project
for every material region in the kit -- one absolute physical scale, applied
the same way regardless of a given mesh's own bounding box, so a small reja
and a long wall segment carry the same texel density.
"""

import glob as _glob
import os

import bpy

TEXEL_SCALE_M = 2048 * 0.0064  # 13.1072 m per 2048^2 tile repeat (VQ-A3 match)

# Ambient-world placeholder palette (docs/town-premise.md S2: S <= 0.35, V <= 0.6
# in HSV, warm bias 20-50 deg only where chromatic). Used whenever no baked
# material set is found under --materials-dir.
PLACEHOLDER_COLOR = {
    "encalado":          (0.58, 0.575, 0.57, 1.0),
    "limestone_dressed": (0.55, 0.545, 0.53, 1.0),
    "terracotta_tile":   (0.55, 0.44, 0.385, 1.0),
    "oak_dark":          (0.10, 0.07, 0.05, 1.0),
    "plaster_smoked":    (0.35, 0.345, 0.34, 1.0),
    "iron_wrought":       (0.12, 0.115, 0.11, 1.0),
}
PLACEHOLDER_ROUGHNESS = {
    "encalado": 0.85, "limestone_dressed": 0.75, "terracotta_tile": 0.8,
    "oak_dark": 0.55, "plaster_smoked": 0.85, "iron_wrought": 0.4,
}

# Candidate directory names the materials worker may use for each family,
# in search order. target/town-materials/ is judged in progress (cand_1..N
# per family, no "selected" pointer yet), so the highest-numbered cand_*
# under any of these is taken as provisional best.
_FAMILY_DIR_CANDIDATES = {
    "encalado":          ["encalado", "m1-encalado"],
    "limestone_dressed": ["limestone_dressed", "dressed_limestone",
                          "m2-dressed-limestone", "dressed-limestone"],
    "terracotta_tile":   ["terracotta_tile", "m3-terracotta-tile",
                          "terracotta-tile"],
    "oak_dark":          ["oak_dark", "m4-oak-dark", "dark-oak", "dark_oak"],
    "plaster_smoked":    ["plaster_smoked", "m5-plaster-smoked",
                          "plaster-smoked"],
    "iron_wrought":      ["iron_wrought", "m6-iron-wrought", "wrought-iron",
                          "wrought_iron"],
}
_BASECOLOR_GLOBS = ["basecolor*.png", "diff*.png", "albedo*.png"]
_NORMAL_GLOBS = ["normal*.png", "nor_gl*.png", "nrm*.png"]
_ROUGHNESS_GLOBS = ["roughness*.png", "rough*.png"]

# Only limestone reads the world-space detail overlay (chisel-scale stone
# grain); the engine defaults every other material's extra to false
# (smirk/engine-renderer/src/mesh/gltf_import.rs).
DETAIL_FAMILY = "limestone_dressed"


def _find_first(directory, patterns):
    for pattern in patterns:
        hits = sorted(_glob.glob(os.path.join(directory, pattern)))
        if hits:
            return hits[0]
    return None


def _resolve_family_dir(materials_dir, family):
    if materials_dir is None:
        return None
    for name in _FAMILY_DIR_CANDIDATES[family]:
        candidate = os.path.join(materials_dir, name)
        if not os.path.isdir(candidate):
            continue
        if _find_first(candidate, _BASECOLOR_GLOBS):
            return candidate
        cand_dirs = sorted(
            (p for p in _glob.glob(os.path.join(candidate, "cand_*")) if os.path.isdir(p)),
            key=lambda p: p.lower(),
        )
        if cand_dirs:
            return cand_dirs[-1]
    return None


def _load_image(path, non_color):
    img = bpy.data.images.load(path, check_existing=True)
    if non_color:
        img.colorspace_settings.name = "Non-Color"
    return img


def _wire_baked_textures(mat, family_dir):
    tree = mat.node_tree
    bsdf = tree.nodes["Principled BSDF"]
    base_path = _find_first(family_dir, _BASECOLOR_GLOBS)
    normal_path = _find_first(family_dir, _NORMAL_GLOBS)
    rough_path = _find_first(family_dir, _ROUGHNESS_GLOBS)
    if base_path:
        node = tree.nodes.new("ShaderNodeTexImage")
        node.image = _load_image(base_path, non_color=False)
        node.location = (-600, 300)
        tree.links.new(node.outputs["Color"], bsdf.inputs["Base Color"])
    if rough_path:
        node = tree.nodes.new("ShaderNodeTexImage")
        node.image = _load_image(rough_path, non_color=True)
        node.location = (-600, 0)
        tree.links.new(node.outputs["Color"], bsdf.inputs["Roughness"])
    if normal_path:
        tex = tree.nodes.new("ShaderNodeTexImage")
        tex.image = _load_image(normal_path, non_color=True)
        tex.location = (-600, -300)
        nmap = tree.nodes.new("ShaderNodeNormalMap")
        nmap.location = (-300, -300)
        tree.links.new(tex.outputs["Color"], nmap.inputs["Color"])
        tree.links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])
    return base_path is not None


def build_materials(materials_dir):
    """Create the six premise materials once per run. Returns
    (materials: dict[name, bpy.types.Material], sources: dict[name, str])
    where sources records "baked:<dir>" or "placeholder" per family, for the
    caller's report."""
    materials = {}
    sources = {}
    for family in PLACEHOLDER_COLOR:
        mat = bpy.data.materials.new(family)
        mat.use_nodes = True
        bsdf = mat.node_tree.nodes["Principled BSDF"]
        bsdf.inputs["Base Color"].default_value = PLACEHOLDER_COLOR[family]
        bsdf.inputs["Roughness"].default_value = PLACEHOLDER_ROUGHNESS[family]
        bsdf.inputs["Metallic"].default_value = 1.0 if family == "iron_wrought" else 0.0

        family_dir = _resolve_family_dir(materials_dir, family)
        used_baked = _wire_baked_textures(mat, family_dir) if family_dir else False
        sources[family] = f"baked:{family_dir}" if used_baked else "placeholder"

        mat["vordar_detail"] = family == DETAIL_FAMILY
        materials[family] = mat
    return materials, sources


def apply_material(obj, mat):
    obj.data.materials.clear()
    obj.data.materials.append(mat)


def project_uv(obj, cube_size=TEXEL_SCALE_M):
    """Box-project the object's single UV layer at the derived physical
    texel scale (see module docstring)."""
    mesh = obj.data
    if not mesh.uv_layers:
        mesh.uv_layers.new(name="UVMap")
    view_layer = bpy.context.view_layer
    prev_active = view_layer.objects.active
    prev_selected = list(bpy.context.selected_objects)
    for o in prev_selected:
        o.select_set(False)
    obj.select_set(True)
    view_layer.objects.active = obj
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.cube_project(cube_size=cube_size, correct_aspect=True,
                             clip_to_bounds=False, scale_to_bounds=False)
    bpy.ops.object.mode_set(mode="OBJECT")
    obj.select_set(False)
    for o in prev_selected:
        o.select_set(True)
    view_layer.objects.active = prev_active
