# Blender-headless: camera-space-normal contact sheet for one raw Hi3DGen
# candidate, ahead of texturing -- untextured geometry reads best in normal
# shading, and reusing proptex.views' camera rig keeps the framing identical
# to the retexture stage's own views.
#
# Usage: blender --background --python mv_ab_render.py -- \
#            <raw.glb> <out_dir> --yaw <fitted_yaw_deg>

import argparse
import sys
import traceback
from pathlib import Path

import bpy  # noqa: F401  (required before importing proptex.* under Blender's Python)
import cv2
import numpy as np

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))
from proptex.scene import import_glb  # noqa: E402
from proptex.views import MV_ELEVATION_DEG, mv_camera_rig, normal_setup, render_exr, view_hint  # noqa: E402

VIEW_RES = 512


def normal_png(rgb):
    """A rendered camera-space-normal EXR's RGB as flipped 8-bit BGR for
    cv2.imwrite -- img_array (render_exr's return) rows are bottom-up."""
    rgb8 = (np.clip(np.flipud(rgb), 0.0, 1.0) * 255.0).round().astype(np.uint8)
    return rgb8[:, :, ::-1]


def render_candidate(raw_glb: Path, out_dir: Path, yaw: float) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    obj = import_glb(raw_glb)
    azimuths = [(yaw + offset) % 360 for offset in (0, 90, 180, 270)]
    specs = [(view_hint(az, MV_ELEVATION_DEG), az, MV_ELEVATION_DEG) for az in azimuths]
    views, rig = mv_camera_rig(obj, specs)

    panels = []
    with normal_setup(obj, rig, VIEW_RES) as cam_obj:
        for i, v in enumerate(views):
            exr_path = out_dir / f"normal_{i}.exr"
            rgb = render_exr(cam_obj, v, exr_path)[:, :, :3]
            exr_path.unlink()
            panels.append(normal_png(rgb))

    sheet_path = out_dir / "contact_sheet.png"
    cv2.imwrite(str(sheet_path), cv2.hconcat(panels))
    return sheet_path


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="mv_ab_render.py")
    parser.add_argument("raw_glb", type=Path)
    parser.add_argument("out_dir", type=Path)
    parser.add_argument("--yaw", type=float, required=True, help="Fitted front yaw (mv_ab_metrics.py's fitted_yaw_deg); the 4 panels sit at +0/90/180/270 from it.")
    args = parser.parse_args(argv)

    try:
        sheet_path = render_candidate(args.raw_glb.resolve(), args.out_dir.resolve(), args.yaw)
    except Exception:
        traceback.print_exc()
        sys.exit(1)
    print(f"OK: wrote {sheet_path}")


if __name__ == "__main__":
    main()
