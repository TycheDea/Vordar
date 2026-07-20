# Positive control for metrics.py (A6.2): render the same rig with a hard sun
# and a flat white material, so rendered luma is pure shading.
#
# Without this, metrics.py's scores are unfalsifiable -- a near-zero baked
# fraction could equally mean "no baked lighting" or "the depth-derived normals
# cannot see lighting". This pins the top of the scale on real geometry.
#
# Also re-renders depth, which doubles as a mesh-identity check: compare against
# the bake-off's own depth_0.png, and if the silhouettes disagree the normals do
# not align and the measurement is void.
#
# Usage: blender --background --python lit_control.py -- <mesh.glb> <out_dir>
import sys
from pathlib import Path

import bpy
from mathutils import Vector

BAKEOFF_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(BAKEOFF_DIR.parent))
import prop_texture as pt  # noqa: E402


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    if len(argv) != 2:
        pt.fail("usage: lit_control.py -- <mesh.glb> <out_dir>")
    mesh, out_dir = Path(argv[0]).resolve(), Path(argv[1]).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 16
    scene.cycles.use_denoising = False

    clean = pt.import_glb(mesh)
    hires = pt.import_glb(mesh)
    views, rig = pt.mv_camera_rig(clean)
    pt.render_depth_views(clean, hires, views, rig, out_dir)
    bpy.data.objects.remove(hires, do_unlink=True)

    mat = bpy.data.materials.new("white")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (0.8, 0.8, 0.8, 1.0)
    bsdf.inputs["Roughness"].default_value = 1.0
    bsdf.inputs["Metallic"].default_value = 0.0
    clean.data.materials.clear()
    clean.data.materials.append(mat)

    sun_data = bpy.data.lights.new("sun", type="SUN")
    sun_data.energy = 5.0
    sun = bpy.data.objects.new("sun", sun_data)
    scene.collection.objects.link(sun)
    sun.rotation_euler = (0.9, 0.0, 0.6)

    # same ortho placement render_depth_views uses, so lit_i is pixel-aligned
    # with depth_i
    cam_data = bpy.data.cameras.new("lit_cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = rig["ortho_scale"]
    cam_data.clip_start = views[0]["near"] * 0.5
    cam_data.clip_end = views[0]["far"] * 2.0
    cam_obj = bpy.data.objects.new("lit_cam", cam_data)
    scene.collection.objects.link(cam_obj)
    scene.camera = cam_obj

    scene.render.resolution_x = scene.render.resolution_y = pt.MV_RES
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_depth = "8"
    for i, v in enumerate(views[:1]):
        cam_obj.location = Vector(v["cam"])
        cam_obj.rotation_euler = Vector(v["f"]).to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = str(out_dir / f"lit_{i}.png")
        bpy.ops.render.render(write_still=True)
        print(f"wrote lit_{i}.png")


if __name__ == "__main__":
    main()
