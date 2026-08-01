"""Rocalba town-kit building types (docs/town-premise.md S5/S6).

Every builder returns (objects, dims) -- dims is a plain dict of the
measurements a caller (build_town_kit.py) might want to assert or report.
Local frame: Z-up, ground z=0, footprint centered near local (0,0); "front"
(the street facade carrying the door) is +Y unless noted. Wall thickness and
door/window proportions are this pilot's own reasoned defaults -- footprints
are asked to be a near-match to the chapter03 graybox collision shells, not
an exact one.
"""

import math
import random

import bpy
from mathutils import Matrix

from . import geo
from . import materials as matlib


WALL_THICKNESS = 0.45
DOOR_THICKNESS = 0.06
SHUTTER_THICKNESS = 0.04
REVEAL_MIN = 0.15  # docs-required minimum reveal depth at openings

assert WALL_THICKNESS >= REVEAL_MIN


QUOIN_PROUD = 0.05
# Courses interpenetrate rather than merely touch: two separately-exported
# boxes sharing an exact horizontal plane z-fight there, and any positive
# clearance is a real void that renders as a black slot between courses
# (the "each block glued on" read, P3.0 gate fix 4).
QUOIN_BOND = 0.012


def _quoins(name, corner_xy, height, material, wall_thickness, z0=0.0, block=0.55):
    """Corner quoins as an ashlar alternating-course stack: real dressed
    quoins alternate long/short face length course to course and never
    repeat an identical block height -- a uniform block size read as an
    excluded, machine-repeated brick (G2 D7/D8).

    Each block's two outer faces are anchored to the wall's own outer
    planes (+ `QUOIN_PROUD`), not to the wall centreline corner -- real
    dressed quoins are coursed into the wall, so the long/short alternation
    shows as varying face length only, never as varying protrusion past the
    render. Successive courses bond: each block overlaps its neighbours by
    `QUOIN_BOND`, so the chain reads as one stack and the 1.5 cm bevel
    arris is the only joint line."""
    rng = random.Random(name)
    sx = math.copysign(1.0, corner_xy[0]) if corner_xy[0] != 0.0 else 1.0
    sy = math.copysign(1.0, corner_xy[1]) if corner_xy[1] != 0.0 else 1.0
    objs = []
    z = z0
    i = 0
    while z < z0 + height - 1e-6:
        pitch = block * rng.uniform(0.75, 1.35)
        top = min(z + pitch, z0 + height)
        bot = max(z0, z - QUOIN_BOND)
        h = top - bot + (QUOIN_BOND if top < z0 + height else 0.0)
        if h > 0.05:
            long_x = i % 2 == 0
            bx = block * (rng.uniform(1.15, 1.35) if long_x else rng.uniform(0.75, 0.9))
            by = block * (rng.uniform(0.75, 0.9) if long_x else rng.uniform(1.15, 1.35))
            cx_b = corner_xy[0] + sx * (wall_thickness / 2.0 + QUOIN_PROUD - bx / 2.0)
            cy_b = corner_xy[1] + sy * (wall_thickness / 2.0 + QUOIN_PROUD - by / 2.0)
            objs.append(geo.make_box(f"{name}_q{i}", (cx_b, cy_b, bot + h / 2.0),
                                     (bx, by, h), material, bevel=0.015))
        z += pitch
        i += 1
    return objs


def make_reja(name, center, width, height, material, bar_spacing=0.15, bar_radius=0.012,
              scroll=True):
    """A wrought-iron grille built from curve bars (poly curves with a
    round bevel profile), converted to mesh and joined into one object."""
    cx, cy, cz = center
    parts = []
    n_vert = max(2, int(width / bar_spacing) + 1)
    for i in range(n_vert):
        x = cx - width / 2.0 + i * (width / (n_vert - 1))
        pts = [(x, cy, cz - height / 2.0), (x, cy, cz + height / 2.0)]
        if scroll and i == n_vert // 2:
            # a small decorative scroll atop the centre bar
            pts += [(x + bar_spacing * 0.4, cy, cz + height / 2.0 + bar_spacing * 0.3),
                    (x, cy, cz + height / 2.0 + bar_spacing * 0.6),
                    (x - bar_spacing * 0.4, cy, cz + height / 2.0 + bar_spacing * 0.3)]
        cd = geo.new_poly_curve(pts)
        parts.append(geo.curve_bars_to_mesh(f"{name}_v{i}", cd, material, bar_radius))
    n_horiz = 3
    for j in range(n_horiz):
        z = cz - height / 2.0 + j * (height / (n_horiz - 1))
        pts = [(cx - width / 2.0, cy, z), (cx + width / 2.0, cy, z)]
        cd = geo.new_poly_curve(pts)
        parts.append(geo.curve_bars_to_mesh(f"{name}_h{j}", cd, material, bar_radius))

    view_layer = bpy.context.view_layer
    for o in bpy.context.selected_objects:
        o.select_set(False)
    for p in parts:
        p.select_set(True)
    view_layer.objects.active = parts[0]
    bpy.ops.object.join()
    merged = view_layer.objects.active
    merged.name = name
    merged.data.name = name
    merged.select_set(False)
    return merged


def _door_fill(name, center_xy, width, height, sill, material, wall_thickness):
    cz = sill + height / 2.0
    return geo.make_box(name, (center_xy[0], center_xy[1], cz),
                         (width * 0.94, DOOR_THICKNESS, height * 0.96),
                         material, bevel=0.01)


def _window_fill(name, center_xy, width, height, sill, iron_mat, oak_mat, wall_thickness):
    cz = sill + height / 2.0
    reja = make_reja(f"{name}_reja", (center_xy[0], center_xy[1] + wall_thickness * 0.35, cz),
                      width * 0.85, height * 0.85, iron_mat)
    shutter = geo.make_box(f"{name}_shutter", (center_xy[0], center_xy[1] - wall_thickness * 0.35, cz),
                            (width * 0.92, SHUTTER_THICKNESS, height * 0.92), oak_mat)
    return [reja, shutter]


def _shell_union(objs, joined_name):
    """Weld every shell piece (walls, decks, gable infill, all blocks)
    into one solid via a SINGLE multi-operand boolean UNION, not
    bpy.ops.object.join() and not chained pairwise unions: a plain join
    only concatenates mesh data, leaving touching pieces as coincident
    faces (the degenerate input that silently dropped a whole wall face
    at the corner, G2 D6), while chained pairwise unions re-tessellate
    the accumulating mesh at every step and pile up doubled faces and
    sliver seams wherever operands share exactly coplanar faces (flush
    facade bands, the common ground plane). One EXACT arrangement over
    all pieces at once resolves those coincidences once, coherently."""
    view_layer = bpy.context.view_layer
    base = objs[0]
    for other in objs[1:]:
        # The boolean keeps an operand face's material only when the
        # base already carries that material as a slot; otherwise the
        # face silently falls back to slot 0 (probed on Blender 5.2) --
        # which turned the terracotta roof decks into encalado.
        for mat in other.data.materials:
            if mat.name not in base.data.materials:
                base.data.materials.append(mat)
    operands = bpy.data.collections.new(f"{joined_name}_operands")
    bpy.context.scene.collection.children.link(operands)
    for other in objs[1:]:
        operands.objects.link(other)
    mod = base.modifiers.new("shell_union", type="BOOLEAN")
    mod.operation = "UNION"
    mod.solver = "EXACT"
    mod.operand_type = "COLLECTION"
    mod.collection = operands
    view_layer.objects.active = base
    bpy.ops.object.modifier_apply(modifier=mod.name)
    for other in objs[1:]:
        bpy.data.objects.remove(other, do_unlink=True)
    bpy.data.collections.remove(operands)
    base.name = joined_name
    base.data.name = joined_name
    base.select_set(False)
    return base


def _finalize_shell(objs, deck, name, union=True):
    """Finalize shell UVs, then (unless the caller unions the shell itself,
    casa_corner) weld the shell pieces into one sealed solid. The UV pass
    runs before any join/boolean: the deck panels already carry their own
    explicit per-slope UV (geo._roof_deck_panel) and must not be
    re-projected, and giving every other shell piece its box-projected UV
    now means the boolean only ever has to interpolate UV across a cut,
    never re-derive it. The union is what keeps the casa roof invariant
    (verify._roof_slope_faults) true by construction: loose gable-infill
    top faces lie embedded inside the deck slab as large encalado planes
    parallel to the slope, and only the boolean dissolves them.
    vordar_uv_final tells build_town_kit.py's own project_uv pass to leave
    these alone."""
    deck_ids = {id(o) for o in deck}
    for o in objs:
        if _is_shell_name(o.name):
            if id(o) not in deck_ids:
                matlib.project_uv(o)
            o["vordar_uv_final"] = True
    if not union:
        return objs
    shell = [o for o in objs if _is_shell_name(o.name)]
    detail = [o for o in objs if not _is_shell_name(o.name)]
    merged = _shell_union(shell, f"{name}_shell")
    merged["vordar_uv_final"] = True
    return [merged] + detail


def build_casa_shell(name, mats, width, depth, wall_height, pitch_deg,
                      front_openings, side_openings_left=None, side_openings_right=None,
                      wall_thickness=WALL_THICKNESS, union=True):
    objs = []
    enc = mats["encalado"]
    lime = mats["limestone_dressed"]
    oak = mats["oak_dark"]
    iron = mats["iron_wrought"]

    def build_wall(tag, center_xy, length, axis, openings):
        wall_objs = geo.wall_with_openings(f"{name}_{tag}", center_xy, length, wall_thickness,
                                            wall_height, axis, enc, openings=openings)
        objs.extend(wall_objs)
        for i, o in enumerate(openings):
            if axis == "x":
                op_xy = (center_xy[0] + o["offset"], center_xy[1])
            else:
                op_xy = (center_xy[0], center_xy[1] + o["offset"])
            if o["kind"] == "door":
                objs.append(_door_fill(f"{name}_{tag}_door{i}", op_xy, o["width"], o["height"],
                                        o.get("sill", 0.0), oak, wall_thickness))
            else:
                objs.extend(_window_fill(f"{name}_{tag}_win{i}", op_xy, o["width"], o["height"],
                                          o.get("sill", 0.0), iron, oak, wall_thickness))

    build_wall("front", (0.0, depth / 2.0), width, "x", front_openings)
    build_wall("back", (0.0, -depth / 2.0), width, "x", [])
    build_wall("left", (-width / 2.0, 0.0), depth, "y", side_openings_left or [])
    build_wall("right", (width / 2.0, 0.0), depth, "y", side_openings_right or [])

    for cx in (-width / 2.0, width / 2.0):
        for cy in (-depth / 2.0, depth / 2.0):
            objs.extend(_quoins(f"{name}_quoin_{cx:.1f}_{cy:.1f}", (cx, cy), wall_height, lime,
                                 wall_thickness))

    deck, tiles, ridge_z = geo.gable_roof(f"{name}_roof", (0.0, 0.0), width, depth, wall_height,
                                          pitch_deg, mats["terracotta_tile"], mats["terracotta_tile"])
    objs.extend(deck)
    objs.extend(tiles)

    # gable-end infill: the left/right walls (perpendicular to the ridge)
    # only reach the eave line, so close the triangular attic void above
    # them up to the ridge on both ends.
    for gx in (-width / 2.0, width / 2.0):
        objs.append(geo.gable_infill(f"{name}_gable_{'left' if gx < 0 else 'right'}",
                                      gx, wall_thickness, depth, wall_height, ridge_z, enc))

    objs = _finalize_shell(objs, deck, name, union=union)
    return objs, {"width": width, "depth": depth, "wall_height": wall_height, "ridge_height": ridge_z}


def build_casa_small_a(mats):
    return build_casa_shell(
        "casa_small_a", mats, width=6.0, depth=8.0, wall_height=4.0, pitch_deg=28.0,
        front_openings=[
            {"kind": "door", "offset": -1.6, "width": 1.05, "height": 2.15, "sill": 0.0},
            {"kind": "window", "offset": 1.6, "width": 0.9, "height": 1.1, "sill": 1.3},
        ],
    )


def build_casa_small_b(mats):
    objs, dims = build_casa_shell(
        "casa_small_b", mats, width=5.6, depth=7.4, wall_height=3.8, pitch_deg=30.0,
        front_openings=[
            {"kind": "door", "offset": 1.3, "width": 1.0, "height": 2.1, "sill": 0.0},
            {"kind": "window", "offset": -1.4, "width": 0.8, "height": 1.0, "sill": 1.35},
        ],
        side_openings_right=[
            {"kind": "window", "offset": 1.8, "width": 0.7, "height": 0.9, "sill": 1.4},
        ],
    )
    return objs, dims


def build_casa_two_story(mats):
    name = "casa_two_story"
    width, depth = 7.0, 10.0
    h1, h2 = 3.4, 3.0
    pitch = 26.0
    objs = []
    enc, lime, oak, iron = (mats["encalado"], mats["limestone_dressed"],
                             mats["oak_dark"], mats["iron_wrought"])

    def band(tag, z0, height, front_ops):
        wall_objs = geo.wall_with_openings(f"{name}_front_{tag}", (0.0, depth / 2.0), width,
                                            WALL_THICKNESS, height, "x", enc, openings=front_ops, z0=z0)
        objs.extend(wall_objs)
        for i, o in enumerate(front_ops):
            xy = (o["offset"], depth / 2.0)
            if o["kind"] == "door":
                objs.append(_door_fill(f"{name}_{tag}_door{i}", xy, o["width"], o["height"],
                                        z0 + o.get("sill", 0.0), oak, WALL_THICKNESS))
            else:
                objs.extend(_window_fill(f"{name}_{tag}_win{i}", xy, o["width"], o["height"],
                                          z0 + o.get("sill", 0.0), iron, oak, WALL_THICKNESS))

    band("ground", 0.0, h1, [
        {"kind": "door", "offset": -2.2, "width": 1.1, "height": 2.2, "sill": 0.0},
        {"kind": "window", "offset": 2.1, "width": 0.9, "height": 1.1, "sill": 1.2},
    ])
    band("upper", h1, h2, [
        {"kind": "window", "offset": -2.1, "width": 0.9, "height": 1.2, "sill": 0.9},
        {"kind": "window", "offset": 2.1, "width": 0.9, "height": 1.2, "sill": 0.9},
    ])

    for tag, center_xy, length, axis in (
        ("back", (0.0, -depth / 2.0), width, "x"),
        ("left", (-width / 2.0, 0.0), depth, "y"),
        ("right", (width / 2.0, 0.0), depth, "y"),
    ):
        objs.extend(geo.wall_with_openings(f"{name}_{tag}", center_xy, length, WALL_THICKNESS,
                                            h1 + h2, axis, enc, openings=[]))

    for cx in (-width / 2.0, width / 2.0):
        for cy in (-depth / 2.0, depth / 2.0):
            objs.extend(_quoins(f"{name}_quoin_{cx:.1f}_{cy:.1f}", (cx, cy), h1 + h2, lime,
                                 WALL_THICKNESS))

    deck, tiles, ridge_z = geo.gable_roof(f"{name}_roof", (0.0, 0.0), width, depth, h1 + h2,
                                          pitch, mats["terracotta_tile"], mats["terracotta_tile"])
    objs.extend(deck)
    objs.extend(tiles)
    for gx in (-width / 2.0, width / 2.0):
        objs.append(geo.gable_infill(f"{name}_gable_{'left' if gx < 0 else 'right'}",
                                      gx, WALL_THICKNESS, depth, h1 + h2, ridge_z, enc))
    objs = _finalize_shell(objs, deck, name)
    return objs, {"width": width, "depth": depth, "wall_height": h1 + h2, "ridge_height": ridge_z}


def _is_shell_name(name):
    """Structural shell pieces (walls, opening reveals, roof deck, gable
    ends) vs. decorative detail (tiles, quoins, door/window fills) -- only
    the shell participates in a casa's footprint/roof boolean union."""
    return any(tag in name for tag in ("_wall", "_sill", "_head", "_roof_deck_", "_gable_"))


def build_casa_corner(mats):
    """L-footprint corner casa: a main block plus a wing rotated 90 degrees
    about its own centre so its ridge runs perpendicular to the main
    block's, then positioned to share a full wall-thickness seam along one
    side. The two shells are joined and boolean-unioned into a single
    continuous mesh so the roofs meet in a real valley intersection and the
    walls read as one sealed footprint, not two boxes touching at a corner."""
    name = "casa_corner"
    main_objs, main_dims = build_casa_shell(
        f"{name}_main", mats, width=6.0, depth=6.0, wall_height=4.0, pitch_deg=28.0,
        front_openings=[{"kind": "door", "offset": 0.0, "width": 1.1, "height": 2.2, "sill": 0.0}],
        union=False,
    )
    wing_objs, wing_dims = build_casa_shell(
        f"{name}_wing", mats, width=4.0, depth=4.5, wall_height=3.8, pitch_deg=28.0,
        front_openings=[{"kind": "window", "offset": 0.0, "width": 0.9, "height": 1.1, "sill": 1.2}],
        union=False,
    )
    # Rotating -90 deg about Z (about the wing's own origin, before
    # translating) swaps its footprint to 4.5 x 4.0 and turns its ridge to
    # run along world Y -- perpendicular to main's, the precondition for a
    # real valley. The offset then shares a full wall thickness with main's
    # right side, over a 4 m band, rather than a diagonal corner touch.
    # Baked into the mesh data, not set as an object transform: the detail
    # objects' UV projection (build_town_kit's project_uv pass) works in
    # mesh-local space, so an object-level rotation would leave the wing
    # tiles' texture courses turned 90 deg in world (the louvre-slat read).
    dx, dy = 4.8, 1.0
    wing_xform = Matrix.Translation((dx, dy, 0.0)) @ Matrix.Rotation(-math.pi / 2.0, 4, "Z")
    for o in wing_objs:
        o.data.transform(wing_xform)

    main_shell = [o for o in main_objs if _is_shell_name(o.name)]
    main_detail = [o for o in main_objs if not _is_shell_name(o.name)]
    wing_shell = [o for o in wing_objs if _is_shell_name(o.name)]
    wing_detail = [o for o in wing_objs if not _is_shell_name(o.name)]

    main_merged = _shell_union(main_shell + wing_shell, f"{name}_main_shell")
    main_merged["vordar_uv_final"] = True

    objs = [main_merged] + main_detail + wing_detail
    return objs, {"main": main_dims, "wing": wing_dims, "offset": (dx, dy)}


def build_wall_segment(mats):
    name = "wall_segment"
    length, thickness, height = 4.0, 0.6, 2.6
    lime = mats["limestone_dressed"]
    body = geo.make_box(f"{name}_body", (0.0, 0.0, height / 2.0), (length, thickness, height),
                         lime, bevel=0.02)
    coping = geo.make_box(f"{name}_coping", (0.0, 0.0, height + 0.075),
                           (length + 0.1, thickness + 0.2, 0.15), lime, bevel=0.02)
    rng = random.Random(7)
    rubble = []
    for i in range(5):
        x = rng.uniform(-length / 2.0 + 0.3, length / 2.0 - 0.3)
        h = rng.uniform(0.1, 0.35)
        rubble.append(geo.make_box(f"{name}_rubble{i}", (x, thickness * 0.3, height + 0.15 + h / 2.0),
                                    (rng.uniform(0.2, 0.4), rng.uniform(0.15, 0.3), h), lime, bevel=0.01))
    objs = [body, coping] + rubble
    return objs, {"length": length, "thickness": thickness, "height": height}


def build_gate_arch(mats):
    name = "gate_arch"
    wall_length, thickness, springline = 6.4, 0.9, 3.6
    opening_width = 3.2
    half_span = opening_width / 2.0
    rise = half_span  # semicircular arch
    lime = mats["limestone_dressed"]
    jamb_width = (wall_length - opening_width) / 2.0
    objs = []
    for side in (-1, 1):
        cx = side * (opening_width / 2.0 + jamb_width / 2.0)
        objs.append(geo.make_box(f"{name}_jamb_{side}", (cx, 0.0, springline / 2.0),
                                  (jamb_width, thickness, springline), lime, bevel=0.02))
    wedges, r, center_z, theta0 = geo.barrel_shell(f"{name}_arch", 0.0, (-thickness / 2.0, thickness / 2.0),
                                                    springline, half_span, rise, thickness * 0.7,
                                                    lime, n_wedges=12, sweep_axis="x")
    objs.extend(wedges)
    peak_z = center_z + r
    wall_top = peak_z + 0.4

    # The wall above the springing spans the full face; the arch's own void
    # is then carved out of it with a true cylinder so it wraps flush around
    # the voussoir ring instead of floating above it as a disconnected flat
    # slab, with the coping bearing on a continuous surface.
    head = geo.make_box(f"{name}_head", (0.0, 0.0, (springline + wall_top) / 2.0),
                         (wall_length, thickness, wall_top - springline), lime)
    bore = geo.make_cylinder(f"{name}_bore_tmp", (0.0, 0.0, center_z), r - 0.02, thickness * 2.0,
                              None, segments=24, rotation=Matrix.Rotation(math.pi / 2.0, 3, "X"))
    mod = head.modifiers.new("arch_bore", type="BOOLEAN")
    mod.operation = "DIFFERENCE"
    mod.solver = "EXACT"
    mod.object = bore
    bpy.context.view_layer.objects.active = head
    bpy.ops.object.modifier_apply(modifier=mod.name)
    bpy.data.objects.remove(bore, do_unlink=True)
    objs.append(head)

    objs.append(geo.make_box(f"{name}_coping", (0.0, 0.0, wall_top + 0.075),
                              (wall_length + 0.1, thickness + 0.2, 0.15), lime, bevel=0.02))
    return objs, {"wall_length": wall_length, "opening_width": opening_width,
                  "opening_height": springline, "wall_top": wall_top}


def build_well_basin(mats):
    name = "well_basin"
    radius = 1.25
    basin_h = 0.9
    shaft_radius = radius * 0.55
    shaft_depth = 2.0
    lime = mats["limestone_dressed"]
    oak = mats["oak_dark"]
    objs = [geo.make_well_shaft(f"{name}_basin", (0.0, 0.0, 0.0), radius, basin_h,
                                 shaft_radius, shaft_depth, lime)]
    post_h = 1.8
    post_positions = [(radius * 0.75, 0.0), (-radius * 0.75, 0.0)]
    for i, (px, py) in enumerate(post_positions):
        objs.append(geo.make_box(f"{name}_post{i}", (px, py, basin_h + post_h / 2.0),
                                  (0.16, 0.16, post_h), oak, bevel=0.01))
    # centered over the shaft (x=0), matching the beam's own midpoint
    beam_len = 2.0 * post_positions[0][0]
    objs.append(geo.make_box(f"{name}_beam", (0.0, 0.0, basin_h + post_h + 0.08),
                              (beam_len + 0.2, 0.16, 0.16), oak, bevel=0.01))
    return objs, {"radius": radius, "basin_height": basin_h, "post_height": post_h,
                  "shaft_radius": shaft_radius, "shaft_depth": shaft_depth}


def build_reja_set(mats):
    name = "reja_set"
    iron = mats["iron_wrought"]
    sizes = [("small", 0.6, 0.9), ("medium", 0.8, 1.1), ("large", 1.0, 1.4)]
    objs = []
    gap = 1.5
    x = 0.0
    for tag, w, h in sizes:
        objs.append(make_reja(f"{name}_{tag}", (x, 0.0, h / 2.0 + 0.1), w, h, iron))
        x += gap
    return objs, {"sizes": sizes}


CHAPEL_NAVE_WIDTH = 7.0
CHAPEL_NAVE_LENGTH = 16.0
CHAPEL_WALL_THICKNESS = 0.6
CHAPEL_SPRINGLINE = 7.5
CHAPEL_VAULT_RISE = 3.0
CHAPEL_DOOR = {"width": 2.4, "height": 3.2}
# masonry left exposed (unlined) at every wall top, so the collapsed vault's
# rim always shows the wall's real ~0.6 m thickness rather than the thin
# (0.06 m) interior plaster liner
CHAPEL_RIM_EXPOSED = 0.3
# The bay the vault tore out of. Its side walls are broken masonry down to
# CHAPEL_CROWN_LOW at mid-span, recovering to CHAPEL_CROWN_HIGH at the two
# ends -- which must stay under the intact crown (CHAPEL_SPRINGLINE) or the
# contrast that reads as loss inverts.
CHAPEL_COLLAPSE_X = (1.0, 7.6)
CHAPEL_CROWN_LOW = 4.55
CHAPEL_CROWN_HIGH = 7.30
# The nave paving stands proud of the earth outside. Local z = 0 is the
# prop's own placement height, which for the chapel is exactly the town's
# ground plane (client::ground::GROUND_TOP_Y = -0.5, chapel placed at
# y = -0.5) -- a floor slab topping out at 0 is coplanar with the ground
# mesh and z-fights it into a diagonal moire at gameplay distance.
CHAPEL_FLOOR_TOP = 0.05
# The east facade's wall panels are set back inside the 0.6 m thickness so
# the portal and oculus voussoir rings, which keep the full thickness,
# stand proud and throw a shadow line. The footprint is frozen
# (footprints.ron records the 20.229 m x-span verbatim, D5 margin 0.02 m),
# so depth can only be recovered inward, never added outward.
CHAPEL_EAST_RECESS = 0.20


def build_chapel(mats):
    name = "chapel"
    lime = mats["limestone_dressed"]
    plaster = mats["plaster_smoked"]
    oak = mats["oak_dark"]
    iron = mats["iron_wrought"]
    t = CHAPEL_WALL_THICKNESS
    half_w = CHAPEL_NAVE_WIDTH / 2.0
    half_l = CHAPEL_NAVE_LENGTH / 2.0
    springline = CHAPEL_SPRINGLINE
    objs = []

    # F5(a) — the ragged crown belongs to the collapsed bay's own wall, not
    # to a frieze of blocks resting on a level datum. Span A (intact) and
    # span C (the surviving east corner buttressing the espadaña wall) are
    # ordinary walls to the springline; the bay between them is built as a
    # run of masonry columns whose tops follow a collapse funnel -- deepest
    # mid-span, recovering toward both ends -- and cross the neighbouring
    # spans' height nowhere along its length. A third of the columns carry a
    # part-thickness leaf on top, so the wall's own 0.6 m section breaks
    # into separate courses instead of presenting one continuous top plane.
    span_defs = [
        ("A", -half_l - t / 2.0, CHAPEL_COLLAPSE_X[0], springline,
         [{"offset": -0.35, "width": 0.5, "height": 1.6, "sill": 4.2}]),
        ("C", CHAPEL_COLLAPSE_X[1], half_l + t / 2.0, springline, []),
    ]
    bx0, bx1 = CHAPEL_COLLAPSE_X
    collapse_profile = {}
    for sign in (-1, 1):
        cy = sign * (half_w + t / 2.0)
        profile = collapse_profile.setdefault(sign, [])
        for tag, x0, x1, h, openings in span_defs:
            objs.extend(geo.wall_with_openings(f"{name}_side_{sign}_{tag}", ((x0 + x1) / 2.0, cy),
                                                x1 - x0, t, h, "x", lime, openings=openings))

        rng = random.Random(f"collapse{sign}")
        cursor = bx0
        i = 0
        while cursor < bx1 - 1e-6:
            step = min(rng.uniform(0.30, 0.95), bx1 - cursor)
            u = (cursor + step / 2.0 - bx0) / (bx1 - bx0)
            funnel = CHAPEL_CROWN_LOW + (CHAPEL_CROWN_HIGH - CHAPEL_CROWN_LOW) * abs(2.0 * u - 1.0) ** 1.5
            top = min(CHAPEL_CROWN_HIGH, max(4.2, funnel + rng.uniform(-0.5, 0.5)))
            # Columns overlap by 1 cm: two boxes meeting on an exact shared
            # plane leave a coincident face pair that z-fights.
            objs.append(geo.make_box(f"{name}_side_{sign}_B_wall{i}",
                                     (cursor + step / 2.0, cy, top / 2.0),
                                     (step + 0.01, t, top), lime, bevel=0.02, bevel_segments=1))
            leaf_t = rng.uniform(0.22, 0.36)
            leaf_h = min(rng.uniform(0.14, 0.55), CHAPEL_CROWN_HIGH - top)
            face = rng.choice((-1.0, 1.0))
            if rng.random() < 0.4 and leaf_h > 0.08:
                objs.append(geo.make_box(
                    f"{name}_side_{sign}_B_leaf{i}",
                    (cursor + step / 2.0, cy + face * (t - leaf_t) / 2.0,
                     top + leaf_h / 2.0 - 0.02),
                    (step * rng.uniform(0.55, 0.95), leaf_t, leaf_h + 0.02), lime,
                    bevel=0.02, bevel_segments=1))
            profile.append((cursor, cursor + step, top))
            cursor += step
            i += 1

    # Polygonal apse closing the west end (under the intact half of the
    # vault): a fan of flat wall segments swept through half_w around the
    # nave's own west corners, so it attaches with no gap, capped by a
    # shallow conical roof.
    apse_center = (-half_l, 0.0)
    n_apse_sides = 5
    apse_thetas = [math.pi / 2.0 + i * (math.pi / n_apse_sides) for i in range(n_apse_sides + 1)]
    apse_pts = [(apse_center[0] + half_w * math.cos(th), apse_center[1] + half_w * math.sin(th))
                for th in apse_thetas]
    for i in range(n_apse_sides):
        p0, p1 = apse_pts[i], apse_pts[i + 1]
        seg_len = math.hypot(p1[0] - p0[0], p1[1] - p0[1])
        ang = math.atan2(p1[1] - p0[1], p1[0] - p0[0])
        mid = ((p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0, springline / 2.0)
        objs.append(geo.make_box(f"{name}_apse_wall{i}", mid, (seg_len, t, springline), lime,
                                  rotation=Matrix.Rotation(ang, 3, "Z")))
    objs.append(geo.cone_cap(f"{name}_apse_cap", apse_center, half_w + t / 2.0, springline,
                              springline + 1.0, lime, apse_thetas[0], apse_thetas[-1],
                              n_sides=n_apse_sides))

    apse_len = CHAPEL_NAVE_WIDTH + t
    east_cx = half_l + t / 2.0
    # Recessed panels: the wall face steps back inside the thickness, the
    # rings and the espadaña keep it, so x = 8.6 (the frozen footprint
    # plane) is still reached and the rings now stand CHAPEL_EAST_RECESS
    # proud of the masonry around them.
    east_panel_cx = east_cx - CHAPEL_EAST_RECESS / 2.0
    east_objs = geo.wall_with_openings(f"{name}_east", (east_panel_cx, 0.0), apse_len,
                                        t - CHAPEL_EAST_RECESS, springline,
                                        "y", lime, openings=[
                                            {"offset": 0.0, "width": CHAPEL_DOOR["width"],
                                             "height": CHAPEL_DOOR["height"], "sill": 0.0}])
    objs.extend(east_objs)
    east_head = next(o for o in east_objs if "_head" in o.name)

    # F3 — portal surround: a true semicircular voussoir ring entirely
    # within the 0.6 m wall thickness (r_out 1.55 extrados, r_in 1.20 = half
    # the 2.4 m door opening, so the intrados matches the opening exactly).
    # Bore the receiving masonry the same DIFFERENCE/EXACT pattern
    # build_gate_arch already uses for its own arch bore.
    portal_wedges, _, _, _ = geo.barrel_shell(f"{name}_portal", 0.0, (8.0, 8.6), 3.2, 1.55, 1.55,
                                               0.35, lime, n_wedges=11, sweep_axis="y")
    objs.extend(portal_wedges)
    portal_bore = geo.make_cylinder(f"{name}_portal_bore_tmp", (east_cx, 0.0, 3.2), 1.53, 2.0,
                                     None, segments=24, rotation=Matrix.Rotation(math.pi / 2.0, 3, "Y"))
    mod = east_head.modifiers.new("portal_bore", type="BOOLEAN")
    mod.operation, mod.solver, mod.object = "DIFFERENCE", "EXACT", portal_bore
    bpy.context.view_layer.objects.active = east_head
    bpy.ops.object.modifier_apply(modifier=mod.name)
    bpy.data.objects.remove(portal_bore, do_unlink=True)

    # F4a — oculus: a 1.0 m round light dressed with its own full-circle
    # voussoir ring (r_in 0.50, r_out 0.85), the portal's 0.35 m band on a
    # closed sweep. The receiving masonry is bored to just inside the
    # extrados so the ring fills the hole rather than burying its own back
    # face in solid wall -- the same relation the portal ring has to its
    # 1.53 m bore.
    OCULUS_Z = 5.9
    OCULUS_R_OUT = 0.85
    oculus_wedges, _, _, _ = geo.barrel_shell(f"{name}_oculus", 0.0, (8.0, 8.6), OCULUS_Z,
                                               OCULUS_R_OUT, OCULUS_R_OUT, 0.35, lime,
                                               n_wedges=16, sweep_axis="y",
                                               phi_range=(-math.pi, math.pi))
    objs.extend(oculus_wedges)
    oculus_bore = geo.make_cylinder(f"{name}_oculus_tmp", (east_cx, 0.0, OCULUS_Z),
                                     OCULUS_R_OUT - 0.02, 2.0,
                                     None, segments=24, rotation=Matrix.Rotation(math.pi / 2.0, 3, "Y"))
    mod = east_head.modifiers.new("oculus_bore", type="BOOLEAN")
    mod.operation, mod.solver, mod.object = "DIFFERENCE", "EXACT", oculus_bore
    bpy.context.view_layer.objects.active = east_head
    bpy.ops.object.modifier_apply(modifier=mod.name)
    bpy.data.objects.remove(oculus_bore, do_unlink=True)

    liner_h = springline
    liner_t = 0.06
    # F4b — the saetera opening pierces the plaster liner too, or the hole
    # shows the liner's own back face; liner_hi is built z0=2.0, so its
    # matching sill is the wall opening's sill minus that offset.
    saetera_liner_opening = {"offset": -0.5, "width": 0.5, "height": 1.6, "sill": 2.2}
    liner_top = springline - CHAPEL_RIM_EXPOSED
    for sign in (-1, 1):
        inner_face_y = sign * half_w
        y_liner = inner_face_y - sign * (liner_t / 2.0)
        objs.extend(geo.wall_with_openings(f"{name}_liner_lo_{sign}", (0.0, y_liner),
                                            CHAPEL_NAVE_LENGTH, liner_t, 2.0, "x", lime, openings=[]))
        # The high liner stops CHAPEL_RIM_EXPOSED below whatever masonry
        # stands over it. Behind the collapsed bay that masonry is the
        # broken profile, not the springline, so a full-height sheet there
        # would stand as a 6 cm plaster blade above its own wall.
        objs.extend(geo.wall_with_openings(f"{name}_liner_hi_{sign}", ((bx0 - half_l) / 2.0, y_liner),
                                            bx0 + half_l, liner_t, liner_top - 2.0, "x",
                                            plaster, openings=[saetera_liner_opening], z0=2.0))
        for k, (x0, x1, top) in enumerate(collapse_profile[sign]):
            h = min(liner_top, top - CHAPEL_RIM_EXPOSED) - 2.0
            if h > 0.05:
                objs.append(geo.make_box(f"{name}_liner_hi_{sign}_B{k}",
                                          ((x0 + x1) / 2.0, y_liner, 2.0 + h / 2.0),
                                          (x1 - x0 + 0.01, liner_t, h), plaster))
        objs.extend(geo.wall_with_openings(f"{name}_liner_hi_{sign}_C", ((bx1 + half_l) / 2.0, y_liner),
                                            half_l - bx1, liner_t, liner_top - 2.0, "x",
                                            plaster, openings=[], z0=2.0))
    apse_inner_x = -half_l
    x_liner = apse_inner_x + liner_t / 2.0
    objs.extend(geo.wall_with_openings(f"{name}_liner_lo_apse", (x_liner, 0.0), CHAPEL_NAVE_WIDTH,
                                        liner_t, 2.0, "y", lime, openings=[]))
    objs.extend(geo.wall_with_openings(f"{name}_liner_hi_apse", (x_liner, 0.0), CHAPEL_NAVE_WIDTH,
                                        liner_t, springline - 2.0 - CHAPEL_RIM_EXPOSED, "y", plaster,
                                        openings=[], z0=2.0))

    wedges, r, center_z, theta0 = geo.barrel_shell(f"{name}_vault", 0.0, (-half_l, 0.0), springline,
                                                    half_w, CHAPEL_VAULT_RISE, 0.4, lime,
                                                    n_wedges=18, sweep_axis="y")
    objs.extend(wedges)
    peak_z = center_z + r

    # F5(b) — ragged fracture lip: each wedge's break line (extrude_ends)
    # varies instead of a uniform 0.5 m cut, and n_wedges/jitter now match
    # the vault's own 18 wedges with zero jitter, so the shared x=0 corner
    # is radially identical and the break reads purely as a step in x.
    # Haunch ribs (near the springing) cantilever up to 2.20 m into the
    # void; crown ribs (mid-span) die within ~0.17 m -- the crown falls
    # first and the haunches survive, physically right.
    ends_rng = random.Random(3)
    lip_ends = [max(0.10, min(2.20, (0.15 + 1.55 * abs(2.0 * i / 17.0 - 1.0))
                               * ends_rng.uniform(0.70, 1.30)))
                for i in range(18)]
    lip_wedges, _, _, _ = geo.barrel_shell(f"{name}_lip", 0.0, (0.0, 0.0), springline, half_w,
                                            CHAPEL_VAULT_RISE, 0.4, lime, n_wedges=18,
                                            sweep_axis="y", radial_jitter=0.0, seed=3,
                                            extrude_ends=lip_ends)
    objs.extend(lip_wedges)

    # F5(c) — the aftermath. Pieces are drawn from the vault's own size
    # family (18 wedges on r_out 3.5417 give ~1.24 m arc faces, so the
    # voussoirs that fell are metre-scale, not gravel), yawed and tilted off
    # axis, bedded below the paving rather than resting on it, and spread
    # across the whole collapsed span instead of clustering at its west end.
    rng = random.Random(11)
    bx0, bx1 = CHAPEL_COLLAPSE_X
    i = 0
    for kind, count in (("voussoir", 6), ("spall", 11)):
        for _ in range(count):
            if kind == "voussoir":
                size = (rng.uniform(0.85, 1.30), rng.uniform(0.40, 0.62), rng.uniform(0.34, 0.52))
            else:
                size = (rng.uniform(0.22, 0.55), rng.uniform(0.18, 0.42), rng.uniform(0.14, 0.30))
            rx = rng.uniform(bx0 + 0.3, bx1 - 0.3)
            ry = rng.uniform(-half_w + 0.6, half_w - 0.6)
            rot = (Matrix.Rotation(rng.uniform(0.0, 2.0 * math.pi), 3, "Z")
                   @ Matrix.Rotation(rng.uniform(-0.32, 0.32), 3, "Y")
                   @ Matrix.Rotation(rng.uniform(-0.22, 0.22), 3, "X"))
            bed = rng.uniform(0.25, 0.5) * size[2]
            objs.append(geo.make_box(f"{name}_rubble{i}", (rx, ry, CHAPEL_FLOOR_TOP + size[2] / 2.0 - bed),
                                      size, lime, bevel=0.02, bevel_segments=1, rotation=rot))
            i += 1

    floor_t = 0.15
    objs.append(geo.make_box(f"{name}_floor", (0.0, 0.0, CHAPEL_FLOOR_TOP - floor_t / 2.0),
                              (CHAPEL_NAVE_LENGTH, CHAPEL_NAVE_WIDTH, floor_t), lime))

    # Standing open (docs/town-premise.md S6): each leaf is hinged at its
    # jamb and swung 90 degrees flat against the inner wall face, not lying
    # across the opening -- the chapel is the one building the player must
    # be able to walk through.
    door_leaf_w = CHAPEL_DOOR["width"] / 2.0 * 0.92
    hinge_x = east_cx - t / 2.0
    for sign in (1, -1):
        hinge_y = sign * (CHAPEL_DOOR["width"] / 2.0)
        leaf_center = (hinge_x - door_leaf_w / 2.0, hinge_y - sign * DOOR_THICKNESS / 2.0,
                       CHAPEL_DOOR["height"] / 2.0)
        objs.append(geo.make_box(f"{name}_doorleaf_{sign}", leaf_center,
                                  (door_leaf_w, DOOR_THICKNESS, CHAPEL_DOOR["height"] * 0.96), oak))

    # F1 — espadaña (bell gable): coplanar with the east wall (x in
    # [8.0, 8.6]) -- the load-bearing decision that keeps the measured
    # footprint from moving. Both tronera cuts are DIFFERENCE/EXACT booleans
    # applied in sequence, the same pattern build_gate_arch already uses for
    # its own bore.
    ESPADANA_EAVE_Z = 11.2
    ESPADANA_RIDGE_Z = 12.4
    esp_body = geo.make_box(f"{name}_espadana_body", (east_cx, 0.0, 9.35), (0.6, 3.6, 3.7),
                             lime, bevel=0.02)
    tronera_straight = geo.make_box(f"{name}_espadana_tronera_tmp1", (east_cx, 0.0, 9.05),
                                     (1.0, 1.6, 1.7), None)
    mod = esp_body.modifiers.new("tronera_straight", type="BOOLEAN")
    mod.operation, mod.solver, mod.object = "DIFFERENCE", "EXACT", tronera_straight
    bpy.context.view_layer.objects.active = esp_body
    bpy.ops.object.modifier_apply(modifier=mod.name)
    bpy.data.objects.remove(tronera_straight, do_unlink=True)

    tronera_round = geo.make_cylinder(f"{name}_espadana_tronera_tmp2", (east_cx, 0.0, 9.9), 0.8, 1.0,
                                       None, segments=24, rotation=Matrix.Rotation(math.pi / 2.0, 3, "Y"))
    mod = esp_body.modifiers.new("tronera_round", type="BOOLEAN")
    mod.operation, mod.solver, mod.object = "DIFFERENCE", "EXACT", tronera_round
    bpy.context.view_layer.objects.active = esp_body
    bpy.ops.object.modifier_apply(modifier=mod.name)
    bpy.data.objects.remove(tronera_round, do_unlink=True)
    objs.append(esp_body)

    objs.append(geo.gable_infill(f"{name}_espadana_gable", east_cx, t, 3.6, ESPADANA_EAVE_Z,
                                  ESPADANA_RIDGE_Z, lime))

    # F2 — bell + cross, `iron_wrought` (premise S3 assigns bell and gate
    # fittings to wrought iron; the cross rides the same slot so no fifth
    # material family is bound).
    # The bell is read as a silhouette inside a lit opening, so the profile
    # is the whole cue: a flared waist off a 0.60 m mouth, then a narrow
    # crown under the yoke. The yoke clears the tronera's round head
    # (springing z = 9.9) instead of lying across it, and spans just inside
    # the 0.80 m bore so it beds 1 cm into each jamb.
    objs.append(geo.make_cylinder(f"{name}_bell_mouth", (east_cx, 0.0, 9.50), 0.30, 0.40, iron,
                                   segments=16, radius_top=0.19))
    objs.append(geo.make_cylinder(f"{name}_bell_crown", (east_cx, 0.0, 9.76), 0.11, 0.16, iron,
                                   segments=16, radius_top=0.09))
    objs.append(geo.make_box(f"{name}_bell_yoke", (east_cx, 0.0, 9.83), (0.12, 1.62, 0.10), iron))
    objs.append(geo.make_box(f"{name}_cross_v", (east_cx, 0.0, 12.78), (0.09, 0.10, 0.76), iron))
    objs.append(geo.make_box(f"{name}_cross_h", (east_cx, 0.0, 12.92), (0.09, 0.48, 0.10), iron))

    xs = [v.co.x for o in objs for v in o.data.vertices]
    ys = [v.co.y for o in objs for v in o.data.vertices]
    zs = [v.co.z for o in objs for v in o.data.vertices]

    dims = {
        "nave_width": CHAPEL_NAVE_WIDTH, "nave_length": CHAPEL_NAVE_LENGTH,
        "springline": springline, "vault_peak": peak_z,
        "door_width": CHAPEL_DOOR["width"], "door_height": CHAPEL_DOOR["height"],
        "footprint_x": max(xs) - min(xs), "footprint_y": max(ys) - min(ys),
        "espadana_apex": ESPADANA_RIDGE_Z, "overall_height": max(zs),
    }
    return objs, dims


BUILDERS = {
    "casa_small_a": build_casa_small_a,
    "casa_small_b": build_casa_small_b,
    "casa_two_story": build_casa_two_story,
    "casa_corner": build_casa_corner,
    "wall_segment": build_wall_segment,
    "gate_arch": build_gate_arch,
    "chapel": build_chapel,
    "well_basin": build_well_basin,
    "reja_set": build_reja_set,
}
