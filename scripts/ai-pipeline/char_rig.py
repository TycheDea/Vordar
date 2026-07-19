# Blender-headless: transplant the canonical Mixamo skeleton onto a
# generated character mesh -> one engine-ready .glb (Phase A4.4).
#
#   - the Character.fbx armature is kept EXACTLY as imported (units
#     invariant: the engine's weapon-socket path bakes the FBX cm scale,
#     client/vordar-client/src/weapons.rs:204 — applying or clearing the
#     armature transform breaks sockets silently); only the MESH is moved
#     into the armature's space, by uniform scale + translation fitted to
#     the skeleton's own T-pose landmarks (crown/ground/hips) — the
#     openpose conditioning lock makes this near-rigid, and the stats
#     record the shoulder-band span residual as A4.11 decision data if it
#     wasn't
#   - skin weights come from Blender's automatic weights (bone heat);
#     rig-quality gate: weightless verts > 0.5%, or a bone-heat solver
#     error, is a structural failure -> exit non-zero WITH the numbers (a
#     failed candidate is A4.11 decision-gate data, never silently
#     patched). One internal fallback for the weight computation only: a
#     voxel-remeshed proxy is weighted and its weights transferred to the
#     real mesh (no shipped geometry changes; recorded in the stats)
#   - finger prune, socket bones, clip stash, height/ground bake, and glb
#     export settings are mixamo_rig.py, shared with mixamo_to_glb.py
#   - prints one JSON stats line (the only '{'-prefixed stdout line) for
#     the chained generation manifest
#
# Usage: blender --background --python char_rig.py -- \
#            <textured.glb> <Character.fbx> <clips_dir> <out.glb> [--height M]

import argparse
import json
import os
import sys
import tempfile
import traceback
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR.parent / "asset-pipeline"))
import mixamo_rig  # noqa: E402

BONE_PREFIX = "mixamorig:"
# Landmark bands are cut at ±5% of skeleton height around a joint — wide
# enough to always catch torso/leg cross-sections on a full-body mesh,
# narrow enough that a hips band never reaches the shoulders.
BAND_FRACTION = 0.05
# A vertex whose dominant joint sits further than this from it (metres,
# point-to-bone-segment) is counted as weight bleed: 30 cm exceeds any
# plausible limb radius at humanoid scale.
BLEED_DISTANCE = 0.30
WEIGHTLESS_LIMIT_FRACTION = 0.005
# Weighting-proxy voxel size: ~2 cm closes generation-scale surface
# noise/self-intersections while keeping limbs from fusing to the torso.
PROXY_VOXEL_SIZE = 0.02
SOLVER_ERROR_TEXT = "failed to find solution"


def fail(msg):
    print(f"char_rig: {msg}", file=sys.stderr)
    sys.exit(1)


def run_captured(fn) -> str:
    """Run fn with stdout+stderr redirected at the fd level (Blender
    operator reports print from C, past sys.stdout), echo the text back,
    and return it — the only way to capture bone-heat solver warnings
    headless."""
    sys.stdout.flush()
    sys.stderr.flush()
    saved = os.dup(1), os.dup(2)
    with tempfile.TemporaryFile(mode="w+b") as tmp:
        os.dup2(tmp.fileno(), 1)
        os.dup2(tmp.fileno(), 2)
        try:
            fn()
        finally:
            sys.stdout.flush()
            sys.stderr.flush()
            os.dup2(saved[0], 1)
            os.dup2(saved[1], 2)
            os.close(saved[0])
            os.close(saved[1])
        tmp.seek(0)
        text = tmp.read().decode("utf-8", "replace")
    sys.stdout.write(text)
    return text


def vert_coords(me):
    co = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", co)
    return co.reshape(-1, 3)


def joint_head(armature, name):
    return armature.matrix_world @ armature.data.bones[BONE_PREFIX + name].head_local


def select_only(objs, active):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = active


def auto_parent(mesh_obj, armature) -> str:
    select_only([mesh_obj, armature], armature)
    return run_captured(lambda: bpy.ops.object.parent_set(type="ARMATURE_AUTO"))


def solver_warnings(text) -> list:
    return [line.strip() for line in text.splitlines()
            if "Bone Heat" in line or SOLVER_ERROR_TEXT in line]


def dist_point_segment(p, a, b) -> float:
    ab = b - a
    denom = ab.length_squared
    t = 0.0 if denom == 0.0 else max(0.0, min(1.0, (p - a).dot(ab) / denom))
    return (p - (a + ab * t)).length


def weight_metrics(mesh_obj, armature):
    """(weightless count, bleed count): verts with no deform-bone weight,
    and verts whose dominant joint is over BLEED_DISTANCE away."""
    mw = armature.matrix_world
    segments = {b.name: (mw @ b.head_local, mw @ b.tail_local)
                for b in armature.data.bones if b.use_deform}
    group_bone = {g.index: g.name for g in mesh_obj.vertex_groups
                  if g.name in segments}
    mesh_mw = mesh_obj.matrix_world
    weightless = 0
    bleed = 0
    for v in mesh_obj.data.vertices:
        total, best_w, best_g = 0.0, 0.0, None
        for vg in v.groups:
            if vg.group in group_bone:
                total += vg.weight
                if vg.weight > best_w:
                    best_w, best_g = vg.weight, vg.group
        if total <= 1e-8:
            weightless += 1
            continue
        head, tail = segments[group_bone[best_g]]
        if dist_point_segment(mesh_mw @ v.co, head, tail) > BLEED_DISTANCE:
            bleed += 1
    return weightless, bleed


def structural_failure(warnings, weightless, vert_count) -> bool:
    return any(SOLVER_ERROR_TEXT in w for w in warnings) \
        or weightless > WEIGHTLESS_LIMIT_FRACTION * vert_count


def unbind(mesh_obj):
    for mod in [m for m in mesh_obj.modifiers if m.type == "ARMATURE"]:
        mesh_obj.modifiers.remove(mod)
    world = mesh_obj.matrix_world.copy()
    mesh_obj.parent = None
    mesh_obj.matrix_world = world
    mesh_obj.vertex_groups.clear()


def proxy_weights(mesh_obj, armature) -> str:
    """Weight a voxel-remeshed copy, transfer its weights to the real mesh
    by nearest vertex, and bind the real mesh without regenerating weights.
    Shipped geometry is untouched."""
    proxy = mesh_obj.copy()
    proxy.data = mesh_obj.data.copy()
    bpy.context.collection.objects.link(proxy)
    mod = proxy.modifiers.new("voxel", "REMESH")
    mod.mode = "VOXEL"
    mod.voxel_size = PROXY_VOXEL_SIZE
    select_only([proxy], proxy)
    bpy.ops.object.modifier_apply(modifier=mod.name)

    text = auto_parent(proxy, armature)

    dt = mesh_obj.modifiers.new("weights", "DATA_TRANSFER")
    dt.object = proxy
    dt.use_vert_data = True
    dt.data_types_verts = {"VGROUP_WEIGHTS"}
    dt.vert_mapping = "NEAREST"
    select_only([mesh_obj], mesh_obj)
    bpy.ops.object.datalayout_transfer(modifier=dt.name)
    bpy.ops.object.modifier_apply(modifier=dt.name)
    bpy.data.objects.remove(proxy, do_unlink=True)

    select_only([mesh_obj, armature], armature)
    bpy.ops.object.parent_set(type="ARMATURE")
    return text


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="char_rig.py")
    parser.add_argument("textured_glb")
    parser.add_argument("character_fbx")
    parser.add_argument("clips_dir")
    parser.add_argument("out_glb")
    parser.add_argument("--height", type=float, default=mixamo_rig.TARGET_HEIGHT)
    args = parser.parse_args(argv)

    mixamo_rig.new_scene()

    # ---- canonical skeleton in, its own meshes out ----
    bpy.ops.import_scene.fbx(filepath=args.character_fbx)
    armatures = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    if len(armatures) != 1:
        fail(f"expected exactly 1 armature in {args.character_fbx}, "
             f"found {len(armatures)}")
    armature = armatures[0]
    for o in [o for o in bpy.context.scene.objects if o.type == "MESH"]:
        bpy.data.objects.remove(o, do_unlink=True)

    # ---- generated mesh in: one object, transforms baked into verts ----
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.gltf(filepath=args.textured_glb)
    imported = set(bpy.context.scene.objects) - before
    meshes = [o for o in imported if o.type == "MESH"]
    extras = [o for o in imported if o.type != "MESH"]
    if not meshes:
        fail(f"no mesh in {args.textured_glb}")
    select_only(meshes, meshes[0])
    if len(meshes) > 1:
        bpy.ops.object.join()
    mesh_obj = bpy.context.view_layer.objects.active
    world = mesh_obj.matrix_world.copy()
    mesh_obj.parent = None
    mesh_obj.matrix_world = world
    for o in extras:
        bpy.data.objects.remove(o, do_unlink=True)
    select_only([mesh_obj], mesh_obj)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    me = mesh_obj.data

    # ---- fit the mesh into the armature's space (uniform scale + move) ----
    crown_z = joint_head(armature, "HeadTop_End").z
    ground_z = min((armature.matrix_world @ b.head_local).z
                   for b in armature.data.bones)
    skel_height = crown_z - ground_z
    hips = joint_head(armature, "Hips")
    shoulder_z = (joint_head(armature, "LeftArm").z
                  + joint_head(armature, "RightArm").z) / 2.0
    skel_span = abs(joint_head(armature, "LeftHand").x
                    - joint_head(armature, "RightHand").x)

    co = vert_coords(me)
    mesh_height = float(co[:, 2].max() - co[:, 2].min())
    if mesh_height < 1e-6:
        fail("degenerate mesh: zero height")
    fit_scale = skel_height / mesh_height
    me.transform(Matrix.Diagonal((fit_scale, fit_scale, fit_scale, 1.0)))

    co = vert_coords(me)
    me.transform(Matrix.Translation((0.0, 0.0, ground_z - float(co[:, 2].min()))))
    co = vert_coords(me)
    band = co[np.abs(co[:, 2] - hips.z) <= BAND_FRACTION * skel_height]
    if len(band) == 0:
        fail("no mesh cross-section at hips height — fit landmarks unusable")
    cx = float(band[:, 0].min() + band[:, 0].max()) / 2.0
    cy = float(band[:, 1].min() + band[:, 1].max()) / 2.0
    me.transform(Matrix.Translation((hips.x - cx, hips.y - cy, 0.0)))

    co = vert_coords(me)
    shoulder_band = co[np.abs(co[:, 2] - shoulder_z) <= BAND_FRACTION * skel_height]
    mesh_span = float(shoulder_band[:, 0].max() - shoulder_band[:, 0].min()) \
        if len(shoulder_band) else 0.0

    # ---- automatic weights + rig-quality gate ----
    vert_count = len(me.vertices)
    warnings = solver_warnings(auto_parent(mesh_obj, armature))
    weightless, bleed = weight_metrics(mesh_obj, armature)
    proxy_used = False
    if structural_failure(warnings, weightless, vert_count):
        proxy_used = True
        unbind(mesh_obj)
        warnings += solver_warnings(proxy_weights(mesh_obj, armature))
        weightless, bleed = weight_metrics(mesh_obj, armature)

    stats = {
        "verts": vert_count,
        "fit_scale": fit_scale,
        "skeleton_height": skel_height,
        "shoulder_span_mesh": mesh_span,
        "shoulder_span_skeleton": skel_span,
        "weightless_verts": weightless,
        "weightless_fraction": weightless / vert_count,
        "bleed_verts_over_30cm": bleed,
        "solver_warnings": warnings,
        "weight_proxy_used": proxy_used,
    }
    if proxy_used and structural_failure(warnings, weightless, vert_count):
        print(json.dumps(stats))
        fail(f"rig-quality gate: weightless {weightless}/{vert_count} "
             f"({weightless / vert_count:.2%}, limit "
             f"{WEIGHTLESS_LIMIT_FRACTION:.2%}), "
             f"{len(warnings)} solver warning(s) — candidate is decision-gate data")

    # ---- shared machinery: prune, sockets, clips, bake, export ----
    mixamo_rig.prune_fingers(armature, [mesh_obj])
    mixamo_rig.add_socket_bones(armature)
    mixamo_rig.stash_clips(armature, args.clips_dir)
    bake_scale = mixamo_rig.bake_height(armature, [mesh_obj], args.height)
    mixamo_rig.export_glb(args.out_glb)

    depsgraph = bpy.context.evaluated_depsgraph_get()
    zs = [(mesh_obj.matrix_world @ v.co).z
          for v in mesh_obj.evaluated_get(depsgraph).data.vertices]
    stats.update({
        "bones": len(armature.data.bones),
        "actions": len(bpy.data.actions),
        "height_target": args.height,
        "bake_scale": bake_scale,
        "height": max(zs) - min(zs),
        "min_y": min(zs),
        "out_glb": str(args.out_glb),
    })
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
