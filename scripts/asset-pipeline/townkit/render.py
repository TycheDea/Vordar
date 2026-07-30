"""Quick CPU-only headless preview render: fit a camera to the current
scene's mesh objects, light it neutrally, and render one PNG so a human can
eyeball silhouettes without opening Blender."""

import math

import bpy
from mathutils import Vector


def _scene_bbox():
    mn = Vector((math.inf, math.inf, math.inf))
    mx = Vector((-math.inf, -math.inf, -math.inf))
    for obj in bpy.context.scene.objects:
        if obj.type != "MESH":
            continue
        for corner in obj.bound_box:
            world = obj.matrix_world @ Vector(corner)
            mn.x, mn.y, mn.z = min(mn.x, world.x), min(mn.y, world.y), min(mn.z, world.z)
            mx.x, mx.y, mx.z = max(mx.x, world.x), max(mx.y, world.y), max(mx.z, world.z)
    return mn, mx


def render_preview(out_path, resolution=(640, 480), samples=24, az_deg=35.0, el_deg=28.0, dist_scale=2.4):
    """Render one angle. Defaults are the original three-quarter view;
    pass a low `el_deg` (and a tighter `dist_scale`) for a street-level shot
    that looks INTO gable ends and along wall tops rather than down onto
    roofs, so open gables and thin exposed rims can't hide behind the
    three-quarter view's favourable angle."""
    mn, mx = _scene_bbox()
    center = (mn + mx) / 2.0
    radius = max((mx - mn).length / 2.0, 0.5)

    empty = bpy.data.objects.new("preview_target", None)
    bpy.context.collection.objects.link(empty)
    empty.location = center

    cam_data = bpy.data.cameras.new("preview_cam")
    cam_data.lens = 35
    cam_obj = bpy.data.objects.new("preview_cam", cam_data)
    bpy.context.collection.objects.link(cam_obj)
    dist = radius * dist_scale
    az, el = math.radians(az_deg), math.radians(el_deg)
    cam_obj.location = center + Vector((
        dist * math.cos(el) * math.cos(az),
        dist * math.cos(el) * math.sin(az),
        dist * math.sin(el),
    ))
    constraint = cam_obj.constraints.new("TRACK_TO")
    constraint.target = empty
    constraint.track_axis = "TRACK_NEGATIVE_Z"
    constraint.up_axis = "UP_Y"
    bpy.context.scene.camera = cam_obj

    sun_data = bpy.data.lights.new("preview_sun", type="SUN")
    sun_data.energy = 3.0
    sun_obj = bpy.data.objects.new("preview_sun", sun_data)
    bpy.context.collection.objects.link(sun_obj)
    sun_obj.rotation_euler = (math.radians(55.0), 0.0, math.radians(35.0))

    world = bpy.data.worlds.new("preview_world")
    world.use_nodes = True
    bg = world.node_tree.nodes.get("Background")
    if bg:
        bg.inputs[0].default_value = (0.45, 0.47, 0.5, 1.0)
        bg.inputs[1].default_value = 1.0
    bpy.context.scene.world = world

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = samples
    scene.render.resolution_x, scene.render.resolution_y = resolution
    scene.render.image_settings.file_format = "PNG"
    scene.render.filepath = str(out_path)
    bpy.ops.render.render(write_still=True)

    img = bpy.data.images.load(str(out_path))
    pixels = list(img.pixels)
    mean = sum(pixels) / len(pixels) if pixels else 0.0
    bpy.data.images.remove(img)

    # Cleanup so a second render_preview call on the same scene (the
    # street-level angle) doesn't accumulate a second sun/camera.
    for o in (cam_obj, sun_obj, empty):
        bpy.data.objects.remove(o, do_unlink=True)
    return mean
