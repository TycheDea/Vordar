# Shared Blender-headless machinery for putting the canonical Mixamo
# skeleton + clip library into an engine-ready .glb. Used by
# mixamo_to_glb.py (Mixamo auto-rigged FBX character) and
# scripts/ai-pipeline/char_rig.py (generated mesh, transplanted skeleton).
#
# Contract: callers keep the imported armature's own object transforms
# untouched except through bake_height (the engine's weapon-socket path
# compensates for the FBX cm scale — client/vordar-client/src/weapons.rs:204
# — so applying/clearing armature transforms breaks sockets silently).

from pathlib import Path

import bpy

TARGET_HEIGHT = 1.744  # the VRoid body's authored height, metres
FLOOR_TOP = -0.5
FINGERS = ("Thumb", "Index", "Middle", "Ring", "Pinky")
CLIP_RENAMES = {"jump": "leap"}
CLIP_SKIP = {"tpose"}
BONE_PREFIX = "mixamorig:"
# A vertex whose dominant joint sits further than this from it (metres,
# point-to-bone-segment) is counted as weight bleed: 30 cm exceeds any
# plausible limb radius at humanoid scale.
BLEED_DISTANCE = 0.30
WEIGHTLESS_LIMIT_FRACTION = 0.005
# The Mixamo end bones: no runtime references — callers delete them for
# good via trim_end_bones, folding any weights into their parents.
# The socket bones (handslot.r/l, head) are also non-deforming leaves but
# ARE runtime-referenced (weapons.rs, vfx.rs, content_lint.rs), so they
# are re-added by add_socket_bones, never trimmed.
END_BONES = tuple(BONE_PREFIX + n
                  for n in ("HeadTop_End", "LeftToe_End", "RightToe_End"))


def dist_point_segment(p, a, b) -> float:
    ab = b - a
    denom = ab.length_squared
    t = 0.0 if denom == 0.0 else max(0.0, min(1.0, (p - a).dot(ab) / denom))
    return (p - (a + ab * t)).length


def deform_segments(armature):
    mw = armature.matrix_world
    return {b.name: (mw @ b.head_local, mw @ b.tail_local)
            for b in armature.data.bones if b.use_deform}


def weight_metrics(mesh_obj, armature):
    """(weightless count, bleed count): verts with no deform-bone weight,
    and verts whose dominant joint is over BLEED_DISTANCE away."""
    segments = deform_segments(armature)
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


def trim_end_bones(armature, mesh_obj):
    """Delete the Mixamo end bones, folding any weights on them into their
    parents first — an orphaned vertex group would silently shrink its
    verts (the armature modifier ignores groups without a bone, leaving
    weight sums < 1)."""
    parents = {name: armature.data.bones[name].parent.name
               for name in END_BONES}
    for name, parent in parents.items():
        group = mesh_obj.vertex_groups.get(name)
        if group is not None:
            parent_group = mesh_obj.vertex_groups.get(parent) \
                or mesh_obj.vertex_groups.new(name=parent)
            for v in mesh_obj.data.vertices:
                for vg in v.groups:
                    if vg.group == group.index and vg.weight > 0.0:
                        parent_group.add([v.index], vg.weight, "ADD")
            mesh_obj.vertex_groups.remove(group)
    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.mode_set(mode="EDIT")
    for name in END_BONES:
        armature.data.edit_bones.remove(armature.data.edit_bones[name])
    bpy.ops.object.mode_set(mode="OBJECT")


def new_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.render.fps = 30  # Mixamo clips are exported at 30 fps


def prune_fingers(armature, meshes):
    """Merge finger-bone vertex weights into the hand bones, then delete the
    finger bones (65-bone Mixamo skeleton > the 64-joint palette cap)."""

    def hand_for(name: str) -> str:
        return next(b.name for b in armature.data.bones
                    if b.name.endswith("LeftHand" if "Left" in name else "RightHand"))

    finger_bones = [b.name for b in armature.data.bones
                    if any(f in b.name for f in FINGERS)]

    for mesh in meshes:
        finger_groups = [g for g in mesh.vertex_groups if g.name in finger_bones]
        if not finger_groups:
            continue
        finger_idx = {g.index: g.name for g in finger_groups}
        for v in mesh.data.vertices:
            for vg in v.groups:
                if vg.group in finger_idx and vg.weight > 0.0:
                    hand = mesh.vertex_groups.get(hand_for(finger_idx[vg.group])) \
                        or mesh.vertex_groups.new(name=hand_for(finger_idx[vg.group]))
                    hand.add([v.index], vg.weight, "ADD")
        for g in finger_groups:
            mesh.vertex_groups.remove(g)

    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.mode_set(mode="EDIT")
    for name in finger_bones:
        armature.data.edit_bones.remove(armature.data.edit_bones[name])
    bpy.ops.object.mode_set(mode="OBJECT")


def add_socket_bones(armature):
    """Engine SocketConfig bones (handslot.r, handslot.l, head) as
    non-deforming children of the Mixamo hand/head bones."""
    bpy.context.view_layer.objects.active = armature
    bpy.ops.object.mode_set(mode="EDIT")
    for socket, parent_suffix in [("handslot.r", "RightHand"),
                                  ("handslot.l", "LeftHand"),
                                  ("head", "Head")]:
        parent = next(b for b in armature.data.edit_bones
                      if b.name.endswith(parent_suffix) and "Top" not in b.name)
        bone = armature.data.edit_bones.new(socket)
        bone.head = parent.head
        bone.tail = parent.head + (parent.tail - parent.head) * 0.25
        bone.parent = parent
        bone.use_deform = False
    bpy.ops.object.mode_set(mode="OBJECT")


def stash_clips(armature, clips_dir):
    """Stash every clips/*.fbx as a muted NLA action named after the file
    (jump.fbx -> "leap"; tpose.fbx skipped), so each exports as a glTF
    animation with that name under export mode ACTIONS."""
    anim_data = armature.animation_data_create()
    if anim_data.action:
        # The character download ships its own T-pose action; delete it from
        # bpy.data — the ACTIONS export mode exports every compatible action,
        # assigned or not.
        stray = anim_data.action
        anim_data.action = None
        bpy.data.actions.remove(stray)

    for fbx in sorted(Path(clips_dir).glob("*.fbx")):
        slot = CLIP_RENAMES.get(fbx.stem, fbx.stem)
        if fbx.stem in CLIP_SKIP:
            continue
        before = set(bpy.data.actions)
        imported_before = set(bpy.context.scene.objects)
        bpy.ops.import_scene.fbx(filepath=str(fbx))
        action = next(a for a in set(bpy.data.actions) - before)
        action.name = slot
        track = anim_data.nla_tracks.new()
        track.name = slot
        track.strips.new(slot, max(1, int(action.frame_range[0])), action)
        track.mute = True
        for obj in set(bpy.context.scene.objects) - imported_before:
            bpy.data.objects.remove(obj, do_unlink=True)


def bake_height(armature, meshes, target_height=TARGET_HEIGHT):
    """Bake height + ground offset onto the armature OBJECT (a non-joint
    ancestor -> engine Skeleton::root; never onto the hips joint, clips
    animate it). Returns the applied uniform scale factor."""
    depsgraph = bpy.context.evaluated_depsgraph_get()
    corners = [obj.matrix_world @ v.co
               for obj in meshes
               for v in obj.evaluated_get(depsgraph).data.vertices]
    min_z = min(c.z for c in corners)
    max_z = max(c.z for c in corners)
    s = target_height / (max_z - min_z)
    armature.scale = [c * s for c in armature.scale]
    armature.location.z = armature.location.z * s + FLOOR_TOP - min_z * s
    return s


def export_glb(out_glb):
    bpy.ops.export_scene.gltf(
        filepath=str(out_glb),
        export_format="GLB",
        export_animation_mode="ACTIONS",
        export_optimize_animation_size=False,
    )
