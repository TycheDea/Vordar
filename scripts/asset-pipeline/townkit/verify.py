"""Re-import a just-exported glTF into a fresh scene and check it: material
slot names against the six-material vocabulary, the vordar_detail extra on
limestone materials, UV layers, loose geometry, boundary-edge counts, and
that no curved-ring assembly (arch voussoirs, vault, rubble lip) has a
significant inward-pointing face or an unmitered joint gap between its
wedges -- the two ways a barrel_shell ring renders as a dark, jagged band."""

import math
import re
from pathlib import Path

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
    has its own legitimate open (unglazed) rim by design.

    Exact endpoint pairing alone is not enough: the EXACT boolean solver
    tessellates each of an edge's two adjacent faces independently, so one
    side may carry the edge as a single span and the other as several
    sub-segments (a T-junction seam). Those copies never share a rounded
    key, yet the surface is closed there -- so a candidate only stays open
    if no other collinear edge of the same object covers its midpoint."""
    edge_counts = _global_edge_face_counts(mesh_objs)
    reports = []
    for obj in mesh_objs:
        if not _WALL_NAME_RE.search(obj.name):
            continue
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bm.edges.ensure_lookup_table()
        candidates = [e for e in bm.edges
                      if len(e.link_faces) == 1 and edge_counts.get(_edge_key(e, obj), 0) <= 1]
        segments = None
        if candidates:
            segments = [(obj.matrix_world @ e.verts[0].co, obj.matrix_world @ e.verts[1].co,
                          _edge_key(e, obj)) for e in bm.edges]

        def covered(e, tol=1e-3):
            a = obj.matrix_world @ e.verts[0].co
            b = obj.matrix_world @ e.verts[1].co
            mid = (a + b) / 2.0
            own = _edge_key(e, obj)
            for p, q, key in segments:
                if key == own:
                    continue
                pq = q - p
                ll = pq.length_squared
                if ll < 1e-12:
                    continue
                t = (mid - p).dot(pq) / ll
                if -1e-6 <= t <= 1.0 + 1e-6 and ((p + t * pq) - mid).length < tol:
                    return True
            return False

        boundary = [e for e in candidates if not covered(e)]
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


def _roof_slope_faults(mesh_objs, min_cluster_area=2.0, nz_lo=0.3, nz_hi=0.985,
                        relief_lo=0.02, relief_hi=0.5, relief_min_area=1.0,
                        ridge_min_rise=0.075):
    """Casa gable-roof invariant: every large up-sloped plane on a casa is a
    roof deck, so (1) its faces must all carry terracotta_tile -- a
    material-dropping merge leaves the slope reading as wall plaster -- and
    (2) each slope must carry tile relief: terracotta faces parallel to the
    deck at offsets above it (the barrel tiles' crests). A slope whose tiles
    ended up under the deck has nothing above it and fails (2) even when its
    deck material is right.

    C5 (ridge_bare) folds in here rather than as a separate pass: this
    function already clusters faces by plane and already identifies each
    family's deck (the largest-area cluster), so checking the ridge's
    height above that deck costs no new geometry pass. The deck's own UV
    span is reported but not graded -- the deck is the one roof surface the
    camera never sees, buried under a full course of barrel tiles and the
    ridge caps, so a UV fault measured on it says nothing about the shipped
    pixels (that is `_uv_patch_repeat_faults`' job). Returns
    (slopes, faults)."""
    # C5's own signature: F6 names its cover-tile course "..._ridge_<i>", the
    # only terracotta_tile geometry not built as a slope-aligned deck or
    # tile -- an ordinary barrel tile's own discretized (7-segment) faces
    # can, by construction-angle coincidence, spike above the flat deck by
    # more than a bare-arris threshold at some segment, so a plain "highest
    # terracotta face vs. deck height" scan can't tell a ridge cap from that
    # noise; naming is the reliable signal, height confirms it actually sits
    # above the deck rather than merely existing.
    ridge_max_z = None
    for obj in mesh_objs:
        if "_ridge_" not in obj.name:
            continue
        if not (obj.data.materials and obj.data.materials[0]
                and obj.data.materials[0].name == "terracotta_tile"):
            continue
        mw = obj.matrix_world
        for v in obj.data.vertices:
            z = (mw @ v.co).z
            ridge_max_z = z if ridge_max_z is None else max(ridge_max_z, z)

    faces = []  # (normal, offset, area, material_name)
    uv_extent = {}  # cluster key -> [u_min, u_max, v_min, v_max]
    z_extent = {}  # cluster key -> highest vertex world z among its faces
    for obj in mesh_objs:
        mw = obj.matrix_world
        nmat = mw.to_3x3()
        mats = obj.data.materials
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        uv_layer = bm.loops.layers.uv.active
        for f in bm.faces:
            slot = mats[f.material_index] if mats and f.material_index < len(mats) else None
            mat = slot.name if slot else None

            n = nmat @ f.normal
            if n.length < 1e-9:
                continue
            n = n.normalized()
            if not (nz_lo < n.z < nz_hi):
                continue
            area = f.calc_area()
            if area < 1e-4:
                continue
            center = mw @ f.calc_center_median()
            off = n.dot(center)
            key = (round(n.x, 2), round(n.y, 2), round(n.z, 2), round(off, 2))
            face_max_z = max((mw @ lp.vert.co).z for lp in f.loops)
            z_extent[key] = face_max_z if key not in z_extent else max(z_extent[key], face_max_z)
            if uv_layer is not None:
                ext = uv_extent.setdefault(key, [1e18, -1e18, 1e18, -1e18])
                for loop in f.loops:
                    u, v = loop[uv_layer].uv
                    ext[0], ext[1] = min(ext[0], u), max(ext[1], u)
                    ext[2], ext[3] = min(ext[2], v), max(ext[3], v)
            faces.append((n, off, area, mat, key))
        bm.free()

    clusters = {}
    for n, off, area, mat, key in faces:
        c = clusters.setdefault(key, {"normal": Vector((0, 0, 0)), "offset": off,
                                       "area": 0.0, "materials": set()})
        c["normal"] += n * area
        c["area"] += area
        c["materials"].add(mat)
    large = {k: c for k, c in clusters.items() if c["area"] >= min_cluster_area}
    for c in large.values():
        c["normal"].normalize()

    families = []  # each: list of (key, cluster) with dot-similar normals
    for k, c in large.items():
        for fam in families:
            if clusters[fam[0][0]]["normal"].dot(c["normal"]) > 0.95:
                fam.append((k, c))
                break
        else:
            families.append([(k, c)])

    slopes = []
    faults = []
    for fam in families:
        deck_key, deck = max(fam, key=lambda kc: kc[1]["area"])
        for k, c in fam:
            if c["materials"] != {"terracotta_tile"}:
                faults.append({"kind": "slope_material",
                                "normal": [round(v, 3) for v in c["normal"]],
                                "offset": round(c["offset"], 3),
                                "area": round(c["area"], 2),
                                "materials": sorted(str(m) for m in c["materials"])})
        relief_area = sum(
            area for n, off, area, mat, key in faces
            if mat == "terracotta_tile" and n.dot(deck["normal"]) > 0.95
            and relief_lo < off - deck["offset"] < relief_hi)
        slope = {"normal": [round(v, 3) for v in deck["normal"]],
                 "offset": round(deck["offset"], 3),
                 "deck_area": round(deck["area"], 2),
                 "relief_area": round(relief_area, 2)}
        u_lo, u_hi, v_lo, v_hi = uv_extent.get(deck_key, (0.0, 0.0, 0.0, 0.0))
        slope["u_span"] = [round(u_lo, 4), round(u_hi, 4)]
        slope["v_span"] = [round(v_lo, 4), round(v_hi, 4)]
        slopes.append(slope)
        if relief_area < relief_min_area:
            faults.append({"kind": "slope_flat", **slope})

        # C5: the ridge cap course, if present, must clear this slope's own
        # deck top by ridge_min_rise -- a bare arris has no "_ridge_"
        # geometry above the deck at all.
        deck_max_z = z_extent.get(deck_key)
        if deck_max_z is not None:
            if ridge_max_z is None or ridge_max_z - deck_max_z < ridge_min_rise:
                faults.append({"kind": "ridge_bare", **slope,
                                "ridge_max_z": round(ridge_max_z, 3) if ridge_max_z is not None else None,
                                "deck_max_z": round(deck_max_z, 3)})
    return slopes, faults


def _uv_patch_repeat_faults(mesh_objs, uv_tol=1e-4, pos_tol=0.05):
    """No two objects standing in different places may display literally
    the same texels. Blender's cube_project origins each projection on the
    object's own median, so every congruent object landed on the identical
    UV set -- a slope's barrel-tile course, a ridge's cover tiles and a
    vault's voussoirs each stamping one patch N times, with every dark
    region inside it recurring as a band down the fall line.

    The comparison is the whole per-object UV multiset, not its bounding
    rectangle: a rectangle is dominated by whichever faces span the most
    texture, and on a barrel tile those are the buried flanks, whose UVs
    legitimately do not vary along their own normal. Two tiles can share a
    rectangle while their visible crests sample different texels, so the
    rectangle over-fires. This reads exactly the objects the renderer
    draws, and it is blind to material and to type."""
    groups = {}
    for obj in mesh_objs:
        mesh = obj.data
        if not mesh.uv_layers:
            continue
        uvs = sorted((round(lp.uv.x / uv_tol), round(lp.uv.y / uv_tol))
                     for lp in mesh.uv_layers.active.data)
        if not uvs:
            continue
        mat = mesh.materials[0].name if mesh.materials and mesh.materials[0] else None
        mw = obj.matrix_world
        pts = [mw @ v.co for v in mesh.vertices]
        center = tuple(sum(p[i] for p in pts) / len(pts) for i in range(3))
        groups.setdefault((mat, tuple(uvs)), []).append((obj.name, center))

    faults = []
    for (mat, uvs), members in groups.items():
        if len(members) < 2:
            continue
        base = members[0][1]
        if all(max(abs(m[1][i] - base[i]) for i in range(3)) <= pos_tol for m in members):
            continue  # genuinely coincident objects, not a repeated stamp
        names = sorted(m[0] for m in members)
        faults.append({"kind": "uv_patch_repeat", "material": mat,
                        "count": len(members), "objects": names[:4]})
    return faults


def _material_verts(mesh_objs, material_name):
    """World-space vertices of every object whose (single) material slot is
    `material_name` -- every townkit procedural piece carries exactly one
    material, so this is a cheap per-object filter rather than a per-face
    one."""
    for obj in mesh_objs:
        if obj.data.materials and obj.data.materials[0] and obj.data.materials[0].name == material_name:
            mw = obj.matrix_world
            for v in obj.data.vertices:
                yield mw @ v.co


_QUOIN_BLOCK_RE = re.compile(r"^(.*_quoin_.*)_q(\d+)$")
_LIP_WEDGE_RE = re.compile(r"^chapel_lip_wedge\d+$")
_PORTAL_WEDGE_RE = re.compile(r"^chapel_portal_wedge\d+$")


def _crown_datum_share(mesh_objs, x_lo, x_hi, quantum=0.02, ground=0.5):
    """Longest surviving horizontal masonry line in the collapsed bay, as a
    fraction of the bay's length.

    A per-bin height *range* cannot see this defect and did not: a level
    coped wall carrying a nibbled frieze of blocks supplies the range from
    the frieze while the datum under it -- the blocks' shared bottom plane
    and the wall top they rest on -- supplies the crenellated read. So this
    measures the datum itself: every horizontal masonry edge (each piece's
    own top and bottom) contributes its x-extent to a bucket at that
    height, and the fullest bucket is the longest level line a viewer can
    trace. Every piece also shares the ground plane legitimately, so edges
    below `ground` are dropped; nothing else in a wall is horizontal
    between the footing and the crown, which is why the measurement is flat
    for any `ground` from 0.2 to 4.0 m rather than set by it."""
    buckets = {-1: {}, 1: {}}
    for obj in mesh_objs:
        if not (obj.data.materials and obj.data.materials[0]
                and obj.data.materials[0].name == "limestone_dressed"):
            continue
        mw = obj.matrix_world
        pts = [mw @ v.co for v in obj.data.vertices]
        # The whole piece must lie in the side wall's own 3.5..4.1 band:
        # the nave's plaster liner and its limestone dado reach y = 3.44 and
        # carry a legitimate full-length horizontal top at z = 2.0, which is
        # an interior dado, not a crown datum.
        if not all(3.45 <= abs(p.y) <= 4.15 for p in pts):
            continue
        lo_x = max(x_lo, min(p.x for p in pts))
        hi_x = min(x_hi, max(p.x for p in pts))
        if hi_x - lo_x <= 1e-6:
            continue
        side = 1 if sum(p.y for p in pts) >= 0.0 else -1
        for z in (min(p.z for p in pts), max(p.z for p in pts)):
            if z < ground:
                continue
            k = round(z / quantum)
            buckets[side][k] = buckets[side].get(k, 0.0) + (hi_x - lo_x)
    per_side = [max(b.values()) / (x_hi - x_lo) for b in buckets.values() if b]
    return max(per_side) if per_side else 0.0


def _chapel_collapse_faults(mesh_objs, bin_size=0.25):
    """F5's collapse read, checked geometrically rather than trusted from
    the build script: the collapsed bay's wall-top height must actually
    vary (the old level-coped crown read as excluded even where the vault
    tore out), the intact bay's crown must stay level (the contrast is the
    read -- a ragged intact crown is as wrong as a level broken one), and
    the fracture lip's wedges must reach staggered x extents (a clean cut
    reads as artificial)."""
    bins = {}
    for p in _material_verts(mesh_objs, "limestone_dressed"):
        if 3.4 <= abs(p.y) <= 4.2:
            b = math.floor(p.x / bin_size)
            bins[b] = max(bins.get(b, -1e18), p.z)

    def span_range(lo, hi):
        zs = [z for b, z in bins.items() if lo <= b * bin_size < hi]
        return (max(zs) - min(zs)) if zs else 0.0

    faults = []
    collapsed = span_range(1.0, 7.6)
    if collapsed < 0.50:
        faults.append({"kind": "collapse_crown_flat", "range": round(collapsed, 3)})
    share = _crown_datum_share(mesh_objs, 1.0, 7.6)
    if share >= 0.30:
        faults.append({"kind": "collapse_crown_datum", "share": round(share, 3)})
    intact = span_range(-8.3, 0.0)
    if intact > 0.05:
        faults.append({"kind": "collapse_crown_ragged_intact", "range": round(intact, 3)})

    lip_x_max = [max((obj.matrix_world @ v.co).x for v in obj.data.vertices)
                 for obj in mesh_objs if _LIP_WEDGE_RE.match(obj.name)]
    if lip_x_max:
        spread = max(lip_x_max) - min(lip_x_max)
        if spread < 1.00:
            faults.append({"kind": "lip_spread_small", "spread": round(spread, 3)})
    return faults


def _chapel_signature_faults(mesh_objs):
    """Presence checks for the chapel's whole point (F1-F3): a silent loss
    of the espadaña, the bell, the cross, or the portal ring would still
    pass every geometric hygiene check above, so these four bounding-box
    reads exist purely to catch that."""
    faults = []

    espadana = [p for p in _material_verts(mesh_objs, "limestone_dressed")
                if p.z >= 11.5 and 7.9 <= p.x <= 8.7 and abs(p.y) <= 2.0]
    if not espadana:
        faults.append({"kind": "espadana_missing"})

    iron_pts = list(_material_verts(mesh_objs, "iron_wrought"))
    if not iron_pts:
        faults.append({"kind": "iron_wrought_unbound"})
    else:
        if max(p.z for p in iron_pts) < 12.8:
            faults.append({"kind": "cross_missing"})
        if not any(9.2 <= p.z <= 10.1 for p in iron_pts):
            faults.append({"kind": "bell_missing"})

    portal_wedges = [o for o in mesh_objs
                      if _PORTAL_WEDGE_RE.match(o.name) and o.get("vordar_arc_axis") is not None]
    if len(portal_wedges) != 11:
        faults.append({"kind": "portal_wedge_count", "count": len(portal_wedges)})
    return faults


def _quoin_flush_faults(mesh_objs, tol=0.08):
    """Rider 3.1: a `_quoin_` object's four horizontal bbox faces must sit
    within `tol` of, or inside, some `encalado` wall plane sharing that
    outward normal -- checked per-plane (not against a single per-model
    extreme) so casa_corner's re-entrant corner, with planes from both the
    main block and the wing, is covered too. `tol` bounds how far a course
    may stand out; it is buildings.QUOIN_PROUD (0.05) plus margin, and its
    job is to catch a chain whose projection varies course to course (the
    0.146 m jag the centreline-anchored build shipped), not to re-author
    the authored reveal.

    Flushness alone was never the defect the render showed. Successive
    courses also have to bond: any clear air between one block's top and
    the next block's bottom is a slot the camera looks into, and it is what
    made each block read as glued on rather than coursed in
    (`quoin_course_void`)."""
    planes = {"+x": [], "-x": [], "+y": [], "-y": []}
    for obj in mesh_objs:
        if not (obj.data.materials and obj.data.materials[0] and obj.data.materials[0].name == "encalado"):
            continue
        mw = obj.matrix_world
        nmat = mw.to_3x3()
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        for f in bm.faces:
            n = nmat @ f.normal
            if n.length < 1e-9:
                continue
            n = n.normalized()
            center = mw @ f.calc_center_median()
            if abs(n.x) > 0.95 and abs(n.y) < 0.05:
                planes["+x" if n.x > 0 else "-x"].append(center.x)
            elif abs(n.y) > 0.95 and abs(n.x) < 0.05:
                planes["+y" if n.y > 0 else "-y"].append(center.y)
        bm.free()

    faults = []
    for obj in mesh_objs:
        if "_quoin_" not in obj.name:
            continue
        mw = obj.matrix_world
        xs = [(mw @ v.co).x for v in obj.data.vertices]
        ys = [(mw @ v.co).y for v in obj.data.vertices]
        for key, val in (("+x", max(xs)), ("-x", min(xs)), ("+y", max(ys)), ("-y", min(ys))):
            candidates = planes.get(key, [])
            if not candidates:
                continue
            ok = any(val <= p + tol for p in candidates) if key.startswith("+") \
                else any(val >= p - tol for p in candidates)
            if not ok:
                faults.append({"kind": "quoin_proud", "object": obj.name,
                                "face": key, "value": round(val, 4)})

    chains = {}
    for obj in mesh_objs:
        m = _QUOIN_BLOCK_RE.match(obj.name)
        if not m:
            continue
        mw = obj.matrix_world
        zs = [(mw @ v.co).z for v in obj.data.vertices]
        chains.setdefault(m.group(1), []).append((int(m.group(2)), min(zs), max(zs)))
    for prefix, blocks in chains.items():
        blocks.sort()
        for (_, _, z_top), (j, z_bot, _) in zip(blocks, blocks[1:]):
            if z_bot - z_top > 0.005:
                faults.append({"kind": "quoin_course_void", "chain": prefix,
                                "above": j, "void": round(z_bot - z_top, 4)})
    return faults


def verify_export(path):
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
        "roof_slopes": [],
        "roof_faults": [],
        "chapel_faults": [],
        "quoin_faults": [],
        "uv_repeat_faults": [],
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
    report["uv_repeat_faults"] = _uv_patch_repeat_faults(mesh_objs)
    stem = Path(str(path)).stem
    if stem.startswith("casa"):
        report["roof_slopes"], report["roof_faults"] = _roof_slope_faults(mesh_objs)
        report["quoin_faults"] = _quoin_flush_faults(mesh_objs)
    elif stem == "chapel":
        report["chapel_faults"] = _chapel_collapse_faults(mesh_objs) + _chapel_signature_faults(mesh_objs)

    report["ok"] = (not report["bad_material_names"] and not report["missing_uv"]
                     and report["loose_verts"] == 0 and report["loose_edges"] == 0
                     and not report["normals_faults"] and not bad_gaps
                     and not report["open_wall_faces"] and not report["roof_faults"]
                     and not report["chapel_faults"] and not report["quoin_faults"]
                     and not report["uv_repeat_faults"])
    return report
