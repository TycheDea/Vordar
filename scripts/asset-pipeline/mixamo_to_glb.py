# Blender-headless: Mixamo auto-rigged character + clip library -> one
# engine-ready .glb (Phase 5).
#
#   - prunes finger bones (65-bone Mixamo skeleton > the 64-joint palette
#     cap); their vertex weights merge into the hand bones
#   - adds the engine's socket bones (handslot.r, handslot.l, head) as
#     non-deforming children of the Mixamo hand/head bones
#   - stashes every clips/*.fbx as an NLA action named after the file
#     (jump.fbx -> "leap"; tpose.fbx skipped), so each exports as a glTF
#     animation with that name
#   - bakes height + ground offset onto the armature OBJECT (a non-joint
#     ancestor -> engine Skeleton::root; never onto the hips joint, clips
#     animate it)
#
# Material fixes (alpha mask, metallic/roughness) happen in the node
# post-pass (fix_glb_materials.mjs) — Blender's FBX import loses them.
#
# Usage: blender --background --python mixamo_to_glb.py -- \
#            <Character.fbx> <clips_dir> <out.glb>

import sys
from pathlib import Path

import bpy

TARGET_HEIGHT = 1.744  # the VRoid body's authored height, metres
FLOOR_TOP = -0.5
FINGERS = ("Thumb", "Index", "Middle", "Ring", "Pinky")
CLIP_RENAMES = {"jump": "leap"}
CLIP_SKIP = {"tpose"}

argv = sys.argv[sys.argv.index("--") + 1:]
character_fbx, clips_dir, out_glb = argv

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.context.scene.render.fps = 30  # Mixamo clips are exported at 30 fps

bpy.ops.import_scene.fbx(filepath=character_fbx)
armature = next(o for o in bpy.context.scene.objects if o.type == "ARMATURE")
meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]


def hand_for(name: str) -> str:
    return next(b.name for b in armature.data.bones
                if b.name.endswith("LeftHand" if "Left" in name else "RightHand"))


# ---- prune fingers: merge weights into the hand, then delete the bones ----
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

# ---- socket bones (engine SocketConfig: handslot.r, handslot.l, head) ----
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

# ---- clips -> stashed NLA actions (exporter mode ACTIONS: one anim each) ----
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

# ---- height + ground offset on the armature object (Skeleton::root) ----
depsgraph = bpy.context.evaluated_depsgraph_get()
corners = [obj.matrix_world @ v.co
           for obj in meshes
           for v in obj.evaluated_get(depsgraph).data.vertices]
min_z = min(c.z for c in corners)
max_z = max(c.z for c in corners)
s = TARGET_HEIGHT / (max_z - min_z)
armature.scale = [c * s for c in armature.scale]
armature.location.z = armature.location.z * s + FLOOR_TOP - min_z * s

bpy.ops.export_scene.gltf(
    filepath=out_glb,
    export_format="GLB",
    export_animation_mode="ACTIONS",
    export_optimize_animation_size=False,
)
print(f"wrote {out_glb}: {len(armature.data.bones)} bones, "
      f"{len(bpy.data.actions)} actions, scale {s:.4f}")
