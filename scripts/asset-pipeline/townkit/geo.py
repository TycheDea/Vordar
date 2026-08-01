"""Procedural geometry primitives for the Rocalba town kit.

Blender stays Z-up (native) throughout generation; export_scene.gltf's
export_yup=True (same flag prop_texture.py's _write_glb uses) converts to
glTF's Y-up on export, so no axis juggling is needed here. Each building is
built centered near local (0, 0) on the XY ground plane with Z as height; a
single arbitrary "front" face (+Y, or +X where noted per-type) carries the
door/gate opening -- exact world-facing orientation is a Phase 2 placement
concern, not this pilot's.
"""

import math
import random

import bmesh
import bpy
from mathutils import Matrix, Vector

from . import materials as matlib


def _link(obj):
    bpy.context.collection.objects.link(obj)
    return obj


def _finalize(bm, name, material):
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    _link(obj)
    if material is not None:
        matlib.apply_material(obj, material)
    return obj


def make_box(name, center, size, material, rotation=None, bevel=0.0, bevel_segments=2):
    """A box of world-space `size` centered at `center` (both 3-tuples),
    optionally rotated by a 3x3 `rotation` matrix about its center, with
    exposed edges chamfered by `bevel` (world units)."""
    bm = bmesh.new()
    ret = bmesh.ops.create_cube(bm, size=1.0)
    verts = ret["verts"]
    scale = Matrix.Diagonal((size[0], size[1], size[2], 1.0))
    rot = rotation.to_4x4() if rotation is not None else Matrix.Identity(4)
    xform = Matrix.Translation(Vector(center)) @ rot @ scale
    bmesh.ops.transform(bm, matrix=xform, verts=verts)
    if bevel > 0.0:
        bmesh.ops.bevel(bm, geom=bm.edges[:], offset=bevel,
                         segments=bevel_segments, affect="EDGES", clamp_overlap=True)
    return _finalize(bm, name, material)


def make_halfcyl(name, center, radius, length, material, rotation=None, segments=8):
    """A half-cylinder (barrel-tile silhouette) of given `radius`, running
    `length` along its local X axis, bulging toward local +Z."""
    bm = bmesh.new()
    ring = []
    for i in range(segments + 1):
        ang = math.pi * i / segments  # 0..pi, local Y-Z semicircle
        y = radius * math.cos(ang)
        z = radius * math.sin(ang)
        ring.append([bm.verts.new((-length / 2.0, y, z)),
                     bm.verts.new((length / 2.0, y, z))])
    for i in range(segments):
        a0, a1 = ring[i]
        b0, b1 = ring[i + 1]
        bm.faces.new((a0, a1, b1, b0))
    # Cap both open ends of the flute: the curved surface alone leaves the
    # tube open there, and at the eave -- where the end faces outward into
    # open sky rather than into the neighbouring roof -- the unlit inside
    # face of that open end reads as a dark sawtooth tooth (G2 D3).
    bm.faces.new([pair[0] for pair in reversed(ring)])
    bm.faces.new([pair[1] for pair in ring])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    verts = bm.verts[:]
    rot = rotation.to_4x4() if rotation is not None else Matrix.Identity(4)
    xform = Matrix.Translation(Vector(center)) @ rot
    bmesh.ops.transform(bm, matrix=xform, verts=verts)
    return _finalize(bm, name, material)


def make_cylinder(name, center, radius, depth, material, segments=24, rotation=None,
                   radius_top=None):
    """A capped cylinder of given radius/depth along its local Z, rotated
    and placed like `make_box`. Used as a boolean-carve tool (e.g. the gate
    arch's opening) where a smooth true circle is needed. `radius_top`, when
    given, tapers the +Z end to a different radius -- a truncated cone, the
    flare of the chapel bell."""
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=segments,
                           radius1=radius,
                           radius2=radius if radius_top is None else radius_top,
                           depth=depth)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    verts = bm.verts[:]
    rot = rotation.to_4x4() if rotation is not None else Matrix.Identity(4)
    xform = Matrix.Translation(Vector(center)) @ rot
    bmesh.ops.transform(bm, matrix=xform, verts=verts)
    return _finalize(bm, name, material)


def make_well_shaft(name, center, outer_radius, height, shaft_radius, shaft_depth,
                     material, segments=8):
    """An octagonal well basin open down its own shaft: a solid outer rim
    from ground to `height`, a top ledge annulus with the shaft opening cut
    through it, and an inner shaft wall descending to a floor `shaft_depth`
    below ground -- so the basin reads as a real hole, not a flat cap."""
    bm = bmesh.new()

    def ring(z, r):
        return [bm.verts.new((r * math.cos(2.0 * math.pi * i / segments),
                               r * math.sin(2.0 * math.pi * i / segments), z))
                for i in range(segments)]

    outer_top = ring(height, outer_radius)
    outer_bot = ring(0.0, outer_radius)
    inner_top = ring(height, shaft_radius)
    inner_bot = ring(-shaft_depth, shaft_radius)

    def band(bot, top):
        for i in range(segments):
            j = (i + 1) % segments
            bm.faces.new((bot[i], bot[j], top[j], top[i]))

    band(outer_bot, outer_top)
    band(inner_bot, inner_top)
    for i in range(segments):
        j = (i + 1) % segments
        bm.faces.new((outer_top[i], outer_top[j], inner_top[j], inner_top[i]))
    base_center = bm.verts.new((0.0, 0.0, 0.0))
    for i in range(segments):
        j = (i + 1) % segments
        bm.faces.new((outer_bot[j], outer_bot[i], base_center))
    floor_center = bm.verts.new((0.0, 0.0, -shaft_depth))
    for i in range(segments):
        j = (i + 1) % segments
        bm.faces.new((inner_bot[i], inner_bot[j], floor_center))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    verts = bm.verts[:]
    bmesh.ops.transform(bm, matrix=Matrix.Translation(Vector(center)), verts=verts)
    return _finalize(bm, name, material)


def cone_cap(name, center_xy, radius, base_z, peak_z, material, theta_start, theta_end, n_sides=6):
    """A shallow fan-of-triangles roof closing a polygonal apse: rim at
    `base_z` sweeping `theta_start`..`theta_end` around `center_xy`, drawn up
    to a single ridge point at `peak_z`."""
    bm = bmesh.new()
    apex = bm.verts.new((center_xy[0], center_xy[1], peak_z))
    rim = []
    for i in range(n_sides + 1):
        th = theta_start + i * (theta_end - theta_start) / n_sides
        rim.append(bm.verts.new((center_xy[0] + radius * math.cos(th),
                                  center_xy[1] + radius * math.sin(th), base_z)))
    for i in range(n_sides):
        bm.faces.new((rim[i], rim[i + 1], apex))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return _finalize(bm, name, material)


def gable_infill(name, x, thickness, depth, eave_z, ridge_z, material):
    """Triangular pediment wall closing a gable roof's end: base at eave_z
    spanning the full depth, apex at ridge_z above the centre -- matches the
    roof's own pitch exactly so it sits flush under the sloped deck with no
    gap into the attic void."""
    half_depth = depth / 2.0
    bm = bmesh.new()
    xm, xp = x - thickness / 2.0, x + thickness / 2.0
    v = {}
    for tag, xx in (("m", xm), ("p", xp)):
        v[f"{tag}_l"] = bm.verts.new((xx, -half_depth, eave_z))
        v[f"{tag}_r"] = bm.verts.new((xx, half_depth, eave_z))
        v[f"{tag}_t"] = bm.verts.new((xx, 0.0, ridge_z))
    bm.faces.new((v["m_l"], v["m_r"], v["m_t"]))
    bm.faces.new((v["p_l"], v["p_r"], v["p_t"]))
    bm.faces.new((v["m_l"], v["m_t"], v["p_t"], v["p_l"]))
    bm.faces.new((v["m_t"], v["m_r"], v["p_r"], v["p_t"]))
    bm.faces.new((v["m_l"], v["p_l"], v["p_r"], v["m_r"]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    return _finalize(bm, name, material)


def wall_with_openings(name, center_xy, length, thickness, height, axis,
                        material, openings=(), z0=0.0, bevel=0.0):
    """Structural wall as axis-aligned box segments; every opening (door or
    window) leaves a real gap with full wall-thickness reveals on the jamb
    and header faces. `openings`: iterable of
    {"offset": <along-axis offset from center>, "width", "height", "sill"}.
    Returns the list of created wall-segment objects."""
    cx, cy = center_xy
    half = length / 2.0
    ordered = sorted(openings, key=lambda o: o["offset"])
    segments = []
    cursor = -half
    for o in ordered:
        s = o["offset"] - o["width"] / 2.0
        e = o["offset"] + o["width"] / 2.0
        if s > cursor + 1e-6:
            segments.append((cursor, s, None))
        segments.append((s, e, o))
        cursor = e
    if cursor < half - 1e-6:
        segments.append((cursor, half, None))

    def add_seg(tag, along_center, along_len, z_bot, z_top):
        if along_len <= 1e-6 or z_top - z_bot <= 1e-6:
            return
        if axis == "x":
            size = (along_len, thickness, z_top - z_bot)
            center = (cx + along_center, cy, z_bot + (z_top - z_bot) / 2.0)
        else:
            size = (thickness, along_len, z_top - z_bot)
            center = (cx, cy + along_center, z_bot + (z_top - z_bot) / 2.0)
        objs.append(make_box(f"{name}_{tag}", center, size, material, bevel=bevel))

    objs = []
    for i, (s, e, o) in enumerate(segments):
        along_len = e - s
        along_center = (s + e) / 2.0
        if o is None:
            add_seg(f"wall{i}", along_center, along_len, z0, z0 + height)
        else:
            sill = o.get("sill", 0.0)
            add_seg(f"sill{i}", along_center, along_len, z0, z0 + sill)
            add_seg(f"head{i}", along_center, along_len, z0 + sill + o["height"], z0 + height)
    return objs


def _roof_deck_panel(name, panel_center, length, run, thickness, rot, material, slope="a"):
    """A flat roof-slope deck panel with an explicit per-plane UV baked in
    directly, instead of the generic box projection every other object gets
    (materials.project_uv's cube_project, which picks one of 6 world-axis
    directions per face -- fine for axis-aligned walls, but it can't know a
    sloped face's own eave-to-ridge direction, so it maps different slopes
    at different scales/orientations and leaves a scale+orientation break
    at the valley where two slopes meet, per G2 D1/D2/D5). U (still local X
    before `rot` is applied) is the ridge-line course axis; V (local Y) is
    the eave-to-ridge slope axis -- exactly the panel's own two in-plane
    directions, since the deck is flat and `rot` is a rigid rotation that
    doesn't distort them. One absolute world scale (materials.TEXEL_SCALE_M)
    everywhere makes every roof plane -- and both sides of the valley --
    match the kit's canonical 6.4 mm/texel density.

    UV origin is shifted to the panel's own corner (not its signed centre)
    and offset 0.02 tiles clear of the U=0/V=0 boundary, and the two slopes
    (`slope` 'a'/'b') carry different V biases: co.x/co.y = 0 is where a
    REPEAT-wrapped tile's own seam sits, so a centred UV put that seam at
    the panel's exact middle -- a mirror-symmetric luminance band across the
    fall line, worse the closer the camera (N3). The bias must stay U=0.02
    with no per-panel scaling: a translated-only origin can't re-cross the
    boundary regardless of `ext_length`, so long roofs (casa_two_story) stay
    seam-free without a length-dependent tolerance."""
    bm = bmesh.new()
    ret = bmesh.ops.create_cube(bm, size=1.0)
    verts = ret["verts"]
    scale = Matrix.Diagonal((length, run, thickness, 1.0))
    bmesh.ops.transform(bm, matrix=scale, verts=verts)
    uv_layer = bm.loops.layers.uv.new("UVMap")
    u0 = length / 2.0
    v0 = run / 2.0
    vbias = 0.0 if slope == "a" else 0.31
    for f in bm.faces:
        for loop in f.loops:
            co = loop.vert.co
            loop[uv_layer].uv = ((co.x + u0) / matlib.TEXEL_SCALE_M + 0.02,
                                  (co.y + v0) / matlib.TEXEL_SCALE_M + 0.02 + vbias)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    rot4 = rot.to_4x4() if rot is not None else Matrix.Identity(4)
    xform = Matrix.Translation(Vector(panel_center)) @ rot4
    bmesh.ops.transform(bm, matrix=xform, verts=bm.verts[:])
    obj = _finalize(bm, name, material)
    obj["vordar_uv_final"] = True
    return obj


def gable_roof(name, center_xy, length, depth, eave_z, pitch_deg,
               deck_material, tile_material, tile_radius=0.15,
               deck_thickness=0.06, tile_overlap=1.18, gable_axis="x",
               eave_overhang=0.35, verge_overhang=0.3):
    """A symmetric two-pitch tiled roof: ridge runs along `gable_axis`
    (the building's long facade), sloping down to eaves that overhang
    `eave_overhang` past the wall face at +/- depth/2, and past the gable
    faces at +/- length/2 by `verge_overhang`. The verge must clear the
    gable wall's full half-thickness: a deck stopping at the wall span
    leaves the gable infill's sloped top half-exposed as a plaster plane
    parallel to the roof pitch (verify._roof_slope_faults). Tiles are
    arrayed half-cylinders sitting flush on the deck and running the whole
    slope on both sides -- the barrel-tile silhouette stays legible at mid
    distance rather than reading as a flat plane. Returns (deck_objs,
    tile_objs, ridge_height); `ridge_height` is the structural ridge (the
    overhangs extend the eave and verge only, they never raise the ridge)."""
    cx, cy = center_xy
    pitch = math.radians(pitch_deg)
    half_depth = depth / 2.0
    rise = half_depth * math.tan(pitch)
    ridge_z = eave_z + rise
    ext_half_depth = half_depth + eave_overhang
    ext_rise = ext_half_depth * math.tan(pitch)
    run = math.hypot(ext_half_depth, ext_rise)
    ext_length = length + 2.0 * verge_overhang

    deck_objs = []
    tile_objs = []
    for sign in (1, -1):
        outer_y = cy + sign * ext_half_depth
        outer_z = ridge_z - ext_rise
        panel_center = (cx, (outer_y + cy) / 2.0, (ridge_z + outer_z) / 2.0)
        # Local +Z must be the OUTWARD slope normal on both sides: the tiles
        # sit at local +Z and bulge along it. A pure X-rotation can point
        # local +Y eave-to-ridge on the +sign slope only by turning +Z into
        # the attic, which buries that slope's tiles under the deck -- so
        # that slope first spins 180 deg about its own Z, keeping +Y
        # eave-to-ridge and +Z outward on both.
        pitch_ext = math.atan2(ext_rise, ext_half_depth)
        if sign > 0:
            rot = Matrix.Rotation(-pitch_ext, 3, "X") @ Matrix.Rotation(math.pi, 3, "Z")
        else:
            rot = Matrix.Rotation(pitch_ext, 3, "X")
        deck = _roof_deck_panel(f"{name}_deck_{'a' if sign > 0 else 'b'}", panel_center,
                                 ext_length, run, deck_thickness, rot, deck_material,
                                 slope="a" if sign > 0 else "b")
        deck_objs.append(deck)

        tile_pitch = tile_radius * 2.0 / tile_overlap
        n_tiles = max(1, int(ext_length / tile_pitch))
        outward = deck_thickness / 2.0
        for i in range(n_tiles):
            tx = -ext_length / 2.0 + (i + 0.5) * (ext_length / n_tiles)
            local = Vector((tx, 0.0, outward))
            world = Vector(panel_center) + rot @ local
            # make_halfcyl's local X is its length axis and local Z is its
            # bulge axis; the deck panel's local Y is the slope-run
            # direction and local Z is its outward normal, so swap X<->Y
            # about local Z before applying the panel's own world rotation.
            tile = make_halfcyl(f"{name}_tile_{'a' if sign > 0 else 'b'}_{i}",
                                world, tile_radius, run * 0.98, tile_material,
                                rotation=rot @ Matrix.Rotation(math.pi / 2.0, 3, "Z"),
                                segments=7)
            tile_objs.append(tile)

    # Caballete: a half-cylinder course astride the bare ridge arris the two
    # decks meet at -- `gable_roof` otherwise leaves it uncapped. Runs along
    # world X (`gable_axis="x"`, the only value any kit type uses), so no
    # rotation is needed; caps stay inside `ext_length` so no casa AABB moves.
    cap_len = 0.42
    n_caps = max(1, int(ext_length / cap_len))
    for i in range(n_caps):
        tx = -ext_length / 2.0 + (i + 0.5) * (ext_length / n_caps)
        tile_objs.append(make_halfcyl(f"{name}_ridge_{i}",
                                       (cx + tx, cy, ridge_z + tile_radius * 0.35),
                                       tile_radius * 1.15, cap_len * 1.05, tile_material,
                                       segments=7))
    return deck_objs, tile_objs, ridge_z


def _wedge_uv(bm, sweep_center, center_z, sweep_axis, r_ref):
    """Cylindrical UV for one barrel_shell wedge, in the ring's own frame:
    arc length (`s`, measured on `r_ref` so neighbouring wedges continue),
    radius (`r`) and the extrusion coordinate (`e`).

    A voussoir ring box-projected like flat wall inherits the tile's own
    horizontal courses, so a dressed arch reads as the wall it is cut into
    and its blocks never read as radial (the P3.0 gate's portal-ring
    finding). Mapping arc length onto one axis turns those courses across
    the arc, which is what a voussoir joint is. Faces are classified by
    which of the three frame directions their normal follows, since no
    single pair of axes is non-degenerate on all six faces of a wedge."""
    uv_layer = bm.loops.layers.uv.new("UVMap")
    sweep_i = 0 if sweep_axis == "x" else 1
    extrude_i = 1 - sweep_i

    def frame(co):
        du = co[sweep_i] - sweep_center
        dz = co[2] - center_z
        return co[extrude_i], math.hypot(du, dz), r_ref * math.atan2(du, dz)

    for f in bm.faces:
        n = f.normal
        centre = f.calc_center_median()
        du = centre[sweep_i] - sweep_center
        dz = centre[2] - center_z
        rad = math.hypot(du, dz) or 1.0
        radial = (du / rad, dz / rad)
        n_e = abs(n[extrude_i])
        n_r = abs(n[sweep_i] * radial[0] + n[2] * radial[1])
        n_t = max(0.0, 1.0 - n_e - n_r)
        for loop in f.loops:
            e, rr, s = frame(loop.vert.co)
            if n_e >= n_r and n_e >= n_t:
                uv = (s, rr)          # the annulus the camera faces
            elif n_r >= n_t:
                uv = (e, s)           # intrados / extrados
            else:
                uv = (e, rr)          # radial joint face
            loop[uv_layer].uv = (uv[0] / matlib.TEXEL_SCALE_M, uv[1] / matlib.TEXEL_SCALE_M)


def barrel_shell(name, sweep_center, extrude_range, springline_z, half_span,
                  rise, thickness, material, n_wedges, sweep_axis,
                  phi_range=None, radial_jitter=0.0, seed=1, extrude_ends=None):
    """A segmental/semicircular vault or arch as a swept ring of true
    trapezoidal stone-block wedges: each wedge's tangential faces are actual
    radial planes (both containing the sweep axis), so neighbours share an
    exact coincident face with no gap or overlap at the joint -- the defect
    a rectangular-box approximation leaves, exposing a poorly-lit sliver
    that renders as a dark, jagged band. `sweep_axis` is the horizontal axis
    the arc curves across ('x' for the gate arch, 'y' for the chapel's
    transverse vault); the extrusion runs along the other horizontal axis
    over `extrude_range` (start, end). `phi_range` overrides the default
    +/-theta0 span (used for a partial rubble-lip ring); `radial_jitter`
    randomizes each wedge's own radius for a broken-edge look. Each wedge
    object carries its arc centre/radii as extras so a re-imported glb can
    check its own faces for inward-pointing (flipped) normals without
    needing the build-time parameters again. `extrude_ends`, if given, is a
    per-wedge sequence overriding `extrude_range[1]` -- a ragged fracture
    lip whose break line varies wedge to wedge rather than a uniform cut."""
    r = (half_span ** 2 + rise ** 2) / (2.0 * rise)
    center_z = springline_z + rise - r
    theta0 = math.atan2(half_span, r - rise)
    lo, hi = phi_range if phi_range is not None else (-theta0, theta0)
    rng = random.Random(seed)
    e0, e1_default = extrude_range

    def ring_point(phi, rad):
        return math.sin(phi) * rad, center_z + math.cos(phi) * rad

    objs = []
    for i in range(n_wedges):
        phi_lo = lo + i * (hi - lo) / n_wedges
        phi_hi = lo + (i + 1) * (hi - lo) / n_wedges
        jitter = rng.uniform(-radial_jitter, radial_jitter)
        r_out = r + jitter
        r_in = r_out - thickness
        e1 = extrude_ends[i] if extrude_ends is not None else e1_default

        bm = bmesh.new()
        v = {}
        for rtag, rad in (("i", r_in), ("o", r_out)):
            for ptag, phi in (("lo", phi_lo), ("hi", phi_hi)):
                u, z = ring_point(phi, rad)
                for etag, e in (("0", e0), ("1", e1)):
                    co = (sweep_center + u, e, z) if sweep_axis == "x" else (e, sweep_center + u, z)
                    v[f"{rtag}{ptag}{etag}"] = bm.verts.new(co)
        bm.faces.new((v["ilo0"], v["ilo1"], v["ihi1"], v["ihi0"]))
        bm.faces.new((v["olo0"], v["olo1"], v["ohi1"], v["ohi0"]))
        bm.faces.new((v["ilo0"], v["olo0"], v["ohi0"], v["ihi0"]))
        bm.faces.new((v["ilo1"], v["olo1"], v["ohi1"], v["ihi1"]))
        bm.faces.new((v["ilo0"], v["ilo1"], v["olo1"], v["olo0"]))
        bm.faces.new((v["ihi0"], v["ihi1"], v["ohi1"], v["ohi0"]))
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
        _wedge_uv(bm, sweep_center, center_z, sweep_axis, r)
        obj = _finalize(bm, f"{name}_wedge{i}", material)
        obj["vordar_uv_final"] = True
        obj["vordar_arc_axis"] = sweep_axis
        obj["vordar_arc_u"] = float(sweep_center)
        obj["vordar_arc_z"] = float(center_z)
        obj["vordar_r_out"] = float(r_out)
        obj["vordar_r_in"] = float(r_in)
        objs.append(obj)
    return objs, r, center_z, theta0


def curve_bars_to_mesh(name, curve_data, material, bevel_depth):
    curve_data.bevel_depth = bevel_depth
    curve_data.bevel_resolution = 2
    curve_data.fill_mode = "FULL"
    curve_obj = bpy.data.objects.new(name + "_curve", curve_data)
    _link(curve_obj)
    view_layer = bpy.context.view_layer
    prev_active = view_layer.objects.active
    prev_selected = list(bpy.context.selected_objects)
    for o in prev_selected:
        o.select_set(False)
    curve_obj.select_set(True)
    view_layer.objects.active = curve_obj
    bpy.ops.object.convert(target="MESH")
    mesh_obj = view_layer.objects.active
    mesh_obj.name = name
    mesh_obj.data.name = name
    matlib.apply_material(mesh_obj, material)
    mesh_obj.select_set(False)
    for o in prev_selected:
        o.select_set(True)
    view_layer.objects.active = prev_active
    return mesh_obj


def new_poly_curve(points, closed=False):
    curve_data = bpy.data.curves.new("bar", type="CURVE")
    curve_data.dimensions = "3D"
    spline = curve_data.splines.new("POLY")
    spline.points.add(len(points) - 1)
    for i, p in enumerate(points):
        spline.points[i].co = (p[0], p[1], p[2], 1.0)
    spline.use_cyclic_u = closed
    return curve_data
