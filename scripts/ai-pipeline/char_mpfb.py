# Blender-headless MPFB2 parametric character (A4.C2): build a clothed
# monk body with MPFB's artist-authored mixamo weights, bring it to the
# canonical Mixamo T-pose, and bind it to the Character.fbx skeleton —
# no bone-heat, no weight prediction anywhere on this path.
#
#   - the Character.fbx armature is kept EXACTLY as imported (units
#     invariant: the engine's weapon-socket path bakes the FBX cm scale,
#     client/vordar-client/src/weapons.rs:204); only the MESH moves into
#     its space, and only bake_height touches the armature object
#   - the MPFB armature is a build tool: it poses the meshes into the
#     canonical T-pose (its 52 bone names are an exact subset of the
#     canonical 65) and is deleted before binding
#   - MPFB rest pose faces -Y where canonical faces +Y — a 180 deg yaw,
#     not a mirror — so pose targets and the final rigid fit both carry
#     Rz(pi)
#   - prints one JSON stats line (the only '{'-prefixed stdout line) for
#     the chained generation manifest
#
# Usage: blender --background --python char_mpfb.py -- <out.glb> [--height M]

import argparse
import importlib
import json
import math
import sys
import traceback
from pathlib import Path

import bpy
from mathutils import Matrix, Vector

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR.parent / "asset-pipeline"))
import mixamo_rig  # noqa: E402
sys.path.insert(0, str(SCRIPT_DIR))
from proptex.registry import resolve_class  # noqa: E402

REPO = SCRIPT_DIR.parent.parent
CHARACTER_FBX = REPO / "content/source/characters/mixamo/Character.fbx"
CLIPS_DIR = REPO / "content/source/characters/mixamo/clips"

# Monk fixture body — the A4.C2 checkpoint judges these proportions on
# the rendered sheets; gaunt penitent ballpark until then.
MACROS = {
    "gender": 1.0,
    "age": 0.6,
    "muscle": 0.4,
    "weight": 0.35,
    "height": 0.5,
    "proportions": 0.5,
    "cupsize": 0.5,
    "firmness": 0.5,
    "race": {"asian": 0.0, "caucasian": 1.0, "african": 0.0},
}
SKIN = "old_caucasian_male.mhmat"
GARMENTS = ("donitz_monk_robe.mhclo", "donitz_monk_robe_hood.mhclo")
EYES = "low-poly.mhclo"

YAW_180 = Matrix.Rotation(math.pi, 4, "Z")


def fail(msg):
    print(f"char_mpfb: {msg}", file=sys.stderr)
    sys.exit(1)


def select_only(objs, active):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = active


def resolve_mpfb():
    """MPFB2 installs as a Blender extension whose package name depends on
    the repository it was installed into — a bare `import mpfb` never
    works. Resolve it the way MPFB's own sample scripts do: scan
    sys.modules for the registered package."""
    name = next((n for n in sys.modules if n.endswith(".mpfb")), None)
    if name is None:
        fail("MPFB2 extension is not loaded — install mpfb2 via "
             "bpy.ops.extensions.package_install_files(..., "
             "repo='user_default', enable_on_install=True) "
             "+ wm.save_userpref()")

    def service(module, cls):
        return getattr(importlib.import_module(f"{name}.services.{module}"),
                       cls)
    return service


def asset_path(asset_service, fragment, subdir):
    path = asset_service.find_asset_absolute_path(fragment, subdir)
    if not path:
        fail(f"MPFB asset not found: {fragment} (subdir {subdir})")
    return path


def new_meshes(before):
    return [o for o in set(bpy.context.scene.objects) - before
            if o.type == "MESH"]


def build_character(service):
    """Basemesh + mixamo rig + garments + eyes + skin, targets baked.
    Returns (basemesh, src_arm, garment meshes, eye meshes)."""
    human_service = service("humanservice", "HumanService")
    target_service = service("targetservice", "TargetService")
    asset_service = service("assetservice", "AssetService")

    macros = target_service.get_default_macro_info_dict()
    macros.update(MACROS)
    basemesh = human_service.create_human(True, True, True, True, 0.1, macros)
    # Baked before the rig and garments: both are fitted from
    # basemesh.data.vertices, which shape keys do not reach.
    target_service.bake_targets(basemesh)
    src_arm = human_service.add_builtin_rig(basemesh, "mixamo")

    garments = []
    for fragment in GARMENTS:
        before = set(bpy.context.scene.objects)
        human_service.add_mhclo_asset(
            asset_path(asset_service, fragment, "clothes"), basemesh,
            asset_type="Clothes", subdiv_levels=0)
        garments += new_meshes(before)

    before = set(bpy.context.scene.objects)
    human_service.add_mhclo_asset(
        asset_path(asset_service, EYES, "eyes"), basemesh,
        asset_type="Eyes", subdiv_levels=0)
    eyes = new_meshes(before)

    human_service.set_character_skin(
        asset_path(asset_service, SKIN, "skins"), basemesh,
        skin_type="MAKESKIN")
    return basemesh, src_arm, garments, eyes


def ensure_weights(basemesh, src_arm, garments, eyes):
    """Every mesh must deform correctly under the MPFB armature BEFORE
    the T-pose is applied through it. Garments normally arrive with
    MPFB-interpolated weights; a garment below coverage falls back to a
    surface-interpolated transfer from the body. Eyes follow the head
    rigidly. Returns the per-mesh weight-source stats dict."""
    bone_names = {b.name for b in src_arm.data.bones}
    sources = {"basemesh": "authored"}

    def coverage(mesh):
        bone_groups = {g.index for g in mesh.vertex_groups
                       if g.name in bone_names}
        covered = sum(
            1 for v in mesh.data.vertices
            if any(vg.group in bone_groups and vg.weight > 0.0
                   for vg in v.groups))
        return covered / max(1, len(mesh.data.vertices))

    for mesh in garments:
        if coverage(mesh) >= 1.0 - mixamo_rig.WEIGHTLESS_LIMIT_FRACTION:
            sources[mesh.name] = "mpfb_interpolated"
            continue
        dt = mesh.modifiers.new("weights", "DATA_TRANSFER")
        dt.object = basemesh
        dt.use_vert_data = True
        dt.data_types_verts = {"VGROUP_WEIGHTS"}
        dt.vert_mapping = "POLYINTERP_NEAREST"
        select_only([mesh], mesh)
        bpy.ops.object.datalayout_transfer(modifier=dt.name)
        bpy.ops.object.modifier_apply(modifier=dt.name)
        sources[mesh.name] = "data_transfer"

    head_group = mixamo_rig.BONE_PREFIX + "Head"
    for mesh in eyes:
        for g in list(mesh.vertex_groups):
            mesh.vertex_groups.remove(g)
        group = mesh.vertex_groups.new(name=head_group)
        group.add(list(range(len(mesh.data.vertices))), 1.0, "REPLACE")
        sources[mesh.name] = "rigid_head"
    return sources


def import_canonical_armature():
    before = set(bpy.context.scene.objects)
    bpy.ops.import_scene.fbx(filepath=str(CHARACTER_FBX))
    imported = set(bpy.context.scene.objects) - before
    armatures = [o for o in imported if o.type == "ARMATURE"]
    if len(armatures) != 1:
        fail(f"expected exactly 1 armature in {CHARACTER_FBX}, "
             f"found {len(armatures)}")
    armature = armatures[0]
    for o in imported:
        if o.type == "MESH":
            bpy.data.objects.remove(o, do_unlink=True)
    # Character.fbx ships its own T-pose action; drop it (stash_clips
    # provides the real ones). Removing it exposes the FBX importer's
    # junk static pose (the assigned action used to mask it on every
    # evaluation), so force the pose back to rest explicitly.
    if armature.animation_data and armature.animation_data.action:
        armature.animation_data.action = None
    for a in list(bpy.data.actions):
        bpy.data.actions.remove(a)
    for b in armature.pose.bones:
        b.matrix_basis = Matrix.Identity(4)
    return armature


def align_to_tpose(src_arm, canon_arm):
    """Pose each name-matched MPFB bone (parents first) with the shortest
    arc taking its world direction onto the canonical counterpart's rest
    direction pre-rotated into the MPFB frame by the 180 deg yaw. Roll
    stays free — the residual angle in stats is the checkpoint's forearm
    twist evidence. Returns the max residual in degrees."""
    canon_mw3 = canon_arm.matrix_world.to_3x3()
    targets = {
        b.name: (YAW_180.to_3x3() @ canon_mw3
                 @ (b.tail_local - b.head_local)).normalized()
        for b in canon_arm.data.bones}
    mw = src_arm.matrix_world
    order = []

    def walk(bone):
        order.append(bone.name)
        for child in bone.children:
            walk(child)
    for bone in src_arm.data.bones:
        if bone.parent is None:
            walk(bone)

    unmatched = [n for n in order if n not in targets]
    if unmatched:
        fail(f"MPFB bones missing from the canonical skeleton: {unmatched}")

    def world_dir(pose_bone):
        return ((mw @ pose_bone.matrix).to_3x3()
                @ Vector((0.0, 1.0, 0.0))).normalized()

    for name in order:
        bpy.context.view_layer.update()
        pb = src_arm.pose.bones[name]
        rot = world_dir(pb).rotation_difference(targets[name]) \
            .to_matrix().to_4x4()
        head = (mw @ pb.matrix).translation
        new_world = Matrix.Translation(head) @ rot \
            @ Matrix.Translation(-head) @ (mw @ pb.matrix)
        pb.matrix = mw.inverted() @ new_world

    bpy.context.view_layer.update()
    return max(
        math.degrees(world_dir(src_arm.pose.bones[n]).angle(targets[n]))
        for n in order)


def apply_build_modifiers(meshes):
    """Bake every build modifier (armature pose, helper/delete masks)
    into the vertices, in stack order. Vertex groups survive."""
    for mesh in meshes:
        if mesh.data.shape_keys:
            # me.transform() later moves base vertices but NOT shape-key
            # data — export would silently read stale geometry.
            fail(f"mesh {mesh.name} still carries shape keys after "
                 f"bake_targets — cannot proceed")
        select_only([mesh], mesh)
        for mod in [m.name for m in mesh.modifiers]:
            bpy.ops.object.modifier_apply(modifier=mod)


def fit_into_canonical(meshes, src_arm, canon_arm):
    """Rigid yaw + uniform scale + translation taking the T-posed MPFB
    meshes onto the canonical skeleton. Scale landmark: Head joint to
    lowest-joint span — HeadTop_End does not exist on the MPFB rig, and
    Head-to-ground is the tallest span present on BOTH skeletons.
    Translation aligns the Hips heads exactly."""
    mw = src_arm.matrix_world

    def posed_head(name):
        return mw @ src_arm.pose.bones[name].matrix.translation

    hips = mixamo_rig.BONE_PREFIX + "Hips"
    head = mixamo_rig.BONE_PREFIX + "Head"
    src_hips = posed_head(hips)
    src_span = posed_head(head).z - min(
        posed_head(b.name).z for b in src_arm.data.bones)

    canon_mw = canon_arm.matrix_world
    canon_hips = canon_mw @ canon_arm.data.bones[hips].head_local
    canon_span = (canon_mw @ canon_arm.data.bones[head].head_local).z - min(
        (canon_mw @ b.head_local).z for b in canon_arm.data.bones)

    s = canon_span / src_span
    scale = Matrix.Scale(s, 4)
    fit = Matrix.Translation(canon_hips - scale @ YAW_180 @ src_hips) \
        @ scale @ YAW_180

    for mesh in meshes:
        world = mesh.matrix_world.copy()
        mesh.parent = None
        mesh.matrix_world = world
        select_only([mesh], mesh)
        bpy.ops.object.transform_apply(location=True, rotation=True,
                                       scale=True)
        mesh.data.transform(fit)
        mesh.data.update()
    return s


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="char_mpfb.py")
    parser.add_argument("out_glb")
    parser.add_argument("--height", type=float,
                        default=mixamo_rig.TARGET_HEIGHT)
    args = parser.parse_args(argv)

    mixamo_rig.new_scene()
    service = resolve_mpfb()

    basemesh, src_arm, garments, eyes = build_character(service)
    weight_sources = ensure_weights(basemesh, src_arm, garments, eyes)
    meshes = [basemesh] + garments + eyes

    canon_arm = import_canonical_armature()
    residual = align_to_tpose(src_arm, canon_arm)
    apply_build_modifiers(meshes)
    fit_scale = fit_into_canonical(meshes, src_arm, canon_arm)
    bpy.data.objects.remove(src_arm, do_unlink=True)

    select_only(meshes, meshes[0])
    bpy.ops.object.join()
    mesh_obj = bpy.context.view_layer.objects.active

    # The proven export contract (char_rig.py): mesh data in the
    # armature's object space, mesh a plain child with identity local
    # transform — the glTF exporter is only proven against this shape.
    me = mesh_obj.data
    me.transform(canon_arm.matrix_world.inverted())
    # Direct data mutation does not tag the mesh for re-evaluation, and
    # the glTF exporter reads the EVALUATED mesh.
    me.update()
    mesh_obj.parent = canon_arm
    mesh_obj.matrix_parent_inverse = Matrix.Identity(4)
    mesh_obj.matrix_basis = Matrix.Identity(4)
    bpy.context.view_layer.update()

    mod = mesh_obj.modifiers.new("Armature", "ARMATURE")
    mod.object = canon_arm

    mixamo_rig.prune_fingers(canon_arm, [mesh_obj])
    mixamo_rig.add_socket_bones(canon_arm)
    mixamo_rig.trim_end_bones(canon_arm, mesh_obj)

    # Character MR contract: constant dielectric AND opaque, like every
    # shipped race. MPFB's MakeSkin node trees carry a transparency chain
    # + blended surface method, which the glTF exporter turns into
    # alphaMode BLEND — the engine then renders the whole character
    # translucent (robe see-through, eyes visible through the hood).
    skin = resolve_class("character_skin")
    for slot in mesh_obj.material_slots:
        mat = slot.material
        if not mat or not mat.node_tree:
            continue
        mat.surface_render_method = "DITHERED"
        for node in mat.node_tree.nodes:
            if node.type != "BSDF_PRINCIPLED":
                continue
            for input_name, value in (("Metallic", skin["metallic"]),
                                      ("Roughness", skin["roughness"]),
                                      ("Alpha", 1.0)):
                socket = node.inputs[input_name]
                for link in list(socket.links):
                    mat.node_tree.links.remove(link)
                socket.default_value = value

    vert_count = len(me.vertices)
    weightless, bleed = mixamo_rig.weight_metrics(mesh_obj, canon_arm)
    stats = {
        "verts": vert_count,
        "pose_align_max_residual_deg": residual,
        "fit_scale": fit_scale,
        "weight_sources": weight_sources,
        "weightless_verts": weightless,
        "weightless_fraction": weightless / vert_count,
        "bleed_verts_over_30cm": bleed,
    }
    if weightless > mixamo_rig.WEIGHTLESS_LIMIT_FRACTION * vert_count:
        print(json.dumps(stats))
        fail(f"rig-quality gate: weightless {weightless}/{vert_count} "
             f"({weightless / vert_count:.2%}, limit "
             f"{mixamo_rig.WEIGHTLESS_LIMIT_FRACTION:.2%})")

    mixamo_rig.stash_clips(canon_arm, CLIPS_DIR)
    bake_scale = mixamo_rig.bake_height(canon_arm, [mesh_obj], args.height)
    mixamo_rig.export_glb(args.out_glb)

    depsgraph = bpy.context.evaluated_depsgraph_get()
    zs = [(mesh_obj.matrix_world @ v.co).z
          for v in mesh_obj.evaluated_get(depsgraph).data.vertices]
    stats.update({
        "bones": len(canon_arm.data.bones),
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
