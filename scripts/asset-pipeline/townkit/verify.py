"""Re-import a just-exported glb into a fresh scene and check it: material
slot names against the six-material vocabulary, the vordar_detail extra on
limestone materials, UV layers, loose geometry, boundary-edge counts, and
that no curved-ring assembly (arch voussoirs, vault, rubble lip) has a
significant inward-pointing face or an unmitered joint gap between its
wedges -- the two ways a barrel_shell ring renders as a dark, jagged band."""

import re

import bmesh
import bpy
from mathutils import Vector

ALLOWED_MATERIALS = {
    "encalado", "limestone_dressed", "terracotta_tile",
    "oak_dark", "plaster_smoked", "iron_wrought",
}

_WEDGE_RE = re.compile(r"^(.*)_wedge(\d+)$")
_WALL_NAME_RE = re.compile(r"(wall|_shell$)", re.IGNORECASE)


def _face_radii(f, mw, axis, u_center, z_center):
    """Per-vertex radius from the arc centre, for classifying whether a
    face is a true radial (constant-radius) face -- a face with vertices at
    genuinely different radii is an end/joint face, not extrados/intrados,
    and checking it against "outward" is meaningless (its normal is
    legitimately tangential)."""
    radii = []
    for v in f.verts:
        p = mw @ v.co
        if axis == "x":
            rad_vec = Vector((p.x - u_center, 0.0, p.z - z_center))
        else:
            rad_vec = Vector((0.0, p.y - u_center, p.z - z_center))
        radii.append(rad_vec.length)
    return radii


def _radial_normal_faults(obj, min_dot=0.3, min_area=1e-4, flat_tol=1e-3):
    """A barrel_shell wedge carries its arc centre/radii as extras. A face
    whose vertices all sit at (near) the same radius is a true radial face:
    if that radius is r_out (extrados) it must point away from the arc
    centre; if r_in (intrados -- e.g. a vault's visible ceiling, which
    legitimately faces the centre) it must point toward it. Either sign
    flipping indicates a genuinely inverted face rather than the ring's own
    curvature. End/joint faces (vertices spanning r_in..r_out) aren't radial
    and are skipped. Returns (checked, faults)."""
    axis = obj.get("vordar_arc_axis")
    if axis is None:
        return 0, []
    u_center = obj["vordar_arc_u"]
    z_center = obj["vordar_arc_z"]
    r_out = obj["vordar_r_out"]
    r_in = obj["vordar_r_in"]
    thickness = r_out - r_in
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bm.faces.ensure_lookup_table()
    mw = obj.matrix_world
    nmat = mw.to_3x3()
    checked = 0
    faults = []
    for f in bm.faces:
        if f.calc_area() < min_area:
            continue
        radii = _face_radii(f, mw, axis, u_center, z_center)
        if max(radii) - min(radii) > thickness * flat_tol:
            continue  # end/joint face, not a constant-radius radial face
        mean_r = sum(radii) / len(radii)
        normal = nmat @ f.normal
        if normal.length < 1e-9:
            continue
        normal.normalize()
        center = mw @ f.calc_center_median()
        if axis == "x":
            radial = Vector((center.x - u_center, 0.0, center.z - z_center))
        else:
            radial = Vector((0.0, center.y - u_center, center.z - z_center))
        if radial.length < 1e-6:
            continue
        radial.normalize()
        d = normal.dot(radial)
        if abs(mean_r - r_out) <= abs(mean_r - r_in):
            checked += 1
            if d < min_dot:
                faults.append({"object": obj.name, "side": "extrados", "dot": round(d, 3)})
        else:
            checked += 1
            if d > -min_dot:
                faults.append({"object": obj.name, "side": "intrados", "dot": round(d, 3)})
    bm.free()
    return checked, faults


def _wedge_joint_gaps(mesh_objs):
    """Adjacent wedges of a ring should share exact coincident corners (the
    tangential faces are true radial planes) -- a box-approximated wedge
    (flat tangential faces rotated independently per block) leaves a real
    gap here, exposing a poorly-lit internal face as the black, jagged
    band defect. Tolerance scales with the ring's own radial jitter, so an
    intentionally ragged (jittered) rim isn't flagged."""
    groups = {}
    for obj in mesh_objs:
        m = _WEDGE_RE.match(obj.name)
        if m and obj.get("vordar_arc_axis") is not None:
            groups.setdefault(m.group(1), []).append((int(m.group(2)), obj))

    reports = []
    for prefix, wedges in groups.items():
        wedges.sort(key=lambda p: p[0])
        r_outs = [o["vordar_r_out"] for _, o in wedges]
        jitter_spread = max(r_outs) - min(r_outs)
        tolerance = max(0.005, jitter_spread * 1.5)
        for (i, obj_i), (j, obj_j) in zip(wedges, wedges[1:]):
            verts_i = [obj_i.matrix_world @ v.co for v in obj_i.data.vertices]
            verts_j = [obj_j.matrix_world @ v.co for v in obj_j.data.vertices]
            gap = min((a - b).length for a in verts_i for b in verts_j)
            reports.append({"prefix": prefix, "pair": [i, j], "gap": round(gap, 5),
                             "tolerance": round(tolerance, 5), "ok": gap <= tolerance})
    return reports


def _edge_key(e, obj):
    mw = obj.matrix_world
    a = tuple(round(c, 4) for c in (mw @ e.verts[0].co))
    b = tuple(round(c, 4) for c in (mw @ e.verts[1].co))
    return (a, b) if a <= b else (b, a)


def _global_edge_face_counts(mesh_objs):
    """glTF export splits a mesh into one primitive per material slot, and
    primitives don't share vertex buffers -- a face on a material seam
    (e.g. a wall's own top edge meeting the roof deck, encalado vs
    terracotta_tile) gets its edge duplicated into two positionally-
    coincident but topologically disconnected copies, one per primitive.
    Reimported, each copy alone looks like a 1-face boundary edge even
    though the seam isn't a real hole. Counting every edge globally by
    world-space endpoint position (across every object/material) tells the
    two apart: a real hole's edge position is used once in the whole
    scene; a material-seam split is used twice."""
    counts = {}
    for obj in mesh_objs:
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        for e in bm.edges:
            k = _edge_key(e, obj)
            counts[k] = counts.get(k, 0) + len(e.link_faces)
        bm.free()
    return counts


def _open_wall_faces(mesh_objs, min_perimeter=1.0):
    """A wall shell (a solid box, or several boolean-unioned into one --
    casa_corner's valley join) should be fully watertight: door/window
    openings are gaps *between* separate solid pieces (sill/head boxes
    flanking empty space), never a hole cut into one, so a healthy wall
    object has ~zero genuinely open edges. A boolean that silently drops a
    whole wall face (an exactly-coincident face pair confusing the solver,
    G2 D6) leaves a real hole with a boundary loop sized like the missing
    wall. Candidate open edges are clustered by shared endpoint position
    (glTF reimport splits vertices per loop/UV-seam, so this can't rely on
    shared vertex identity, same reason _wedge_joint_gaps is position- not
    vertex-based) and any loop whose perimeter clears `min_perimeter` is
    flagged -- scoped to wall-shell objects only, since roof-tile geometry
    has its own legitimate open (unglazed) rim by design."""
    edge_counts = _global_edge_face_counts(mesh_objs)
    reports = []
    for obj in mesh_objs:
        if not _WALL_NAME_RE.search(obj.name):
            continue
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bm.edges.ensure_lookup_table()
        boundary = [e for e in bm.edges
                    if len(e.link_faces) == 1 and edge_counts.get(_edge_key(e, obj), 0) <= 1]
        if not boundary:
            bm.free()
            continue

        def key(v):
            return tuple(round(c, 4) for c in (obj.matrix_world @ v.co))

        parent = {}

        def find(k):
            parent.setdefault(k, k)
            while parent[k] != k:
                parent[k] = parent[parent[k]]
                k = parent[k]
            return k

        def union(a, b):
            ra, rb = find(a), find(b)
            if ra != rb:
                parent[ra] = rb

        for e in boundary:
            union(key(e.verts[0]), key(e.verts[1]))

        clusters = {}
        for e in boundary:
            root = find(key(e.verts[0]))
            clusters.setdefault(root, []).append(e)
        for edges in clusters.values():
            perim = sum((e.verts[0].co - e.verts[1].co).length for e in edges)
            if perim >= min_perimeter:
                reports.append({"object": obj.name, "loop_edges": len(edges),
                                 "perimeter": round(perim, 3)})
        bm.free()
    return reports


def verify_glb(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(path))

    mesh_objs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    report = {
        "mesh_count": len(mesh_objs),
        "material_names": set(),
        "bad_material_names": [],
        "detail_extras": {},
        "missing_uv": [],
        "loose_verts": 0,
        "loose_edges": 0,
        "boundary_edges": 0,
        "total_tris": 0,
        "normals_checked": 0,
        "normals_faults": [],
        "joint_gaps": [],
        "open_wall_faces": [],
    }

    for mat in bpy.data.materials:
        report["material_names"].add(mat.name)
        if mat.name not in ALLOWED_MATERIALS:
            report["bad_material_names"].append(mat.name)
        report["detail_extras"][mat.name] = mat.get("vordar_detail", None)

    for obj in mesh_objs:
        mesh = obj.data
        if not mesh.uv_layers:
            report["missing_uv"].append(obj.name)
        bm = bmesh.new()
        bm.from_mesh(mesh)
        bm.faces.ensure_lookup_table()
        mesh.calc_loop_triangles()
        report["total_tris"] += len(mesh.loop_triangles)
        for v in bm.verts:
            if len(v.link_faces) == 0:
                report["loose_verts"] += 1
        for e in bm.edges:
            nfaces = len(e.link_faces)
            if nfaces == 0:
                report["loose_edges"] += 1
            elif nfaces == 1:
                report["boundary_edges"] += 1
        bm.free()

        checked, faults = _radial_normal_faults(obj)
        report["normals_checked"] += checked
        report["normals_faults"].extend(faults)

    report["joint_gaps"] = _wedge_joint_gaps(mesh_objs)
    bad_gaps = [g for g in report["joint_gaps"] if not g["ok"]]
    report["open_wall_faces"] = _open_wall_faces(mesh_objs)

    report["ok"] = (not report["bad_material_names"] and not report["missing_uv"]
                     and report["loose_verts"] == 0 and report["loose_edges"] == 0
                     and not report["normals_faults"] and not bad_gaps
                     and not report["open_wall_faces"])
    return report
