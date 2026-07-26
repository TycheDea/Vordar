"""Blender scene primitives shared by the prop-texture bakes.

Scene state is global in Blender, so every override here is scoped: enter
`scene_state` to change render settings or link objects, and the scene is
whole again on exit whether the block returned or raised.
"""

from contextlib import contextmanager

import bpy
import numpy as np


class SceneError(Exception):
    pass


def blender_id():
    """Blender's build identity, the toolchain string every Blender-side
    stage carries in its cache params. The version string alone repeats
    across builds that render differently, so the build hash rides with it."""
    return f"{bpy.app.version_string} {bpy.app.build_hash.decode()}"


def import_glb(path):
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.gltf(filepath=str(path))
    new = [o for o in set(bpy.context.scene.objects) - before if o.type == "MESH"]
    if not new:
        raise SceneError(f"no mesh in {path}")
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


def img_array(img):
    """Image pixels as a (h, w, 4) float array; row 0 = image bottom."""
    w, h = img.size
    px = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(px)
    return px.reshape(h, w, 4)


def new_image(name, srgb, fill, size, float_buffer=False):
    img = bpy.data.images.new(name, size, size, alpha=False, float_buffer=float_buffer)
    img.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
    img.generated_color = (*fill, 1.0)
    return img


def save_png(img, path):
    """Write an image datablock to path as PNG, encoded through its own
    colorspace setting (sRGB for basecolor, Non-Color for data maps)."""
    img.filepath_raw = str(path)
    img.file_format = "PNG"
    img.save()


def bake_material(obj):
    """Fresh single-material node tree on obj; returns the tree."""
    mat = bpy.data.materials.new("bake")
    mat.use_nodes = True
    mat.node_tree.nodes.clear()
    obj.data.materials.clear()
    obj.data.materials.append(mat)
    return mat.node_tree


def emit_bake(clean, tree, image, atlas):
    """EMIT-bake into image via the tree's active image node (created here)."""
    n_bake = tree.nodes.new("ShaderNodeTexImage")
    n_bake.image = image
    tree.nodes.active = n_bake
    select_only([clean], clean)
    bpy.ops.object.bake(type="EMIT", margin=8, use_clear=True, uv_layer=atlas)


def emission_graph(obj):
    """(tree, emission_node) -- the shared spine of every EMIT bake here."""
    tree = bake_material(obj)
    n_emit = tree.nodes.new("ShaderNodeEmission")
    n_out = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(n_emit.outputs["Emission"], n_out.inputs["Surface"])
    return tree, n_emit


@contextmanager
def scene_state(resolution=None, file_format=None, color_depth=None, samples=None):
    scene = bpy.context.scene
    render = scene.render
    snapshot = {
        "resolution_x": render.resolution_x,
        "resolution_y": render.resolution_y,
        "resolution_percentage": render.resolution_percentage,
        "file_format": render.image_settings.file_format,
        "color_depth": render.image_settings.color_depth,
        "filepath": render.filepath,
        "samples": scene.cycles.samples,
        "camera": scene.camera,
    }
    before_objects = set(scene.collection.objects)

    if resolution is not None:
        render.resolution_x = render.resolution_y = resolution
        render.resolution_percentage = 100
    if file_format is not None:
        render.image_settings.file_format = file_format
    if color_depth is not None:
        render.image_settings.color_depth = color_depth
    if samples is not None:
        scene.cycles.samples = samples

    try:
        yield
    finally:
        # a raising bake must not leave the scene at overridden settings
        # (e.g. AO_SAMPLES) for everything that runs after it
        render.resolution_x = snapshot["resolution_x"]
        render.resolution_y = snapshot["resolution_y"]
        render.resolution_percentage = snapshot["resolution_percentage"]
        render.image_settings.file_format = snapshot["file_format"]
        render.image_settings.color_depth = snapshot["color_depth"]
        render.filepath = snapshot["filepath"]
        scene.cycles.samples = snapshot["samples"]
        scene.camera = snapshot["camera"]

        for obj in set(scene.collection.objects) - before_objects:
            data = obj.data if obj.type == "CAMERA" else None
            bpy.data.objects.remove(obj, do_unlink=True)
            if data is not None:
                bpy.data.cameras.remove(data)
