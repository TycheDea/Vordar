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
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    verts = bm.verts[:]
    rot = rotation.to_4x4() if rotation is not None else Matrix.Identity(4)
    xform = Matrix.Translation(Vector(center)) @ rot
    bmesh.ops.transform(bm, matrix=xform, verts=verts)
    return _finalize(bm, name, material)


def make_octagon_prism(name, center, radius, height, material, bevel=0.0):
    bm = bmesh.new()
    ret = bmesh.ops.create_circle(bm, cap_ends=True, cap_tris=False,
                                   segments=8, radius=radius,
                                   matrix=Matrix.Identity(4))
    base_verts = ret["verts"]
    ret2 = bmesh.ops.extrude_face_region(bm, geom=bm.faces[:])
    top_verts = [v for v in ret2["geom"] if isinstance(v, bmesh.types.BMVert)]
    bmesh.ops.translate(bm, verts=top_verts, vec=(0.0, 0.0, height))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces[:])
    if bevel > 0.0:
        edges = [e for e in bm.edges if e.is_boundary or e.calc_face_angle(0.0) > 0.35]
        bmesh.ops.bevel(bm, geom=edges, offset=bevel, segments=2,
                         affect="EDGES", clamp_overlap=True)
    verts = bm.verts[:]
    xform = Matrix.Translation(Vector((center[0], center[1], center[2])))
    bmesh.ops.transform(bm, matrix=xform, verts=verts)
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


def gable_roof(name, center_xy, length, depth, eave_z, pitch_deg,
               deck_material, tile_material, tile_radius=0.11,
               deck_thickness=0.06, tile_overlap=1.18, gable_axis="x"):
    """A symmetric two-pitch tiled roof: ridge runs along `gable_axis`
    (the building's long facade), sloping down to eaves at +/- depth/2.
    Tiles are arrayed half-cylinders on both slopes -- the barrel-tile
    silhouette stays legible at mid distance rather than reading as a flat
    plane. Returns (deck_objs, tile_objs, ridge_height)."""
    cx, cy = center_xy
    pitch = math.radians(pitch_deg)
    half_depth = depth / 2.0
    rise = half_depth * math.tan(pitch)
    ridge_z = eave_z + rise
    run = math.hypot(half_depth, rise)

    deck_objs = []
    tile_objs = []
    for sign in (1, -1):
        eave_y = cy + sign * half_depth
        panel_center = (cx, (eave_y + cy) / 2.0, (eave_z + ridge_z) / 2.0)
        theta = math.atan2(rise, -sign * half_depth)
        rot = Matrix.Rotation(theta, 3, "X")
        deck = make_box(f"{name}_deck_{'a' if sign > 0 else 'b'}", panel_center,
                         (length, run, deck_thickness), deck_material, rotation=rot)
        deck_objs.append(deck)

        tile_pitch = tile_radius * 2.0 / tile_overlap
        n_tiles = max(1, int(length / tile_pitch))
        outward = tile_radius + deck_thickness / 2.0
        for i in range(n_tiles):
            tx = -length / 2.0 + (i + 0.5) * (length / n_tiles)
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
    return deck_objs, tile_objs, ridge_z


def barrel_shell(name, sweep_center, extrude_range, springline_z, half_span,
                  rise, thickness, material, n_wedges, sweep_axis,
                  phi_range=None, radial_jitter=0.0, seed=1):
    """A segmental/semicircular vault or arch as a swept ring of wedge
    blocks -- real stone-block geometry rather than a smooth shell.
    `sweep_axis` is the horizontal axis the arc curves across ('x' for the
    gate arch, 'y' for the chapel's transverse vault); the extrusion runs
    along the other horizontal axis over `extrude_range` (start, end).
    `phi_range` overrides the default +/-theta0 span (used for a partial
    rubble-lip ring); `radial_jitter` randomizes each wedge's radius for a
    broken-edge look."""
    r = (half_span ** 2 + rise ** 2) / (2.0 * rise)
    center_z = springline_z + rise - r
    theta0 = math.atan2(half_span, r - rise)
    lo, hi = phi_range if phi_range is not None else (-theta0, theta0)
    rng = random.Random(seed)
    e0, e1 = extrude_range
    run_extent = e1 - e0
    run_mid = (e0 + e1) / 2.0
    objs = []
    for i in range(n_wedges):
        phi = lo + (i + 0.5) * (hi - lo) / n_wedges
        r_mid = r - thickness / 2.0 + rng.uniform(-radial_jitter, radial_jitter)
        tangential = r_mid * (hi - lo) / n_wedges * 2.0
        sweep = sweep_center + r_mid * math.sin(phi)
        z = center_z + r_mid * math.cos(phi)
        if sweep_axis == "x":
            size = (tangential, run_extent, thickness)
            center = (sweep, run_mid, z)
            rot = Matrix.Rotation(phi, 3, "Y")
        else:
            size = (run_extent, tangential, thickness)
            center = (run_mid, sweep, z)
            rot = Matrix.Rotation(-phi, 3, "X")
        objs.append(make_box(f"{name}_wedge{i}", center, size, material, rotation=rot))
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
