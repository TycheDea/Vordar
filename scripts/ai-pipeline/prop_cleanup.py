# Blender-headless: Hi3DGen raw prop mesh -> normalized clean.glb +
# pre-decimation _hires.glb (Phase A3.5).
#
#   - strips loose fragments whose bbox diagonal is under 2% of the whole
#     mesh's (sparse-lattice floaters); runs BEFORE normalization so a
#     floater can't skew the height/ground fit
#   - strips interior faces: a face with no ray escaping the mesh from just
#     outside its own surface can never reach a camera, so it is removed
#     before the tri budget and UV atlas are spent describing it
#   - uniform-scales to --height, origin at the footprint centroid (bbox
#     center of the ground-contact band), mesh bottom exactly at y=0 in
#     the exported +Y-up glb — zone props sit on the ground plane via
#     pos y=-0.5 in zones.ron, the model itself must carry no offset
#   - --symmetrize (opt-in): finds the best-fit vertical mirror plane and
#     mirrors the --symmetrize-keep half across it; exported orientation
#     is unchanged (the mesh is rotated back), so existing zone placements
#     stay valid
#   - exports <clean stem>_hires.glb before decimating (A3.6's high-poly
#     normal-bake source), then collapse-decimates to --tri-budget
#   - xatlas-unwraps the decimated mesh into a single UV atlas; every
#     prop_texture.py bake targets this layer, so the atlas stays stable
#     across texture re-runs (the hires mesh needs no UVs — the normal
#     bake is selected-to-active onto the clean mesh's layer)
#   - prints one JSON stats line (the only '{'-prefixed stdout line) for
#     gen_prop.py's chained manifest
#
# Structural failures (no mesh, no faces, zero area, zero height,
# unreachable budget) exit non-zero: a broken candidate is A3.12
# decision-gate data, never silently patched.
#
# Usage: blender --background --python prop_cleanup.py -- \
#            <raw.glb> <clean.glb> [--height M] [--tri-budget N]

import argparse
import json
import sys
import traceback
from pathlib import Path

import bmesh
import bpy
import numpy as np
import xatlas
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

# Loose islands under 2% of the mesh's bbox diagonal (~4 cm on a 1.8 m
# prop) are generation floaters — far below any feature that reads at
# game camera distance; real detached-looking parts (candle arms, wax)
# are an order of magnitude larger.
FRAGMENT_DIAG_FRACTION = 0.02
# Ground-contact band: vertices within the bottom 2.5% of the target
# height (~4.5 cm at 1.8 m) define the footprint the prop stands on.
CONTACT_BAND_FRACTION = 0.025
# UV atlas packing: resolution must match the asset's texture_size (the
# registry's per-asset bake resolution prop_texture.py resolves) so the
# padding (island gutter) is real texels there; 4 px keeps islands apart
# under the normal bake's margin=8 dilation.
UV_ATLAS_RESOLUTION = 1024
UV_ATLAS_PADDING_PX = 4
# Rays cast through the bbox to measure two_crossing_ray_fraction: enough
# samples to average out the sampling noise a single ray direction carries.
RAY_SAMPLE_COUNT = 200
# Interior-face visibility test: rays cast per face over the outward
# hemisphere to decide whether any of them escapes the mesh.
INTERIOR_RAY_COUNT = 64
# Ray origin offset along the face normal, as a fraction of the mesh's
# bbox diagonal -- clears the originating face without biasing the test
# toward nearby parallel geometry.
INTERIOR_RAY_EPS_FRACTION = 1e-4


def fail(msg):
    print(f"prop_cleanup: {msg}", file=sys.stderr)
    sys.exit(1)


def vert_coords(me):
    co = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", co)
    return co.reshape(-1, 3)


def tri_count(me):
    me.calc_loop_triangles()
    return len(me.loop_triangles)


def bbox_diag(objs):
    corners = np.array([c for o in objs for c in o.bound_box])
    return float(np.linalg.norm(corners.max(axis=0) - corners.min(axis=0)))


def geometry_health(me):
    """Watertightness/topology diagnostics on the final mesh. A boundary
    edge borders exactly one face (a hole); euler_number is V-E+F, which
    is 2 for a closed genus-0 manifold and departs from it under
    non-manifold or open topology; component_count is the number of
    vertex-connectivity islands; two_crossing_ray_fraction is the share
    of random rays through the bbox whose face-crossing count is 2, the
    signature a closed surface gives any ray that enters and exits once."""
    bm = bmesh.new()
    bm.from_mesh(me)
    bm.verts.ensure_lookup_table()
    bm.edges.ensure_lookup_table()

    boundary_edge_count = sum(1 for e in bm.edges if len(e.link_faces) == 1)
    euler_number = len(bm.verts) - len(bm.edges) + len(bm.faces)

    visited = set()
    component_count = 0
    for seed in bm.verts:
        if seed.index in visited:
            continue
        component_count += 1
        stack = [seed]
        visited.add(seed.index)
        while stack:
            v = stack.pop()
            for e in v.link_edges:
                other = e.other_vert(v)
                if other.index not in visited:
                    visited.add(other.index)
                    stack.append(other)

    bvh = BVHTree.FromBMesh(bm)
    co = np.array([v.co for v in bm.verts])
    bbox_min, bbox_max = co.min(axis=0), co.max(axis=0)
    diag = float(np.linalg.norm(bbox_max - bbox_min))
    margin = 0.25 * diag
    rng = np.random.default_rng(0)
    two_crossings = 0
    for _ in range(RAY_SAMPLE_COUNT):
        origin = Vector(rng.uniform(bbox_min - margin, bbox_max + margin))
        target = Vector(rng.uniform(bbox_min, bbox_max))
        direction = (target - origin).normalized()
        crossings = 0
        o, remaining = origin, diag + 2 * margin
        for _ in range(64):
            hit, _n, _i, dist = bvh.ray_cast(o, direction, remaining)
            if hit is None:
                break
            crossings += 1
            remaining -= dist + 1e-6
            o = hit + direction * 1e-6
            if remaining <= 0:
                break
        if crossings == 2:
            two_crossings += 1

    bm.free()
    return {
        "is_watertight": boundary_edge_count == 0,
        "boundary_edge_count": boundary_edge_count,
        "euler_number": euler_number,
        "component_count": component_count,
        "two_crossing_ray_fraction": round(two_crossings / RAY_SAMPLE_COUNT, 4),
    }


def _hemisphere_dirs(count):
    """`count` unit directions over a hemisphere around local +Z,
    deterministic; direction 0 is +Z itself so a genuinely exterior face --
    which typically sees open space along its own normal -- escapes on the
    first ray instead of paying for the rest."""
    rng = np.random.default_rng(0)
    z = np.concatenate(([1.0], rng.uniform(0.0, 1.0, count - 1)))
    phi = np.concatenate(([0.0], rng.uniform(0.0, 2 * np.pi, count - 1)))
    r = np.sqrt(np.clip(1.0 - z * z, 0.0, 1.0))
    return np.stack([r * np.cos(phi), r * np.sin(phi), z], axis=1)


def strip_interior_faces(obj, me):
    """Deletes faces no ray escapes the mesh from: sample INTERIOR_RAY_COUNT
    directions over the outward hemisphere from a point just off each
    face's surface (+eps along its normal); a face where every one of them
    re-enters the mesh is enclosed by other geometry and unreachable by any
    camera. bpy.ops.mesh.select_interior_faces() runs first as a cheap
    filter -- faces whose edges are shared by more than two faces, caught
    from edge topology alone with no raycasting -- but it cannot see a
    manifold inner wall welded to the outer shell at its silhouette edges,
    which only the raycast test catches. Returns the triangle count removed."""
    tris_before = tri_count(me)

    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="DESELECT")
    bpy.ops.mesh.select_interior_faces()
    bpy.ops.object.mode_set(mode="OBJECT")
    interior = {p.index for p in me.polygons if p.select}

    bm = bmesh.new()
    bm.from_mesh(me)
    bm.faces.ensure_lookup_table()
    bvh = BVHTree.FromBMesh(bm)
    eps = INTERIOR_RAY_EPS_FRACTION * bbox_diag([obj])
    dirs = _hemisphere_dirs(INTERIOR_RAY_COUNT)

    for f in bm.faces:
        if f.index in interior:
            continue
        n = f.normal
        if n.length_squared < 1e-12:
            continue
        n = n.normalized()
        t1 = n.orthogonal().normalized()
        t2 = n.cross(t1)
        origin = f.calc_center_median() + n * eps
        if not any(bvh.ray_cast(origin, t1 * x + t2 * y + n * z)[0] is None
                  for x, y, z in dirs):
            interior.add(f.index)

    if interior:
        bmesh.ops.delete(bm, geom=[bm.faces[i] for i in interior], context="FACES")
        bm.to_mesh(me)
        me.update()
    bm.free()

    return tris_before - tri_count(me)


def export_glb(path):
    bpy.ops.export_scene.gltf(filepath=str(path), export_format="GLB",
                              export_yup=True)


def unwrap_atlas(me):
    """xatlas UV atlas on the clean mesh; returns (charts, utilization)."""
    # xatlas preserves face and corner order, so uvs[out_idx] aligns with
    # the loop sequence — but only for a pure-triangle mesh (loops are
    # then exactly the flattened polygon vertex triples).
    if len(me.loops) != 3 * len(me.polygons):
        fail("clean mesh is not pure triangles — cannot unwrap")
    idx = np.empty(len(me.loops), dtype=np.uint32)
    me.polygons.foreach_get("vertices", idx)
    atlas = xatlas.Atlas()
    atlas.add_mesh(vert_coords(me).astype(np.float32), idx.reshape(-1, 3))
    pack = xatlas.PackOptions()
    pack.resolution = UV_ATLAS_RESOLUTION
    pack.padding = UV_ATLAS_PADDING_PX
    atlas.generate(pack_options=pack, verbose=False)
    if atlas.atlas_count != 1:
        fail(f"xatlas packed {atlas.atlas_count} atlases (expected 1)")
    _, out_idx, uvs = atlas.get_mesh(0)
    layer = me.uv_layers.new(name="atlas")
    layer.uv.foreach_set("vector",
                         uvs[out_idx.ravel()].astype(np.float32).ravel())
    return int(atlas.chart_count), float(atlas.utilization)


def reflect(points, normal, offset):
    """Reflect points about the plane {p . normal = offset}."""
    return points - 2.0 * ((points @ normal) - offset)[:, None] * normal[None, :]


def plane_normal(theta):
    # Props stand upright, so the mirror plane is vertical: its normal
    # lies in the Blender-frame XY plane (Z is up here; glTF +Y-up comes
    # from export_yup), parameterized by yaw alone.
    return np.array([np.cos(theta), np.sin(theta), 0.0])


def build_kdtree(points):
    import mathutils
    tree = mathutils.kdtree.KDTree(len(points))
    for i, p in enumerate(points):
        tree.insert(mathutils.Vector(p), i)
    tree.balance()
    return tree


def find_symmetry_plane(co):
    """Best-fit vertical mirror plane of the vertex cloud: (theta, offset,
    score) minimizing the mean nearest-neighbor distance from the reflected
    samples back to the mesh. Deterministic (fixed sampling seed)."""
    rng = np.random.default_rng(0)
    sample = co[rng.choice(len(co), min(len(co), 20000), replace=False)]
    tree = build_kdtree(sample)
    probes = sample[:5000]

    def score(theta, offset):
        refl = reflect(probes, plane_normal(theta), offset)
        return sum(tree.find(p)[2] for p in refl) / len(refl)

    centroid = sample.mean(axis=0)

    def centroid_offset(theta):
        return float(centroid @ plane_normal(theta))

    coarse = [(score(t, centroid_offset(t)), t)
              for t in np.deg2rad(np.arange(0.0, 180.0, 2.0))]
    _, best_t = min(coarse)
    fine = [(score(t, centroid_offset(t)), t)
            for t in best_t + np.deg2rad(np.arange(-2.0, 2.0, 0.25))]
    _, best_t = min(fine)
    offsets = [(score(best_t, centroid_offset(best_t) + d),
                centroid_offset(best_t) + d)
               for d in np.arange(-0.05, 0.055, 0.005)]
    best_score, best_d = min(offsets)
    return float(best_t), float(best_d), float(best_score)


def symmetrize(obj, me, keep):
    """Mirror the `keep` half ('+x'/'-x' in the plane-aligned frame) across
    the mesh's best-fit vertical mirror plane, preserving orientation."""
    co = vert_coords(me)
    theta, offset, score_before = find_symmetry_plane(co)
    plane_point = offset * plane_normal(theta)

    align = Matrix.Rotation(-theta, 4, "Z") @ Matrix.Translation(-plane_point)
    me.transform(align)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    # Blender's direction enum names the source half: 'POSITIVE_X' = "+X to -X".
    bpy.ops.mesh.symmetrize(direction="POSITIVE_X" if keep == "+x" else "NEGATIVE_X")
    bpy.ops.object.mode_set(mode="OBJECT")
    me.transform(align.inverted())

    co = vert_coords(me)
    tree = build_kdtree(co)
    refl = reflect(co, plane_normal(theta), offset)
    score_after = sum(tree.find(p)[2] for p in refl) / len(refl)
    return {
        "theta_deg": round(np.rad2deg(theta), 2),
        "offset_m": round(offset, 4),
        "keep": keep,
        "score_before_m": round(score_before, 5),
        "score_after_m": round(float(score_after), 5),
    }


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    parser = argparse.ArgumentParser(prog="prop_cleanup.py")
    parser.add_argument("raw_glb")
    parser.add_argument("clean_glb")
    parser.add_argument("--height", type=float, default=1.8)
    parser.add_argument("--tri-budget", type=int, default=15000)
    parser.add_argument("--symmetrize", action="store_true")
    parser.add_argument("--symmetrize-keep", choices=["+x", "-x"], default="+x",
                        help="Half to mirror, in the plane-aligned frame "
                             "(pass as --symmetrize-keep=-x)")
    args = parser.parse_args(argv)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=args.raw_glb)

    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    if not meshes:
        fail(f"no mesh in {args.raw_glb}")

    # ---- one object, world transforms baked into the vertex data ----
    for o in bpy.context.scene.objects:
        o.select_set(o in meshes)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    world = obj.matrix_world.copy()
    obj.parent = None
    obj.matrix_world = world
    for o in list(bpy.data.objects):
        if o is not obj:
            bpy.data.objects.remove(o, do_unlink=True)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)

    raw_tris = tri_count(obj.data)
    if raw_tris == 0:
        fail("mesh has no faces")

    # ---- loose fragments out, before any measurement trusts the bbox ----
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.separate(type="LOOSE")
    bpy.ops.object.mode_set(mode="OBJECT")

    parts = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    threshold = FRAGMENT_DIAG_FRACTION * bbox_diag(parts)
    keep = [o for o in parts if bbox_diag([o]) >= threshold]
    drop = [o for o in parts if o not in keep]
    fragment_tris = sum(tri_count(o.data) for o in drop)
    for o in drop:
        bpy.data.objects.remove(o, do_unlink=True)
    obj = max(keep, key=lambda o: bbox_diag([o]))
    for o in keep:
        o.select_set(True)
    bpy.context.view_layer.objects.active = obj
    if len(keep) > 1:
        bpy.ops.object.join()
    me = obj.data

    areas = np.empty(len(me.polygons))
    me.polygons.foreach_get("area", areas)
    if areas.sum() < 1e-8:
        fail("zero-area mesh")

    # ---- interior faces out, before any budget is spent describing them ----
    interior_tris_removed = strip_interior_faces(obj, me)
    if len(me.polygons) == 0:
        fail("interior-face strip removed the entire mesh")

    # ---- scale to target height (Blender Z = exported glTF +Y) ----
    co = vert_coords(me)
    raw_height = float(co[:, 2].max() - co[:, 2].min())
    if raw_height < 1e-6:
        fail("degenerate mesh: zero height")
    scale = args.height / raw_height
    me.transform(Matrix.Diagonal((scale, scale, scale, 1.0)))

    # ---- optional mirror across the best-fit vertical plane ----
    # Runs before re-origin so the footprint centroid is computed on the
    # final (symmetric) geometry.
    symmetrize_stats = None
    if args.symmetrize:
        symmetrize_stats = symmetrize(obj, me, args.symmetrize_keep)

    # ---- origin at footprint centroid, bottom on the ground ----
    co = vert_coords(me)
    min_z = float(co[:, 2].min())
    band = co[co[:, 2] <= min_z + CONTACT_BAND_FRACTION * args.height]
    cx = float(band[:, 0].min() + band[:, 0].max()) / 2.0
    cy = float(band[:, 1].min() + band[:, 1].max()) / 2.0
    me.transform(Matrix.Translation((-cx, -cy, -min_z)))

    clean = Path(args.clean_glb)
    hires = clean.with_name(clean.stem + "_hires.glb")
    hires_tris = tri_count(me)
    export_glb(hires)

    # ---- decimate to budget (collapse keeps the silhouette) ----
    clean_tris = hires_tris
    if clean_tris > args.tri_budget:
        for _ in range(3):
            mod = obj.modifiers.new("decimate", "DECIMATE")
            mod.decimate_type = "COLLAPSE"
            mod.use_collapse_triangulate = True
            mod.ratio = args.tri_budget / clean_tris
            bpy.ops.object.modifier_apply(modifier=mod.name)
            clean_tris = tri_count(me)
            if clean_tris <= args.tri_budget:
                break
        if clean_tris > args.tri_budget:
            fail(f"decimation stalled at {clean_tris} tris "
                 f"(budget {args.tri_budget})")
        # collapse drifts the extremes by millimetres: re-seat the bottom
        # exactly on the ground (rigid shift, hires alignment preserved)
        co = vert_coords(me)
        me.transform(Matrix.Translation((0.0, 0.0, -float(co[:, 2].min()))))

    uv_charts, uv_utilization = unwrap_atlas(me)
    export_glb(clean)

    co = vert_coords(me)
    stats = {
        "raw_tris": raw_tris,
        "fragments_removed": len(drop),
        "fragment_tris_removed": fragment_tris,
        "interior_tris_removed": interior_tris_removed,
        "hires_tris": hires_tris,
        "clean_tris": clean_tris,
        "height_target": args.height,
        "clean_height": float(co[:, 2].max() - co[:, 2].min()),
        "clean_min_y": float(co[:, 2].min()),
        "uv_charts": uv_charts,
        "uv_utilization": round(uv_utilization, 4),
        "hires_glb": str(hires),
        "clean_glb": str(clean),
    }
    stats.update(geometry_health(me))
    if symmetrize_stats is not None:
        stats["symmetrize"] = symmetrize_stats
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
