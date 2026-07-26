"""Multi-view camera rig and ortho depth/normal renders for the
ControlNet-depth multiview retexture (prop_texture.py's basecolor path).

near_far is the package's single near/far definition: depth_setup bakes a
near->1/far->0 ramp and view_weight later inverts that exact ramp, so every
view and every render must agree on the same pair or depth comparisons go
silently wrong.
"""

from contextlib import contextmanager
from math import cos, radians, sin

import bpy
import cv2
import numpy as np
from mathutils import Vector

from proptex.scene import emission_graph, img_array, save_png, scene_state

MV_ELEVATION_DEG = 15.0

MV_DEPTH_DILATE_PX = 5  # conditioning-PNG silhouette grow (Paint3D's setting):
# the model then paints material past the true edge, so blend taps at the
# silhouette read object color; only the PNG is dilated — the float depth
# must keep the true silhouette for the occlusion test and pad_edges mask


def view_hint(az_deg, el_deg=MV_ELEVATION_DEG):
    if el_deg >= 60:
        return "top-down view"
    a = az_deg % 360.0
    if a <= 30 or a >= 330:
        hint = "front view"
    elif 150 <= a <= 210:
        hint = "back view"
    elif 80 <= a <= 100 or 260 <= a <= 280:
        hint = "side view"
    else:
        hint = "three-quarter view"
    if el_deg >= 30:
        hint += ", seen from above"
    elif el_deg <= -20:
        hint += ", seen from below"
    return hint


def near_far(rig):
    """Ortho near/far for every view; the depth ramp bakes near->1/far->0 and
    view_weight inverts it, so one definition is a correctness requirement,
    not tidiness."""
    return rig["half"], 3.0 * rig["half"]


def mv_view(hint, az_deg, el_deg, rig):
    az, el = radians(az_deg), radians(el_deg)
    d = np.array([sin(az) * cos(el), -cos(az) * cos(el), sin(el)])  # center -> camera
    f = -d  # camera forward
    s = np.cross(f, [0.0, 0.0, 1.0])
    s /= np.linalg.norm(s)
    u = np.cross(s, f)
    dist = 2.0 * rig["half"]
    near, far = near_far(rig)
    return {
        "hint": hint, "azimuth_deg": az_deg, "elevation_deg": el_deg,
        "cam": (rig["lo"] + rig["hi"]) / 2 + d * dist, "f": f, "s": s, "u": u,
        "near": near, "far": far,
    }


def mv_camera_rig(clean, specs):
    me = clean.data
    co = np.empty(len(me.vertices) * 3, dtype=np.float32)
    me.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    lo, hi = co.min(axis=0), co.max(axis=0)
    radius = float(np.linalg.norm(hi - lo) / 2) * 1.05
    rig = {"lo": lo, "hi": hi, "half": radius, "ortho_scale": 2.0 * radius}
    return [mv_view(*spec, rig) for spec in specs], rig


@contextmanager
def isolated(obj):
    """Hides every other mesh in the scene for the block, so only obj's
    depth/normal renders, restoring each one's prior hide_render (not a
    hardcoded False) in a finally so a raise inside the block can't leave
    any of them hidden for everything rendered after."""
    others = [o for o in bpy.context.scene.objects
              if o.type == "MESH" and o is not obj]
    prior = {o: o.hide_render for o in others}
    for o in others:
        o.hide_render = True
    try:
        yield
    finally:
        for o, was in prior.items():
            o.hide_render = was


@contextmanager
def ortho_camera(rig, view_res):
    """Every ortho render (depth or normal, base or candidate) shares this
    render configuration -- spelled once here so it cannot drift between
    callers, restored via scene_state on exit."""
    with scene_state(resolution=view_res, file_format="OPEN_EXR", color_depth="32"):
        scene = bpy.context.scene
        near, far = near_far(rig)
        cam_data = bpy.data.cameras.new("mv_cam")
        cam_data.type = "ORTHO"
        cam_data.ortho_scale = rig["ortho_scale"]
        cam_data.clip_start = near * 0.5
        cam_data.clip_end = far * 2.0
        cam_obj = bpy.data.objects.new("mv_cam", cam_data)
        scene.collection.objects.link(cam_obj)
        scene.camera = cam_obj
        yield cam_obj


@contextmanager
def depth_setup(clean, rig, view_res):
    """Depth-ramp emission material (near=1, far=0) + shared ortho camera;
    every view shares near/far because the rig is size-only."""
    near, far = near_far(rig)
    tree, n_emit = emission_graph(clean)
    n_cam = tree.nodes.new("ShaderNodeCameraData")
    n_map = tree.nodes.new("ShaderNodeMapRange")
    n_map.clamp = True
    n_map.inputs["From Min"].default_value = near
    n_map.inputs["From Max"].default_value = far
    n_map.inputs["To Min"].default_value = 1.0
    n_map.inputs["To Max"].default_value = 0.0
    tree.links.new(n_cam.outputs["View Z Depth"], n_map.inputs["Value"])
    tree.links.new(n_map.outputs["Result"], n_emit.inputs["Color"])
    with ortho_camera(rig, view_res) as cam_obj, isolated(clean):
        yield cam_obj


@contextmanager
def normal_setup(clean, rig, view_res):
    """Camera-space normal emission material, encoded n*0.5+0.5 with +X
    right / +Y up / +Z toward camera -- the exact conditioning encoding the
    MaterialAnything estimator was trained on (their pipeline builds it
    from PyTorch3D view-space normals via a diag(-1,1,-1) remap; Blender's
    camera space already has these axes, so a plain transform suffices)."""
    tree, n_emit = emission_graph(clean)
    n_geo = tree.nodes.new("ShaderNodeNewGeometry")
    n_xform = tree.nodes.new("ShaderNodeVectorTransform")
    n_xform.vector_type = "NORMAL"
    n_xform.convert_from = "WORLD"
    n_xform.convert_to = "CAMERA"
    n_madd = tree.nodes.new("ShaderNodeVectorMath")
    n_madd.operation = "MULTIPLY_ADD"
    n_madd.inputs[1].default_value = (0.5, 0.5, 0.5)
    n_madd.inputs[2].default_value = (0.5, 0.5, 0.5)
    tree.links.new(n_geo.outputs["Normal"], n_xform.inputs["Vector"])
    tree.links.new(n_xform.outputs["Vector"], n_madd.inputs[0])
    tree.links.new(n_madd.outputs["Vector"], n_emit.inputs["Color"])
    with ortho_camera(rig, view_res) as cam_obj, isolated(clean):
        yield cam_obj


def load_exr(exr_path):
    """An EXR's pixels as a (h, w, 4) array, row 0 = image bottom."""
    img = bpy.data.images.load(str(exr_path))
    px = img_array(img).copy()
    bpy.data.images.remove(img)
    return px


def render_exr(cam_obj, v, exr_path):
    scene = bpy.context.scene
    cam_obj.location = Vector(v["cam"])
    cam_obj.rotation_euler = Vector(v["f"]).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = str(exr_path)
    bpy.ops.render.render(write_still=True)
    return load_exr(exr_path)


def read_depth(exr_path):
    """A view's float depth ramp back from its EXR."""
    return load_exr(exr_path)[:, :, 0]


def render_depth_view(cam_obj, v, out_dir):
    """One view's ortho depth ramp (near=1, far=0) into out_dir: depth.exr
    for reprojection, plus depth.png for the ControlNet conditioning
    input."""
    depth = render_exr(cam_obj, v, out_dir / "depth.exr")[:, :, 0]
    cond = cv2.dilate(depth, np.ones((3, 3), np.float32),
                      iterations=MV_DEPTH_DILATE_PX)
    h, w = cond.shape
    png = bpy.data.images.new("mv_depth", w, h)
    png.colorspace_settings.name = "Non-Color"
    rgba = np.empty((w * h, 4), dtype=np.float32)
    rgba[:, 0] = rgba[:, 1] = rgba[:, 2] = cond.ravel()
    rgba[:, 3] = 1.0
    png.pixels.foreach_set(rgba.ravel())
    save_png(png, out_dir / "depth.png")
    bpy.data.images.remove(png)


def render_normal_view(cam_obj, v, depth, out_dir):
    """One view's camera-space normal render + object mask into out_dir, the
    estimator's conditioning inputs: normal.png (linear n*0.5+0.5, white
    background like upstream's renders) and mask.png (255 inside the
    silhouette). Written top-down via cv2 for the PIL-side runner;
    img_array rows are bottom-up, hence the flips."""
    scratch = out_dir / "normal_render.exr"
    rgb = render_exr(cam_obj, v, scratch)[:, :, :3]
    scratch.unlink()
    obj = depth > 0.01
    rgb[~obj] = 1.0
    rgb8 = (np.clip(np.flipud(rgb), 0.0, 1.0) * 255.0).round().astype(np.uint8)
    cv2.imwrite(str(out_dir / "normal.png"), rgb8[:, :, ::-1])
    cv2.imwrite(str(out_dir / "mask.png"), np.flipud(obj).astype(np.uint8) * 255)
