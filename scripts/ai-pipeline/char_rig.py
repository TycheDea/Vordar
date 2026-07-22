# Blender-headless halves of the character rig stage (Phase A4.4), on
# either side of the SkinTokens skin prediction (char_skin.py):
#
#   fit     transplant the canonical Mixamo skeleton, fit the generated
#           mesh into its space, prune fingers, add socket bones,
#           rigid-bind -> the SkinTokens handoff glb
#   finish  rebuild the same scene, adopt SkinTokens' predicted weights,
#           trim the Mixamo end bones, rig-quality gate, clips, height
#           bake -> one engine-ready glb
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
#   - finish rebuilds the scene from the same inputs instead of importing
#     fit.glb: the fit math is deterministic, and a glTF round-trip of the
#     armature object transform would break the socket-scale invariant
#     above
#   - SkinTokens re-emits generic bone names and drops non-deforming leaf
#     bones; the skeleton is ours, so canonical names are recovered by
#     descending both hierarchies in lockstep (min-total-distance sibling
#     assignment), and only END_BONES may go unmatched. Rig-quality
#     gate: a failed match, a round-trip-dropped deforming bone, or
#     weightless verts > 0.5% is a structural failure -> exit non-zero
#     WITH the numbers (a failed candidate is A4.11 decision-gate data,
#     never silently patched)
#   - finger prune, socket bones, clip stash, height/ground bake, and glb
#     export settings are mixamo_rig.py, shared with mixamo_to_glb.py
#   - each subcommand prints one JSON stats line (the only '{'-prefixed
#     stdout line) for the chained generation manifest
#
# Usage: blender --background --python char_rig.py -- \
#            fit <textured.glb> <Character.fbx> <fit.glb>
#        blender --background --python char_rig.py -- \
#            finish <textured.glb> <Character.fbx> <skinned.glb> <clips_dir> <out.glb> [--height M]

import argparse
import json
import sys
import traceback
from itertools import permutations
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR.parent / "asset-pipeline"))
import mixamo_rig  # noqa: E402

# Landmark bands are cut at ±5% of skeleton height around a joint — wide
# enough to always catch torso/leg cross-sections on a full-body mesh,
# narrow enough that a hips band never reaches the shoulders.
BAND_FRACTION = 0.05
# Joint positions round-trip SkinTokens' 256-level quantizer with up to
# ~5 cm error at human scale, and a chain-tip joint absorbs its dropped
# leaf, landing anywhere along the merged segment (their Head sits at our
# crown) — both measured on the human rig. So the sanity gate on the
# hierarchy-assigned pairs bounds the distance to the assigned bone's
# SEGMENT, not its head: a correct assignment stays ~5 cm off-segment, a
# structurally wrong one jumps past limb spacing.
MATCH_EPSILON = 0.10


def fail(msg):
    print(f"char_rig: {msg}", file=sys.stderr)
    sys.exit(1)


def vert_coords(me):
    co = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", co)
    return co.reshape(-1, 3)


def joint_head(armature, name):
    return armature.matrix_world @ \
        armature.data.bones[mixamo_rig.BONE_PREFIX + name].head_local


def select_only(objs, active):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = active


def build_scene(textured_glb, character_fbx):
    """Canonical skeleton + generated mesh fitted into its space, fingers
    pruned, sockets added — the shared scene both subcommands start from.
    Returns (armature, mesh_obj, fit stats)."""
    mixamo_rig.new_scene()

    # ---- canonical skeleton in, its own meshes out ----
    bpy.ops.import_scene.fbx(filepath=character_fbx)
    armatures = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    if len(armatures) != 1:
        fail(f"expected exactly 1 armature in {character_fbx}, "
             f"found {len(armatures)}")
    armature = armatures[0]
    for o in [o for o in bpy.context.scene.objects if o.type == "MESH"]:
        bpy.data.objects.remove(o, do_unlink=True)
    # Character.fbx ships its own T-pose action; drop it now — the
    # ACTIONS-mode export writes every action, and fit.glb must carry none
    # (finish gets its actions from stash_clips). Removing it exposes the
    # FBX importer's junk static pose (the assigned action used to mask it
    # on every evaluation), so force the pose back to rest explicitly.
    if armature.animation_data and armature.animation_data.action:
        armature.animation_data.action = None
    for a in list(bpy.data.actions):
        bpy.data.actions.remove(a)
    for b in armature.pose.bones:
        b.matrix_basis = Matrix.Identity(4)

    # ---- generated mesh in: one object, transforms baked into verts ----
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.gltf(filepath=textured_glb)
    imported = set(bpy.context.scene.objects) - before
    meshes = [o for o in imported if o.type == "MESH"]
    extras = [o for o in imported if o.type != "MESH"]
    if not meshes:
        fail(f"no mesh in {textured_glb}")
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
    if me.shape_keys:
        # me.transform() below moves base vertices but NOT shape-key data,
        # and evaluation/export read through the active key — the mesh
        # would silently keep its pre-fit geometry.
        fail("textured mesh carries shape keys — unsupported input")

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

    # ---- adopt the incumbent rig's object structure: mesh data in the
    # armature's object space, mesh a plain child with identity local
    # transform. The glTF exporter is only proven against this shape (FBX
    # import delivers skinned meshes this way); a world-space mesh bound
    # through parent_set's parent-inverse compensation exports with
    # inconsistent inverse-bind matrices ----
    me.transform(armature.matrix_world.inverted())
    # Direct data mutation does not tag the mesh for re-evaluation, and
    # the glTF exporter reads the EVALUATED mesh — without this it can
    # export the stale pre-fit vertices (the old bone-heat path was saved
    # only by its operator calls forcing depsgraph flushes).
    me.update()
    mesh_obj.parent = armature
    mesh_obj.matrix_parent_inverse = Matrix.Identity(4)
    mesh_obj.matrix_basis = Matrix.Identity(4)
    bpy.context.view_layer.update()

    mixamo_rig.prune_fingers(armature, [mesh_obj])
    mixamo_rig.add_socket_bones(armature)

    stats = {
        "verts": len(me.vertices),
        "fit_scale": fit_scale,
        "skeleton_height": skel_height,
        "shoulder_span_mesh": mesh_span,
        "shoulder_span_skeleton": skel_span,
    }
    return armature, mesh_obj, stats


def rigid_bind(mesh_obj, armature):
    """Weight-1 bind of every vertex to its nearest deform-bone segment —
    the minimal valid skin for the SkinTokens handoff glb (the glTF
    exporter writes a skin only for a weighted mesh, and SkinTokens'
    validated input shape is a skinned glb). Prediction replaces these
    weights wholesale."""
    segments = mixamo_rig.deform_segments(armature)
    groups = {name: mesh_obj.vertex_groups.new(name=name) for name in segments}
    mesh_mw = mesh_obj.matrix_world
    for v in mesh_obj.data.vertices:
        p = mesh_mw @ v.co
        name = min(segments,
                   key=lambda n: mixamo_rig.dist_point_segment(p, *segments[n]))
        groups[name].add([v.index], 1.0, "REPLACE")
    mod = mesh_obj.modifiers.new("Armature", "ARMATURE")
    mod.object = armature


def match_joints(source_arm, canon_arm):
    """SkinTokens bone name -> canonical deform-bone name, by descending
    both hierarchies in lockstep from the roots: at each matched parent,
    children are paired by the minimum-total-distance assignment. Raw
    nearest-position matching cannot work here — the quantizer's ~5 cm
    error exceeds the neck/shoulder and toe-base/toe-end spacings — but
    within one sibling set the correct assignment wins globally, and every
    chain link is forced by its parent. Canonical deform bones left
    unmatched must be exactly the round-trip-dropped END_BONES."""
    mapping = {}
    unmatched = []
    max_dist = 0.0

    def head(arm, b):
        return arm.matrix_world @ b.head_local

    def deform_children(b):
        return [c for c in b.children if c.use_deform]

    def skip_subtree(canon_bone):
        unmatched.append(canon_bone.name)
        for c in deform_children(canon_bone):
            skip_subtree(c)

    def pair_level(src_bones, canon_bones, context):
        nonlocal max_dist
        if len(src_bones) > len(canon_bones):
            fail(f"joint match gate: {context} has {len(src_bones)} "
                 f"round-trip bone(s) for {len(canon_bones)} canonical "
                 f"candidate(s) — candidate is decision-gate data")
        best = min(permutations(canon_bones, len(src_bones)),
                   key=lambda perm: sum(
                       (head(source_arm, s) - head(canon_arm, c)).length
                       for s, c in zip(src_bones, perm)))
        for s, c in zip(src_bones, best):
            dist = mixamo_rig.dist_point_segment(
                head(source_arm, s),
                canon_arm.matrix_world @ c.head_local,
                canon_arm.matrix_world @ c.tail_local)
            if dist > MATCH_EPSILON:
                fail(f"joint match gate: {s.name} assigned to {c.name} at "
                     f"{dist:.3f} m (limit {MATCH_EPSILON}) — candidate is "
                     f"decision-gate data")
            max_dist = max(max_dist, dist)
            mapping[s.name] = c.name
            pair_level(list(s.children), deform_children(c),
                       f"children of {c.name}")
        for c in canon_bones:
            if c not in best:
                skip_subtree(c)

    src_roots = [b for b in source_arm.data.bones if b.parent is None]
    canon_roots = [b for b in canon_arm.data.bones
                   if b.parent is None and b.use_deform]
    pair_level(src_roots, canon_roots, "root level")

    dropped = sorted(unmatched)
    extra = set(dropped) - set(mixamo_rig.END_BONES)
    if extra:
        fail(f"joint match gate: skeleton round-trip dropped deforming "
             f"bone(s) {sorted(extra)} — candidate is decision-gate data")
    return mapping, max_dist, dropped


def adopt_weights(mesh_obj, armature, skinned_glb):
    """Take SkinTokens' predicted vertex groups from skinned.glb onto our
    mesh under canonical bone names, then bind. Vertex positions are the
    same mesh (their transfer path preserves it), so the NEAREST mapping
    is exact — the same argument the old seam-weld copy-back used."""
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.gltf(filepath=skinned_glb)
    imported = set(bpy.context.scene.objects) - before
    st_arms = [o for o in imported if o.type == "ARMATURE"]
    # Skinned meshes only: importing a glb with a skin also creates a
    # groupless bone-shape display mesh ("Icosphere") that must not be
    # joined into the weight source.
    st_meshes = sorted((o for o in imported
                        if o.type == "MESH" and o.vertex_groups),
                       key=lambda o: o.name)
    if len(st_arms) != 1 or not st_meshes:
        fail(f"expected 1 armature and >=1 skinned mesh in {skinned_glb}, "
             f"found {len(st_arms)} armature(s), {len(st_meshes)} skinned "
             f"mesh(es)")
    select_only(st_meshes, st_meshes[0])
    if len(st_meshes) > 1:
        bpy.ops.object.join()
    st_mesh = bpy.context.view_layer.objects.active

    mapping, max_dist, dropped = match_joints(st_arms[0], armature)
    for g in st_mesh.vertex_groups:
        if g.name not in mapping:
            fail(f"skinned mesh has a vertex group '{g.name}' matching no "
                 f"joint of its own skeleton")
        g.name = mapping[g.name]

    dt = mesh_obj.modifiers.new("weights", "DATA_TRANSFER")
    dt.object = st_mesh
    dt.use_vert_data = True
    dt.data_types_verts = {"VGROUP_WEIGHTS"}
    dt.vert_mapping = "NEAREST"
    select_only([mesh_obj], mesh_obj)
    bpy.ops.object.datalayout_transfer(modifier=dt.name)
    bpy.ops.object.modifier_apply(modifier=dt.name)
    # join() already deleted the merged-away objects, so sweep what is
    # still alive rather than iterating the stale imported set.
    for o in [o for o in bpy.context.scene.objects if o not in before]:
        bpy.data.objects.remove(o, do_unlink=True)

    mod = mesh_obj.modifiers.new("Armature", "ARMATURE")
    mod.object = armature
    return {
        "matched_joints": len(mapping),
        "joint_match_max_distance_m": max_dist,
        "roundtrip_dropped_bones": dropped,
    }


def cmd_fit(args):
    armature, mesh_obj, stats = build_scene(args.textured_glb, args.character_fbx)
    rigid_bind(mesh_obj, armature)
    mixamo_rig.export_glb(args.out_glb)
    stats["out_glb"] = str(args.out_glb)
    print(json.dumps(stats))


def cmd_finish(args):
    armature, mesh_obj, stats = build_scene(args.textured_glb, args.character_fbx)

    stats.update(adopt_weights(mesh_obj, armature, args.skinned_glb))
    mixamo_rig.trim_end_bones(armature, mesh_obj)

    # ---- rig-quality gate ----
    vert_count = len(mesh_obj.data.vertices)
    weightless, bleed = mixamo_rig.weight_metrics(mesh_obj, armature)
    stats.update({
        "weightless_verts": weightless,
        "weightless_fraction": weightless / vert_count,
        "bleed_verts_over_30cm": bleed,
    })
    if weightless > mixamo_rig.WEIGHTLESS_LIMIT_FRACTION * vert_count:
        print(json.dumps(stats))
        fail(f"rig-quality gate: weightless {weightless}/{vert_count} "
             f"({weightless / vert_count:.2%}, limit "
             f"{mixamo_rig.WEIGHTLESS_LIMIT_FRACTION:.2%}) — candidate is "
             f"decision-gate data")

    # ---- shared machinery: clips, bake, export ----
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


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="char_rig.py")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("fit")
    p.add_argument("textured_glb")
    p.add_argument("character_fbx")
    p.add_argument("out_glb")
    p.set_defaults(func=cmd_fit)

    p = sub.add_parser("finish")
    p.add_argument("textured_glb")
    p.add_argument("character_fbx")
    p.add_argument("skinned_glb")
    p.add_argument("clips_dir")
    p.add_argument("out_glb")
    p.add_argument("--height", type=float, default=mixamo_rig.TARGET_HEIGHT)
    p.set_defaults(func=cmd_finish)

    args = parser.parse_args(argv)
    args.func(args)


try:
    main()
except SystemExit:
    raise
except Exception:
    # without --python-exit-code Blender exits 0 on an uncaught script
    # exception — route every failure through an explicit non-zero exit
    traceback.print_exc()
    sys.exit(1)
