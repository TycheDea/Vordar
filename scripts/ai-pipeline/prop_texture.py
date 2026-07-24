# Blender-headless: texture a cleaned Hi3DGen prop mesh (Phase A3.6; both
# strategy rulings in tasks/ai-pipeline/a3.md -> "Texture strategy log").
#
# Every bake targets the clean mesh's UV atlas, unwrapped once by
# prop_cleanup.py (xatlas) — this stage fails if the mesh arrives without
# UVs, so the atlas is identical across texture re-runs of a candidate.
#
# Two basecolor strategies share one channels contract:
#   - projection (default): concept image planar-projected along the
#     concept view axis and EMIT-baked into basecolor. The
#     projection passes through the mesh, so back faces receive the
#     mirrored front read -- the near-symmetric prop classes this pipeline
#     targets survive that.
#   - multiview (escalation for prop classes needing true material register
#     or strong backsides): ortho depth renders of the clean mesh from
#     MV_VIEWS cameras feed a ControlNet-depth text-to-image base via a ComfyUI
#     server whose lifecycle lives entirely inside this stage. Opposite
#     views are tiled side by side into ONE conditioning canvas per
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
#   - MR, projection strategy: two declared constants (--metallic/
#     --roughness) carried by the glTF scalar factors rather than a map.
#   - MR, multiview strategy: per-texel. Each generated view is decomposed
#     by MaterialAnything's material estimator (prop_pbr.py, subprocess in
#     its own venv) into a delit albedo and a roughness/metallic image,
#     conditioned on a camera-space normal render of the same view; the
#     albedo blend becomes the basecolor (the lit gen.png is conditioning
#     intermediate only) and the rm blend packs into a glTF
#     metallicRoughnessTexture. Classifying material from the BASECOLOR
#     value was tried and retired (A6.1, tasks/ai-pipeline/research/
#     a6-1-mr-contract.md): luma conflates albedo, shading and material.
#   - prints one JSON stats line (the only '{'-prefixed stdout line) for
#     gen_prop.py's chained manifest.
#
# Usage: blender --background --python prop_texture.py -- \
#            <clean.glb> <hires.glb> <concept.png> <textured.glb> \
#            [--strategy projection|multiview] [--subject STR] [--seed N] \
#            [--metallic F] [--roughness F] [--dielectric]

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import time
import traceback
from math import cos, radians, sin
from pathlib import Path

import bpy
import cv2
import numpy as np
from mathutils import Vector

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
import comfy_run  # noqa: E402  (stdlib-only, safe under Blender's Python)

TEXTURE_SIZE = 1024
# Blender-world direction the concept camera looks FROM. Hi3DGen +
# prop_cleanup emit the concept view on the glTF +Z face, which the
# Blender glTF importer maps to -Y (verified on the A3.4 smoke mesh:
# the +Y projection renders mirrored against the concept). The multiview
# rig keeps the same convention: azimuth 0 is the -Y camera.
FRONT_AXIS = "-Y"
# Declared MR defaults (projection strategy only; multiview estimates
# per-texel MR): non-metal, matching every existing race model and the
# stone/wood/cloth props the art direction calls for. Metal props override.
DEFAULT_METALLIC = 0.0
DEFAULT_ROUGHNESS = 0.8

MV_WORKFLOW = SCRIPT_DIR / "workflows" / "prop_multiview.json"
# The MaterialAnything estimator runs in its own venv, not Blender's Python:
# its diffusers pin (0.28.2) predates everything else in the pipeline.
MA_PYTHON = Path(r"C:\tools\MaterialAnything") / "venv" / "Scripts" / "python.exe"


MV_ELEVATION_DEG = 15.0
# Two-resolution contract: MV_RES sets the ortho depth/normal renders and the
# Z-Image + Fun ControlNet-Union generation canvas (sharper source imagery
# baked into the atlas); prop_pbr.py's estimator always downscales its input
# to EST_RES=768 (its pinned training resolution) and upscales the result
# back to MV_RES, so raising MV_RES never touches the estimator's fixed input
# size.


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


# Rebound in main() when --azimuths is passed: planar props degenerate to a
# sliver in exact side depth views, which frees the base to hallucinate an
# unrelated object into them — oblique azimuths keep the conditioning real.
MV_VIEWS = [(view_hint(a), a, MV_ELEVATION_DEG) for a in (0.0, 90.0, 180.0, 270.0)]
MV_RES = 1024
MV_WEIGHT_EXPONENT = 2.0
MV_OCCLUSION_EPS = 0.02  # meters; props are ~1.8 m tall (prop_cleanup)
MV_EDGE_PAD_PX = 8  # the base bleeds background across the depth edge; padding
# the object's colors outward keeps edge texels on-material without
# shrinking thin members (erosion would erase a ~6 px scroll arm entirely)
MV_DEPTH_DILATE_PX = 5  # conditioning-PNG silhouette grow (Paint3D's setting):
# the model then paints material past the true edge, so blend taps at the
# silhouette read object color; only the PNG is dilated — the float depth
# must keep the true silhouette for the occlusion test and pad_edges mask
MV_COVERAGE_EPS = 1e-4  # a texel counts as covered above this blend weight;
# shared by blend_views and the extra-view predictor so they cannot drift
# Coverage-driven extra views (clean-room Text2Tex next-best-view):
MV_EXTRA_MAX = 2  # at most one extra canvas (two side-by-side views)
MV_EXTRA_MIN_GAIN = 0.03  # an extra view must newly cover >=3% of island
# texels; a smaller residue is scattered enough that Telea inpaint suffices
MV_EXTRA_CANDIDATE_AZIMUTHS = tuple(range(0, 360, 30))
MV_EXTRA_CANDIDATE_ELEVATIONS = (-35.0, 15.0, 55.0)  # on standing props the
# uncovered set is dominated by DOWN-facing texels (cup/arm/base undersides:
# 45% of the candelabra's holes vs 33% up-facing), so a below view recovers
# about twice what the best above view does
MV_EXTRA_TOP_ELEVATION = 75.0  # near-top recovery view; at 90 the camera
# basis s = f x z degenerates to zero
MV_EXTRA_MIN_SEP_DEG = 20.0  # candidates within this of an existing view
# direction see nearly the same texels — skip before spending a render


def fail(msg):
    print(f"prop_texture: {msg}", file=sys.stderr)
    sys.exit(1)


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


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


def img_array(img):
    """Image pixels as a (h, w, 4) float array; row 0 = image bottom."""
    w, h = img.size
    px = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(px)
    return px.reshape(h, w, 4)


def concept_stats(img):
    """Alpha bbox (uv space) + mean opaque color of the concept image."""
    px = img_array(img)
    h, w = px.shape[:2]
    opaque = px[:, :, 3] > 0.1
    opaque_fraction = float(opaque.mean())
    if opaque_fraction >= 0.995:
        fail(f"concept image has no usable alpha matte ({opaque_fraction:.1%} opaque) -- "
             "pass a BiRefNet-matted concept_rgba.png, not a raw RGB concept image "
             "(a full-frame bbox and a washed-out fill color is silent degeneration, not a fit)")
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


def new_image(name, srgb, fill, float_buffer=False, size=None):
    # size=None (not size=TEXTURE_SIZE): a literal default binds to
    # TEXTURE_SIZE's value at module-load time and would stay 1024 even
    # after --texture-size reassigns the global, since every call site
    # below omits size and relies on this default reacting to the override.
    if size is None:
        size = TEXTURE_SIZE
    img = bpy.data.images.new(name, size, size, alpha=False, float_buffer=float_buffer)
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


def emit_bake(clean, tree, image, atlas):
    """EMIT-bake into image via the tree's active image node (created here)."""
    n_bake = tree.nodes.new("ShaderNodeTexImage")
    n_bake.image = image
    tree.nodes.active = n_bake
    select_only([clean], clean)
    bpy.ops.object.bake(type="EMIT", margin=8, use_clear=True, uv_layer=atlas)


# ---------------------------------------------------------------------------
# Strategy 1: projection bake
# ---------------------------------------------------------------------------

def basecolor_projection(clean, atlas, concept_png):
    me = clean.data
    concept = bpy.data.images.load(str(Path(concept_png).resolve()))
    bbox_uv, mean_color = concept_stats(concept)
    project_uvs(me, bbox_uv)

    # emission bake copies colors exactly -- no lighting, 1 sample
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
    tree.links.new(n_uv.outputs["UV"], n_tex.inputs["Vector"])
    tree.links.new(n_tex.outputs["Color"], n_mix.inputs["Color2"])
    tree.links.new(n_tex.outputs["Alpha"], n_mix.inputs["Fac"])
    tree.links.new(n_mix.outputs["Color"], n_emit.inputs["Color"])
    tree.links.new(n_emit.outputs["Emission"], n_out.inputs["Surface"])
    emit_bake(clean, tree, base_img, atlas)
    me.uv_layers.remove(me.uv_layers["proj"])

    extras = {
        "strategy": "blender_projection_bake",
        "front_axis": FRONT_AXIS,
        "concept_alpha_bbox_uv": [round(v, 4) for v in bbox_uv],
        "fill_color": [round(c, 4) for c in mean_color],
    }
    return base_img, extras


# ---------------------------------------------------------------------------
# Strategy 2: multi-view retexture (ControlNet-depth)
# ---------------------------------------------------------------------------

def mv_view(hint, az_deg, el_deg, rig):
    az, el = radians(az_deg), radians(el_deg)
    d = np.array([sin(az) * cos(el), -cos(az) * cos(el), sin(el)])  # center -> camera
    f = -d  # camera forward
    s = np.cross(f, [0.0, 0.0, 1.0])
    s /= np.linalg.norm(s)
    u = np.cross(s, f)
    dist = 2.0 * rig["half"]
    return {
        "hint": hint, "azimuth_deg": az_deg, "elevation_deg": el_deg,
        "cam": (rig["lo"] + rig["hi"]) / 2 + d * dist, "f": f, "s": s, "u": u,
        "near": dist - rig["half"], "far": dist + rig["half"],
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


def _ortho_camera(rig):
    scene = bpy.context.scene
    scene.render.resolution_x = scene.render.resolution_y = MV_RES
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_depth = "32"
    near, far = rig["half"], 3.0 * rig["half"]
    cam_data = bpy.data.cameras.new("mv_cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = rig["ortho_scale"]
    cam_data.clip_start = near * 0.5
    cam_data.clip_end = far * 2.0
    cam_obj = bpy.data.objects.new("mv_cam", cam_data)
    scene.collection.objects.link(cam_obj)
    scene.camera = cam_obj
    return cam_obj


def _depth_setup(clean, rig):
    """Depth-ramp emission material (near=1, far=0) + shared ortho camera;
    every view shares near/far because the rig is size-only."""
    near, far = rig["half"], 3.0 * rig["half"]
    tree = bake_material(clean)
    n_cam = tree.nodes.new("ShaderNodeCameraData")
    n_map = tree.nodes.new("ShaderNodeMapRange")
    n_map.clamp = True
    n_map.inputs["From Min"].default_value = near
    n_map.inputs["From Max"].default_value = far
    n_map.inputs["To Min"].default_value = 1.0
    n_map.inputs["To Max"].default_value = 0.0
    n_emit = tree.nodes.new("ShaderNodeEmission")
    n_out = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(n_cam.outputs["View Z Depth"], n_map.inputs["Value"])
    tree.links.new(n_map.outputs["Result"], n_emit.inputs["Color"])
    tree.links.new(n_emit.outputs["Emission"], n_out.inputs["Surface"])
    return _ortho_camera(rig)


def _normal_setup(clean, rig):
    """Camera-space normal emission material, encoded n*0.5+0.5 with +X
    right / +Y up / +Z toward camera — the exact conditioning encoding the
    MaterialAnything estimator was trained on (their pipeline builds it
    from PyTorch3D view-space normals via a diag(-1,1,-1) remap; Blender's
    camera space already has these axes, so a plain transform suffices)."""
    tree = bake_material(clean)
    n_geo = tree.nodes.new("ShaderNodeNewGeometry")
    n_xform = tree.nodes.new("ShaderNodeVectorTransform")
    n_xform.vector_type = "NORMAL"
    n_xform.convert_from = "WORLD"
    n_xform.convert_to = "CAMERA"
    n_madd = tree.nodes.new("ShaderNodeVectorMath")
    n_madd.operation = "MULTIPLY_ADD"
    n_madd.inputs[1].default_value = (0.5, 0.5, 0.5)
    n_madd.inputs[2].default_value = (0.5, 0.5, 0.5)
    n_emit = tree.nodes.new("ShaderNodeEmission")
    n_out = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(n_geo.outputs["Normal"], n_xform.inputs["Vector"])
    tree.links.new(n_xform.outputs["Vector"], n_madd.inputs[0])
    tree.links.new(n_madd.outputs["Vector"], n_emit.inputs["Color"])
    tree.links.new(n_emit.outputs["Emission"], n_out.inputs["Surface"])
    return _ortho_camera(rig)


def _render_exr(cam_obj, v, exr_path):
    scene = bpy.context.scene
    cam_obj.location = Vector(v["cam"])
    cam_obj.rotation_euler = Vector(v["f"]).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = str(exr_path)
    bpy.ops.render.render(write_still=True)
    exr = bpy.data.images.load(str(exr_path))
    px = img_array(exr).copy()
    bpy.data.images.remove(exr)
    return px


def _render_depth(cam_obj, v, exr_path):
    return _render_exr(cam_obj, v, exr_path)[:, :, 0]


def render_depth_views(clean, hires, views, rig, work_dir, start=0):
    """Render each view's ortho depth ramp (near=1, far=0): float array for
    reprojection, plus an 8-bit PNG for the ControlNet conditioning input.
    start offsets the depth_<i> numbering so extra views appended after the
    base set don't collide with it."""
    cam_obj = _depth_setup(clean, rig)
    hires.hide_render = True
    depths = []
    for i, v in enumerate(views, start=start):
        depth = _render_depth(cam_obj, v, work_dir / f"depth_{i}.exr")
        depths.append(depth)

        cond = cv2.dilate(depth, np.ones((3, 3), np.float32),
                          iterations=MV_DEPTH_DILATE_PX)
        png = bpy.data.images.new(f"mv_depth_{i}", MV_RES, MV_RES)
        png.colorspace_settings.name = "Non-Color"
        rgba = np.empty((MV_RES * MV_RES, 4), dtype=np.float32)
        rgba[:, 0] = rgba[:, 1] = rgba[:, 2] = cond.ravel()
        rgba[:, 3] = 1.0
        png.pixels.foreach_set(rgba.ravel())
        png.filepath_raw = str(work_dir / f"depth_{i}.png")
        png.file_format = "PNG"
        png.save()
        bpy.data.images.remove(png)
    hires.hide_render = False
    return depths


def render_normal_views(clean, hires, views, depths, rig, work_dir):
    """Camera-space normal render + object mask per view, the estimator's
    conditioning inputs: normal_<i>.png (linear n*0.5+0.5, white background
    like upstream's renders) and mask_<i>.png (255 inside the silhouette).
    Written top-down via cv2 for the PIL-side runner; img_array rows are
    bottom-up, hence the flips."""
    cam_obj = _normal_setup(clean, rig)
    hires.hide_render = True
    scratch = work_dir / "normal_render.exr"
    for i, (v, depth) in enumerate(zip(views, depths)):
        rgb = _render_exr(cam_obj, v, scratch)[:, :, :3]
        obj = depth > 0.01
        rgb[~obj] = 1.0
        rgb8 = (np.clip(np.flipud(rgb), 0.0, 1.0) * 255.0).round().astype(np.uint8)
        cv2.imwrite(str(work_dir / f"normal_{i}.png"), rgb8[:, :, ::-1])
        cv2.imwrite(str(work_dir / f"mask_{i}.png"),
                    np.flipud(obj).astype(np.uint8) * 255)
    hires.hide_render = False
    scratch.unlink(missing_ok=True)


def view_pairs(n, n_extra=0):
    """Opposite-azimuth pairs under even spacing (front+back, left+right);
    an odd base count leaves its last view solo. The n_extra picked views at
    the tail are not part of that even spacing, so they must not interleave
    with the base pairing — they pair among themselves in pick order."""
    base = n - n_extra
    half = base // 2
    pairs = [(i, i + half) for i in range(half)]
    if base % 2:
        pairs.append((base - 1,))
    for i in range(base, n, 2):
        pairs.append((i, i + 1) if i + 1 < n else (i,))
    return pairs


def generate_views(views, work_dir, subject, seed, n_extra=0):
    """One ControlNet-depth pass per opposite-view pair: both depth maps
    tiled side by side into a single conditioning canvas, the decoded
    canvas split back into per-view crops -> <work_dir>/view_<i>/gen.png.
    Pairs whose crops all exist are skipped, so a killed run resumes.
    Returns per-view provenance entries."""
    template = json.loads(MV_WORKFLOW.read_text(encoding="utf-8"))
    pairs = view_pairs(len(views), n_extra)
    missing = [k for k, pair in enumerate(pairs)
               if not all((work_dir / f"view_{i}" / "gen.png").exists() for i in pair)]
    if missing:
        with comfy_run.server():
            for k in missing:
                pair = pairs[k]
                canvas_dir = work_dir / f"canvas_{k}"
                canvas_dir.mkdir(parents=True, exist_ok=True)
                canvas_png = canvas_dir / "depth.png"
                cv2.imwrite(str(canvas_png), cv2.hconcat(
                    [cv2.imread(str(work_dir / f"depth_{i}.png")) for i in pair]))
                if len(pair) == 1:
                    hint = views[pair[0]]["hint"]
                else:
                    hint = (f"two views of the same object side by side, "
                            f"left: {views[pair[0]]['hint']}, "
                            f"right: {views[pair[1]]['hint']}")
                input_name = f"vordar_mv_{sha256_file(canvas_png)[:8]}_{k}.png"
                shutil.copyfile(canvas_png, comfy_run.COMFY_INPUT_DIR / input_name)
                wf = json.loads(json.dumps(template))
                for node in wf.values():
                    inputs = node.get("inputs", {})
                    # keyed by class_type: node-id keying silently broke
                    # once already (see the seed comment below)
                    if node.get("class_type") == "EmptySD3LatentImage":
                        inputs["width"] = MV_RES * len(pair)
                        inputs["height"] = MV_RES
                    for key, value in inputs.items():
                        if isinstance(value, str):
                            inputs[key] = (value.replace("{subject}", subject)
                                           .replace("{view_hint}", hint)
                                           .replace("{depth_image}", input_name))
                        elif key == "seed":
                            inputs[key] = seed * 100 + k
                manifest = comfy_run.run_workflow(wf, canvas_dir, wait_timeout=300)
                pngs = [o for o in manifest["outputs"] if o["filename"].endswith(".png")]
                if len(pngs) != 1:
                    fail(f"canvas {k}: expected exactly 1 PNG output, got {len(pngs)}")
                gen = cv2.imread(pngs[0]["saved_as"])
                if gen.shape[:2] != (MV_RES, MV_RES * len(pair)):
                    fail(f"canvas {k}: got {gen.shape[1]}x{gen.shape[0]}, "
                         f"expected {MV_RES * len(pair)}x{MV_RES}")
                for slot, i in enumerate(pair):
                    view_dir = work_dir / f"view_{i}"
                    view_dir.mkdir(parents=True, exist_ok=True)
                    cv2.imwrite(str(view_dir / "gen.png"),
                                gen[:, slot * MV_RES:(slot + 1) * MV_RES])
                (comfy_run.COMFY_INPUT_DIR / input_name).unlink()

    pair_of = {i: (k, slot)
               for k, pair in enumerate(pairs) for slot, i in enumerate(pair)}
    metas = []
    for i, v in enumerate(views):
        k, slot = pair_of[i]
        manifest = json.loads((work_dir / f"canvas_{k}" / "manifest.json").read_text(encoding="utf-8"))
        metas.append({
            "hint": v["hint"],
            "azimuth_deg": v["azimuth_deg"],
            "elevation_deg": v["elevation_deg"],
            "canvas": k,
            "canvas_slot": slot,
            # every node's resolved seed: keying one sampler node id silently
            # returned null the moment the workflow graph changed
            "seeds": manifest["seed"],
            "prompt_id": manifest["prompt_id"],
            "prompts": manifest["prompts"],
            "models": manifest["models"],
            "depth_png_sha256": sha256_file(work_dir / f"depth_{i}.png"),
            "gen_png_sha256": sha256_file(work_dir / f"view_{i}" / "gen.png"),
        })
    return metas


def bake_geometry_atlas(clean, atlas, rig):
    """Bake per-texel world position (normalized to mesh bounds), world
    normal (mapped to [0,1]) and an island mask into the atlas."""
    lo = rig["lo"]
    extent = np.maximum(rig["hi"] - lo, 1e-6)

    tree = bake_material(clean)
    n_geo = tree.nodes.new("ShaderNodeNewGeometry")
    n_emit = tree.nodes.new("ShaderNodeEmission")
    n_out = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(n_emit.outputs["Emission"], n_out.inputs["Surface"])

    n_sub = tree.nodes.new("ShaderNodeVectorMath")
    n_sub.operation = "SUBTRACT"
    n_sub.inputs[1].default_value = tuple(lo)
    n_div = tree.nodes.new("ShaderNodeVectorMath")
    n_div.operation = "DIVIDE"
    n_div.inputs[1].default_value = tuple(extent)
    tree.links.new(n_geo.outputs["Position"], n_sub.inputs[0])
    tree.links.new(n_sub.outputs["Vector"], n_div.inputs[0])
    tree.links.new(n_div.outputs["Vector"], n_emit.inputs["Color"])
    pos_img = new_image("mv_pos", srgb=False, fill=(0, 0, 0), float_buffer=True)
    emit_bake(clean, tree, pos_img, atlas)

    n_madd = tree.nodes.new("ShaderNodeVectorMath")
    n_madd.operation = "MULTIPLY_ADD"
    n_madd.inputs[1].default_value = (0.5, 0.5, 0.5)
    n_madd.inputs[2].default_value = (0.5, 0.5, 0.5)
    tree.links.new(n_geo.outputs["Normal"], n_madd.inputs[0])
    tree.links.new(n_madd.outputs["Vector"], n_emit.inputs["Color"])
    nrm_img = new_image("mv_nrm", srgb=False, fill=(0.5, 0.5, 0.5), float_buffer=True)
    emit_bake(clean, tree, nrm_img, atlas)

    n_emit.inputs["Color"].default_value = (1.0, 1.0, 1.0, 1.0)
    for link in list(n_emit.inputs["Color"].links):
        tree.links.remove(link)
    mask_img = new_image("mv_mask", srgb=False, fill=(0, 0, 0))
    emit_bake(clean, tree, mask_img, atlas)

    pos = lo + img_array(pos_img).reshape(-1, 4)[:, :3].astype(np.float64) * extent
    nrm = img_array(nrm_img).reshape(-1, 4)[:, :3].astype(np.float64) * 2.0 - 1.0
    nrm /= np.maximum(np.linalg.norm(nrm, axis=1, keepdims=True), 1e-9)
    island = img_array(mask_img).reshape(-1, 4)[:, 0] > 0.5
    for img in (pos_img, nrm_img, mask_img):
        bpy.data.images.remove(img)
    return pos, nrm, island


def bilinear(arr, px, py):
    """Sample a (h, w) or (h, w, c) array at continuous pixel coords."""
    h, w = arr.shape[:2]
    x = np.clip(px, 0.0, w - 1.0)
    y = np.clip(py, 0.0, h - 1.0)
    x0 = np.clip(np.floor(x).astype(np.int64), 0, w - 2)
    y0 = np.clip(np.floor(y).astype(np.int64), 0, h - 2)
    fx, fy = x - x0, y - y0
    if arr.ndim == 3:
        fx, fy = fx[:, None], fy[:, None]
    a = arr[y0, x0] * (1 - fx) + arr[y0, x0 + 1] * fx
    b = arr[y0 + 1, x0] * (1 - fx) + arr[y0 + 1, x0 + 1] * fx
    return a * (1 - fy) + b * fy


def pad_edges(colors, mask, iterations):
    """Dilate on-mask colors outward so samples just past the silhouette
    read the object's material instead of the generated background."""
    for _ in range(iterations):
        grown = mask.copy()
        for src, dst in (((slice(1, None),), (slice(None, -1),)),
                         ((slice(None, -1),), (slice(1, None),)),
                         ((slice(None), slice(1, None)), (slice(None), slice(None, -1))),
                         ((slice(None), slice(None, -1)), (slice(None), slice(1, None)))):
            fill = mask[src] & ~grown[dst]
            colors[dst][fill] = colors[src][fill]
            grown[dst] |= mask[src]
        mask = grown
    return colors


def view_weight(v, depth, rig, pos, nrm):
    """Per-texel geometric blend weight of one view: squared facing, zeroed
    outside the ortho frustum or occluded by nearer geometry in the view's
    float depth render. A texel on the mesh projects onto its own surface,
    so no silhouette test: only occlusion disqualifies it. Returns
    (weight, px, py); px/py let blend_views sample the generated colors."""
    depth = depth.astype(np.float64)
    h, w = depth.shape
    rel = pos - v["cam"]
    px = ((rel @ v["s"]) / rig["half"] * 0.5 + 0.5) * w - 0.5
    py = ((rel @ v["u"]) / rig["half"] * 0.5 + 0.5) * h - 0.5
    zc = rel @ v["f"]
    inside = (px >= 0) & (px <= w - 1) & (py >= 0) & (py <= h - 1)
    z_rend = v["far"] - bilinear(depth, px, py) * (v["far"] - v["near"])
    visible = zc <= z_rend + MV_OCCLUSION_EPS
    weight = np.maximum(0.0, nrm @ -v["f"]) ** MV_WEIGHT_EXPONENT
    weight *= (inside & visible).astype(np.float64)
    return weight, px, py


def pick_extra_views(clean, hires, views, depths, rig, pos, nrm, island, work_dir):
    """Greedy next-best-view pick over an azimuth/elevation candidate grid:
    coverage is purely geometric (facing/frustum/occlusion, never the
    generated pixels), so extras are picked before any generation and a
    re-run re-derives the same picks — the gen.png resume stays valid."""
    uncovered = island.copy()
    for v, depth in zip(views, depths):
        uncovered &= ~(view_weight(v, depth, rig, pos, nrm)[0] > MV_COVERAGE_EPS)

    specs = [(view_hint(a, e), a, e)
             for e in MV_EXTRA_CANDIDATE_ELEVATIONS
             for a in MV_EXTRA_CANDIDATE_AZIMUTHS]
    specs.append((view_hint(0.0, MV_EXTRA_TOP_ELEVATION), 0.0,
                  MV_EXTRA_TOP_ELEVATION))
    min_dot = cos(radians(MV_EXTRA_MIN_SEP_DEG))
    cands = [(spec, mv_view(*spec, rig)) for spec in specs]
    cands = [(spec, cand) for spec, cand in cands
             if all(float(cand["f"] @ v["f"]) < min_dot for v in views)]

    cam_obj = _depth_setup(clean, rig)
    hires.hide_render = True
    scratch = work_dir / "cand_depth.exr"
    masks = [view_weight(cand, _render_depth(cam_obj, cand, scratch),
                         rig, pos, nrm)[0] > MV_COVERAGE_EPS
             for _, cand in cands]
    hires.hide_render = False
    scratch.unlink(missing_ok=True)

    extra_specs, extra_meta = [], []
    island_total = int(island.sum())
    while len(extra_specs) < MV_EXTRA_MAX and cands:
        gains = [int((m & uncovered).sum()) for m in masks]
        best = int(np.argmax(gains))
        if gains[best] < MV_EXTRA_MIN_GAIN * island_total:
            break
        spec, best_view = cands[best]
        uncovered &= ~masks[best]
        keep = [j for j, (_, cand) in enumerate(cands)
                if j != best and float(cand["f"] @ best_view["f"]) < min_dot]
        cands = [cands[j] for j in keep]
        masks = [masks[j] for j in keep]
        extra_specs.append(spec)
        extra_meta.append({
            "hint": spec[0], "azimuth_deg": spec[1], "elevation_deg": spec[2],
            "predicted_gain_texels": gains[best],
            "predicted_gain_frac": round(gains[best] / max(island_total, 1), 4),
        })
    return extra_specs, extra_meta


def blend_views(views, depths, rig, work_dir, pos, nrm, island,
                filename="albedo.png", srgb=True):
    """Facing-weighted, occlusion-tested blend of one per-view image
    channel set into a flat (N, 4) array. Returns the array, the fractional
    island coverage, and the per-texel covered mask (island texels that
    actually received blended weight; island holes are Telea-inpainted
    from their surroundings, off-island texels keep a mean-color fill).
    srgb=False loads the per-view images as Non-Color data (rm maps):
    Blender would otherwise linearize them on load, and the raw material
    values must survive into the Non-Color atlas unchanged."""
    accum = np.zeros((pos.shape[0], 3))
    wsum = np.zeros(pos.shape[0])
    for i, v in enumerate(views):
        gen_img = bpy.data.images.load(str(work_dir / f"view_{i}" / filename))
        if not srgb:
            gen_img.colorspace_settings.name = "Non-Color"
        gen = img_array(gen_img)[:, :, :3].astype(np.float64)
        bpy.data.images.remove(gen_img)
        gen = pad_edges(gen, depths[i] > 0.01, MV_EDGE_PAD_PX)
        weight, px, py = view_weight(v, depths[i], rig, pos, nrm)
        accum += bilinear(gen, px, py) * weight[:, None]
        wsum += weight

    covered = island & (wsum > MV_COVERAGE_EPS)
    out = np.empty((pos.shape[0], 4), dtype=np.float32)
    out[:, 3] = 1.0
    blended = accum[covered] / wsum[covered, None]
    fill = blended.mean(axis=0) if covered.any() else np.full(3, 0.5)
    out[:, :3] = fill
    out[covered, :3] = blended

    holes = island & ~covered
    if holes.any() and covered.any():
        img8 = (np.clip(out[:, :3], 0.0, 1.0) * 255.0).round().astype(np.uint8)
        img8 = img8.reshape(TEXTURE_SIZE, TEXTURE_SIZE, 3)
        mask8 = holes.reshape(TEXTURE_SIZE, TEXTURE_SIZE).astype(np.uint8)
        filled = cv2.inpaint(img8, mask8, 3, cv2.INPAINT_TELEA)
        # only hole texels take the 8-bit inpaint; covered texels keep
        # their float-precision blend
        out[holes, :3] = filled.reshape(-1, 3)[holes] / 255.0

    coverage = float(covered[island].mean()) if island.any() else 0.0
    return out, coverage, covered


def estimate_materials(views, work_dir, seed):
    """Decompose every view_<i>/gen.png into albedo.png + rm.png via the
    MaterialAnything estimator subprocess (prop_pbr.py in its own venv,
    run only when some output is missing — GPU work resumes like gen.png).
    Its stdout is captured: gen_prop.py parses this stage's single
    '{'-prefixed stats line, which a streamed child JSON line would break."""
    missing = [i for i in range(len(views))
               if not all((work_dir / f"view_{i}" / name).exists()
                          for name in ("albedo.png", "rm.png"))]
    if missing:
        proc = subprocess.run(
            [str(MA_PYTHON), str(SCRIPT_DIR / "prop_pbr.py"), str(work_dir),
             "--views", str(len(views)), "--seed", str(seed)],
            capture_output=True, text=True)
        if proc.returncode != 0:
            sys.stderr.write(proc.stdout + proc.stderr)
            fail("material estimator subprocess failed")
    return json.loads((work_dir / "pbr_meta.json").read_text(encoding="utf-8"))


def pbr_multiview(clean, hires, atlas, subject, seed, work_dir, dielectric=False):
    """Full per-texel PBR set from the multiview path: generate lit views,
    decompose each into albedo + roughness/metallic, blend every channel
    through the same facing-weight machinery. The albedo blend is the
    basecolor (delit — the lit gen.png is conditioning intermediate only);
    the rm blend packs into the glTF G=roughness/B=metallic layout, which
    is the estimator's native output layout. dielectric=True zeroes the
    blended metallic channel post-blend: the estimator has no material-class
    prior, so it can read specular highlights on stone/wood/foliage as
    stray metal (measured: cypress cand_31 metal_fraction 0.403)."""
    work_dir.mkdir(parents=True, exist_ok=True)
    views, rig = mv_camera_rig(clean, MV_VIEWS)
    pos, nrm, island = bake_geometry_atlas(clean, atlas, rig)
    depths = render_depth_views(clean, hires, views, rig, work_dir)
    extra_specs, extra_meta = pick_extra_views(clean, hires, views, depths, rig,
                                               pos, nrm, island, work_dir)
    if extra_specs:
        extra_views = [mv_view(*spec, rig) for spec in extra_specs]
        depths += render_depth_views(clean, hires, extra_views, rig, work_dir,
                                     start=len(views))
        views += extra_views
    render_normal_views(clean, hires, views, depths, rig, work_dir)
    view_metas = generate_views(views, work_dir, subject, seed,
                                n_extra=len(extra_specs))
    pbr_meta = estimate_materials(views, work_dir, seed)
    base_px, coverage, covered = blend_views(views, depths, rig, work_dir,
                                             pos, nrm, island)
    rm_px, _, _ = blend_views(views, depths, rig, work_dir, pos, nrm, island,
                              filename="rm.png", srgb=False)

    base_img = new_image("prop_base", srgb=True, fill=(0, 0, 0))
    base_img.pixels.foreach_set(base_px.ravel())
    mr = np.zeros((pos.shape[0], 4), dtype=np.float32)
    mr[:, 1] = rm_px[:, 1]  # estimator R channel dropped: glTF ignores it
    mr[:, 2] = 0.0 if dielectric else rm_px[:, 2]
    mr[:, 3] = 1.0
    mr_img = new_image("prop_mr", srgb=False, fill=(0, 0, 0))
    mr_img.pixels.foreach_set(mr.ravel())

    extras = {
        "strategy": "multiview_controlnet_depth",
        "front_axis": FRONT_AXIS,
        "workflow": MV_WORKFLOW.name,
        "subject": subject,
        "render_resolution": MV_RES,
        "views": view_metas,
        "extra_views": extra_meta,
        "pbr_estimator": pbr_meta,
        "weight_exponent": MV_WEIGHT_EXPONENT,
        "occlusion_eps": MV_OCCLUSION_EPS,
        "edge_pad_px": MV_EDGE_PAD_PX,
        "depth_dilate_px": MV_DEPTH_DILATE_PX,
        "blend_coverage": round(coverage, 4),
        "hole_texels": int((island & ~covered).sum()),
        "dielectric": dielectric,
        "metal_fraction": round(float((rm_px[island, 2] > 0.5).mean())
                                if island.any() else 0.0, 4),
    }
    return base_img, mr_img, extras


# ---------------------------------------------------------------------------

def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="prop_texture.py")
    parser.add_argument("clean_glb")
    parser.add_argument("hires_glb")
    parser.add_argument("concept_png")
    parser.add_argument("textured_glb")
    parser.add_argument("--strategy", choices=["projection", "multiview"], default="projection")
    parser.add_argument("--subject", help="Prompt substituted into the multiview workflow's {subject}")
    parser.add_argument("--seed", type=int, help="Base seed for the multiview passes")
    parser.add_argument("--metallic", type=float, default=DEFAULT_METALLIC,
                        help="Declared metallic constant, projection strategy only "
                             "(default 0: stone/wood/cloth/skin); multiview estimates "
                             "per-texel MR instead")
    parser.add_argument("--roughness", type=float, default=DEFAULT_ROUGHNESS,
                        help="Declared roughness constant, projection strategy only "
                             "(default 0.8)")
    parser.add_argument("--azimuths", default=None, metavar="DEG,DEG,...",
                        help="Multiview camera azimuths (default 0,90,180,270); "
                             "use oblique sides, e.g. 0,60,180,300, for planar props")
    parser.add_argument("--view-res", type=int, default=None,
                        help="Multiview strategy only: per-view depth/normal render and "
                             "Z-Image + Fun ControlNet-Union generation resolution (default "
                             "1024). The MaterialAnything estimator's input stays pinned at "
                             "768x768 regardless (prop_pbr.py downscales/upscales around it) "
                             "-- this only sharpens the source imagery blended into the atlas.")
    parser.add_argument("--texture-size", type=int, default=None,
                        help="Atlas bake resolution for basecolor/normal/MR (default 1024, "
                             "TEXTURE_SIZE). A value above the resolution prop_cleanup.py's "
                             "xatlas packed the atlas for is safe -- island gutters only grow, "
                             "never shrink -- so re-running cleanup is not required to raise "
                             "this.")
    parser.add_argument("--dielectric", action="store_true",
                        help="Multiview strategy only: zero the estimated metallic "
                             "channel post-blend. Explicit opt-in per run for prop "
                             "classes declared non-metal (stone/wood/foliage) -- the "
                             "estimator has no material-class prior and can read "
                             "specular highlights as stray metal.")
    args = parser.parse_args(argv)

    if args.azimuths is not None:
        global MV_VIEWS
        MV_VIEWS = [(view_hint(a), a, MV_ELEVATION_DEG)
                    for a in (float(s) for s in args.azimuths.split(","))]
    if args.view_res is not None:
        global MV_RES
        MV_RES = args.view_res
    if args.texture_size is not None:
        global TEXTURE_SIZE
        TEXTURE_SIZE = args.texture_size

    t0 = time.time()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    clean = import_glb(args.clean_glb)
    hires = import_glb(args.hires_glb)

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 1
    scene.cycles.use_denoising = False

    # ---- atlas: unwrapped once by prop_cleanup (xatlas), consumed here ----
    me = clean.data
    if not me.uv_layers:
        fail("clean mesh carries no UV atlas — re-run prop_cleanup")
    atlas = me.uv_layers[0].name

    # ---- basecolor (+ per-texel MR on the multiview path), per strategy ----
    mr_img = None
    if args.strategy == "projection":
        base_img, extras = basecolor_projection(clean, atlas, args.concept_png)
    else:
        if not args.subject or args.seed is None:
            fail("--strategy multiview requires --subject and --seed")
        work_dir = Path(args.textured_glb).resolve().parent / "multiview"
        base_img, mr_img, extras = pbr_multiview(clean, hires, atlas, args.subject,
                                                 args.seed, work_dir, args.dielectric)
    t_base = time.time()

    # ---- normal: real high-to-low bake from the hires mesh ----
    normal_img = new_image("prop_normal", srgb=False, fill=(0.5, 0.5, 1.0))
    tree = bake_material(clean)
    n_bake = tree.nodes.new("ShaderNodeTexImage")
    n_bake.image = normal_img
    tree.nodes.active = n_bake
    select_only([clean, hires], clean)
    bpy.ops.object.bake(type="NORMAL", normal_space="TANGENT",
                        use_selected_to_active=True, cage_extrusion=0.01,
                        max_ray_distance=0.03, margin=8, use_clear=True,
                        uv_layer=atlas)
    t_normal = time.time()

    # ---- final material + export (hires dropped, images saved so the
    #      glTF exporter embeds them) ----
    mr_stats = ({} if mr_img is not None
                else {"metallic": args.metallic, "roughness": args.roughness})
    tmpdir = tempfile.TemporaryDirectory()
    images_to_save = [(base_img, "base.png"), (normal_img, "normal.png")]
    if mr_img is not None:
        images_to_save.append((mr_img, "mr.png"))
    for img, name in images_to_save:
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
    if mr_img is not None:
        n_mr = tree.nodes.new("ShaderNodeTexImage")
        n_mr.image = mr_img
        n_sep = tree.nodes.new("ShaderNodeSeparateColor")
        tree.links.new(n_mr.outputs["Color"], n_sep.inputs["Color"])
        tree.links.new(n_sep.outputs["Green"], bsdf.inputs["Roughness"])
        tree.links.new(n_sep.outputs["Blue"], bsdf.inputs["Metallic"])
        # metallicFactor/roughnessFactor multiply the packed texture in the
        # glTF spec, so both must be 1.0 for the texture's values to carry
        # through to the exporter unscaled.
        bsdf.inputs["Metallic"].default_value = 1.0
        bsdf.inputs["Roughness"].default_value = 1.0
    else:
        # MR is uniform, so it rides the glTF scalar factors instead of a map.
        bsdf.inputs["Metallic"].default_value = args.metallic
        bsdf.inputs["Roughness"].default_value = args.roughness
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
        **extras,
        "texture_size": TEXTURE_SIZE,
        **mr_stats,
        "base_bake_s": round(t_base - t0, 1),
        "normal_bake_s": round(t_normal - t_base, 1),
        "textured_glb": args.textured_glb,
    }
    print(json.dumps(stats))


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
