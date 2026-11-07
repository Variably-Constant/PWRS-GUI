"""Writes an ExtendScript file that makes one Photoshop document of stroked
shape layers, one layer a case, from a named batch, saves it as a PSD and
closes it, and a JSON file of the cases as asked. ps_run.ps1 runs the
script in Photoshop; ps_read_back.py checks the saved document against the
cases.

    python -u -I ps_stroke_batch.py <batch> <output folder>

Writes <output folder>/<batch>.jsx and <batch>.cases.json; the script saves
<output folder>/<batch>.psd. The batches are the keys of BATCHES.
"""

import json
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# A cubic's handle length for a quarter circle, as a fraction of the radius.
K = 0.5522847498307936


def corner(x, y):
    return [x, y, x, y, x, y]


def rect(x0, y0, x1, y1):
    return [corner(x0, y0), corner(x1, y0), corner(x1, y1), corner(x0, y1)]


def polygon(points):
    return [corner(x, y) for x, y in points]


def ellipse(cx, cy, rx, ry):
    """Four knots clockwise from the top, each with a quarter ellipse's
    handles: the one before it, then the one after it."""
    kx, ky = K * rx, K * ry
    return [
        [cx, cy - ry, cx - kx, cy - ry, cx + kx, cy - ry],
        [cx + rx, cy, cx + rx, cy - ky, cx + rx, cy + ky],
        [cx, cy + ry, cx + kx, cy + ry, cx - kx, cy + ry],
        [cx - rx, cy, cx - rx, cy + ky, cx - rx, cy - ky],
    ]


def round_rect(x0, y0, x1, y1, r):
    """Eight knots clockwise from the top edge's left end: straight sides and
    kappa quarter corners of radius r, each knot's handles the one before
    it, then the one after it."""
    k = K * r
    return [
        [x0 + r, y0, x0 + r - k, y0, x0 + r, y0],
        [x1 - r, y0, x1 - r, y0, x1 - r + k, y0],
        [x1, y0 + r, x1, y0 + r - k, x1, y0 + r],
        [x1, y1 - r, x1, y1 - r, x1, y1 - r + k],
        [x1 - r, y1, x1 - r + k, y1, x1 - r, y1],
        [x0 + r, y1, x0 + r, y1, x0 + r - k, y1],
        [x0, y1 - r, x0, y1 - r + k, x0, y1 - r],
        [x0, y0 + r, x0, y0 + r, x0, y0 + r - k],
    ]


def stroke(**changes):
    s = {"enabled": True, "fillEnabled": False, "width": 1.0, "align": "center", "join": "miter", "cap": "butt",
         "miterLimit": 100.0, "dashes": [], "dashOffset": 0.0, "opacity": 100.0, "color": [0, 0, 0],
         "strokeAdjust": False}
    unknown = set(changes) - set(s)
    if unknown:
        raise ValueError(f"unknown stroke settings {sorted(unknown)}")
    s.update(changes)
    return s


def case(name, knots, closed=True, fill=(255, 0, 0), **stroke_changes):
    return {"name": name, "fill": list(fill), "subpaths": [{"closed": closed, "op": "add", "knots": knots}],
            "stroke": stroke(**stroke_changes)}


def shapes_case(name, knot_lists, fill=(255, 0, 0), **stroke_changes):
    """One layer of several closed subpaths, each added to the shape."""
    return {"name": name, "fill": list(fill),
            "subpaths": [{"closed": True, "op": "add", "knots": k} for k in knot_lists],
            "stroke": stroke(**stroke_changes)}


def edges():
    """Photoshop's coverage of an edge at every 64th of a pixel: per layer,
    64 rectangles 8 pixels square in an 8 by 8 grid of 16-pixel cells, the
    k-th with one edge k/64 of a pixel past a whole pixel, filled with the
    stroke off, or stroked 1 pixel centered with the fill off."""
    def grid(edge):
        out = []
        for k in range(64):
            x, y, f = (k % 8) * 16 + 4, (k // 8) * 16 + 4, k / 64.0
            x0, y0, x1, y1 = x, y, x + 8, y + 8
            if edge == "left":
                x0 += f
            elif edge == "right":
                x1 += f
            elif edge == "top":
                y0 += f
            else:
                y1 += f
            out.append(rect(x0, y0, x1, y1))
        return out

    cases = []
    for edge in ("left", "right", "top", "bottom"):
        cases.append(shapes_case(f"fill {edge}", grid(edge), enabled=False, fillEnabled=True))
        cases.append(shapes_case(f"stroke {edge}", grid(edge), width=1.0))
    return 128, 128, cases


def thirds(points):
    """A closed polygon whose straight edges are written as cubics with
    handles a third and two thirds along each edge."""
    knots = []
    n = len(points)
    for k, (x, y) in enumerate(points):
        px, py = points[k - 1]
        nx, ny = points[(k + 1) % n]
        knots.append([x, y, x + (px - x) / 3.0, y + (py - y) / 3.0, x + (nx - x) / 3.0, y + (ny - y) / 3.0])
    return knots


def curves():
    """How Photoshop flattens curves, fills only: circles of radius 4 to
    128 on whole-pixel centers and one off them, a wide ellipse, and one
    long slanted triangle twice, its edges written with handles on the
    anchors and with handles at the thirds."""
    off = dict(enabled=False, fillEnabled=True)
    triangle = [(620.0, 690.0), (1080.0, 610.0), (850.0, 340.0)]
    return 1100, 720, [
        case("circle 4", ellipse(20, 20, 4, 4), **off),
        case("circle 8", ellipse(50, 20, 8, 8), **off),
        case("circle 16", ellipse(100, 30, 16, 16), **off),
        case("circle 32", ellipse(180, 50, 32, 32), **off),
        case("circle 64", ellipse(320, 90, 64, 64), **off),
        case("circle 128", ellipse(150, 330, 128, 128), **off),
        case("circle 100 fractional", ellipse(430.37, 330.81, 100.0, 100.0), **off),
        case("ellipse 150 60", ellipse(800, 120, 150, 60), **off),
        case("triangle handles on anchors", polygon(triangle), **off),
        case("triangle handles at thirds", thirds(triangle), **off),
    ]


def vee(x, y, angle, length=30.0):
    """An open path of two equal straight legs meeting at `angle` degrees,
    its vertex at the bottom of the 80-pixel cell at x, y."""
    import math
    half = math.radians(angle) / 2.0
    vx, vy = x + 40.0, y + 62.0
    return polygon([(vx - length * math.sin(half), vy - length * math.cos(half)), (vx, vy),
                    (vx + length * math.sin(half), vy - length * math.cos(half))])


def geometry():
    """Stroke geometry, center 6-pixel strokes with the fill off: open
    two-legged paths at four angles under every join and cap; the sharpest
    under five miter limits; dashed rectangles and ellipses under three
    dash sets, every cap and two dash offsets."""
    cases = []
    cell = 80
    row = 0
    for angle in (150, 90, 45, 20):
        column = 0
        for join in ("miter", "round", "bevel"):
            for cap in ("butt", "round", "square"):
                cases.append(case(f"vee {angle} {join} {cap}", vee(column * cell, row * cell, angle), closed=False,
                                  width=6.0, join=join, cap=cap))
                column += 1
        row += 1
    for column, limit in enumerate((1.0, 2.0, 4.0, 10.0, 100.0)):
        cases.append(case(f"vee 20 miter limit {limit:g}", vee(column * cell, row * cell, 20), closed=False,
                          width=6.0, miterLimit=limit))
    row += 1
    column = 0
    for pattern in ([2.0, 1.0], [4.0, 2.0], [1.0, 1.0]):
        for cap in ("butt", "round", "square"):
            x, y = column * cell, row * cell
            name = f"{pattern[0]:g}-{pattern[1]:g} {cap}"
            cases.append(case(f"dash rect {name}", rect(x + 12, y + 14, x + 68, y + 66), width=3.0, cap=cap, dashes=pattern))
            cases.append(case(f"dash ellipse {name}", ellipse(x + 40, y + cell + 40, 26, 22), width=3.0, cap=cap, dashes=pattern))
            column += 1
    row += 2
    for column, offset in enumerate((1.5, 3.0)):
        x, y = column * cell, row * cell
        cases.append(case(f"dash rect offset {offset:g}", rect(x + 12, y + 14, x + 68, y + 66), width=3.0,
                          dashes=[4.0, 2.0], dashOffset=offset))
    row += 1
    return 9 * cell, row * cell, cases


def flatness():
    """Where Photoshop's chord count per cubic doubles, fills only: circles
    every half pixel of radius from 8 to 16 and every two from 32 to 64 in
    140-pixel cells, and ellipses four times as wide as tall."""
    off = dict(enabled=False, fillEnabled=True)
    radii = [8 + 0.5 * k for k in range(17)] + [32 + 2 * k for k in range(17)]
    cases = []
    for n, r in enumerate(radii):
        cx, cy = (n % 8) * 140 + 70, (n // 8) * 140 + 70
        cases.append(case(f"circle {r:g}", ellipse(cx, cy, r, r), **off))
    for k, ry in enumerate((4, 8, 16, 32)):
        cases.append(case(f"ellipse {4 * ry} {ry}", ellipse(650, 760 + 90 * k, 4 * ry, ry), **off))
    return 1300, 1100, cases


def mix():
    """How the fill and the stroke combine, fill red, stroke blue, both on:
    a fractional rectangle stroked inside, outside and centered 2 pixels and
    inside at 50% opacity, a fractional ellipse inside 3, a triangle centered
    3, a whole-pixel rectangle and a fractional pentagon inside 1."""
    on = dict(fillEnabled=True, color=[0, 0, 255])
    pentagon = [(25.37, 8.12), (42.81, 20.46), (36.23, 41.9), (13.68, 41.2), (7.92, 20.33)]
    cells = [
        ("inside 2 rect", rect(10.3, 10.7, 50.3, 52.7), dict(width=2.0, align="inside")),
        ("outside 2 rect", rect(10.3, 10.7, 50.3, 52.7), dict(width=2.0, align="outside")),
        ("center 2 rect", rect(10.3, 10.7, 50.3, 52.7), dict(width=2.0)),
        ("inside 2 rect opacity 50", rect(10.3, 10.7, 50.3, 52.7), dict(width=2.0, align="inside", opacity=50.0)),
        ("inside 3 ellipse", ellipse(30.37, 31.81, 20.6, 17.3), dict(width=3.0, align="inside")),
        ("center 3 triangle", polygon([(8, 52), (54, 46), (30, 9)]), dict(width=3.0)),
        ("inside 1 rect whole", rect(10, 12, 50, 50), dict(width=1.0, align="inside")),
        ("inside 1 pentagon", polygon(pentagon), dict(width=1.0, align="inside")),
    ]
    cases = []
    for n, (name, knots, changes) in enumerate(cells):
        dx = n * 70
        moved = [[v + (dx if i % 2 == 0 else 0) for i, v in enumerate(k)] for k in knots]
        cases.append(case(name, moved, **on, **changes))
    return 70 * len(cells), 70, cases


def moved():
    """Whether Photoshop's slanted-edge sampling depends on where the shape
    sits: one triangle, filled, at whole-pixel moves of (0, 0), (1, 0),
    (0, 1) and (256, 256)."""
    off = dict(enabled=False, fillEnabled=True)
    base = [(14.0, 48.0), (49.0, 43.0), (29.0, 15.0)]
    cases = []
    for dx, dy in ((0, 0), (1, 0), (0, 1), (256, 256)):
        cases.append(case(f"triangle moved {dx} {dy}", polygon([(x + dx, y + dy) for x, y in base]), **off))
    return 320, 320, cases


def wedges():
    """One slanted edge at a time, filled: trapezoids whose left edge leans
    by 1/8 to 4 pixels across per pixel down, every other edge axis-aligned,
    the left edge starting on a whole pixel or 0.37 past one, each drawn
    clockwise and counterclockwise."""
    off = dict(enabled=False, fillEnabled=True)
    cases = []
    n = 0
    for lean in (0.125, 0.25, 0.5, 1.0, 2.0, 4.0):
        for start in (0.0, 0.37):
            x, y = (n % 6) * 210 + 10, (n // 6) * 60 + 10
            top_left = (x + start, y)
            bottom_left = (x + start + 40 * lean, y + 40)
            right = x + 200
            pts = [top_left, (right, y), (right, y + 40), bottom_left]
            cases.append(case(f"wedge {lean:g} {start:g} cw", polygon(pts), **off))
            cases.append(case(f"wedge {lean:g} {start:g} ccw", polygon(list(reversed(pts))), **off))
            n += 1
    return 1270, 140, cases


def mirrors():
    """The wedges' slanted edge in the other three placings, filled: a left
    edge leaning left, a right edge leaning right, a right edge leaning
    left, each leaning 1, 2 and 4 pixels across per pixel down, other edges
    axis-aligned."""
    off = dict(enabled=False, fillEnabled=True)
    cases = []
    for row, lean in enumerate((1.0, 2.0, 4.0)):
        y = row * 60 + 10
        shift = 40 * lean
        x = 10
        cases.append(case(f"left leaning left {lean:g}", polygon([(x + shift, y), (x + 200, y), (x + 200, y + 40), (x, y + 40)]), **off))
        x = 230
        cases.append(case(f"right leaning right {lean:g}", polygon([(x, y), (x + 20, y), (x + 20 + shift, y + 40), (x, y + 40)]), **off))
        x = 450
        cases.append(case(f"right leaning left {lean:g}", polygon([(x, y), (x + 20 + shift, y), (x + 20, y + 40), (x, y + 40)]), **off))
    return 680, 190, cases


def depth():
    """How the slanted-edge shift changes down an edge: 100-pixel-tall
    trapezoids, filled, whose left edge leans 2 pixels per pixel to the left
    or to the right, each started 0 to 0.03 pixels across in steps of
    0.005, other edges axis-aligned, one per 120-pixel band."""
    off = dict(enabled=False, fillEnabled=True)
    cases = []
    band = 0
    for lean_name, lean in (("left", -2.0), ("right", 2.0)):
        for k in range(7):
            start = 0.005 * k
            y = band * 120 + 10
            top_x = 10 + start + (200 if lean < 0 else 0)
            bottom_x = top_x + 100 * lean
            pts = [(top_x, y), (430 + start, y), (430 + start, y + 100), (bottom_x, y + 100)]
            cases.append(case(f"depth {lean_name} {start:g}", polygon(pts), **off))
            band += 1
    return 450, band * 120, cases


def scan(lean, steps=120, height=48, columns=10):
    """Where Photoshop puts each sample row's crossing on a slanted edge:
    one trapezoid a layer, filled, its left edge leaning `lean` pixels
    across per pixel down over `height` pixels, other edges axis-aligned,
    the whole shape moved right by m / (17 * steps) pixels for m from 0 to
    steps - 1, so every crossing passes exactly one sample over the batch."""
    off = dict(enabled=False, fillEnabled=True)
    run = abs(lean) * height
    cell_w, cell_h = int(run) + 40, height + 8
    cases = []
    for m in range(steps):
        dx = m / (17.0 * steps)
        x, y = (m % columns) * cell_w + 10, (m // columns) * cell_h + 4
        top, bottom = (x + dx, x + dx + run) if lean > 0 else (x + dx + run, x + dx)
        right = x + run + 20 + dx
        pts = [(top, y), (right, y), (right, y + height), (bottom, y + height)]
        cases.append(case(f"scan {lean:g} {m}", polygon(pts), **off))
    rows = (steps + columns - 1) // columns
    return columns * cell_w + 10, rows * cell_h + 8, cases


def random_edges(count=160, columns=10, seed=20261030):
    """One slanted edge at a time on fractional geometry, filled:
    trapezoids whose top and bottom sit at random fractions of a pixel, the
    slanted left edge starting at a random fraction across and leaning
    0.1 to 2.5 pixels per pixel either way, the right edge vertical on a
    whole pixel."""
    import random

    rng = random.Random(seed)
    off = dict(enabled=False, fillEnabled=True)
    cell_w, cell_h = 100, 44
    cases = []
    for n in range(count):
        x, y = (n % columns) * cell_w + 4, (n // columns) * cell_h + 4
        top = y + 2 + rng.random()
        bottom = y + 32 + rng.random()
        lean = rng.choice((-1, 1)) * rng.uniform(0.1, 2.5)
        run = lean * (bottom - top)
        start = x + 4 + rng.random() + (abs(run) if lean < 0 else 0.0)
        right = x + 96
        pts = [(start, top), (right, top), (right, bottom), (start + run, bottom)]
        cases.append(case(f"random {n}", polygon(pts), **off))
    rows = (count + columns - 1) // columns
    return columns * cell_w + 8, rows * cell_h + 8, cases


def split_edges():
    """Which part of fractional geometry the slanted-edge rule misses,
    filled trapezoids 40 pixels tall with the left edge slanted:
    "ytop" ones lean +-1 or +-2 with a whole-pixel run, both ends moved down
    by a fraction of a pixel; "run" ones start and end on whole pixel rows
    with runs a fraction of a sample longer than whole."""
    off = dict(enabled=False, fillEnabled=True)
    cases = []
    n = 0
    columns, cell_w, cell_h = 8, 140, 50
    for lean in (1.0, -1.0, 2.0, -2.0):
        for frac in (1 / 64, 1 / 8, 0.25, 1 / 3, 0.5, 2 / 3, 0.75, 0.875, 0.97):
            x, y = (n % columns) * cell_w + 4, (n // columns) * cell_h + 4
            run = 40 * lean
            top_x = x + 4 + (abs(run) if lean < 0 else 0.0)
            pts = [(top_x, y + frac), (x + 136, y + frac), (x + 136, y + 40 + frac), (top_x + run, y + 40 + frac)]
            cases.append(case(f"ytop {lean:g} {frac:.4f}", polygon(pts), **off))
            n += 1
    for lean in (1.0, -1.0, 2.0, -2.0):
        for extra in (1 / 17, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.5):
            x, y = (n % columns) * cell_w + 4, (n // columns) * cell_h + 4
            run = 40 * lean + (extra if lean > 0 else -extra) / 17.0
            top_x = x + 4 + (abs(run) if lean < 0 else 0.0)
            pts = [(top_x, y), (x + 136, y), (x + 136, y + 40), (top_x + run, y + 40)]
            cases.append(case(f"run {lean:g} {extra:.4f}", polygon(pts), **off))
            n += 1
    rows = (n + columns - 1) // columns
    return columns * cell_w + 8, rows * cell_h + 8, cases


def circle_scan(radius, steps=120, columns=10):
    """Where Photoshop puts each sample row's crossing of a curve: one
    circle a layer, filled, on a whole-pixel center moved right by
    m / (17 * steps) pixels for m from 0 to steps - 1."""
    off = dict(enabled=False, fillEnabled=True)
    cell = int(2 * radius) + 16
    cases = []
    for m in range(steps):
        dx = m / (17.0 * steps)
        cx, cy = (m % columns) * cell + 8 + radius, (m // columns) * cell + 8 + radius
        cases.append(case(f"circle scan {m}", ellipse(cx + dx, cy, radius, radius), **off))
    rows = (steps + columns - 1) // columns
    return columns * cell + 8, rows * cell + 8, cases


def bend_scan(bow, steps=120, columns=10, step_x=None, step_y=0.0):
    """Where Photoshop puts the chords of a gently bent edge: trapezoids,
    filled, whose left side is one cubic from (x, y) to (x + 24, y + 96)
    with both handles at the thirds pushed `bow` pixels out from the chord,
    other sides straight, the whole shape moved right by m * step_x pixels
    (1 / (17 * steps) unless given) and down by m * step_y pixels for m from
    0 to steps - 1."""
    import math

    off = dict(enabled=False, fillEnabled=True)
    run, drop = 24.0, 96.0
    length = math.hypot(run, drop)
    nx, ny = -drop / length * bow, run / length * bow
    cell_w, cell_h = 80, 104
    cases = []
    for m in range(steps):
        dx = m * (step_x if step_x is not None else 1.0 / (17.0 * steps))
        x, y = (m % columns) * cell_w + 8 + dx, (m // columns) * cell_h + 4 + m * step_y
        p0, p3 = (x, y), (x + run, y + drop)
        c0 = (x + run / 3 + nx, y + drop / 3 + ny)
        c1 = (x + 2 * run / 3 + nx, y + 2 * drop / 3 + ny)
        knots = [
            [p0[0], p0[1], p0[0], p0[1], c0[0], c0[1]],
            [p3[0], p3[1], c1[0], c1[1], p3[0], p3[1]],
            corner(x + 60, y + drop),
            corner(x + 60, y),
        ]
        cases.append(case(f"bend scan {m}", knots, **off))
    rows = (steps + columns - 1) // columns
    return columns * cell_w + 16, rows * cell_h + 8, cases


def bend_grid(bows, offsets=16, columns=16):
    """Where the 2-to-4 chord boundary of a gently bent edge sits: bend_scan's
    trapezoid for each bow in `bows` (pixels) at x moved by k / offsets of a
    sample for k from 0 to offsets - 1, one layer each, named
    "bend <bow> <k>"."""
    import math

    off = dict(enabled=False, fillEnabled=True)
    run, drop = 24.0, 96.0
    length = math.hypot(run, drop)
    cell_w, cell_h = 80, 104
    cases = []
    n = 0
    for bow in bows:
        nx, ny = -drop / length * bow, run / length * bow
        for k in range(offsets):
            dx = k / (17.0 * offsets)
            x, y = (n % columns) * cell_w + 8 + dx, (n // columns) * cell_h + 4
            p0, p3 = (x, y), (x + run, y + drop)
            c0 = (x + run / 3 + nx, y + drop / 3 + ny)
            c1 = (x + 2 * run / 3 + nx, y + 2 * drop / 3 + ny)
            knots = [
                [p0[0], p0[1], p0[0], p0[1], c0[0], c0[1]],
                [p3[0], p3[1], c1[0], c1[1], p3[0], p3[1]],
                corner(x + 60, y + drop),
                corner(x + 60, y),
            ]
            cases.append(case(f"bend {bow:.3f} {k}", knots, **off))
            n += 1
    rows = (n + columns - 1) // columns
    return columns * cell_w + 16, rows * cell_h + 8, cases


def stroke_scan(radius, width, align, steps=120, columns=10):
    """Where Photoshop puts each sample row's crossing of a stroke's offset
    side: one circle a layer, stroked with the fill off, on a whole-pixel
    center moved right by m / (17 * steps) pixels for m from 0 to steps - 1,
    named "stroke scan <m>"."""
    cell = int(2 * (radius + width)) + 16
    cases = []
    for m in range(steps):
        dx = m / (17.0 * steps)
        cx, cy = (m % columns) * cell + cell // 2, (m // columns) * cell + cell // 2
        cases.append(case(f"stroke scan {m}", ellipse(cx + dx, cy, radius, radius), width=width, align=align))
    rows = (steps + columns - 1) // columns
    return columns * cell, rows * cell, cases


def dash_scan(radius, width, align, dashes, steps=120, columns=10, offset=0.0):
    """Where Photoshop puts each sample row's crossing of a dashed stroke's
    sides: one circle of four kappa quarters a layer, stroked with the
    pattern `dashes` (in widths) begun `offset` pixels in, with the fill
    off, on a whole-pixel center moved right by m / (17 * steps) pixels for
    m from 0 to steps - 1, named "dash scan <m>"."""
    cell = int(2 * (radius + width)) + 16
    cases = []
    for m in range(steps):
        dx = m / (17.0 * steps)
        cx, cy = (m % columns) * cell + cell // 2, (m // columns) * cell + cell // 2
        cases.append(case(f"dash scan {m}", circle_arcs(cx + dx, cy, radius, 4), width=width, align=align,
                          dashes=list(dashes), dashOffset=offset))
    rows = (steps + columns - 1) // columns
    return columns * cell, rows * cell, cases


def fill_arcs_scan(radius, sweeps, steps=120, columns=10):
    """Where Photoshop puts each sample row's crossing of a filled circle of
    circular-arc cubics with the given sweeps (degrees, clockwise from the
    top), one a layer on a whole-pixel center moved right by
    m / (17 * steps) pixels for m from 0 to steps - 1, stroke off, named
    "fill scan <m>"."""
    cell = int(2 * radius) + 16
    cases = []
    for m in range(steps):
        dx = m / (17.0 * steps)
        cx, cy = (m % columns) * cell + cell // 2, (m // columns) * cell + cell // 2
        cases.append({"name": f"fill scan {m}", "fill": [255, 0, 0],
                      "subpaths": [{"closed": True, "op": "add",
                                    "knots": circle_mixed_arcs(cx + dx, cy, radius, sweeps)}],
                      "stroke": stroke(enabled=False, fillEnabled=True)})
    rows = (steps + columns - 1) // columns
    return columns * cell, rows * cell, cases


def dash_position_scan(radius=64.0, width=8.0, align="inside", length=16.0, step=4.0, columns=10):
    """Where along a circle a dash's path edge is flattened coarsely or
    finely: one circle of four kappa quarters a layer, stroked with a
    single dash `length` pixels long (the gap longer than the circle), the
    dash offset set so the dash starts s pixels along the stroke's own
    start (the lowest point for inside and outside), s from 0 in `step`
    pixel steps to the circumference less the dash, fill off. Named
    "dashpos s<s>"."""
    import math

    around = 2.0 * math.pi * radius
    gap = 50.0 * width
    cell = int(2 * (radius + width)) + 16
    cases = []
    n = 0
    s = 0.0
    while s + length <= around:
        cx, cy = (n % columns) * cell + cell // 2, (n // columns) * cell + cell // 2
        offset = length + gap - s if s > 0 else 0.0
        cases.append(case(f"dashpos s{s:g}", circle_arcs(cx, cy, radius, 4), width=width, align=align,
                          dashes=[length / width, gap / width], dashOffset=offset))
        n += 1
        s += step
    rows = (n + columns - 1) // columns
    return columns * cell, rows * cell, cases


def dash_width_alignment_scan(radius=64.0, length=16.0, starts=(56.0, 128.0, 160.0, 4.0), columns=8):
    """Whether a lone dash's path edge depends on the stroke: for each dash
    start s (pixels along the stroke from its start, the lowest point for
    inside and outside), a circle of four kappa quarters with a single dash
    `length` pixels long, stroked inside at widths 2, 4, 8 and 16 and
    outside and center at width 8, fill off. Named
    "dashwa s<s> <alignment> w<width>"."""
    import math

    around = 2.0 * math.pi * radius
    settings = [("inside", 2.0), ("inside", 4.0), ("inside", 8.0), ("inside", 16.0), ("outside", 8.0),
                ("center", 8.0)]
    cell = int(2 * (radius + 16.0)) + 16
    cases = []
    n = 0
    for s in starts:
        for align, width in settings:
            gap = around + length
            cx, cy = (n % columns) * cell + cell // 2, (n // columns) * cell + cell // 2
            offset = length + gap - s if s > 0 else 0.0
            cases.append(case(f"dashwa s{s:g} {align} w{width:g}", circle_arcs(cx, cy, radius, 4), width=width,
                              align=align, dashes=[length / width, gap / width], dashOffset=offset))
            n += 1
    rows = (n + columns - 1) // columns
    return columns * cell, rows * cell, cases


def stroke_lines():
    """Whether Photoshop's stroker cuts curve segments into pieces: open
    straight segments at four slopes, written with the handles on the
    anchors ("line") and at the thirds ("cubic"), center 4 with butt caps;
    and a triangle both ways under each alignment at width 3; fill off."""
    cell = 80
    cases = []
    runs = [(40.0, 30.0), (30.0, 45.0), (45.0, 13.0), (12.0, 50.0)]
    for col, (dx, dy) in enumerate(runs):
        x, y = col * cell + 16 + 0.3, 12 + 0.45
        a, b = (x, y), (x + dx, y + dy)
        line = [[a[0], a[1], a[0], a[1], a[0], a[1]], [b[0], b[1], b[0], b[1], b[0], b[1]]]
        third = [[a[0], a[1], a[0], a[1], a[0] + dx / 3, a[1] + dy / 3],
                 [b[0], b[1], b[0] - dx / 3, b[1] - dy / 3, b[0], b[1]]]
        cases.append(case(f"line {col}", line, closed=False, width=4.0, cap="butt"))
        cases.append(case(f"cubic {col}", third, closed=False, width=4.0, cap="butt"))
    triangle = [(14.0, 48.0), (49.0, 43.0), (29.0, 15.0)]
    row = 1
    for col, align in enumerate(("inside", "center", "outside")):
        pts = [(col * cell + 8 + px + 0.3, row * cell + 8 + py + 0.45) for px, py in triangle]
        cases.append(case(f"triangle line {align}", polygon(pts), width=3.0, align=align))
        cases.append(case(f"triangle cubic {align}", thirds(pts), width=3.0, align=align))
    return 4 * cell, 2 * cell, cases


def stroke_arcs():
    """How Photoshop strokes open curves: center strokes with butt caps on a
    quarter circle (one cubic) and on a half circle (two), radius 32, at
    widths 1, 2, 4 and 8, the fill off, named "<quarter|half> <width>"."""
    cell = 96
    k = K * 32.0
    cases = []
    for col, width in enumerate([1.0, 2.0, 4.0, 8.0]):
        cx, cy = col * cell + 24, 48
        quarter = [[cx, cy - 32, cx, cy - 32, cx + k, cy - 32],
                   [cx + 32, cy, cx + 32, cy - k, cx + 32, cy]]
        cases.append(case(f"quarter {width:g}", quarter, closed=False, width=width, cap="butt"))
        cy2 = cell + 48
        half = [[cx, cy2 - 32, cx, cy2 - 32, cx + k, cy2 - 32],
                [cx + 32, cy2, cx + 32, cy2 - k, cx + 32, cy2 + k],
                [cx, cy2 + 32, cx + k, cy2 + 32, cx, cy2 + 32]]
        cases.append(case(f"half {width:g}", half, closed=False, width=width, cap="butt"))
    return 4 * cell, 2 * cell, cases


def stroke_arc_radii(radii=(8.0, 16.0, 32.0, 64.0, 128.0)):
    """How Photoshop's stroke on a curve depends on curvature: open quarter
    circles (one cubic, from the top round to the right) of each radius at
    center widths 2 and 6, butt caps, fill off, named
    "quarter r<radius> <width> at <cx> <cy>" so its sides can be measured
    against exact circles."""
    cases = []
    y = 8
    for radius in radii:
        x = 8
        for width in (2.0, 6.0):
            cx, cy = x + 8, y + 8 + radius
            k = K * radius
            quarter = [[cx, cy - radius, cx, cy - radius, cx + k, cy - radius],
                       [cx + radius, cy, cx + radius, cy - k, cx + radius, cy]]
            cases.append(case(f"quarter r{radius:g} {width:g} at {cx:g} {cy:g}", quarter, closed=False,
                              width=width, cap="butt"))
            x += int(radius) + 32
        y += int(radius) + 24
    return 2 * (int(max(radii)) + 32) + 16, y + 8, cases


def arc_cubic(cx, cy, radius, start, sweep):
    """One cubic for the circular arc about cx, cy from `start` degrees
    (0 is the top, growing clockwise on screen) through `sweep` degrees
    (negative runs counterclockwise), handles (4/3) tan(sweep/4) r: its two
    knots."""
    a0, a1 = math.radians(start), math.radians(start + sweep)
    p0 = (cx + radius * math.sin(a0), cy - radius * math.cos(a0))
    p3 = (cx + radius * math.sin(a1), cy - radius * math.cos(a1))
    h = 4.0 / 3.0 * math.tan(math.radians(sweep) / 4.0) * radius
    p1 = (p0[0] + h * math.cos(a0), p0[1] + h * math.sin(a0))
    p2 = (p3[0] - h * math.cos(a1), p3[1] - h * math.sin(a1))
    return [[p0[0], p0[1], p0[0], p0[1], p1[0], p1[1]],
            [p3[0], p3[1], p2[0], p2[1], p3[0], p3[1]]]


def stroke_arc_angles():
    """Open single-cubic circular arcs, center stroke 2, butt caps, fill
    off: at radius 128, sweeps of 30, 45, 60, 90, 120, 150 and 180 degrees
    from the top clockwise, the quarter run counterclockwise from the right
    end, and the quarter turned to start at 90, 180 and 270 degrees, named
    "arc <start> <sweep> r128 2 at <cx> <cy>"."""
    cell = 300
    specs = [(0, s) for s in (30, 45, 60, 90, 120, 150, 180)] + [(90, -90), (90, 90), (180, 90), (270, 90)]
    cases = []
    for i, (start, sweep) in enumerate(specs):
        cx, cy = (i % 4) * cell + cell // 2, (i // 4) * cell + cell // 2
        cases.append(case(f"arc {start} {sweep} r128 2 at {cx} {cy}", arc_cubic(cx, cy, 128.0, start, sweep),
                          closed=False, width=2.0, cap="butt"))
    return 4 * cell, 3 * cell, cases


def stroke_arc_mix():
    """Open single cubics, center stroke 2, butt caps, fill off, each in a
    300-pixel cell about its center: circular arcs of radius 128 from the top
    clockwise through 65-85 and 95-115 degrees ("arc 0 <sweep> r128 2 at
    <cx> <cy>"); quarter arcs of radius 128 whose handles are the given
    shares of the radius instead of kappa ("quarter r128 2 h<h1>,<h2> at
    <cx> <cy>"); quarter ellipses from the top clockwise with kappa handles
    per axis ("ellipse <rx> <ry> 2 at <cx> <cy>")."""
    cell = 300
    cases = []
    slots = iter(range(100))

    def place():
        i = next(slots)
        return (i % 5) * cell + cell // 2, (i // 5) * cell + cell // 2

    for sweep in (65, 70, 75, 80, 85, 95, 100, 105, 110, 115):
        cx, cy = place()
        cases.append(case(f"arc 0 {sweep} r128 2 at {cx} {cy}", arc_cubic(cx, cy, 128.0, 0, sweep),
                          closed=False, width=2.0, cap="butt"))
    for h1, h2 in ((0.45, 0.45), (0.5, 0.5), (0.6, 0.6), (0.65, 0.65), (0.5, 0.6)):
        cx, cy = place()
        r = 128.0
        knots = [[cx, cy - r, cx, cy - r, cx + h1 * r, cy - r],
                 [cx + r, cy, cx + r, cy - h2 * r, cx + r, cy]]
        cases.append(case(f"quarter r128 2 h{h1:g},{h2:g} at {cx} {cy}", knots, closed=False, width=2.0, cap="butt"))
    for rx, ry in ((128.0, 64.0), (64.0, 128.0)):
        cx, cy = place()
        knots = [[cx, cy - ry, cx, cy - ry, cx + K * rx, cy - ry],
                 [cx + rx, cy, cx + rx, cy - K * ry, cx + rx, cy]]
        cases.append(case(f"ellipse {rx:g} {ry:g} 2 at {cx} {cy}", knots, closed=False, width=2.0, cap="butt"))
    rows = (next(slots) + 4) // 5
    return 5 * cell, rows * cell, cases


def stroke_handles():
    """Open quarter cubics from the top round to the right, center strokes
    of widths 2, 6 and 12, butt caps, fill off, at radius 64 and 128 with
    both handles the given share of the radius, named
    "quarter r<r> <width> h<h>,<h> at <cx> <cy>"."""
    cell = 300
    cases = []
    i = 0
    for r in (64.0, 128.0):
        for h in (0.45, 0.5, 0.6, 0.65):
            for width in (2.0, 6.0, 12.0):
                cx, cy = (i % 6) * cell + 80, (i // 6) * cell + 220
                knots = [[cx, cy - r, cx, cy - r, cx + h * r, cy - r],
                         [cx + r, cy, cx + r, cy - h * r, cx + r, cy]]
                cases.append(case(f"quarter r{r:g} {width:g} h{h:g},{h:g} at {cx} {cy}", knots, closed=False,
                                  width=width, cap="butt"))
                i += 1
    return 6 * cell, ((i + 5) // 6) * cell, cases


def stroke_arc_skew():
    """Open single cubics on circular arcs of radius 128 from the top
    clockwise through 45, 60, 75, 90 and 105 degrees with both handles 0.8
    or 1.2 times the circle's (4/3) tan(sweep/4) r, center stroke 2, butt
    caps, fill off, named "arc 0 <sweep> r128 2 x<factor> at <cx> <cy>"."""
    cell = 300
    cases = []
    i = 0
    for factor in (0.8, 1.2):
        for sweep in (45, 60, 75, 90, 105):
            cx, cy = (i % 5) * cell + cell // 2, (i // 5) * cell + cell // 2
            knots = arc_cubic(cx, cy, 128.0, 0, sweep)
            for knot, sign in ((knots[0], 1), (knots[1], -1)):
                # Scale the knot's handle about its anchor.
                j = 4 if sign == 1 else 2
                knot[j] = knot[0] + (knot[j] - knot[0]) * factor
                knot[j + 1] = knot[1] + (knot[j + 1] - knot[1]) * factor
            cases.append(case(f"arc 0 {sweep} r128 2 x{factor:g} at {cx} {cy}", knots, closed=False,
                              width=2.0, cap="butt"))
            i += 1
    return 5 * cell, 2 * cell, cases


def stroke_circle_radii():
    """Circles (four kappa quarters) of radius 16, 64 and 128 on whole-pixel
    centers, inside and outside strokes of widths 2, 4 and 8, fill off,
    named "<alignment> <width> circle r<radius> at <cx> <cy>"."""
    cases = []
    y = 8
    for radius in (16, 64, 128):
        cell = 2 * radius + 32
        x = 0
        for align in ("inside", "outside"):
            for width in (2.0, 4.0, 8.0):
                cx, cy = x + cell // 2, y + cell // 2
                cases.append(case(f"{align} {width:g} circle r{radius} at {cx} {cy}",
                                  ellipse(cx, cy, radius, radius), width=width, align=align))
                x += cell
        y += cell
    return 6 * (2 * 128 + 32), y + 8, cases


def circle_arcs(cx, cy, radius, n):
    """A circle as n equal circular-arc cubics clockwise from the top, each
    knot's handles (4/3) tan(sweep/4) r along the tangent: the one before
    it, then the one after it."""
    h = 4.0 / 3.0 * math.tan(math.radians(360.0 / n) / 4.0) * radius
    knots = []
    for k in range(n):
        a = math.radians(360.0 * k / n)
        x, y = cx + radius * math.sin(a), cy - radius * math.cos(a)
        tx, ty = math.cos(a), math.sin(a)
        knots.append([x, y, x - h * tx, y - h * ty, x + h * tx, y + h * ty])
    return knots


def stroke_circle_arcs():
    """Circles of radius 64 and 128 on whole-pixel centers built from 3, 4,
    6, 8 and 12 equal arc cubics, inside and outside strokes of widths 2, 4
    and 8, fill off, one row per radius and arc count, named
    "<alignment> <width> circle r<radius> n<n> at <cx> <cy>"."""
    cases = []
    y = 0
    for radius in (64, 128):
        cell = 2 * radius + 40
        for n in (3, 4, 6, 8, 12):
            x = 0
            for align in ("inside", "outside"):
                for width in (2.0, 4.0, 8.0):
                    cx, cy = x + cell // 2, y + cell // 2
                    cases.append(case(f"{align} {width:g} circle r{radius} n{n} at {cx} {cy}",
                                      circle_arcs(cx, cy, radius, n), width=width, align=align))
                    x += cell
            y += cell
    return 6 * (2 * 128 + 40), y, cases


def circle_mixed_arcs(cx, cy, radius, sweeps):
    """A circle as circular-arc cubics of the given sweeps (degrees, summing
    to 360) clockwise from the top, each arc's handles (4/3) tan(sweep/4) r
    along the tangent: per knot the handle before it (the arc ending there),
    then the one after it (the arc starting there)."""
    knots = []
    start = 0.0
    for k, sweep in enumerate(sweeps):
        before = sweeps[k - 1]
        a = math.radians(start)
        x, y = cx + radius * math.sin(a), cy - radius * math.cos(a)
        tx, ty = math.cos(a), math.sin(a)
        h_in = 4.0 / 3.0 * math.tan(math.radians(before) / 4.0) * radius
        h_out = 4.0 / 3.0 * math.tan(math.radians(sweep) / 4.0) * radius
        knots.append([x, y, x - h_in * tx, y - h_in * ty, x + h_out * tx, y + h_out * ty])
        start += sweep
    return knots


SWEEP_SETS = {
    "a": (20, 25, 35, 40, 50, 55, 65, 70),
    "b": (15, 22.5, 30, 37.5, 52.5, 60, 67.5, 75),
    "c": (10, 12.5, 17.5, 27.5, 32.5, 42.5, 47.5, 57.5, 62.5, 50),
    "d": (45, 45, 90, 90, 30, 60),
    "e": (80, 85, 95, 100),
    "f": (33.7, 56.3, 33.7, 56.3, 33.7, 56.3, 90),
}


def stroke_circle_sweeps():
    """Circles of radius 256 on whole-pixel centers, each built from
    circular-arc cubics of one of SWEEP_SETS' sweeps, inside and outside
    strokes of width 8, fill off, named
    "<alignment> 8 circle r256 sweeps <set> at <cx> <cy>"."""
    cases = []
    cell = 2 * 256 + 40
    for row, align in enumerate(("inside", "outside")):
        for col, name in enumerate(SWEEP_SETS):
            cx, cy = col * cell + cell // 2, row * cell + cell // 2
            cases.append(case(f"{align} 8 circle r256 sweeps {name} at {cx} {cy}",
                              circle_mixed_arcs(cx, cy, 256, SWEEP_SETS[name]), width=8.0, align=align))
    return len(SWEEP_SETS) * cell, 2 * cell, cases


SWEEP_SETS_FINE = {
    "g": (10, 12.5, 15, 17.5, 20, 22.5, 25, 27.5, 30, 32.5, 57.5, 90),
    "h": (35, 37.5, 40, 42.5, 47.5, 50, 52.5, 55),
    "i": (57.5, 62.5, 65, 67.5, 72.5, 35),
    "j": (70, 75, 77.5, 82.5, 55),
    "k": (80, 85, 87.5, 92.5, 15),
    "l": (95, 97.5, 100, 67.5),
}


def stroke_circle_sweeps_fine():
    """Circles of radius 128 on whole-pixel centers, each built from
    circular-arc cubics of one of SWEEP_SETS_FINE's sweeps, inside and
    outside strokes of width 8, fill off, named
    "<alignment> 8 circle r128 sweeps <set> at <cx> <cy>"."""
    cases = []
    cell = 2 * 128 + 40
    for row, align in enumerate(("inside", "outside")):
        for col, name in enumerate(SWEEP_SETS_FINE):
            cx, cy = col * cell + cell // 2, row * cell + cell // 2
            cases.append(case(f"{align} 8 circle r128 sweeps {name} at {cx} {cy}",
                              circle_mixed_arcs(cx, cy, 128, SWEEP_SETS_FINE[name]), width=8.0, align=align))
    return len(SWEEP_SETS_FINE) * cell, 2 * cell, cases


def stroke_circle_sweeps_scan(low=20.0, high=100.0, step=0.25):
    """Circles of radius 128 on whole-pixel centers that between them hold
    one circular-arc cubic of every sweep from `low` to `high` degrees in
    `step` steps, each circle the next sweeps in order while they fit, closed
    by one more arc of what is left of 360 degrees (at least 20); inside
    strokes of width 8, fill off, named
    "inside 8 circle r128 scan <index> at <cx> <cy>"."""
    sweeps = [low + step * i for i in range(int(round((high - low) / step)) + 1)]
    circles = []
    current = []
    for sweep in sweeps:
        if sum(current) + sweep > 340.0:
            circles.append(current + [360.0 - sum(current)])
            current = []
        current.append(sweep)
    if current:
        rest = 360.0 - sum(current)
        while rest > 100.0:
            current.append(90.0)
            rest -= 90.0
        circles.append(current + [rest])
    cases = []
    cell = 2 * 128 + 40
    columns = 8
    for index, arcs in enumerate(circles):
        cx, cy = (index % columns) * cell + cell // 2, (index // columns) * cell + cell // 2
        cases.append(case(f"inside 8 circle r128 scan {index} at {cx} {cy}", circle_mixed_arcs(cx, cy, 128, arcs),
                          width=8.0, align="inside"))
    return columns * cell, ((len(circles) + columns - 1) // columns) * cell, cases


def fill_wide_arcs():
    """Fill-only circles on whole-pixel centers built from circular-arc
    cubics turning more than 90 degrees: 2 and 3 equal arcs, and mixed
    sweeps 100/110/150 and 95/125/140, at radius 64 and 128, and the 4-arc
    circle for reference, named "fill circle r<radius> <sweeps> at <cx> <cy>"."""
    sets = [(180, 180), (120, 120, 120), (100, 110, 150), (95, 125, 140), (90, 90, 90, 90)]
    cases = []
    y = 0
    for radius in (64, 128):
        cell = 2 * radius + 40
        for col, sweeps in enumerate(sets):
            cx, cy = col * cell + cell // 2, y + cell // 2
            label = "-".join(f"{s:g}" for s in sweeps)
            knots = circle_mixed_arcs(cx, cy, radius, sweeps)
            cases.append({"name": f"fill circle r{radius} {label} at {cx} {cy}", "fill": [255, 0, 0],
                          "subpaths": [{"closed": True, "op": "add", "knots": knots}],
                          "stroke": stroke(enabled=False, fillEnabled=True)})
        y += cell
    return len(sets) * (2 * 128 + 40), y, cases


def fill_big_circles():
    """Fill-only circles on whole-pixel centers: four kappa quarters at
    radius 160, 192, 224, 256, 320, 384 and 512, and two 180-degree arcs at
    radius 64, 96, 128 and 160, one row each, named
    "fill circle r<radius> <sweeps> at <cx> <cy>"."""
    cases = []
    y = 0
    for radius in (160, 192, 224, 256, 320, 384, 512):
        cell = 2 * radius + 40
        cx, cy = cell // 2, y + cell // 2
        cases.append({"name": f"fill circle r{radius} 90-90-90-90 at {cx} {cy}", "fill": [255, 0, 0],
                      "subpaths": [{"closed": True, "op": "add", "knots": circle_mixed_arcs(cx, cy, radius,
                                                                                          (90, 90, 90, 90))}],
                      "stroke": stroke(enabled=False, fillEnabled=True)})
        y += cell
    x = 2 * 512 + 40
    y2 = 0
    for radius in (64, 96, 128, 160):
        cell = 2 * radius + 40
        cx, cy = x + cell // 2, y2 + cell // 2
        cases.append({"name": f"fill circle r{radius} 180-180 at {cx} {cy}", "fill": [255, 0, 0],
                      "subpaths": [{"closed": True, "op": "add", "knots": circle_mixed_arcs(cx, cy, radius,
                                                                                          (180, 180))}],
                      "stroke": stroke(enabled=False, fillEnabled=True)})
        y2 += cell
    return x + 2 * 160 + 40, max(y, y2), cases


def stroke_round_caps():
    """Open straight segments 60 pixels long (two knots, handles on their
    anchors) on whole-pixel ends, horizontal, vertical and at 30 degrees,
    center strokes of widths 4, 8, 16, 32 and 64 with round caps, then the
    same with square caps, fill off, named
    "cap <round|square> <width> <angle> at <x0> <y0>"."""
    cases = []
    y = 0
    for cap in ("round", "square"):
        for width in (4, 8, 16, 32, 64):
            cell = 60 + width + 40
            x = 0
            for angle in (0, 90, 30):
                a = math.radians(angle)
                x0, y0 = x + 20 + width // 2, y + 20 + width // 2
                x1, y1 = x0 + round(60 * math.cos(a)), y0 + round(60 * math.sin(a))
                cases.append(case(f"cap {cap} {width} {angle} at {x0} {y0}", [corner(x0, y0), corner(x1, y1)],
                                  closed=False, width=float(width), cap=cap))
                x += cell
            y += cell
    return 3 * (60 + 64 + 40), y, cases


def stroke_round_joins():
    """Open two-segment straight paths (vees, arms of 50 pixels, handles on
    their anchors) whose second arm turns 30, 60, 90, 120 and 150 degrees
    (clockwise on screen) from the first, which runs right from the start;
    the corner on a whole pixel; center strokes of widths 8, 16 and 32 with
    round joins and butt caps, fill off, named
    "join round <width> turn <degrees> at <cx> <cy>"."""
    cases = []
    cell = 2 * 50 + 32 + 40
    for row, width in enumerate((8, 16, 32)):
        for col, turn in enumerate((30, 60, 90, 120, 150)):
            cx, cy = col * cell + cell // 2, row * cell + cell // 2
            a = math.radians(turn)
            start = (cx - 50, cy)
            end = (cx + 50 * math.cos(a), cy + 50 * math.sin(a))
            cases.append(case(f"join round {width} turn {turn} at {cx} {cy}",
                              [corner(*start), corner(cx, cy), corner(*end)], closed=False,
                              width=float(width), join="round", cap="butt"))
    return 5 * cell, 3 * cell, cases


def corner_shapes(cx, cy):
    """Closed shapes with corners between curves or curves and lines, about
    cx, cy (whole pixels): "lens" two 90-degree arcs of radius 90 meeting at
    two corners, "pie" a quarter disc of radius 64 (two radii and a kappa
    quarter), "d" a half disc of radius 48 (two kappa quarters and the
    diameter), clockwise, each knot's handles the one before it, then the
    one after it."""
    r = 90.0
    h = K * r
    # The lens: arcs about (cx, cy + c) and (cx, cy - c) through the corners
    # (cx - a, cy) and (cx + a, cy), each turning 90 degrees.
    a = r / math.sqrt(2.0)
    c = a
    lens = [
        [cx - a, cy, cx - a + h / math.sqrt(2.0), cy + h / math.sqrt(2.0), cx - a + h / math.sqrt(2.0), cy - h / math.sqrt(2.0)],
        [cx + a, cy, cx + a - h / math.sqrt(2.0), cy - h / math.sqrt(2.0), cx + a - h / math.sqrt(2.0), cy + h / math.sqrt(2.0)],
    ]
    del c
    q = 64.0
    k = K * q
    pie = [
        [cx - q / 2, cy + q / 2, cx - q / 2, cy + q / 2, cx - q / 2, cy + q / 2],
        [cx - q / 2, cy - q / 2, cx - q / 2, cy - q / 2, cx - q / 2 + k, cy - q / 2],
        [cx + q / 2, cy + q / 2, cx + q / 2, cy + q / 2 - k, cx + q / 2, cy + q / 2],
    ]
    d = 48.0
    kd = K * d
    dee = [
        [cx - d, cy + d / 2, cx - d, cy + d / 2, cx - d, cy + d / 2 - kd],
        [cx, cy - d / 2, cx - kd, cy - d / 2, cx + kd, cy - d / 2],
        [cx + d, cy + d / 2, cx + d, cy + d / 2 - kd, cx + d, cy + d / 2],
    ]
    return {"lens": lens, "pie": pie, "d": dee}


def stroke_corner_shapes():
    """The corner_shapes under inside, center and outside strokes of widths
    4 and 8, miter joins then round joins, fill off, named
    "<shape> <alignment> <width> <join> at <cx> <cy>"."""
    cases = []
    cell = 200
    row = 0
    for join in ("miter", "round"):
        for width in (4.0, 8.0):
            col = 0
            for name in ("lens", "pie", "d"):
                for align in ("inside", "center", "outside"):
                    cx, cy = col * cell + cell // 2, row * cell + cell // 2
                    cases.append(case(f"{name} {align} {width:g} {join} at {cx} {cy}", corner_shapes(cx, cy)[name],
                                      width=width, align=align, join=join))
                    col += 1
            row += 1
    return 9 * cell, row * cell, cases


def stroke_dashes_curves():
    """Dashed center strokes of width 4 with butt caps, fill off: circles
    (four kappa quarters) of radius 64 with dash sets [2, 2], [4, 2] and
    [1, 3] and of radius 128 with [4, 2]; an open quarter arc of radius 128
    with [2, 2]; an open straight line 200 pixels long with [2, 2], named
    "dash <shape> <set> at <x> <y>"."""
    cases = []
    specs = [("circle r64", [2.0, 2.0]), ("circle r64", [4.0, 2.0]), ("circle r64", [1.0, 3.0]),
             ("circle r128", [4.0, 2.0]), ("quarter r128", [2.0, 2.0]), ("line 200", [2.0, 2.0])]
    cell = 300
    for i, (shape, dashes) in enumerate(specs):
        cx, cy = (i % 3) * cell + cell // 2, (i // 3) * cell + cell // 2
        label = "-".join(f"{v:g}" for v in dashes)
        if shape.startswith("circle"):
            r = int(shape.split(" ")[1][1:])
            knots, closed = ellipse(cx, cy, r, r), True
        elif shape.startswith("quarter"):
            knots, closed = arc_cubic(cx - 64, cy + 64, 128.0, 0, 90), False
        else:
            knots, closed = [corner(cx - 100, cy), corner(cx + 100, cy)], False
        cases.append(case(f"dash {shape} {label} at {cx} {cy}", knots, closed=closed, width=4.0, cap="butt",
                          dashes=dashes))
    return 3 * cell, 2 * cell, cases


def stroke_open_arc_caps():
    """Open single-cubic quarter arcs of radius 64 and 128 (from the top
    clockwise) under center strokes of widths 4 and 8 with butt, round and
    square caps, fill off, named "open r<radius> <width> <cap> at <cx> <cy>"."""
    cases = []
    cell = 200
    i = 0
    for radius in (64, 128):
        for width in (4.0, 8.0):
            for cap in ("butt", "round", "square"):
                cx, cy = (i % 6) * cell + 30, (i // 6) * cell + 160
                cases.append(case(f"open r{radius} {width:g} {cap} at {cx} {cy}",
                                  arc_cubic(cx, cy + radius - 140, float(radius), 0, 90), closed=False,
                                  width=width, cap=cap))
                i += 1
    return 6 * cell, ((i + 5) // 6) * cell, cases


def stroke_dash_lines():
    """Dashed straight open paths 200 pixels long, center stroke 4, butt
    caps, dash set [2, 2], fill off: at 30 degrees with handles on the
    anchors, horizontal and at 30 degrees with handles at the thirds, named
    "dashline <angle> <anchors|thirds> at <x0> <y0>"."""
    cases = []
    specs = [(30, "anchors"), (0, "thirds"), (30, "thirds")]
    for i, (angle, handles) in enumerate(specs):
        x0, y0 = 20, 40 + i * 140
        a = math.radians(angle)
        x1, y1 = x0 + 200 * math.cos(a), y0 + 200 * math.sin(a)
        if handles == "anchors":
            knots = [corner(x0, y0), corner(x1, y1)]
        else:
            dx, dy = (x1 - x0) / 3.0, (y1 - y0) / 3.0
            knots = [[x0, y0, x0, y0, x0 + dx, y0 + dy], [x1, y1, x1 - dx, y1 - dy, x1, y1]]
        cases.append(case(f"dashline {angle} {handles} at {x0} {y0}", knots, closed=False, width=4.0, cap="butt",
                          dashes=[2.0, 2.0]))
    return 260, 3 * 140 + 80, cases


def stroke_open_corners():
    """Open paths of two quarter arcs of radius 64 meeting at a 90-degree
    corner (the first from its circle's left point up to its top, the
    second from its own left point, that same top, up to its top), center
    strokes of widths 4 and 8, miter, round and bevel joins, butt then
    round caps, fill off, named
    "opencorner <width> <join> <cap> at <x> <y>"."""
    cases = []
    cell_x, cell_y = 220, 220
    i = 0
    r = 64.0
    for cap in ("butt", "round"):
        for width in (4.0, 8.0):
            for join in ("miter", "round", "bevel"):
                c1x, c1y = (i % 6) * cell_x + 30 + r, (i // 6) * cell_y + 200
                first = arc_cubic(c1x, c1y, r, 270, 90)
                second = arc_cubic(c1x + r, c1y - r, r, 270, 90)
                knots = [first[0], [first[1][0], first[1][1], first[1][2], first[1][3], second[0][4], second[0][5]],
                         second[1]]
                cases.append(case(f"opencorner {width:g} {join} {cap} at {c1x:g} {c1y:g}", knots, closed=False,
                                  width=width, join=join, cap=cap))
                i += 1
    return 6 * cell_x, ((i + 5) // 6) * cell_y, cases


def stroke_dash_end_scan(steps=24):
    """One dash per layer, center stroke 8 with butt caps, fill off: its
    length 20 + 0.05 k pixels (dash set [length / 8, 100]) for k = 0 ..
    steps - 1, along an open quarter arc of radius 128 from its top
    clockwise, then along a straight horizontal line, named
    "dashend <arc|line> <k> at <x> <y>" (x, y the path's start)."""
    cases = []
    for shape in ("arc", "line"):
        for k in range(steps):
            col, row = k % 6, k // 6 + (0 if shape == "arc" else 4)
            x, y = col * 180 + 20, row * 120 + 20
            length = 20.0 + 0.05 * k
            if shape == "arc":
                knots = arc_cubic(x, y + 128, 128.0, 0, 90)
            else:
                knots = [corner(x, y), corner(x + 120, y)]
            cases.append(case(f"dashend {shape} {k} at {x} {y}", knots, closed=False, width=8.0, cap="butt",
                              dashes=[length / 8.0, 100.0]))
    return 6 * 180 + 160, 8 * 120 + 160, cases


def stroke_dash_distances():
    """One dash per layer of 10, 30, 50, 80, 120 and 160 pixels (dash set
    [length / 8, 100]) along an open quarter arc of radius 128 from its top
    clockwise, center stroke 8 with butt caps, fill off, named
    "dashdist <length> at <x> <y>" (x, y the path's start)."""
    cases = []
    for i, length in enumerate((10, 30, 50, 80, 120, 160)):
        x, y = (i % 3) * 200 + 20, (i // 3) * 200 + 20
        cases.append(case(f"dashdist {length} at {x} {y}", arc_cubic(x, y + 128, 128.0, 0, 90), closed=False,
                          width=8.0, cap="butt", dashes=[length / 8.0, 100.0]))
    return 3 * 200 + 160, 2 * 200 + 160, cases


def stroke_dash_length_scan():
    """One dash per layer of 8, 12, ..., 196 pixels (dash set
    [length / 8, 100]) along an open quarter arc of radius 128 from its top
    clockwise, center stroke 8 with butt caps, fill off, named
    "dashscan <length> at <x> <y>" (x, y the path's start)."""
    cases = []
    for i, length in enumerate(range(8, 200, 4)):
        x, y = (i % 8) * 180 + 20, (i // 8) * 180 + 20
        cases.append(case(f"dashscan {length} at {x} {y}", arc_cubic(x, y + 128, 128.0, 0, 90), closed=False,
                          width=8.0, cap="butt", dashes=[length / 8.0, 100.0]))
    return 8 * 180 + 160, 6 * 180 + 160, cases


def stroke_dash_radius_scan():
    """One dash per layer along open quarter arcs of radius 32, 64 and 256
    from their top clockwise, 12 lengths each (r32: 4..48 by 4, r64: 8..96
    by 8, r256: 32..384 by 32), dash set [length / 8, 100], center stroke 8
    with butt caps, fill off, named "dashrad r<radius> <length> at <x> <y>"
    (x, y the path's start)."""
    cases = []
    y = 20
    for radius, step in ((32, 4), (64, 8), (256, 32)):
        cell = radius + 60
        for i in range(12):
            length = step * (i + 1)
            x = i * cell + 20
            cases.append(case(f"dashrad r{radius} {length} at {x} {y}", arc_cubic(x, y + radius, float(radius), 0, 90),
                              closed=False, width=8.0, cap="butt", dashes=[length / 8.0, 100.0]))
        y += cell
    return 12 * (256 + 60) + 40, y + 20, cases


def stroke_dash_shape_scan():
    """One dash per layer of 40, 80, 120 and 160 pixels (dash set
    [length / 8, 100]) along open quarter cubics from the top round to the
    right: r128 with both handles 0.45 r ("h45") or 0.65 r ("h65"), and the
    quarter ellipse of radii 128 and 64 with kappa handles ("e128x64");
    center stroke 8 with butt caps, fill off, named
    "dashshape <shape> <length> at <x> <y>" (x, y the path's start)."""
    cases = []
    shapes = [("h45", 128.0, 128.0, 0.45, 0.45), ("h65", 128.0, 128.0, 0.65, 0.65), ("e128x64", 128.0, 64.0, K, K)]
    for row, (name, rx, ry, h1, h2) in enumerate(shapes):
        for col, length in enumerate((40, 80, 120, 160)):
            x, y = col * 200 + 20, row * 180 + 20
            knots = [[x, y, x, y, x + h1 * rx, y], [x + rx, y + ry, x + rx, y + ry - h2 * ry, x + rx, y + ry]]
            cases.append(case(f"dashshape {name} {length} at {x} {y}", knots, closed=False, width=8.0, cap="butt",
                              dashes=[length / 8.0, 100.0]))
    return 4 * 200 + 160, 3 * 180 + 160, cases


def stroke_dash_mid_curve():
    """One dash wholly inside the curve per layer: dash set [2, 10] at
    width 8 (16 px drawn, 80 skipped) with dash offsets 56, 36 and 16 px
    (the dash then runs 40-56, 60-76 or 80-96 px along), on an open r128
    quarter with kappa handles and with handles 0.65 r, from the top round
    to the right; center stroke 8, butt caps, fill off, named
    "dashmid <kappa|h65> <offset> at <x> <y>"."""
    cases = []
    for row, (name, h) in enumerate((("kappa", K), ("h65", 0.65))):
        for col, offset in enumerate((56.0, 36.0, 16.0)):
            x, y = col * 200 + 20, row * 180 + 20
            knots = [[x, y, x, y, x + h * 128.0, y], [x + 128.0, y + 128.0, x + 128.0, y + 128.0 - h * 128.0,
                                                       x + 128.0, y + 128.0]]
            cases.append(case(f"dashmid {name} {offset:g} at {x} {y}", knots, closed=False, width=8.0, cap="butt",
                              dashes=[2.0, 10.0], dashOffset=offset))
    return 3 * 200 + 160, 2 * 180 + 160, cases


def stroke_dash_width_scan():
    """Dashes along an open r128 kappa quarter from the top round to the
    right, center strokes with butt caps, fill off, one layer per (width,
    dash set, dash offset): the 8-on 8-off pattern at widths 2, 4 and 8;
    one 8-pixel dash at 88-96 px along at widths 4 and 8; the w4 pattern
    shifted by 8 px; 8 on 56 off at w4; 16 on 16 off at w8. Named
    "dashwidth w<width> <dash set> off<offset> at <x> <y>"."""
    specs = [(4.0, [2.0, 2.0], 0.0), (8.0, [1.0, 1.0], 0.0), (2.0, [4.0, 4.0], 0.0), (4.0, [2.0, 100.0], 320.0),
             (8.0, [1.0, 100.0], 720.0), (4.0, [2.0, 2.0], 8.0), (4.0, [2.0, 14.0], 0.0), (8.0, [2.0, 2.0], 0.0)]
    cases = []
    for i, (width, dashes, offset) in enumerate(specs):
        x, y = (i % 4) * 200 + 20, (i // 4) * 180 + 20
        knots = [[x, y, x, y, x + K * 128.0, y], [x + 128.0, y + 128.0, x + 128.0, y + 128.0 - K * 128.0,
                                                   x + 128.0, y + 128.0]]
        name = f"dashwidth w{width:g} {'-'.join(f'{d:g}' for d in dashes)} off{offset:g} at {x} {y}"
        cases.append(case(name, knots, closed=False, width=width, cap="butt", dashes=dashes, dashOffset=offset))
    return 4 * 200 + 160, 2 * 180 + 160, cases


def stroke_dash_features():
    """Dash features beyond butt-capped center strokes, fill off: round and
    square dash caps on a 200-pixel straight line and on an r128 kappa
    quarter (8 on 8 off at width 8, and 8 on 24 off with round caps); a
    closed 120 by 80 rectangle at width 4 with 16 on 8 off begun 12 px in, so
    one dash straddles the closing corner, under miter then round joins; an
    r64 circle (four kappa quarters) and the rectangle dashed 16 on 16 off at
    width 8 under inside, outside and center alignment. Named
    "dashfeat <shape> <setting> w<width> <dash set> at <x> <y>" (x, y the
    cell's top left)."""
    cases = []
    cells = []
    for kind in ("round", "square"):
        cells.append((f"line {kind}", "line", dict(cap=kind, width=8.0, dashes=[2.0, 2.0])))
    for kind in ("round", "square"):
        cells.append((f"quarter {kind}", "quarter", dict(cap=kind, width=8.0, dashes=[2.0, 2.0])))
    cells.append(("quarter round", "quarter", dict(cap="round", width=8.0, dashes=[1.0, 3.0])))
    for join in ("miter", "round"):
        cells.append((f"rect {join}-join off12", "rect", dict(join=join, width=4.0, dashes=[4.0, 2.0], dashOffset=12.0)))
    for align in ("inside", "outside", "center"):
        cells.append((f"circle {align}", "circle", dict(align=align, width=8.0, dashes=[2.0, 2.0])))
    for align in ("inside", "outside"):
        cells.append((f"rect {align}", "rect", dict(align=align, width=8.0, dashes=[2.0, 2.0])))
    for i, (setting, shape, stroke_changes) in enumerate(cells):
        x, y = (i % 4) * 260 + 30, (i // 4) * 230 + 30
        if shape == "line":
            knots, closed = [corner(x, y + 20), corner(x + 200, y + 20)], False
        elif shape == "quarter":
            knots = [[x, y, x, y, x + K * 128.0, y],
                     [x + 128.0, y + 128.0, x + 128.0, y + 128.0 - K * 128.0, x + 128.0, y + 128.0]]
            closed = False
        elif shape == "rect":
            knots, closed = rect(x + 20, y + 20, x + 140, y + 100), True
        else:
            knots, closed = circle_arcs(x + 84, y + 84, 64, 4), True
        dash_set = "-".join(f"{d:g}" for d in stroke_changes["dashes"])
        name = f"dashfeat {setting} w{stroke_changes['width']:g} {dash_set} at {x} {y}"
        cases.append(case(name, knots, closed=closed, **stroke_changes))
    return 4 * 260 + 40, 3 * 230 + 40, cases


def stroke_dash_aligned_starts():
    """Where inside and outside dash patterns start: closed paths dashed 16
    on 16 off at width 8 (fill off): regular polygons of 3, 5, 6 and 8
    corners (circumradius 70, first corner at the top, clockwise), circles
    of 3, 6 and 8 equal arcs (r64), and the 120 by 80 rectangle begun at
    its second corner, run counterclockwise, and with 5 knots (a knot added
    mid top edge), inside unless named outside. Named
    "dashstart <shape> <alignment> at <x> <y>" (x, y the cell's top left)."""
    cells = []
    for n in (3, 5, 6, 8):
        cells.append((f"poly{n}", "inside", "poly", n))
    cells.append(("poly5", "outside", "poly", 5))
    for n in (3, 6, 8):
        cells.append((f"circle{n}", "inside", "circle", n))
    for variant in ("from1", "ccw", "knots5"):
        cells.append((f"rect-{variant}", "inside", "rect", variant))
    cases = []
    for i, (shape, align, kind, arg) in enumerate(cells):
        x, y = (i % 4) * 200 + 30, (i // 4) * 200 + 30
        cx, cy = x + 84, y + 84
        if kind == "poly":
            pts = [(cx + 70.0 * math.sin(2 * math.pi * k / arg), cy - 70.0 * math.cos(2 * math.pi * k / arg))
                   for k in range(arg)]
            knots = polygon(pts)
        elif kind == "circle":
            knots = circle_arcs(cx, cy, 64, arg)
        else:
            corners = [(x + 20, y + 40), (x + 140, y + 40), (x + 140, y + 120), (x + 20, y + 120)]
            if arg == "from1":
                corners = corners[1:] + corners[:1]
            elif arg == "ccw":
                corners = corners[::-1]
            else:
                corners = corners[:1] + [(x + 80, y + 40)] + corners[1:]
            knots = polygon(corners)
        cases.append(case(f"dashstart {shape} {align} at {x} {y}", knots, closed=True, width=8.0, align=align,
                          dashes=[2.0, 2.0]))
    return 4 * 200 + 40, 3 * 200 + 40, cases


def stroke_dash_aligned_more():
    """Inside and outside dashes, 16 on 16 off at width 8 unless named, fill
    off: an r64 circle (four kappa quarters) and the 120 by 80 rectangle
    with dash offset 8; both with round caps (inside) and square caps
    (outside); an open r128 kappa quarter (top round to the right) and an
    open L of two 100-pixel legs (right then down) under inside and outside.
    Named "dashmore <shape> <alignment> <setting> at <x> <y>" (x, y the
    cell's top left)."""
    cells = [("circle", "inside", "off8", dict(dashOffset=8.0)), ("rect", "inside", "off8", dict(dashOffset=8.0)),
             ("circle", "inside", "round", dict(cap="round")), ("rect", "inside", "round", dict(cap="round")),
             ("circle", "outside", "square", dict(cap="square")), ("rect", "outside", "square", dict(cap="square")),
             ("quarter", "inside", "open", {}), ("quarter", "outside", "open", {}),
             ("ell", "inside", "open", {}), ("ell", "outside", "open", {})]
    cases = []
    for i, (shape, align, setting, changes) in enumerate(cells):
        x, y = (i % 4) * 220 + 30, (i // 4) * 220 + 30
        closed = shape in ("circle", "rect")
        if shape == "circle":
            knots = circle_arcs(x + 84, y + 84, 64, 4)
        elif shape == "rect":
            knots = rect(x + 20, y + 40, x + 140, y + 120)
        elif shape == "quarter":
            knots = [[x + 20, y + 20, x + 20, y + 20, x + 20 + K * 128.0, y + 20],
                     [x + 148.0, y + 148.0, x + 148.0, y + 148.0 - K * 128.0, x + 148.0, y + 148.0]]
        else:
            knots = [corner(x + 20, y + 20), corner(x + 120, y + 20), corner(x + 120, y + 120)]
        cases.append(case(f"dashmore {shape} {align} {setting} at {x} {y}", knots, closed=closed, width=8.0,
                          align=align, dashes=[2.0, 2.0], **changes))
    return 4 * 220 + 40, 3 * 220 + 40, cases


def fill_wide_arc_radius_scan():
    """Filled circles of three equal 120-degree arc cubics (circle_mixed_arcs,
    clockwise from the top) on whole-pixel centers at radii 32, 40, ..., 160,
    stroke off, named "fill circle3 r<radius> at <cx> <cy>"."""
    cases = []
    x, y, row_h = 0, 0, 0
    for radius in range(32, 168, 8):
        cell = 2 * radius + 24
        if x + cell > 1400:
            x, y, row_h = 0, y + row_h, 0
        cx, cy = x + cell // 2, y + cell // 2
        cases.append({"name": f"fill circle3 r{radius} at {cx} {cy}", "fill": [255, 0, 0],
                      "subpaths": [{"closed": True, "op": "add",
                                    "knots": circle_mixed_arcs(cx, cy, radius, (120, 120, 120))}],
                      "stroke": stroke(enabled=False, fillEnabled=True)})
        x += cell
        row_h = max(row_h, cell)
    return 1400, y + row_h, cases


def stroke_dash_validation():
    """Dash rules on shapes they were not found on, fill off: a closed
    160 by 90 ellipse, a closed 160 by 100 rounded rectangle (corner radius
    24) and an open S of two cubics (an inflection where they meet), center
    width 4 with 12 on 8 off; the ellipse also inside at width 6, 8 on 8 off.
    Named "dashval <shape> <alignment> at <x> <y>"."""
    cases = []
    x0, y0 = 30, 30
    ell = ellipse(x0 + 100, y0 + 70, 80, 45)
    cases.append(case(f"dashval ellipse center at {x0} {y0}", ell, width=4.0, dashes=[3.0, 2.0]))
    x1 = 260
    r_ = 24.0
    k_ = K * r_
    left, top, right, bottom = x1, y0 + 20, x1 + 160, y0 + 120
    rr = [[left + r_, top, left + r_ - k_, top, left + r_, top], [right - r_, top, right - r_, top, right - r_ + k_, top],
          [right, top + r_, right, top + r_ - k_, right, top + r_], [right, bottom - r_, right, bottom - r_, right, bottom - r_ + k_],
          [right - r_, bottom, right - r_ + k_, bottom, right - r_, bottom], [left + r_, bottom, left + r_, bottom, left + r_ - k_, bottom],
          [left, bottom - r_, left, bottom - r_ + k_, left, bottom - r_], [left, top + r_, left, top + r_, left, top + r_ - k_]]
    cases.append(case(f"dashval roundrect center at {x1} {y0}", rr, width=4.0, dashes=[3.0, 2.0]))
    x2 = 480
    s_knots = [[x2, y0 + 120, x2, y0 + 120, x2 + 30, y0 + 20], [x2 + 90, y0 + 70, x2 + 60, y0 + 100, x2 + 120, y0 + 40],
               [x2 + 180, y0 + 20, x2 + 150, y0 + 120, x2 + 180, y0 + 20]]
    cases.append(case(f"dashval scurve center at {x2} {y0}", s_knots, closed=False, width=4.0, dashes=[3.0, 2.0]))
    x3 = 700
    cases.append(case(f"dashval ellipse inside at {x3} {y0}", ellipse(x3 + 100, y0 + 70, 80, 45), width=6.0,
                      align="inside", dashes=[8.0 / 6.0, 8.0 / 6.0]))
    return 940, 200, cases


def stroke_circle_arc_radii():
    """Circles of radius 32, 48, 64, 96, 128, 192 and 256 on whole-pixel
    centers built from 4, 6 and 8 equal arc cubics, inside strokes of
    width 8, fill off, one row per radius, named
    "inside 8 circle r<radius> n<n> at <cx> <cy>"."""
    cases = []
    y = 0
    for radius in (32, 48, 64, 96, 128, 192, 256):
        cell = 2 * radius + 40
        for col, n in enumerate((4, 6, 8)):
            cx, cy = col * cell + cell // 2, y + cell // 2
            cases.append(case(f"inside 8 circle r{radius} n{n} at {cx} {cy}", circle_arcs(cx, cy, radius, n),
                              width=8.0, align="inside"))
        y += cell
    return 3 * (2 * 256 + 40), y, cases


def stroke_round_rects():
    """Rounded rectangles 120 by 80 on whole pixels with corner radii 8, 16
    and 32, under each alignment at widths 2 and 4, fill off, named
    "<alignment> <width> roundrect r<radius>"."""
    cell_x, cell_y = 150, 110
    cases = []
    row = 0
    for radius in (8.0, 16.0, 32.0):
        col = 0
        for align in ("inside", "center", "outside"):
            for width in (2.0, 4.0):
                x0, y0 = col * cell_x + 15, row * cell_y + 15
                cases.append(case(f"{align} {width:g} roundrect r{radius:g}",
                                  round_rect(x0, y0, x0 + 120, y0 + 80, radius), width=width, align=align))
                col += 1
        row += 1
    return 6 * cell_x, 3 * cell_y, cases


def stroke_ellipses_io():
    """Ellipses 128 by 64 and 64 by 128 (radii) on whole-pixel centers,
    inside and outside strokes of widths 2 and 4, fill off, named
    "<alignment> <width> ellipse <rx>x<ry>"."""
    cases = []
    col = 0
    for rx, ry in ((128, 64), (64, 128)):
        for align in ("inside", "outside"):
            for width in (2.0, 4.0):
                cx, cy = col * 270 + 140, 140
                cases.append(case(f"{align} {width:g} ellipse {rx}x{ry}", ellipse(cx, cy, rx, ry),
                                  width=width, align=align))
                col += 1
    return col * 270 + 10, 290, cases


def stroke_ellipses_io_mild():
    """Ellipses of radii 96x64, 128x96, 128x112 and 64x48 on whole-pixel
    centers, inside and outside strokes of widths 1, 2 and 4, fill off, one
    row per ellipse, named "<alignment> <width> ellipse <rx>x<ry>"."""
    cases = []
    y = 0
    for rx, ry in ((96, 64), (128, 96), (128, 112), (64, 48)):
        cell_x, cell_y = 2 * rx + 30, 2 * ry + 30
        col = 0
        for align in ("inside", "outside"):
            for width in (1.0, 2.0, 4.0):
                cx, cy = col * cell_x + cell_x // 2, y + cell_y // 2
                cases.append(case(f"{align} {width:g} ellipse {rx}x{ry}", ellipse(cx, cy, rx, ry),
                                  width=width, align=align))
                col += 1
        y += cell_y
    return 6 * (2 * 128 + 30), y, cases


def stroke_circles():
    """How Photoshop strokes curves, fill off: a circle of radius 32 under
    each alignment at widths 1, 2, 3, 4, 6 and 8, on a whole-pixel center
    and on a fractional one, one layer each, named
    "<alignment> <width> circle <whole|fractional>"."""
    cell = 96
    widths = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0]
    cases = []
    row = 0
    for place, (fx, fy) in (("whole", (0.0, 0.0)), ("fractional", (0.37, 0.81))):
        for align in ("inside", "center", "outside"):
            for col, width in enumerate(widths):
                cx, cy = col * cell + 48 + fx, row * cell + 48 + fy
                cases.append(case(f"{align} {width:g} circle {place}", ellipse(cx, cy, 32, 32),
                                  width=width, align=align))
            row += 1
    return len(widths) * cell, row * cell, cases


def smoke():
    return 200, 100, [
        case("rect 1 center whole", rect(10, 10, 40, 40)),
        case("rect 1 center half", rect(60.5, 10.5, 90.5, 40.5)),
        case("ellipse 3 center", ellipse(125, 25, 15, 15), width=3.0),
        case("triangle 5 center", polygon([(160, 40), (190, 40), (175, 10)]), width=5.0),
        case("rect 2 center filled", rect(10, 60, 40, 90), width=2.0, fillEnabled=True, color=[0, 0, 255]),
    ]


def placement_shapes(x, y):
    """Nine outlines inside the 64-pixel cell at x, y, each kept 12 pixels
    from the cell's edges so a 5-pixel outside stroke stays inside it."""
    cx, cy = x + 32, y + 32
    return [
        ("rect whole", rect(x + 17, y + 20, x + 47, y + 44)),
        ("rect whole reversed", list(reversed(rect(x + 17, y + 20, x + 47, y + 44)))),
        ("rect half", rect(x + 17.5, y + 20.5, x + 47.5, y + 44.5)),
        ("rect 0.3 0.7", rect(x + 17.3, y + 20.7, x + 47.3, y + 44.7)),
        ("diamond", polygon([(cx, cy - 15), (cx + 15, cy), (cx, cy + 15), (cx - 15, cy)])),
        ("triangle", polygon([(x + 14, y + 48), (x + 49, y + 43), (x + 29, y + 15)])),
        ("hexagon fractional", polygon([(x + 21.37, y + 16.12), (x + 41.81, y + 18.46), (x + 49.23, y + 31.9),
                                        (x + 40.05, y + 47.64), (x + 22.68, y + 45.2), (x + 14.92, y + 30.33)])),
        ("ellipse whole", ellipse(cx, cy, 16, 12)),
        ("ellipse fractional", ellipse(cx + 0.37, cy + 0.81, 15.6, 12.3)),
    ]


def placement():
    """Where Photoshop puts a stroke: every alignment at whole, half and
    fractional widths over shapes on whole, half and fractional
    coordinates, fill off, so each layer's pixels are its stroke alone."""
    widths = [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0]
    cases = []
    for column in range(9):
        shape, knots = placement_shapes(column * 64, 0)[column]
        cases.append(case(f"fill {shape}", knots, enabled=False, fillEnabled=True))
    row = 1
    for align in ("center", "inside", "outside"):
        for width in widths:
            for column in range(9):
                shape, knots = placement_shapes(column * 64, row * 64)[column]
                cases.append(case(f"{align} {width:g} {shape}", knots, width=width, align=align))
            row += 1
    return 9 * 64, row * 64, cases


def combine_shapes():
    """How Photoshop combines a shape's subpaths, fills only: overlapping
    pairs (two rectangles, two circles, a diamond over a rectangle, an
    ellipse and a triangle) at fractional coordinates, the second of each
    pair added, subtracted, intersected or excluded; then shapes of three
    subpaths mixing the operations, and shapes whose first subpath
    subtracts or intersects. One 120-pixel cell a layer, six across."""
    def moved(knots, dx, dy):
        return [[v + (dx if k % 2 == 0 else dy) for k, v in enumerate(knot)] for knot in knots]

    pairs = [
        ("rects", rect(10.3, 12.6, 70.45, 80.15), rect(40.7, 35.2, 105.9, 100.55)),
        ("circles", ellipse(45.3, 50.6, 32.2, 32.2), ellipse(72.45, 68.15, 30.7, 30.7)),
        ("diamond", polygon([(15.2, 60.3), (60.1, 12.7), (105.6, 58.9), (59.4, 104.2)]),
         rect(35.35, 30.8, 95.6, 90.25)),
        ("triangle", ellipse(55.5, 55.25, 42.3, 28.6), polygon([(30.2, 95.4), (60.6, 15.3), (100.1, 92.8)])),
    ]
    mixed = [
        ("add subtract add", [("add", rect(8.4, 10.2, 110.6, 108.9)), ("subtract", ellipse(60.15, 58.7, 38.3, 38.3)),
                              ("add", rect(45.25, 44.6, 75.8, 73.35))]),
        ("add intersect exclude", [("add", ellipse(50.6, 55.3, 40.2, 40.2)),
                                   ("intersect", rect(20.45, 15.1, 100.3, 95.7)),
                                   ("exclude", ellipse(70.2, 40.55, 25.4, 25.4))]),
        ("exclude three circles", [("add", ellipse(45.2, 45.7, 30.1, 30.1)), ("exclude", ellipse(75.6, 47.3, 30.1, 30.1)),
                                   ("exclude", ellipse(60.35, 74.15, 30.1, 30.1))]),
        ("subtract first", [("subtract", ellipse(60.3, 60.55, 35.2, 35.2))]),
        ("subtract first then add", [("subtract", ellipse(50.3, 55.55, 35.2, 35.2)),
                                     ("add", rect(55.4, 25.25, 105.7, 95.85))]),
        ("intersect first then add", [("intersect", ellipse(50.3, 55.55, 35.2, 35.2)),
                                      ("add", rect(55.4, 25.25, 105.7, 95.85))]),
    ]
    off = stroke(enabled=False, fillEnabled=True)
    cases = []
    cell = 0
    for op in ("add", "subtract", "intersect", "exclude"):
        for name, first, second in pairs:
            dx, dy = (cell % 6) * 120, (cell // 6) * 120
            cases.append({"name": f"{op} {name}", "fill": [255, 0, 0],
                          "subpaths": [{"closed": True, "op": "add", "knots": moved(first, dx, dy)},
                                       {"closed": True, "op": op, "knots": moved(second, dx, dy)}],
                          "stroke": off})
            cell += 1
    for name, parts in mixed:
        dx, dy = (cell % 6) * 120, (cell // 6) * 120
        cases.append({"name": name, "fill": [255, 0, 0],
                      "subpaths": [{"closed": True, "op": op, "knots": moved(k, dx, dy)} for op, k in parts],
                      "stroke": off})
        cell += 1
    return 6 * 120, ((cell + 5) // 6) * 120, cases


BATCHES = {"smoke": smoke, "placement": placement, "edges": edges, "curves": curves, "geometry": geometry,
           "flatness": flatness, "mix": mix, "moved": moved, "wedges": wedges, "mirrors": mirrors,
           "depth": depth, "random": random_edges, "split": split_edges}
for _lean in (1.0, -1.0, 2.0, -2.0, 4.0, -4.0):
    BATCHES[f"scan{_lean:+g}"] = lambda lean=_lean: scan(lean)
# A lean whose per-pixel-row run is off the 1/16 grid, and a lean 1 edge
# whose run is 0.1 sample longer than whole.
BATCHES["scan+1.4"] = lambda: scan(1.4)
BATCHES["scan-1.4"] = lambda: scan(-1.4)
BATCHES["scan+1run"] = lambda: scan(1.0 + 0.1 / 816.0)
BATCHES["scan-1run"] = lambda: scan(-(1.0 + 0.1 / 816.0))
BATCHES["circlescan32"] = lambda: circle_scan(32.0)
BATCHES["bend2"] = lambda: bend_scan(0.15)
BATCHES["bend4"] = lambda: bend_scan(0.5)
BATCHES["bend2x"] = lambda: bend_scan(0.15, step_x=1.0 / 120.0)
BATCHES["bend2y"] = lambda: bend_scan(0.15, step_x=0.0, step_y=1.0 / 120.0)
BATCHES["bendgrid"] = lambda: bend_grid([0.130 + 0.005 * i for i in range(9)])
# A bow whose chord count stays at 2 over a whole sample, for inversion.
BATCHES["bend2s"] = lambda: bend_scan(0.12)
BATCHES["strokecircle"] = stroke_circles
BATCHES["strokescan"] = lambda: stroke_scan(32.0, 4.0, "inside")
BATCHES["strokearc"] = stroke_arcs
BATCHES["strokelines"] = stroke_lines
BATCHES["strokearcr"] = stroke_arc_radii
BATCHES["strokearcbig"] = lambda: stroke_arc_radii((256.0, 512.0, 1024.0))
BATCHES["strokearcmid"] = lambda: stroke_arc_radii((288.0, 320.0, 384.0, 448.0))
BATCHES["strokearcang"] = stroke_arc_angles
BATCHES["strokearcmix"] = stroke_arc_mix
BATCHES["strokehandles"] = stroke_handles
BATCHES["strokearcskew"] = stroke_arc_skew
BATCHES["strokecircler"] = stroke_circle_radii
BATCHES["strokeroundrect"] = stroke_round_rects
BATCHES["strokeellipseio"] = stroke_ellipses_io
BATCHES["strokeellipseio2"] = stroke_ellipses_io_mild
BATCHES["strokearcmid2"] = lambda: stroke_arc_radii((336.0, 352.0, 368.0))
BATCHES["strokecirclearcs"] = stroke_circle_arcs
BATCHES["strokecirclesweeps"] = stroke_circle_sweeps
BATCHES["strokecirclearcradii"] = stroke_circle_arc_radii
BATCHES["strokecirclesweeps2"] = stroke_circle_sweeps_fine
BATCHES["strokecirclesweeps3"] = stroke_circle_sweeps_scan
BATCHES["fillwidearcs"] = fill_wide_arcs
BATCHES["fillbigcircles"] = fill_big_circles
BATCHES["strokecaps"] = stroke_round_caps
BATCHES["strokejoins"] = stroke_round_joins
BATCHES["strokecorners"] = stroke_corner_shapes
BATCHES["strokedashes"] = stroke_dashes_curves
BATCHES["strokeopencaps"] = stroke_open_arc_caps
BATCHES["strokedashlines"] = stroke_dash_lines
BATCHES["strokeopencorners"] = stroke_open_corners
BATCHES["strokedashend"] = stroke_dash_end_scan
BATCHES["strokedashdist"] = stroke_dash_distances
BATCHES["strokedashscan"] = stroke_dash_length_scan
BATCHES["strokedashrad"] = stroke_dash_radius_scan
BATCHES["strokedashshape"] = stroke_dash_shape_scan
BATCHES["strokedashmid"] = stroke_dash_mid_curve
BATCHES["strokedashwidth"] = stroke_dash_width_scan
BATCHES["strokedashfeat"] = stroke_dash_features
BATCHES["strokedashstart"] = stroke_dash_aligned_starts
BATCHES["strokedashmore"] = stroke_dash_aligned_more
BATCHES["fillwidescan"] = fill_wide_arc_radius_scan
BATCHES["strokedashval"] = stroke_dash_validation
BATCHES["dashscanin"] = lambda: dash_scan(64.0, 8.0, "inside", (2.0, 2.0))
BATCHES["dashposin"] = dash_position_scan
BATCHES["dashwidthalign"] = dash_width_alignment_scan
BATCHES["fillscan120"] = lambda: fill_arcs_scan(64.0, (120.0, 120.0, 120.0))
BATCHES["combine"] = combine_shapes
# One 16-pixel inside dash from s = 84 to 100 along the stroke (ending just
# before the circle's left knot, where the edge runs nearly upright).
BATCHES["dashscan1w8"] = lambda: dash_scan(64.0, 8.0, "inside", (2.0, 402.0 / 8.0), offset=16.0 + 402.0 - 84.0)
BATCHES["dashscan1w2"] = lambda: dash_scan(64.0, 2.0, "inside", (8.0, 402.0 / 2.0), offset=16.0 + 402.0 - 84.0)


def main(batch, out):
    width, height, cases = BATCHES[batch]()
    names = [c["name"] for c in cases]
    if len(set(names)) != len(names):
        raise ValueError("two cases share a name")
    os.makedirs(out, exist_ok=True)
    psd = os.path.abspath(os.path.join(out, batch + ".psd")).replace("\\", "/")
    with open(os.path.join(HERE, "ps_lib.jsx"), encoding="utf-8") as f:
        library = f.read()
    # The script's last expression is what DoJavaScriptFile returns: the
    # saved line, or ERROR with the line and message, which ps_run.ps1
    # turns into a failed run.
    script = library + f"""
var CASES = {json.dumps(cases)};
var outcome;
try {{
    outcome = runCases({json.dumps(batch)}, {width}, {height}, CASES, {json.dumps(psd)});
}} catch (e) {{
    outcome = "ERROR line " + e.line + ": " + e.message;
}}
outcome;
"""
    jsx = os.path.join(out, batch + ".jsx")
    with open(jsx, "w", encoding="ascii", newline="\n") as f:
        f.write(script)
    with open(os.path.join(out, batch + ".cases.json"), "w", encoding="utf-8") as f:
        json.dump({"batch": batch, "width": width, "height": height, "psd": psd, "cases": cases}, f, indent=1)
    print(f"wrote {jsx} with {len(cases)} cases, saving {psd}", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in BATCHES:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2])
