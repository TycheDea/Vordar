# Blender-headless: Mixamo auto-rigged character + clip library -> one
# engine-ready .glb (Phase 5). All rig machinery (finger prune, socket
# bones, clip stash, height bake, export settings) lives in mixamo_rig.py,
# shared with the AI pipeline's char_rig.py.
#
# Material fixes (alpha mask, metallic/roughness) happen in the node
# post-pass (fix_glb_materials.mjs) — Blender's FBX import loses them.
#
# Usage: blender --background --python mixamo_to_glb.py -- \
#            <Character.fbx> <clips_dir> <out.glb>

import sys
from pathlib import Path

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mixamo_rig  # noqa: E402

argv = sys.argv[sys.argv.index("--") + 1:]
character_fbx, clips_dir, out_glb = argv

mixamo_rig.new_scene()
bpy.ops.import_scene.fbx(filepath=character_fbx)
armature = next(o for o in bpy.context.scene.objects if o.type == "ARMATURE")
meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]

mixamo_rig.prune_fingers(armature, meshes)
mixamo_rig.add_socket_bones(armature)
mixamo_rig.stash_clips(armature, clips_dir)
s = mixamo_rig.bake_height(armature, meshes)
mixamo_rig.export_glb(out_glb)

print(f"wrote {out_glb}: {len(armature.data.bones)} bones, "
      f"{len(bpy.data.actions)} actions, scale {s:.4f}")
