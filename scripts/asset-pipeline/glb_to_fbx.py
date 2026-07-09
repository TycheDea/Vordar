# Blender-headless glb -> FBX with embedded textures, for Mixamo's
# auto-rigger (Mixamo accepts FBX/OBJ, not glTF).
#
# Usage: blender --background --python glb_to_fbx.py -- <in.glb> <out.fbx>

import sys

import bpy

argv = sys.argv[sys.argv.index("--") + 1:]
src, dst = argv

# Fresh scene (the default cube/camera/light would ride along otherwise).
bpy.ops.wm.read_factory_settings(use_empty=True)

bpy.ops.import_scene.gltf(filepath=src)

bpy.ops.export_scene.fbx(
    filepath=dst,
    embed_textures=True,
    path_mode="COPY",
    add_leaf_bones=False,
)
print(f"wrote {dst}")
