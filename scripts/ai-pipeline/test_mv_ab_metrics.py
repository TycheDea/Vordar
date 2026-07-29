#!/usr/bin/env python3
"""Plain-assert tests for mv_ab_metrics.py, run under the Hi3DGen venv:
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe scripts/ai-pipeline/test_mv_ab_metrics.py

Both assertions are invariants (an analytic bbox aspect ratio, a yaw
recovered exactly on its own scan grid), not calibrated bands -- a failure
means the instrument is wrong, not that a tolerance needs loosening.
"""
import json
import subprocess
import sys
import tempfile
from pathlib import Path

import cv2
import numpy as np
import trimesh

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mv_ab_metrics as mvab

RAW_GLB = (Path(__file__).resolve().parents[2]
           / "target/prop-solid-validation/chapel_arch_e2e/cand_0/raw.glb")


def test_analytic_projection_truth():
    """A 1x2x3 box viewed head-on along -Y (az=0, el=0) silhouettes as its
    XZ face: a 1x3 rectangle, exactly filling its own bounding box."""
    mesh = trimesh.creation.box(extents=[1, 2, 3])
    rig = mvab.build_rig(mesh)
    mask = mvab.render_mask(mesh, az_deg=0, el_deg=0, rig=rig)

    ys, xs = np.nonzero(mask)
    assert len(xs) > 0, "box projected to an empty mask"
    height = ys.max() - ys.min() + 1
    width = xs.max() - xs.min() + 1
    aspect = height / width
    assert abs(aspect - 3.0) / 3.0 < 0.01, f"expected bbox aspect ~3.0, got {aspect}"

    bbox = mask[ys.min():ys.max() + 1, xs.min():xs.max() + 1]
    fill = (bbox > 0).sum() / bbox.size
    assert fill >= 0.999, f"expected a filled rectangle, got fill fraction {fill}"
    print(f"test_analytic_projection_truth passed (aspect={aspect:.4f}, fill={fill:.4f})")


def test_yaw_fit_recovers_grid_azimuth():
    """A mesh's own silhouette at az=20/el=15, fed back in as the --front
    mask, must fit to yaw=20 (on the 5-degree scan grid) with near-total
    overlap against itself."""
    assert RAW_GLB.exists(), f"missing fixture {RAW_GLB}"
    mesh = mvab.load_mesh(RAW_GLB)
    rig = mvab.build_rig(mesh)
    mask = mvab.render_mask(mesh, az_deg=20, el_deg=15, rig=rig)

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        front_path = tmp / "front.png"
        rgba = np.zeros((mask.shape[0], mask.shape[1], 4), dtype=np.uint8)
        rgba[:, :, :3] = 255
        rgba[:, :, 3] = mask
        cv2.imwrite(str(front_path), rgba)

        out_path = tmp / "metrics.json"
        subprocess.run(
            [sys.executable, str(Path(__file__).resolve().parent / "mv_ab_metrics.py"),
             str(RAW_GLB), "--front", str(front_path), "--out", str(out_path)],
            check=True,
        )
        metrics = json.loads(out_path.read_text())

    assert metrics["fitted_yaw_deg"] == 20, f"expected yaw 20, got {metrics['fitted_yaw_deg']}"
    assert metrics["iou_front"] > 0.99, f"expected iou_front > 0.99, got {metrics['iou_front']}"
    print(f"test_yaw_fit_recovers_grid_azimuth passed "
          f"(yaw={metrics['fitted_yaw_deg']}, iou_front={metrics['iou_front']:.4f})")


def test_gltf_y_up_box_renders_tall_not_wide():
    """A box tall along glTF's Y axis (extents=[1, 3, 1]), exported to .glb
    and loaded through the CLI's own trimesh.load path, must render taller
    than wide (aspect > 2): Y is glTF's up axis, so its tall side is the one
    the Z-up camera math must see as "up" once converted. A box built and
    rendered in the same (Z-up) frame, like test_analytic_projection_truth's,
    can't exercise this -- it never crosses the glTF/Blender frame boundary
    that mv_ab_metrics.py's load path does."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        box = trimesh.creation.box(extents=[1, 3, 1])
        glb_path = tmp / "tall_box.glb"
        box.export(str(glb_path))

        # front reference is a filler; only the rendered mask's own aspect matters here
        front_path = tmp / "front.png"
        dummy = np.zeros((64, 64, 4), dtype=np.uint8)
        dummy[16:48, 16:48] = 255
        cv2.imwrite(str(front_path), dummy)

        out_path = tmp / "metrics.json"
        masks_dir = tmp / "masks"
        subprocess.run(
            [sys.executable, str(Path(__file__).resolve().parent / "mv_ab_metrics.py"),
             str(glb_path), "--front", str(front_path), "--out", str(out_path),
             "--masks-dir", str(masks_dir)],
            check=True,
        )
        rendered = cv2.imread(str(masks_dir / "front_rendered.png"), cv2.IMREAD_UNCHANGED)

    ys, xs = np.nonzero(rendered)
    assert len(xs) > 0, "tall box projected to an empty mask"
    height = ys.max() - ys.min() + 1
    width = xs.max() - xs.min() + 1
    aspect = height / width
    assert aspect > 2.0, f"expected a glTF Y-up box to render taller than wide (aspect > 2), got {aspect}"
    print(f"test_gltf_y_up_box_renders_tall_not_wide passed (aspect={aspect:.4f})")


if __name__ == "__main__":
    test_analytic_projection_truth()
    test_yaw_fit_recovers_grid_azimuth()
    test_gltf_y_up_box_renders_tall_not_wide()
    print("all tests passed")
