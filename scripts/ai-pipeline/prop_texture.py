# Blender-headless: texture a cleaned Hi3DGen prop mesh (Phase A3.6; both
# strategy rulings in tasks/ai-pipeline/a3.md -> "Texture strategy log").
#
# Two basecolor strategies share one channels contract:
#   - projection (default): Smart-UV atlas, concept image planar-projected
#     along the concept view axis and EMIT-baked into basecolor. The
#     projection passes through the mesh, so back faces receive the
#     mirrored front read -- the near-symmetric prop classes this pipeline
#     targets survive that.
#   - multiview (escalation for prop classes needing true material register
#     or strong backsides): ortho depth renders of the clean mesh from
#     MV_VIEWS cameras feed a ControlNet-depth text-to-image base via a ComfyUI
#     server whose lifecycle lives entirely inside this stage; the
#     generated views are reprojected into the atlas and blended with
#     facing weights, depth-occlusion and silhouette tests.
#   - normal map = real high-to-low Cycles bake from <hires.glb> onto the
#     atlas UVs (prop_cleanup.py keeps both meshes rigidly aligned).
#   - MR defaults to two declared constants (--metallic/--roughness), carried
#     by the glTF scalar factors rather than a map. --mr-mask escalates to a
#     per-texel mask: a second depth-conditioned multiview pass renders the
#     prop as a two-tone material-ID image (metal near-white, everything else
#     near-black) through the same camera rig/blend machinery as the
#     basecolor pass, and this stage smoothsteps that render's luma into
#     metallic/roughness and packs them into a glTF metallicRoughnessTexture.
#     Classifying material from the BASECOLOR value was tried and retired
#     (A6.1, tasks/ai-pipeline/research/a6-1-mr-contract.md): luma conflates
#     albedo, shading and material, so the dominant split in a dark render is
#     lit-vs-shadowed, not iron-vs-wax. A dedicated mask render sidesteps
#     that -- the generation prompt states the material split directly.
#   - prints one JSON stats line (the only '{'-prefixed stdout line) for
#     gen_prop.py's chained manifest.
#
# Usage: blender --background --python prop_texture.py -- \
#            <clean.glb> <hires.glb> <concept.png> <textured.glb> \
#            [--strategy projection|multiview] [--subject STR] [--seed N] \
#            [--metallic F] [--roughness F] \
#            [--mr-mask STR] [--metal-roughness F]

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import time
import traceback
import urllib.request
from math import cos, radians, sin
from pathlib import Path

import bpy
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
# Declared MR defaults: non-metal, matching every existing race model and the
# stone/wood/cloth props the art direction calls for. Metal props override.
DEFAULT_METALLIC = 0.0
DEFAULT_ROUGHNESS = 0.8
# Roughness assigned to mask-classified metal texels (--mr-mask only).
DEFAULT_METAL_ROUGHNESS = 0.65
# Mask-pass classification band: a narrow smoothstep straddling the two-tone
# render's midpoint, wide enough to absorb generation/blend noise near the
# metal/non-metal boundary without misreading it as a third material.
MR_MASK_SMOOTHSTEP_EDGES = (0.35, 0.65)

MV_WORKFLOW = SCRIPT_DIR / "workflows" / "prop_multiview.json"


def view_hint(az_deg):
    a = az_deg % 360.0
    if a <= 30 or a >= 330:
        return "front view"
    if 150 <= a <= 210:
        return "back view"
    if 80 <= a <= 100 or 260 <= a <= 280:
        return "side view"
    return "three-quarter view"


# Rebound in main() when --azimuths is passed: planar props degenerate to a
# sliver in exact side depth views, which frees the base to hallucinate an
# unrelated object into them — oblique azimuths keep the conditioning real.
MV_VIEWS = [(view_hint(a), a) for a in (0.0, 90.0, 180.0, 270.0)]
MV_ELEVATION_DEG = 15.0
MV_RES = 1024
MV_WEIGHT_EXPONENT = 2.0
MV_OCCLUSION_EPS = 0.02  # meters; props are ~1.8 m tall (prop_cleanup)
MV_EDGE_PAD_PX = 8  # the base bleeds background across the depth edge; padding
# the object's colors outward keeps edge texels on-material without
# shrinking thin members (erosion would erase a ~6 px scroll arm entirely)
COMFY_PYTHON = Path(r"C:\tools\ComfyUI\python_embeded\python.exe")
COMFY_MAIN = Path(r"C:\tools\ComfyUI\ComfyUI\main.py")
COMFY_INPUT_DIR = Path(r"C:\tools\ComfyUI\ComfyUI\input")


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


def new_image(name, srgb, fill, float_buffer=False, size=TEXTURE_SIZE):
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

def mv_camera_rig(clean):
    me = clean.data
    co = np.empty(len(me.vertices) * 3, dtype=np.float32)
    me.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    lo, hi = co.min(axis=0), co.max(axis=0)
    center = (lo + hi) / 2
    radius = float(np.linalg.norm(hi - lo) / 2) * 1.05
    dist = 2.0 * radius
    el = radians(MV_ELEVATION_DEG)
    views = []
    for hint, az_deg in MV_VIEWS:
        az = radians(az_deg)
        d = np.array([sin(az) * cos(el), -cos(az) * cos(el), sin(el)])  # center -> camera
        f = -d  # camera forward
        s = np.cross(f, [0.0, 0.0, 1.0])
        s /= np.linalg.norm(s)
        u = np.cross(s, f)
        views.append({
            "hint": hint, "azimuth_deg": az_deg,
            "cam": center + d * dist, "f": f, "s": s, "u": u,
            "near": dist - radius, "far": dist + radius,
        })
    rig = {"lo": lo, "hi": hi, "half": radius, "ortho_scale": 2.0 * radius}
    return views, rig


def render_depth_views(clean, hires, views, rig, work_dir):
    """Render each view's ortho depth ramp (near=1, far=0): float array for
    reprojection, plus an 8-bit PNG for the ControlNet conditioning input."""
    scene = bpy.context.scene
    scene.render.resolution_x = scene.render.resolution_y = MV_RES
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_depth = "32"

    tree = bake_material(clean)
    n_cam = tree.nodes.new("ShaderNodeCameraData")
    n_map = tree.nodes.new("ShaderNodeMapRange")
    n_map.clamp = True
    n_map.inputs["From Min"].default_value = views[0]["near"]
    n_map.inputs["From Max"].default_value = views[0]["far"]
    n_map.inputs["To Min"].default_value = 1.0
    n_map.inputs["To Max"].default_value = 0.0
    n_emit = tree.nodes.new("ShaderNodeEmission")
    n_out = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(n_cam.outputs["View Z Depth"], n_map.inputs["Value"])
    tree.links.new(n_map.outputs["Result"], n_emit.inputs["Color"])
    tree.links.new(n_emit.outputs["Emission"], n_out.inputs["Surface"])

    cam_data = bpy.data.cameras.new("mv_cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = rig["ortho_scale"]
    cam_data.clip_start = views[0]["near"] * 0.5
    cam_data.clip_end = views[0]["far"] * 2.0
    cam_obj = bpy.data.objects.new("mv_cam", cam_data)
    scene.collection.objects.link(cam_obj)
    scene.camera = cam_obj

    hires.hide_render = True
    depths = []
    for i, v in enumerate(views):
        cam_obj.location = Vector(v["cam"])
        cam_obj.rotation_euler = Vector(v["f"]).to_track_quat("-Z", "Y").to_euler()
        exr_path = work_dir / f"depth_{i}.exr"
        scene.render.filepath = str(exr_path)
        bpy.ops.render.render(write_still=True)
        exr = bpy.data.images.load(str(exr_path))
        depth = img_array(exr)[:, :, 0].copy()
        bpy.data.images.remove(exr)
        depths.append(depth)

        png = bpy.data.images.new(f"mv_depth_{i}", MV_RES, MV_RES)
        png.colorspace_settings.name = "Non-Color"
        rgba = np.empty((MV_RES * MV_RES, 4), dtype=np.float32)
        rgba[:, 0] = rgba[:, 1] = rgba[:, 2] = depth.ravel()
        rgba[:, 3] = 1.0
        png.pixels.foreach_set(rgba.ravel())
        png.filepath_raw = str(work_dir / f"depth_{i}.png")
        png.file_format = "PNG"
        png.save()
        bpy.data.images.remove(png)
    hires.hide_render = False
    return depths


def comfy_reachable():
    try:
        urllib.request.urlopen(f"{comfy_run.COMFY_URL}/system_stats", timeout=2)
        return True
    except Exception:
        return False


def start_comfy():
    if comfy_reachable():
        fail("a ComfyUI server is already running -- this stage owns the server "
             "lifecycle (A3 VRAM sequencing); stop the external one first")
    proc = subprocess.Popen(
        [str(COMFY_PYTHON), "-s", str(COMFY_MAIN), "--listen", "127.0.0.1", "--port", "8188"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            fail(f"ComfyUI exited during startup (code {proc.returncode})")
        if comfy_reachable():
            return proc
        time.sleep(1)
    proc.kill()
    fail("ComfyUI not ready after 120 s")


def generate_views(views, work_dir, subject, seed):
    """One ControlNet-depth pass per view -> <work_dir>/view_<i>/gen.png.
    Views whose gen.png already exists are skipped, so a killed run resumes.
    Returns per-view provenance entries."""
    template = json.loads(MV_WORKFLOW.read_text(encoding="utf-8"))
    missing = [i for i in range(len(views)) if not (work_dir / f"view_{i}" / "gen.png").exists()]
    if missing:
        proc = start_comfy()
        try:
            for i in missing:
                depth_png = work_dir / f"depth_{i}.png"
                input_name = f"vordar_mv_{sha256_file(depth_png)[:8]}_{i}.png"
                shutil.copyfile(depth_png, COMFY_INPUT_DIR / input_name)
                wf = json.loads(json.dumps(template))
                for node in wf.values():
                    inputs = node.get("inputs", {})
                    for key, value in inputs.items():
                        if isinstance(value, str):
                            inputs[key] = (value.replace("{subject}", subject)
                                           .replace("{view_hint}", views[i]["hint"])
                                           .replace("{depth_image}", input_name))
                        elif key == "seed":
                            inputs[key] = seed * 100 + i
                view_dir = work_dir / f"view_{i}"
                manifest = comfy_run.run_workflow(wf, view_dir, wait_timeout=300)
                pngs = [o for o in manifest["outputs"] if o["filename"].endswith(".png")]
                if len(pngs) != 1:
                    fail(f"view {i}: expected exactly 1 PNG output, got {len(pngs)}")
                shutil.copyfile(pngs[0]["saved_as"], view_dir / "gen.png")
                (COMFY_INPUT_DIR / input_name).unlink()
        finally:
            proc.kill()
            proc.wait()

    metas = []
    for i, v in enumerate(views):
        manifest = json.loads((work_dir / f"view_{i}" / "manifest.json").read_text(encoding="utf-8"))
        metas.append({
            "hint": v["hint"],
            "azimuth_deg": v["azimuth_deg"],
            "elevation_deg": MV_ELEVATION_DEG,
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


def blend_views(views, depths, rig, work_dir, pos, nrm, island):
    """Facing-weighted, occlusion- and silhouette-tested blend of the
    generated views into a flat (N, 4) sRGB basecolor array. Returns the
    array, the fractional island coverage, and the per-texel covered mask
    (island texels that actually received blended weight, as opposed to the
    mean-color fill applied to the rest of the array)."""
    half = rig["half"]
    accum = np.zeros((pos.shape[0], 3))
    wsum = np.zeros(pos.shape[0])
    for i, v in enumerate(views):
        gen_img = bpy.data.images.load(str(work_dir / f"view_{i}" / "gen.png"))
        gen = img_array(gen_img)[:, :, :3].astype(np.float64)
        bpy.data.images.remove(gen_img)
        depth = depths[i].astype(np.float64)
        h, w = depth.shape
        gen = pad_edges(gen, depth > 0.01, MV_EDGE_PAD_PX)

        rel = pos - v["cam"]
        px = ((rel @ v["s"]) / half * 0.5 + 0.5) * w - 0.5
        py = ((rel @ v["u"]) / half * 0.5 + 0.5) * h - 0.5
        zc = rel @ v["f"]
        inside = (px >= 0) & (px <= w - 1) & (py >= 0) & (py <= h - 1)

        # a texel on the mesh projects onto its own surface, so no
        # silhouette test: only nearer geometry (occlusion) disqualifies it
        z_rend = v["far"] - bilinear(depth, px, py) * (v["far"] - v["near"])
        visible = zc <= z_rend + MV_OCCLUSION_EPS

        weight = np.maximum(0.0, nrm @ -v["f"]) ** MV_WEIGHT_EXPONENT
        weight *= (inside & visible).astype(np.float64)
        accum += bilinear(gen, px, py) * weight[:, None]
        wsum += weight

    covered = island & (wsum > 1e-4)
    out = np.empty((pos.shape[0], 4), dtype=np.float32)
    out[:, 3] = 1.0
    blended = accum[covered] / wsum[covered, None]
    fill = blended.mean(axis=0) if covered.any() else np.full(3, 0.5)
    out[:, :3] = fill
    out[covered, :3] = blended
    coverage = float(covered[island].mean()) if island.any() else 0.0
    return out, coverage, covered


def basecolor_multiview(clean, hires, atlas, subject, seed, work_dir):
    work_dir.mkdir(parents=True, exist_ok=True)
    views, rig = mv_camera_rig(clean)
    depths = render_depth_views(clean, hires, views, rig, work_dir)
    view_metas = generate_views(views, work_dir, subject, seed)
    pos, nrm, island = bake_geometry_atlas(clean, atlas, rig)
    base_px, coverage, _ = blend_views(views, depths, rig, work_dir, pos, nrm, island)

    base_img = new_image("prop_base", srgb=True, fill=(0, 0, 0))
    base_img.pixels.foreach_set(base_px.ravel())
    extras = {
        "strategy": "multiview_controlnet_depth",
        "front_axis": FRONT_AXIS,
        "workflow": MV_WORKFLOW.name,
        "subject": subject,
        "render_resolution": MV_RES,
        "views": view_metas,
        "weight_exponent": MV_WEIGHT_EXPONENT,
        "occlusion_eps": MV_OCCLUSION_EPS,
        "edge_pad_px": MV_EDGE_PAD_PX,
        "blend_coverage": round(coverage, 4),
    }
    return base_img, extras


def mr_multiview(clean, hires, atlas, mask_subject, seed, work_dir, metal_roughness, roughness):
    """Second multiview pass, independent cache from the basecolor one: same
    camera rig/depth-conditioned generation/blend machinery, but the prompt
    renders a two-tone material-ID image instead of the prop's basecolor.
    The blend's luma smoothsteps into a per-texel metallic mask; texels the
    blend never covered (see blend_views) default to fully dielectric rather
    than inheriting its mean-color fill, which carries no material meaning."""
    work_dir.mkdir(parents=True, exist_ok=True)
    views, rig = mv_camera_rig(clean)
    depths = render_depth_views(clean, hires, views, rig, work_dir)
    view_metas = generate_views(views, work_dir, mask_subject, seed)
    pos, nrm, island = bake_geometry_atlas(clean, atlas, rig)
    mask_px, coverage, covered = blend_views(views, depths, rig, work_dir, pos, nrm, island)

    lo, hi = MR_MASK_SMOOTHSTEP_EDGES
    lum = 0.2126 * mask_px[:, 0] + 0.7152 * mask_px[:, 1] + 0.0722 * mask_px[:, 2]
    t = np.clip((lum - lo) / (hi - lo), 0.0, 1.0)
    metallic = t * t * (3.0 - 2.0 * t)  # smoothstep: near-white mask = metal
    metallic[~covered] = 0.0  # dielectric default, not the mean-color fill

    mr = np.zeros((pos.shape[0], 4), dtype=np.float32)
    mr[:, 1] = metal_roughness * metallic + roughness * (1.0 - metallic)
    mr[:, 2] = metallic
    mr[:, 3] = 1.0
    mr_img = new_image("prop_mr", srgb=False, fill=(0, 0, 0))
    mr_img.pixels.foreach_set(mr.ravel())

    extras = {
        "mask_subject": mask_subject,
        "workflow": MV_WORKFLOW.name,
        "views": view_metas,
        "blend_coverage": round(coverage, 4),
        "metal_fraction": round(float((metallic[island] > 0.5).mean()) if island.any() else 0.0, 4),
    }
    return mr_img, extras


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
                        help="Declared metallic constant (default 0: stone/wood/cloth/skin)")
    parser.add_argument("--roughness", type=float, default=DEFAULT_ROUGHNESS,
                        help="Declared roughness constant (default 0.8)")
    parser.add_argument("--mr-mask", metavar="SUBJECT",
                        help="Prompt for a second multiview pass rendering a two-tone "
                             "metal/non-metal material-ID image; presence escalates MR "
                             "from the declared scalar factors to a per-texel packed "
                             "metallicRoughnessTexture")
    parser.add_argument("--metal-roughness", type=float, default=DEFAULT_METAL_ROUGHNESS,
                        help="Roughness assigned to mask-classified metal texels (default 0.65)")
    parser.add_argument("--azimuths", default=None, metavar="DEG,DEG,...",
                        help="Multiview camera azimuths (default 0,90,180,270); "
                             "use oblique sides, e.g. 0,60,180,300, for planar props")
    args = parser.parse_args(argv)

    if args.azimuths is not None:
        global MV_VIEWS
        MV_VIEWS = [(view_hint(a), a) for a in (float(s) for s in args.azimuths.split(","))]

    t0 = time.time()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    clean = import_glb(args.clean_glb)
    hires = import_glb(args.hires_glb)

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 1
    scene.cycles.use_denoising = False

    # ---- atlas unwrap (the mesh arrives without UVs) ----
    select_only([clean], clean)
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=radians(66), island_margin=0.003)
    bpy.ops.object.mode_set(mode="OBJECT")
    me = clean.data
    atlas = me.uv_layers[0].name

    # ---- basecolor, per strategy ----
    if args.strategy == "projection":
        base_img, extras = basecolor_projection(clean, atlas, args.concept_png)
    else:
        if not args.subject or args.seed is None:
            fail("--strategy multiview requires --subject and --seed")
        work_dir = Path(args.textured_glb).resolve().parent / "multiview"
        base_img, extras = basecolor_multiview(clean, hires, atlas,
                                               args.subject, args.seed, work_dir)
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

    # ---- MR mask: optional second multiview pass classifying per-texel
    #      material identity (dead without --mr-mask) ----
    mr_img = None
    mr_mask_extras = None
    if args.mr_mask:
        if args.seed is None:
            fail("--mr-mask requires --seed")
        mr_work_dir = Path(args.textured_glb).resolve().parent / "multiview_mr"
        mr_img, mr_mask_extras = mr_multiview(clean, hires, atlas, args.mr_mask, args.seed,
                                              mr_work_dir, args.metal_roughness, args.roughness)
    t_mr = time.time()

    # ---- final material + export (hires dropped, images saved so the
    #      glTF exporter embeds them) ----
    mr_stats = {"metallic": args.metallic, "roughness": args.roughness}
    if mr_img is not None:
        mr_stats["metal_roughness"] = args.metal_roughness
        mr_stats["mr_mask"] = mr_mask_extras
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
    if mr_img is not None:
        stats["mr_mask_bake_s"] = round(t_mr - t_normal, 1)
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
