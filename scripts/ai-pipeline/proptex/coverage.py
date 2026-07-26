"""Island coverage and next-best-view selection for the multiview retexture
atlas (prop_texture.py's basecolor path). Geometry only: coverage is facing/
frustum/occlusion against the depth renders, never the generated pixels.

Also a standalone entry point that reports coverage for an archived
candidate's clean mesh, reproducing the pipeline's geometry path (import,
camera rig, atlas bake, depth renders, next-best-view pick) with no
generation attached.

Usage: blender --background --python proptex/coverage.py -- \
           <clean.glb> --asset NAME --map <hole_map.png>
"""

import sys
from pathlib import Path

if __name__ == "__main__":
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import argparse
import json
import tempfile
from math import cos, radians

import bpy
import cv2
import numpy as np

from proptex.atlas import atlas_size, bake_geometry_atlas, view_weight
from proptex.registry import resolve
from proptex.scene import import_glb, new_image, save_png
from proptex.views import (
    MV_ELEVATION_DEG, depth_setup, mv_camera_rig, mv_view, read_depth,
    render_depth_view, view_hint,
)

MV_COVERAGE_EPS = 1e-4  # a texel counts as covered above this summed blend
# weight; shared by covered_mask's every caller (albedo.py's blend_views and
# pick_extra_views below) so they cannot drift

MAX_HOLE_DEPTH_FRAC = 0.015  # max per-component extrapolation depth, as a
# fraction of atlas width, before Telea is filling material rather than
# scattered noise


class CoverageFailure(Exception):
    pass


# Coverage-driven extra views (clean-room Text2Tex next-best-view):
MV_EXTRA_MAX = 2  # at most one extra canvas (two side-by-side views)
MV_EXTRA_MIN_GAIN = 0.03  # an extra view must newly cover >=3% of island
# texels; below that a pick only trims rims off charts the set already reaches
# and moves no hole component's extrapolation depth
MV_EXTRA_CANDIDATE_AZIMUTHS = tuple(range(0, 360, 30))
MV_EXTRA_CANDIDATE_ELEVATIONS = (-35.0, 15.0, 55.0)  # on standing props the
# uncovered set is dominated by DOWN-facing texels (cup/arm/base undersides:
# 45% of the candelabra's holes vs 33% up-facing), so a below view recovers
# about twice what the best above view does
MV_EXTRA_TOP_ELEVATION = 75.0  # near-top recovery view; at 90 the camera
# basis s = f x z degenerates to zero
MV_EXTRA_MIN_SEP_DEG = 20.0  # candidates within this of an existing view
# direction see nearly the same texels — skip before spending a render


def covered_mask(wsum, island):
    """Island texels a view set's summed blend weight actually reached."""
    return island & (wsum > MV_COVERAGE_EPS)


def view_coverage(views, depths, rig, pos, nrm):
    """Summed per-texel geometric blend weight over a view set.
    Initialise with np.zeros(pos.shape[0]) so an empty view set returns an
    array, not the int 0."""
    wsum = np.zeros(pos.shape[0])
    for v, depth in zip(views, depths):
        wsum += view_weight(v, depth, rig, pos, nrm)[0]
    return wsum


def _hole_labels(covered, island):
    """8-connected component labelling of the hole mask (island & ~covered):
    (n_labels, labels, atlas size)."""
    size = atlas_size(island)
    holes = island & ~covered
    n_labels, labels = cv2.connectedComponents(
        holes.reshape(size, size).astype(np.uint8), connectivity=8)
    return n_labels, labels, size


def hole_component_depths(covered, island):
    """Per 8-connected hole component: the maximum L2 distance, in
    atlas-width fractions, from a hole texel to the nearest *covered*
    texel. The distance transform's source set is the covered mask alone
    -- out-of-island gutter is not a source, so a hole enclosed by texture
    measures about half its width while one running off a chart edge
    measures its full width. Returns a list of {"label", "texels",
    "depth_frac"} sorted deepest-first."""
    n_labels, labels, size = _hole_labels(covered, island)
    if n_labels <= 1:
        return []
    src = (~covered).reshape(size, size).astype(np.uint8)
    dist = cv2.distanceTransform(src, cv2.DIST_L2, cv2.DIST_MASK_PRECISE)
    components = []
    for label in range(1, n_labels):
        mask = labels == label
        components.append({
            "label": label,
            "texels": int(mask.sum()),
            "depth_frac": float(dist[mask].max()) / size,
        })
    components.sort(key=lambda c: -c["depth_frac"])
    return components


def coverage_stats(covered, island):
    """{"blend_coverage": float, "hole_texels": int, "max_hole_depth_frac": float}"""
    blend_coverage = float(covered[island].mean()) if island.any() else 0.0
    holes = island & ~covered
    depths = hole_component_depths(covered, island)
    max_hole_depth_frac = max((c["depth_frac"] for c in depths), default=0.0)
    return {
        "blend_coverage": round(blend_coverage, 4),
        "hole_texels": int(holes.sum()),
        "max_hole_depth_frac": round(max_hole_depth_frac, 4),
    }


def rank_candidates(cands, cand_depths, rig, pos, nrm, target, top_n=3):
    """Candidate directions ranked by how many `target` texels they would
    newly cover, restricted to those with any overlap at all -- the same
    per-candidate scoring pick_extra_views uses, against a caller-chosen
    target mask instead of the full uncovered set."""
    scored = []
    for (spec, cand), depth in zip(cands, cand_depths):
        gain = int(covered_mask(view_coverage([cand], [depth], rig, pos, nrm), target).sum())
        if gain > 0:
            scored.append((gain, spec))
    scored.sort(key=lambda t: -t[0])
    return [{"hint": spec[0], "azimuth_deg": spec[1], "elevation_deg": spec[2],
             "covers_texels": gain}
            for gain, spec in scored[:top_n]]


def extra_candidates(views, rig):
    """The next-best-view candidate directions as (spec, view) pairs: the
    declared azimuth/elevation grid plus one near-top view, minus every
    direction within MV_EXTRA_MIN_SEP_DEG of a view already in the set
    (those see nearly the same texels, so they are dropped before a render
    is spent on them). Nothing here depends on a pick, so the whole
    candidate set — and every candidate's depth render — is addressable
    before the first pick is made."""
    specs = [(view_hint(a, e), a, e)
             for e in MV_EXTRA_CANDIDATE_ELEVATIONS
             for a in MV_EXTRA_CANDIDATE_AZIMUTHS]
    specs.append((view_hint(0.0, MV_EXTRA_TOP_ELEVATION), 0.0,
                  MV_EXTRA_TOP_ELEVATION))
    min_dot = cos(radians(MV_EXTRA_MIN_SEP_DEG))
    cands = [(spec, mv_view(*spec, rig)) for spec in specs]
    return [(spec, cand) for spec, cand in cands
            if all(float(cand["f"] @ v["f"]) < min_dot for v in views)]


def pick_extra_views(views, depths, cands, cand_depths, rig, pos, nrm, island):
    """Greedy next-best-view pick over the candidate set: coverage is purely
    geometric (facing/frustum/occlusion, never the generated pixels), so
    extras are picked before any generation and a re-run re-derives the same
    picks. Returns one entry per pick — its direction and the gain it was
    predicted to add. `cand_depths` is iterated exactly once, so it may be a
    generator that loads one candidate's depth at a time."""
    uncovered = island & ~covered_mask(view_coverage(views, depths, rig, pos, nrm), island)
    masks = [covered_mask(view_coverage([cand], [depth], rig, pos, nrm), island)
             for (_, cand), depth in zip(cands, cand_depths)]
    min_dot = cos(radians(MV_EXTRA_MIN_SEP_DEG))

    extra_meta = []
    island_total = int(island.sum())
    while len(extra_meta) < MV_EXTRA_MAX and cands:
        gains = [int((m & uncovered).sum()) for m in masks]
        best = int(np.argmax(gains))
        if gains[best] < MV_EXTRA_MIN_GAIN * island_total:
            break
        spec, best_view = cands[best]
        uncovered &= ~masks[best]
        keep = [j for j, (_, cand) in enumerate(cands)
                if j != best and float(cand["f"] @ best_view["f"]) < min_dot]
        cands = [cands[j] for j in keep]
        masks = [masks[j] for j in keep]
        extra_meta.append({
            "hint": spec[0], "azimuth_deg": spec[1], "elevation_deg": spec[2],
            "predicted_gain_texels": gains[best],
            "predicted_gain_frac": round(gains[best] / max(island_total, 1), 4),
        })
    return extra_meta


def pick_indices(cands, extra_meta):
    """Index into `cands` of each pick, in pick order: every pick is a
    candidate, so its depth render is already done and is addressed by
    direction rather than by position in a view list."""
    by_dir = {(float(az), float(el)): j for j, ((_, az, el), _) in enumerate(cands)}
    return [by_dir[(float(m["azimuth_deg"]), float(m["elevation_deg"]))]
            for m in extra_meta]


def _render_depths(clean, views, rig, view_res, out_root):
    """Render each view's depth into its own out_root/<i>/ directory;
    returns those directories.

    Deliberately uncached, into a caller-supplied temp root: this report is a
    survey run across archived meshes, and populating the stage cache from it
    would turn a later rebuild of those same props into a partial hit, whose
    chain total would then price a warm run rather than the cold one."""
    out_dirs = [out_root / str(i) for i in range(len(views))]
    with depth_setup(clean, rig, view_res) as cam_obj:
        for v, out_dir in zip(views, out_dirs):
            out_dir.mkdir(parents=True)
            render_depth_view(cam_obj, v, out_dir)
    return out_dirs


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="coverage.py")
    parser.add_argument("clean_glb")
    parser.add_argument("--asset", required=True,
                        help="Registered asset name (content/models/assets.json); resolves "
                             "the azimuths/texture_size/view_res contract (proptex.registry) "
                             "this report measures coverage against")
    parser.add_argument("--map", required=True,
                        help="PNG output path for the hole map: black outside the island, "
                             "dark grey for covered island texels, red for uncovered ones")
    args = parser.parse_args(argv)

    contract = resolve(args.asset)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    clean = import_glb(args.clean_glb)

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 1
    scene.cycles.use_denoising = False

    me = clean.data
    if not me.uv_layers:
        print("coverage: clean mesh carries no UV atlas — re-run prop_cleanup",
              file=sys.stderr)
        sys.exit(1)
    atlas = me.uv_layers[0].name

    with tempfile.TemporaryDirectory() as work_dir_str:
        work_dir = Path(work_dir_str)
        mv_specs = [(view_hint(a), a, MV_ELEVATION_DEG) for a in contract.azimuths]
        views, rig = mv_camera_rig(clean, mv_specs)
        bake_geometry_atlas(clean, atlas, rig, contract.texture_size, work_dir)
        pos, nrm, island = (np.load(work_dir / name)
                            for name in ("pos.npy", "nrm.npy", "island.npy"))
        base_dirs = _render_depths(clean, views, rig, contract.view_res,
                                   work_dir / "base")
        depths = [read_depth(d / "depth.exr") for d in base_dirs]
        base_covered = covered_mask(view_coverage(views, depths, rig, pos, nrm), island)
        base_stats = coverage_stats(base_covered, island)

        cands = extra_candidates(views, rig)
        cand_dirs = _render_depths(clean, [cand for _, cand in cands], rig,
                                   contract.view_res, work_dir / "candidate")
        extra_meta = pick_extra_views(
            views, depths, cands, (read_depth(d / "depth.exr") for d in cand_dirs),
            rig, pos, nrm, island)
        for j in pick_indices(cands, extra_meta):
            views.append(cands[j][1])
            depths.append(read_depth(cand_dirs[j] / "depth.exr"))

        covered = covered_mask(view_coverage(views, depths, rig, pos, nrm), island)
        stats = coverage_stats(covered, island)
        over = [c for c in hole_component_depths(covered, island)
                if c["depth_frac"] > MAX_HOLE_DEPTH_FRAC]

        size = atlas_size(island)
        hole_buf = np.zeros((island.shape[0], 4), dtype=np.float32)
        hole_buf[:, 3] = 1.0
        hole_buf[covered, :3] = 0.25
        hole_buf[island & ~covered, :3] = (1.0, 0.0, 0.0)
        hole_img = new_image("mv_holes", srgb=True, fill=(0, 0, 0), size=size)
        hole_img.pixels.foreach_set(hole_buf.ravel())
        save_png(hole_img, args.map)
        bpy.data.images.remove(hole_img)

        if over:
            _, labels, _ = _hole_labels(covered, island)
            offending = np.isin(labels, [c["label"] for c in over]).reshape(-1)
            directions = rank_candidates(
                cands, (read_depth(d / "depth.exr") for d in cand_dirs),
                rig, pos, nrm, offending)
            depths_str = ", ".join(f"{c['depth_frac']:.2%}" for c in over[:5])
            if len(over) > 5:
                depths_str += f", +{len(over) - 5} more"
            print(f"coverage: {args.asset}: {len(over)} hole component(s) exceed "
                  f"MAX_HOLE_DEPTH_FRAC={MAX_HOLE_DEPTH_FRAC:.3f} (deepest {depths_str})",
                  file=sys.stderr)
            print(f"coverage: uncovered-island map: {args.map}", file=sys.stderr)
            if directions:
                print("coverage: candidate directions that would newly cover them: "
                      + ", ".join(f"{d['hint']} (+{d['covers_texels']} texels)"
                                  for d in directions),
                      file=sys.stderr)
            else:
                print("coverage: no candidate direction newly covers the offending "
                      "component(s)", file=sys.stderr)
            sys.exit(1)

    print(json.dumps({
        "asset": args.asset,
        "coverage": stats,
        "base_coverage": base_stats,
        "extra_views": extra_meta,
        "map": args.map,
    }))


if __name__ == "__main__":
    main()
