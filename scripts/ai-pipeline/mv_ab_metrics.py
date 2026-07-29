#!/usr/bin/env python3
"""Silhouette-IoU and raw-mesh-stats instrument for A/B'ing a raw Hi3DGen
mesh against its multi-view concept art, ahead of texturing.

Projects the mesh with the same ortho camera convention proptex/views.py
uses for the retexture rig (mv_view's d/f/s/u construction, MV_ELEVATION_DEG
elevation, Z-up), so silhouette comparisons here are apples-to-apples with
what the ControlNet-depth stage will actually see. Runs outside Blender, so
the convention is mirrored in plain numpy rather than imported -- and since
trimesh.load keeps the raw glTF Y-up frame instead of the Y-up->Z-up
conversion Blender's glTF importer applies, main() converts vertices
(x, y, z) -> (x, -z, y) once at load, before any camera math runs.

Run under the Hi3DGen venv (trimesh/numpy/opencv-python-headless are
pinned there, no new dependency):
C:\\tools\\Hi3DGen\\venv\\Scripts\\python.exe scripts/ai-pipeline/mv_ab_metrics.py <raw.glb> --front F.png [--back B.png] [--side S.png] --out metrics.json [--masks-dir DIR]
"""
import argparse
import json
from math import cos, radians, sin
from pathlib import Path

import cv2
import numpy as np
import trimesh

MV_ELEVATION_DEG = 15.0  # matches proptex/views.py's MV_ELEVATION_DEG
SCAN_STEP_DEG = 5
CANVAS_PX = 512
NORM_PX = 256
ALPHA_THRESHOLD = 0.8 * 255  # preprocess_image's / check_matte's own bbox cut


def load_mesh(path):
    """Loads a mesh into the Z-up frame view_axes assumes. glTF is Y-up,
    and trimesh.load keeps that raw frame where Blender's importer would
    convert it, so the (x, y, z) -> (x, -z, y) rotation happens here."""
    mesh = trimesh.load(str(path), force="mesh")
    v = mesh.vertices
    mesh.vertices = np.column_stack([v[:, 0], -v[:, 2], v[:, 1]])
    return mesh


def view_axes(az_deg, el_deg):
    """Camera right/up unit vectors for an az/el ortho view, mirroring
    proptex/views.py's mv_view (d = center->camera, f = -d, s = f x Z, u =
    s x f) so renders here align with the retexture rig's convention. Assumes
    a Z-up mesh frame (Blender's convention); callers must convert a raw
    glTF (Y-up) mesh's vertices before rendering with this."""
    az, el = radians(az_deg), radians(el_deg)
    d = np.array([sin(az) * cos(el), -cos(az) * cos(el), sin(el)])
    f = -d
    s = np.cross(f, [0.0, 0.0, 1.0])
    s = s / np.linalg.norm(s)
    u = np.cross(s, f)
    return s, u


def build_rig(mesh):
    """Fixed world window for every view of one mesh: center = bbox center,
    half-width = bbox diagonal/2 * 1.05, matching mv_camera_rig's rig so all
    views of a mesh share one framing."""
    lo, hi = mesh.vertices.min(axis=0), mesh.vertices.max(axis=0)
    return {
        "center": (lo + hi) / 2,
        "half": float(np.linalg.norm(hi - lo) / 2) * 1.05,
    }


def render_mask(mesh, az_deg, el_deg, rig, canvas_px=CANVAS_PX):
    """Rasterizes mesh's ortho silhouette at (az_deg, el_deg) into a
    canvas_px-square uint8 mask (255 = covered): the union of every face's
    projection, without a depth buffer.

    Each face is painted with its own cv2.fillConvexPoly call rather than
    one batched cv2.fillPoly(canvas, all_faces, 255) call -- a single
    multi-contour fillPoly fills by edge parity, not union, so a closed
    surface's front and back faces (an even number of layers at every
    interior silhouette point, by the ray-crossing parity of any closed
    manifold) cancel each other out there instead of union-filling."""
    s, u = view_axes(az_deg, el_deg)
    rel = mesh.vertices - rig["center"]
    half = rig["half"]
    px = ((rel @ s) / (2 * half) + 0.5) * canvas_px
    py = (1.0 - ((rel @ u) / (2 * half) + 0.5)) * canvas_px
    # round (not truncate) to pixel centers -- astype alone truncates toward
    # zero, which biases a coarse mesh's bbox extent asymmetrically
    pts = np.round(np.stack([px, py], axis=1)).astype(np.int32)
    tri = pts[mesh.faces]
    canvas = np.zeros((canvas_px, canvas_px), dtype=np.uint8)
    for t in tri:
        cv2.fillConvexPoly(canvas, t, 255)
    return canvas


def normalize_mask(mask, norm_px=NORM_PX):
    """Crops a mask to its nonzero bbox, letterboxes it to square, and
    resizes to norm_px so a rendered mask and a concept-art mask -- which
    differ in resolution and framing -- land on the same footing for IoU."""
    ys, xs = np.nonzero(mask)
    if len(xs) == 0:
        return np.zeros((norm_px, norm_px), dtype=bool)
    x0, x1 = xs.min(), xs.max() + 1
    y0, y1 = ys.min(), ys.max() + 1
    crop = (mask[y0:y1, x0:x1] > 0).astype(np.uint8) * 255
    h, w = crop.shape
    side = max(h, w)
    letterboxed = np.zeros((side, side), dtype=np.uint8)
    yo, xo = (side - h) // 2, (side - w) // 2
    letterboxed[yo:yo + h, xo:xo + w] = crop
    resized = cv2.resize(letterboxed, (norm_px, norm_px), interpolation=cv2.INTER_NEAREST)
    return resized > 127


def iou(a, b):
    union = np.logical_or(a, b).sum()
    if union == 0:
        return 0.0
    return float(np.logical_and(a, b).sum()) / float(union)


def load_concept_mask(png_path):
    rgba = cv2.imread(str(png_path), cv2.IMREAD_UNCHANGED)
    if rgba is None:
        raise ValueError(f"cannot read {png_path}")
    if rgba.ndim != 3 or rgba.shape[2] != 4:
        raise ValueError(f"{png_path} has no alpha channel")
    return rgba[:, :, 3] > ALPHA_THRESHOLD


def fit_yaw(mesh, rig, front_mask_norm, scan_step=SCAN_STEP_DEG, elevation=MV_ELEVATION_DEG):
    best_az, best_iou = 0, -1.0
    for az in range(0, 360, scan_step):
        val = iou(normalize_mask(render_mask(mesh, az, elevation, rig)), front_mask_norm)
        if val > best_iou:
            best_iou, best_az = val, az
    return best_az, best_iou


def side_fit(mesh, rig, side_mask_norm, yaw, elevation=MV_ELEVATION_DEG):
    """A single side-view panel doesn't say which side, so try both and
    report the better."""
    best_az, best_iou = None, -1.0
    for az in ((yaw + 90) % 360, (yaw - 90) % 360):
        val = iou(normalize_mask(render_mask(mesh, az, elevation, rig)), side_mask_norm)
        if val > best_iou:
            best_iou, best_az = val, az
    return best_az, best_iou


def raw_stats(mesh):
    """Hollow-valid connectivity stats -- no volume/watertight fields, since
    a raw Hi3DGen mesh need not be watertight."""
    components = trimesh.graph.connected_components(
        mesh.face_adjacency, min_len=0, nodes=np.arange(len(mesh.faces)))
    boundary_edges = trimesh.grouping.group_rows(mesh.edges_sorted, require_count=1)
    sizes = [len(c) for c in components] or [0]
    face_count = len(mesh.faces)
    return {
        "vertex_count": int(len(mesh.vertices)),
        "face_count": int(face_count),
        "component_count": int(len(components)),
        "boundary_edge_count": int(len(boundary_edges)),
        "main_face_fraction": float(max(sizes) / face_count) if face_count else 0.0,
    }


def main():
    parser = argparse.ArgumentParser(
        description="Silhouette-IoU and raw-mesh-stats A/B metrics for a raw Hi3DGen mesh.")
    parser.add_argument("mesh", help="raw.glb to measure")
    parser.add_argument("--front", required=True, help="front concept_rgba PNG")
    parser.add_argument("--back", help="back concept_rgba PNG")
    parser.add_argument("--side", help="side concept_rgba PNG")
    parser.add_argument("--out", required=True, help="output metrics.json path")
    parser.add_argument("--masks-dir", help="dump rendered/normalized masks here for eyeball checks")
    args = parser.parse_args()

    mesh = load_mesh(args.mesh)
    rig = build_rig(mesh)

    masks_dir = Path(args.masks_dir) if args.masks_dir else None
    if masks_dir:
        masks_dir.mkdir(parents=True, exist_ok=True)

    front_norm = normalize_mask(load_concept_mask(args.front))
    yaw, iou_front = fit_yaw(mesh, rig, front_norm)

    result = {
        "fitted_yaw_deg": yaw,
        "iou_front": iou_front,
        "elevation_deg": MV_ELEVATION_DEG,
        "scan_step_deg": SCAN_STEP_DEG,
        "canvas_px": CANVAS_PX,
        "norm_px": NORM_PX,
        "raw_stats": raw_stats(mesh),
    }

    if masks_dir:
        cv2.imwrite(str(masks_dir / "front_concept_norm.png"), front_norm.astype(np.uint8) * 255)
        rendered_front = render_mask(mesh, yaw, MV_ELEVATION_DEG, rig)
        cv2.imwrite(str(masks_dir / "front_rendered.png"), rendered_front)
        cv2.imwrite(str(masks_dir / "front_rendered_norm.png"),
                    normalize_mask(rendered_front).astype(np.uint8) * 255)

    if args.back:
        back_norm = normalize_mask(load_concept_mask(args.back))
        back_az = (yaw + 180) % 360
        result["iou_back"] = iou(normalize_mask(render_mask(mesh, back_az, MV_ELEVATION_DEG, rig)), back_norm)
        if masks_dir:
            cv2.imwrite(str(masks_dir / "back_concept_norm.png"), back_norm.astype(np.uint8) * 255)

    if args.side:
        side_norm = normalize_mask(load_concept_mask(args.side))
        side_azimuth, iou_side = side_fit(mesh, rig, side_norm, yaw)
        result["side_azimuth"] = side_azimuth
        result["iou_side"] = iou_side
        if masks_dir:
            cv2.imwrite(str(masks_dir / "side_concept_norm.png"), side_norm.astype(np.uint8) * 255)

    Path(args.out).write_text(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
