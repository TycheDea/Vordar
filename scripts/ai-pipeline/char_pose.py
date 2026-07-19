# Canonical T-pose kit for the character AI pipeline (Phase A4.1): two
# independent steps sharing one file, each importing only what its own
# interpreter provides.
#
#   probe (Blender-headless, prop_cleanup.py's invocation convention):
#     imports content/source/characters/mixamo/Character.fbx, asserts the
#     mixamorig: bone set, dumps every bone's world-space T-pose head/tail
#     position to JSON.
#   draw (Hi3DGen venv python -- pillow lives there, not under Blender's):
#     maps the Mixamo joints onto COCO body-18 keypoints, orthographically
#     projects the front view (world X/Z) and renders an OpenPose-format
#     stick figure for controlnet-openpose-sdxl conditioning (A4.2).
#
# Usage:
#   blender --background --python char_pose.py -- probe <Character.fbx> --out <joints.json>
#   C:\tools\Hi3DGen\venv\Scripts\python.exe char_pose.py draw --joints <joints.json> --out <tpose_openpose.png>

import argparse
import json
import sys
import traceback
from pathlib import Path

BONE_PREFIX = "mixamorig:"
EXPECTED_BONE_COUNT = 65

CANVAS_SIZE = 1024
# Full-body studio-shot framing: the figure's larger extent (height or
# T-pose arm span) fills 80% of the canvas, leaving a generous margin.
FIGURE_FILL_FRACTION = 0.80

COCO_KEYPOINTS = [
    "Nose", "Neck", "RShoulder", "RElbow", "RWrist", "LShoulder", "LElbow",
    "LWrist", "RHip", "RKnee", "RAnkle", "LHip", "LKnee", "LAnkle",
    "REye", "LEye", "REar", "LEar",
]

# Each COCO name maps to the Mixamo bone whose proximal joint ("head") sits
# at that anatomical location -- connected Mixamo bones share a head/tail
# point, so e.g. RShoulder is RightArm's head, not RightShoulder's (that
# bone is the clavicle; its tail is the same point as RightArm's head).
JOINT_SOURCE = {
    "Neck": "Neck", "RShoulder": "RightArm", "RElbow": "RightForeArm",
    "RWrist": "RightHand", "LShoulder": "LeftArm", "LElbow": "LeftForeArm",
    "LWrist": "LeftHand", "RHip": "RightUpLeg", "RKnee": "RightLeg",
    "RAnkle": "RightFoot", "LHip": "LeftUpLeg", "LKnee": "LeftLeg",
    "LAnkle": "LeftFoot",
}

# OpenCV/pytorch-openpose COCO-18 palette and limb sequence (0-indexed;
# 17 limbs -- the reference renderer these match never draws the two
# ear-to-shoulder connections) -- matched so controlnet-openpose-sdxl,
# trained on renders in this exact style, reads the conditioning image.
LIMB_COLORS = [
    (255, 0, 0), (255, 85, 0), (255, 170, 0), (255, 255, 0), (170, 255, 0),
    (85, 255, 0), (0, 255, 0), (0, 255, 85), (0, 255, 170), (0, 255, 255),
    (0, 170, 255), (0, 85, 255), (0, 0, 255), (85, 0, 255), (170, 0, 255),
    (255, 0, 255), (255, 0, 170), (255, 0, 85),
]
LIMBS = [
    (1, 0), (1, 2), (1, 5), (2, 3), (3, 4), (5, 6), (6, 7), (1, 8), (8, 9),
    (9, 10), (1, 11), (11, 12), (12, 13), (0, 14), (0, 15), (14, 16), (15, 17),
]
STICK_WIDTH = 8
KEYPOINT_RADIUS = 8
# The reference renderer dims each stick to 0.6x, then dims the whole
# (still-black-elsewhere) canvas by another 0.6x before drawing keypoints
# at full brightness on top; folds to one multiply since nothing else
# touches the canvas in between.
STICK_COLOR_SCALE = 0.6 * 0.6


def fail(msg):
    print(f"char_pose: {msg}", file=sys.stderr)
    sys.exit(1)


def cmd_probe(argv):
    import bpy  # deferred: only importable under Blender's own python

    parser = argparse.ArgumentParser(prog="char_pose.py probe")
    parser.add_argument("fbx")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=args.fbx)

    armatures = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
    if len(armatures) != 1:
        fail(f"expected exactly 1 armature in {args.fbx}, found {len(armatures)}")
    arm = armatures[0]
    bones = list(arm.data.bones)

    bad = [b.name for b in bones if not b.name.startswith(BONE_PREFIX)]
    if bad:
        fail(f"non-{BONE_PREFIX}-prefixed bone(s) found: {bad}")
    if len(bones) != EXPECTED_BONE_COUNT:
        fail(f"expected {EXPECTED_BONE_COUNT} {BONE_PREFIX} bones, found {len(bones)}: "
             f"{sorted(b.name for b in bones)}")
    print(f"probe: {len(bones)} {BONE_PREFIX} bones, all asserted")

    mw = arm.matrix_world
    joints = {}
    for b in bones:
        head = mw @ b.head_local
        tail = mw @ b.tail_local
        joints[b.name] = {
            "head": [round(head.x, 6), round(head.y, 6), round(head.z, 6)],
            "tail": [round(tail.x, 6), round(tail.y, 6), round(tail.z, 6)],
            # Blender's FBX importer invents a display length for leaf bones
            # (no child to derive one from) by copying the parent segment's
            # length -- e.g. HeadTop_End's tail sits 0.2427 m past its head,
            # exactly Head's own length, not real skull geometry. A leaf
            # bone's tail is therefore not real-world data; head always is.
            "is_leaf": len(b.children) == 0,
        }

    data = {
        "source_fbx": str(Path(args.fbx).resolve()),
        "bone_count": len(bones),
        "up_axis": "Z",
        "joints": joints,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(data, indent=2, sort_keys=True), encoding="utf-8")
    print(f"probe: wrote {args.out}")


def head_derived_points(joints):
    """Nose/eyes/ears have no dedicated Mixamo bone -- derive them from the
    Head bone's own extent (head = base of skull, tail = top of skull).
    World +X is the character's own right (RightArm etc. all read positive
    X in the T-pose probe), so REye/REar sit on the +X side."""
    hx, _, hz = joints[BONE_PREFIX + "Head"]["head"]
    tx, _, tz = joints[BONE_PREFIX + "Head"]["tail"]
    length = tz - hz
    eye_z = hz + 0.65 * length
    return {
        "Nose": ((hx + tx) / 2, (hz + tz) / 2),
        "REye": (hx + 0.18 * length, eye_z),
        "LEye": (hx - 0.18 * length, eye_z),
        "REar": (hx + 0.42 * length, eye_z),
        "LEar": (hx - 0.42 * length, eye_z),
    }


def mixamo_to_coco(joints):
    """World-space (x, z) for each COCO-18 keypoint, in COCO_KEYPOINTS order."""
    derived = head_derived_points(joints)
    points = []
    for name in COCO_KEYPOINTS:
        if name in derived:
            points.append(derived[name])
        else:
            x, _, z = joints[BONE_PREFIX + JOINT_SOURCE[name]]["head"]
            points.append((x, z))
    return points


def full_bounds(joints):
    """(x, z) bounding box over every probed bone's real joint positions --
    the whole-body extent (fingers, toes, head top) a concept photo would
    show, not just the abbreviated COCO-18 subset. Leaf bones' tails are
    Blender-invented display lengths (see probe's is_leaf comment), so only
    a leaf's head -- the real joint one segment up -- is used for those."""
    pts = [j["head"] for j in joints.values()]
    pts += [j["tail"] for j in joints.values() if not j["is_leaf"]]
    xs = [p[0] for p in pts]
    zs = [p[2] for p in pts]
    return min(xs), max(xs), min(zs), max(zs)


def cmd_draw(argv):
    from PIL import Image, ImageDraw  # deferred: Hi3DGen venv only

    parser = argparse.ArgumentParser(prog="char_pose.py draw")
    parser.add_argument("--joints", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)

    data = json.loads(args.joints.read_text(encoding="utf-8"))
    joints = data["joints"]

    min_x, max_x, min_z, max_z = full_bounds(joints)
    extent_x, extent_z = max_x - min_x, max_z - min_z
    center_x, center_z = (min_x + max_x) / 2, (min_z + max_z) / 2
    scale = (CANVAS_SIZE * FIGURE_FILL_FRACTION) / max(extent_x, extent_z)

    def to_screen(x, z):
        # Character's own right (+X) renders on the image's left, as when
        # facing someone -- mirrored, matching OpenPose's L/R convention.
        return (CANVAS_SIZE / 2 - (x - center_x) * scale,
                CANVAS_SIZE / 2 - (z - center_z) * scale)

    keypoints_px = [to_screen(x, z) for x, z in mixamo_to_coco(joints)]

    img = Image.new("RGB", (CANVAS_SIZE, CANVAS_SIZE), (0, 0, 0))
    draw = ImageDraw.Draw(img)
    for i, (a, b) in enumerate(LIMBS):
        color = tuple(int(c * STICK_COLOR_SCALE) for c in LIMB_COLORS[i])
        pa, pb = keypoints_px[a], keypoints_px[b]
        draw.line([pa, pb], fill=color, width=STICK_WIDTH)
        r = STICK_WIDTH / 2
        for p in (pa, pb):  # round the line's square-cut ends into a capsule
            draw.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=color)
    for i, (x, y) in enumerate(keypoints_px):
        r = KEYPOINT_RADIUS
        draw.ellipse([x - r, y - r, x + r, y + r], fill=LIMB_COLORS[i])

    args.out.parent.mkdir(parents=True, exist_ok=True)
    img.save(args.out, format="PNG")

    sidecar = {
        "canvas_size": CANVAS_SIZE,
        "figure_fill_fraction": FIGURE_FILL_FRACTION,
        "scale_px_per_m": scale,
        "figure_pixel_height": extent_z * scale,
        "figure_pixel_width": extent_x * scale,
        "keypoints_px": {name: [round(p[0], 2), round(p[1], 2)]
                         for name, p in zip(COCO_KEYPOINTS, keypoints_px)},
    }
    sidecar_path = args.out.with_suffix(".json")
    sidecar_path.write_text(json.dumps(sidecar, indent=2, sort_keys=True), encoding="utf-8")
    print(f"draw: wrote {args.out} and {sidecar_path}")


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    if not argv or argv[0] not in ("probe", "draw"):
        fail("usage: char_pose.py {probe <fbx> --out <joints.json>|draw --joints <joints.json> --out <png>}")
    step, rest = argv[0], argv[1:]
    (cmd_probe if step == "probe" else cmd_draw)(rest)


try:
    main()
except SystemExit:
    raise
except Exception:
    # without --python-exit-code Blender exits 0 on an uncaught script
    # exception -- route every failure through an explicit non-zero exit
    traceback.print_exc()
    sys.exit(1)
