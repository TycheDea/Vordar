"""The three mesh-to-glTF stages of the prop texture chain: the high-to-low
normal bake, the hires-cage AO bake, and the final material graph and glTF
write. All three take the cleaned mesh and write files a cache entry
declares, so each is skippable independently of the others.
"""

from pathlib import Path

import bpy

from proptex.scene import bake_material, new_image, save_png, scene_state, select_only

AO_SAMPLES = 128  # the EMIT/NORMAL bakes are exact single-ray lookups, but AO
# integrates a hemisphere per texel -- 1 sample would be per-texel dithered
# noise, not the smooth occlusion gradient occlusionTexture needs
AO_DISTANCE_M = 0.15  # Cycles' AO ray length defaults to 10 m (world
# light_settings.distance), which on an archway treats the far leg across
# the opening as an occluder and reads as one big cave shadow. Bounding it
# to voussoir-joint scale keeps the occlusion local: drum joints and capital
# undercuts darken, the opposite side of the arch does not

# Shared selected-to-active rig for both bakes below; named once so the two
# operator calls cannot drift apart.
BAKE_NORMAL_SPACE = "TANGENT"
BAKE_CAGE_EXTRUSION_M = 0.01
BAKE_MAX_RAY_DISTANCE_M = 0.03
BAKE_MARGIN_PX = 8


class ExportError(Exception):
    pass


def _bake_to(clean, hires, image, atlas, **bake_args):
    tree = bake_material(clean)
    n_bake = tree.nodes.new("ShaderNodeTexImage")
    n_bake.image = image
    tree.nodes.active = n_bake
    select_only([clean, hires], clean)
    bpy.ops.object.bake(use_selected_to_active=True,
                        cage_extrusion=BAKE_CAGE_EXTRUSION_M,
                        max_ray_distance=BAKE_MAX_RAY_DISTANCE_M,
                        margin=BAKE_MARGIN_PX, use_clear=True, uv_layer=atlas,
                        **bake_args)


def bake_normal_map(clean, hires, atlas, texture_size, out_dir):
    """Real high-to-low tangent-space normal bake from the hires mesh onto
    the clean mesh's atlas UVs (prop_cleanup.py keeps the two rigidly
    aligned), written to out_dir as normal.png."""
    img = new_image("prop_normal", srgb=False, fill=(0.5, 0.5, 1.0), size=texture_size)
    _bake_to(clean, hires, img, atlas, type="NORMAL", normal_space=BAKE_NORMAL_SPACE)
    save_png(img, out_dir / "normal.png")
    bpy.data.images.remove(img)


def bake_ao_map(clean, hires, atlas, texture_size, out_dir):
    """Cycles AO bake through the same selected-to-active cage as the normal
    bake, so crevices the hires mesh actually forms (drum joints, capital
    undercuts) darken; written to out_dir as occlusion.png."""
    world = bpy.data.worlds.new("bake_world")
    world.light_settings.distance = AO_DISTANCE_M
    bpy.context.scene.world = world
    img = new_image("prop_ao", srgb=False, fill=(1.0, 1.0, 1.0), size=texture_size)
    with scene_state(samples=AO_SAMPLES):
        _bake_to(clean, hires, img, atlas, type="AO")
    save_png(img, out_dir / "occlusion.png")
    bpy.data.images.remove(img)


def _load(path, srgb):
    img = bpy.data.images.load(str(path))
    img.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
    return img


def export_prop(clean, base_png, normal_png, ao_png, contract, out_dir):
    """Build the Principled BSDF material graph on clean from the three
    baked maps and export clean alone as out_dir/textured.glb."""
    base_img = _load(base_png, srgb=True)
    normal_img = _load(normal_png, srgb=False)
    ao_img = _load(ao_png, srgb=False)

    mat = bpy.data.materials.new("prop")
    mat["vordar_detail"] = contract.detail
    mat.use_nodes = True
    tree = mat.node_tree
    bsdf = tree.nodes["Principled BSDF"]
    n_base = tree.nodes.new("ShaderNodeTexImage")
    n_base.image = base_img
    tree.links.new(n_base.outputs["Color"], bsdf.inputs["Base Color"])
    # MR is uniform, so it rides the glTF scalar factors instead of a map.
    bsdf.inputs["Metallic"].default_value = contract.metallic
    bsdf.inputs["Roughness"].default_value = contract.roughness
    n_nrm = tree.nodes.new("ShaderNodeTexImage")
    n_nrm.image = normal_img
    n_nmap = tree.nodes.new("ShaderNodeNormalMap")
    tree.links.new(n_nrm.outputs["Color"], n_nmap.inputs["Color"])
    tree.links.new(n_nmap.outputs["Normal"], bsdf.inputs["Normal"])
    # occlusionTexture has no Principled BSDF input -- the exporter instead
    # looks for a node group named "glTF Material Output" (or the older
    # "glTF Settings") with an "Occlusion" socket fed from the AO image.
    n_ao = tree.nodes.new("ShaderNodeTexImage")
    n_ao.image = ao_img
    settings_tree = bpy.data.node_groups.new("glTF Material Output", "ShaderNodeTree")
    settings_tree.interface.new_socket("Occlusion", socket_type="NodeSocketFloat")
    n_settings = tree.nodes.new("ShaderNodeGroup")
    n_settings.node_tree = settings_tree
    tree.links.new(n_ao.outputs["Color"], n_settings.inputs["Occlusion"])
    clean.data.materials.clear()
    clean.data.materials.append(mat)

    # use_selection=True makes the export self-contained: it writes exactly
    # the selected object regardless of what else is linked in the scene,
    # rather than depending on the scene happening to be clean.
    select_only([clean], clean)
    result = bpy.ops.export_scene.gltf(filepath=str(Path(out_dir) / "textured.glb"),
                                       export_format="GLB", export_yup=True,
                                       export_image_format="AUTO",
                                       export_extras=True, use_selection=True)
    if result != {"FINISHED"}:
        raise ExportError(f"export_scene.gltf returned {result}")
