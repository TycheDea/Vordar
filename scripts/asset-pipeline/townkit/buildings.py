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

from . import geo


WALL_THICKNESS = 0.45
DOOR_THICKNESS = 0.06
SHUTTER_THICKNESS = 0.04
REVEAL_MIN = 0.15  # docs-required minimum reveal depth at openings

assert WALL_THICKNESS >= REVEAL_MIN


def _quoins(name, corner_xy, height, material, z0=0.0, block=0.55, gap=0.03):
    objs = []
    z = z0
    i = 0
    while z < z0 + height - 1e-6:
        h = min(block, z0 + height - z) - gap
        if h > 0.05:
            objs.append(geo.make_box(f"{name}_q{i}", (corner_xy[0], corner_xy[1], z + h / 2.0),
                                      (block, block, h), material, bevel=0.015))
        z += block
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


def build_casa_shell(name, mats, width, depth, wall_height, pitch_deg,
                      front_openings, side_openings_left=None, side_openings_right=None,
                      wall_thickness=WALL_THICKNESS):
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
            objs.extend(_quoins(f"{name}_quoin_{cx:.1f}_{cy:.1f}", (cx, cy), wall_height, lime))

    deck, tiles, ridge_z = geo.gable_roof(f"{name}_roof", (0.0, 0.0), width, depth, wall_height,
                                          pitch_deg, mats["terracotta_tile"], mats["terracotta_tile"])
    objs.extend(deck)
    objs.extend(tiles)

    # gable-end infill above the eave line on the two facades running along X
    # is already covered by the front/back walls reaching wall_height; the
    # triangular gap under the sloped deck is small at this pitch/width and
    # is left open (unseen from the street-facing camera this kit targets).

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
            objs.extend(_quoins(f"{name}_quoin_{cx:.1f}_{cy:.1f}", (cx, cy), h1 + h2, lime))

    deck, tiles, ridge_z = geo.gable_roof(f"{name}_roof", (0.0, 0.0), width, depth, h1 + h2,
                                          pitch, mats["terracotta_tile"], mats["terracotta_tile"])
    objs.extend(deck)
    objs.extend(tiles)
    return objs, {"width": width, "depth": depth, "wall_height": h1 + h2, "ridge_height": ridge_z}


def build_casa_corner(mats):
    """L-footprint corner casa: a main block plus a perpendicular wing,
    footprints overlapping by one wall thickness at the inner corner so the
    exterior silhouette reads sealed (interior of the L-joint is unmodelled,
    matching the exterior-shell-only scope for casas)."""
    name = "casa_corner"
    main_objs, main_dims = build_casa_shell(
        f"{name}_main", mats, width=6.0, depth=6.0, wall_height=4.0, pitch_deg=28.0,
        front_openings=[{"kind": "door", "offset": 0.0, "width": 1.1, "height": 2.2, "sill": 0.0}],
    )
    wing_objs, wing_dims = build_casa_shell(
        f"{name}_wing", mats, width=4.0, depth=4.5, wall_height=3.8, pitch_deg=28.0,
        front_openings=[{"kind": "window", "offset": 0.0, "width": 0.9, "height": 1.1, "sill": 1.2}],
    )
    dx = 6.0 / 2.0 + 4.0 / 2.0 - WALL_THICKNESS
    dy = 6.0 / 2.0 + 4.5 / 2.0 - WALL_THICKNESS
    for o in wing_objs:
        o.location.x += dx
        o.location.y += dy
    objs = main_objs + wing_objs
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
    objs.append(geo.make_box(f"{name}_spandrel", (0.0, 0.0, (peak_z + wall_top) / 2.0),
                              (wall_length, thickness, wall_top - peak_z), lime, bevel=0.02))
    objs.append(geo.make_box(f"{name}_coping", (0.0, 0.0, wall_top + 0.075),
                              (wall_length + 0.1, thickness + 0.2, 0.15), lime, bevel=0.02))
    return objs, {"wall_length": wall_length, "opening_width": opening_width,
                  "opening_height": springline, "wall_top": wall_top}


def build_well_basin(mats):
    name = "well_basin"
    radius = 1.25
    basin_h = 0.9
    lime = mats["limestone_dressed"]
    oak = mats["oak_dark"]
    objs = [geo.make_octagon_prism(f"{name}_basin", (0.0, 0.0, 0.0), radius, basin_h, lime, bevel=0.03)]
    post_h = 1.8
    post_positions = [(radius * 0.75, 0.0), (-radius * 0.75, 0.0)]
    for i, (px, py) in enumerate(post_positions):
        objs.append(geo.make_box(f"{name}_post{i}", (px, py, basin_h + post_h / 2.0),
                                  (0.16, 0.16, post_h), oak, bevel=0.01))
    beam_len = 2.0 * post_positions[0][0]
    objs.append(geo.make_box(f"{name}_beam", (0.0, 0.0, basin_h + post_h + 0.08),
                              (beam_len + 0.2, 0.16, 0.16), oak, bevel=0.01))
    return objs, {"radius": radius, "basin_height": basin_h, "post_height": post_h}


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


def build_chapel(mats):
    name = "chapel"
    lime = mats["limestone_dressed"]
    plaster = mats["plaster_smoked"]
    oak = mats["oak_dark"]
    t = CHAPEL_WALL_THICKNESS
    half_w = CHAPEL_NAVE_WIDTH / 2.0
    half_l = CHAPEL_NAVE_LENGTH / 2.0
    springline = CHAPEL_SPRINGLINE
    objs = []

    side_wall_len = CHAPEL_NAVE_LENGTH + t
    for sign in (-1, 1):
        cy = sign * (half_w + t / 2.0)
        objs.extend(geo.wall_with_openings(f"{name}_side_{sign}", (0.0, cy), side_wall_len, t,
                                            springline, "x", lime, openings=[]))
    apse_len = CHAPEL_NAVE_WIDTH + t
    objs.extend(geo.wall_with_openings(f"{name}_apse", (-half_l - t / 2.0, 0.0), apse_len, t,
                                        springline, "y", lime,
                                        openings=[]))
    east_cx = half_l + t / 2.0
    east_objs = geo.wall_with_openings(f"{name}_east", (east_cx, 0.0), apse_len, t, springline,
                                        "y", lime, openings=[
                                            {"offset": 0.0, "width": CHAPEL_DOOR["width"],
                                             "height": CHAPEL_DOOR["height"], "sill": 0.0}])
    objs.extend(east_objs)

    liner_h = springline
    liner_t = 0.06
    for sign in (-1, 1):
        inner_y = sign * (half_w + liner_t / 2.0 * 0)  # placed at inner face below
        inner_face_y = sign * half_w
        y_liner = inner_face_y - sign * (liner_t / 2.0)
        objs.extend(geo.wall_with_openings(f"{name}_liner_lo_{sign}", (0.0, y_liner),
                                            CHAPEL_NAVE_LENGTH, liner_t, 2.0, "x", lime, openings=[]))
        objs.extend(geo.wall_with_openings(f"{name}_liner_hi_{sign}", (0.0, y_liner),
                                            CHAPEL_NAVE_LENGTH, liner_t, springline - 2.0, "x",
                                            plaster, openings=[], z0=2.0))
    apse_inner_x = -half_l
    x_liner = apse_inner_x + liner_t / 2.0
    objs.extend(geo.wall_with_openings(f"{name}_liner_lo_apse", (x_liner, 0.0), CHAPEL_NAVE_WIDTH,
                                        liner_t, 2.0, "y", lime, openings=[]))
    objs.extend(geo.wall_with_openings(f"{name}_liner_hi_apse", (x_liner, 0.0), CHAPEL_NAVE_WIDTH,
                                        liner_t, springline - 2.0, "y", plaster, openings=[], z0=2.0))

    wedges, r, center_z, theta0 = geo.barrel_shell(f"{name}_vault", 0.0, (-half_l, 0.0), springline,
                                                    half_w, CHAPEL_VAULT_RISE, 0.4, lime,
                                                    n_wedges=18, sweep_axis="y")
    objs.extend(wedges)
    peak_z = center_z + r

    lip_wedges, _, _, _ = geo.barrel_shell(f"{name}_lip", 0.0, (-0.25, 0.25), springline, half_w,
                                            CHAPEL_VAULT_RISE, 0.4, lime, n_wedges=14,
                                            sweep_axis="y", radial_jitter=0.05, seed=3)
    objs.extend(lip_wedges)

    rng = random.Random(11)
    for i in range(7):
        rx = rng.uniform(0.4, 3.5)
        ry = rng.uniform(-half_w + 0.5, half_w - 0.5)
        size = (rng.uniform(0.25, 0.5), rng.uniform(0.2, 0.4), rng.uniform(0.15, 0.3))
        objs.append(geo.make_box(f"{name}_rubble{i}", (rx, ry, size[2] / 2.0), size, lime,
                                  bevel=0.02, rotation=None))

    floor_t = 0.15
    objs.append(geo.make_box(f"{name}_floor", (0.0, 0.0, -floor_t / 2.0),
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

    dims = {
        "nave_width": CHAPEL_NAVE_WIDTH, "nave_length": CHAPEL_NAVE_LENGTH,
        "springline": springline, "vault_peak": peak_z,
        "door_width": CHAPEL_DOOR["width"], "door_height": CHAPEL_DOOR["height"],
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
