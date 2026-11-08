"""Reference values for Wgpu's drawing of a Photoshop layer's vector mask,
from psd-tools 1.26.0: vector masks psd-tools writes from its own records,
and the coverage psd-tools draws for each, vector._draw_path with a brush,
which its compositor's draw_vector_mask is.

Each mask is on a canvas 24 by 16 pixels: a rectangle at fractional
edges, an ellipse of four curved knots, a square with a hole of a second
square joined to it, a rectangle and an ellipse combined by union,
subtraction, intersection and exclusion, an ellipse subtracted first,
from full coverage, an open path, a mask of no paths whose initial fill
rule is 1, a rectangle reaching past the canvas, and a shape smaller
than a pixel.

Each mask's bytes, its vmsk block as psd-tools writes it, go into the
output beside the coverage, each value the bits of the float32 psd-tools
gives, and the box psd-tools' VectorMask.bbox gives it, so the tests read
no file and compare exactly.

It writes the Rust test constants in src/psd_vector_reference.rs. From
the repository's folder, with psd-tools 1.26.0 and its composite extras:
python -u -I tools/psd_vector_reference.py src/psd_vector_reference.rs
"""

import io
import sys
import types

import numpy as np
from psd_tools.api.shape import VectorMask
from psd_tools.composite import vector
from psd_tools.psd.vector import ClosedKnotLinked, ClosedPath, InitialFillRule, OpenKnotLinked, OpenPath, Path, VectorMaskSetting

WIDTH, HEIGHT = 24, 16
# How far along a quarter circle's tangent its handles are, as a fraction
# of the radius, for four cubics that follow a circle closely.
KAPPA = 0.5522847498


def at(x, y):
    """A point in pixels as a path's (y, x) fractions of the canvas."""
    return (y / HEIGHT, x / WIDTH)


def corner(knot, x, y):
    """A knot with both handles on its anchor, a straight edge's corner."""
    return knot(at(x, y), at(x, y), at(x, y))


def rectangle(left, top, right, bottom, operation=1, closed=True):
    knot = ClosedKnotLinked if closed else OpenKnotLinked
    knots = [corner(knot, left, top), corner(knot, right, top), corner(knot, right, bottom), corner(knot, left, bottom)]
    return (ClosedPath if closed else OpenPath)(items=knots, operation=operation, index=0)


def ellipse(cx, cy, rx, ry, operation=1):
    """An ellipse of four knots, top, right, bottom and left, clockwise."""
    kx, ky = KAPPA * rx, KAPPA * ry
    knots = [
        ClosedKnotLinked(at(cx - kx, cy - ry), at(cx, cy - ry), at(cx + kx, cy - ry)),
        ClosedKnotLinked(at(cx + rx, cy - ky), at(cx + rx, cy), at(cx + rx, cy + ky)),
        ClosedKnotLinked(at(cx + kx, cy + ry), at(cx, cy + ry), at(cx - kx, cy + ry)),
        ClosedKnotLinked(at(cx - rx, cy + ky), at(cx - rx, cy), at(cx - rx, cy - ky)),
    ]
    return ClosedPath(items=knots, operation=operation, index=0)


def open_curve():
    """An open path of three knots whose middle one bends it."""
    knots = [
        OpenKnotLinked(at(2.5, 13.0), at(2.5, 13.0), at(6.0, 2.0)),
        OpenKnotLinked(at(9.0, 1.5), at(13.25, 3.5), at(17.0, 5.0)),
        OpenKnotLinked(at(21.5, 14.75), at(21.5, 14.75), at(21.5, 14.75)),
    ]
    return OpenPath(items=knots, operation=1, index=0)


def cases():
    return [
        ("RECTANGLE", [rectangle(3.3, 2.25, 17.6, 11.8)]),
        ("ELLIPSE", [ellipse(12.0, 8.0, 7.5, 5.2)]),
        ("HOLE", [rectangle(2.0, 1.5, 21.5, 14.5), rectangle(7.25, 5.5, 15.75, 10.5, operation=-1)]),
        ("UNION", [rectangle(1.5, 1.5, 12.5, 9.5), ellipse(14.0, 9.0, 7.0, 5.0)]),
        ("SUBTRACT", [rectangle(1.5, 1.5, 18.5, 12.5), ellipse(14.0, 9.0, 7.0, 5.0, operation=2)]),
        ("INTERSECT", [rectangle(1.5, 1.5, 18.5, 12.5), ellipse(14.0, 9.0, 7.0, 5.0, operation=3)]),
        ("EXCLUDE", [rectangle(1.5, 1.5, 18.5, 12.5), ellipse(14.0, 9.0, 7.0, 5.0, operation=0)]),
        ("SUBTRACT_FIRST", [ellipse(12.0, 8.0, 6.5, 4.5, operation=2)]),
        ("OPEN", [open_curve()]),
        ("REVEAL_ALL", [InitialFillRule(value=1)]),
        ("OUTSIDE", [rectangle(-6.5, -3.25, 9.75, 21.5)]),
        ("TINY", [rectangle(10.2, 6.1, 10.7, 6.45)]),
    ]


def coverage(setting):
    """psd-tools' coverage of a vector mask on the canvas."""
    layer = types.SimpleNamespace(
        vector_mask=VectorMask(setting),
        _psd=types.SimpleNamespace(viewbox=(0, 0, WIDTH, HEIGHT), width=WIDTH, height=HEIGHT),
    )
    return vector._draw_path(layer, brush={"color": 255})[:, :, 0]


def rust_numbers(name, kind, values, doc, per_line, show):
    lines = [f"/// {doc}", f"pub(crate) static {name}: [{kind}; {len(values)}] = ["]
    for start in range(0, len(values), per_line):
        lines.append("    " + ", ".join(show(v) for v in values[start:start + per_line]) + ",")
    lines.append("];")
    return "\n".join(lines)


def main(out_path):
    parts = [
        "\n".join([
            "//! Reference values for psd_vector::tests, written by",
            "//! tools/psd_vector_reference.py from psd-tools 1.26.0: vector masks",
            "//! psd-tools writes, and the coverage psd-tools draws for each on a",
            f"//! canvas {WIDTH} by {HEIGHT}, as the bits of each float32 value.",
        ]),
        f"pub(crate) const WIDTH: usize = {WIDTH};\npub(crate) const HEIGHT: usize = {HEIGHT};",
    ]
    for name, items in cases():
        setting = VectorMaskSetting(version=3, flags=0, path=Path(items))
        buffer = io.BytesIO()
        setting.write(buffer)
        data = buffer.getvalue()
        reread = VectorMaskSetting.read(io.BytesIO(data))
        covered = np.ascontiguousarray(coverage(reread), dtype=np.float32)
        bits = covered.view(np.uint32).ravel().tolist()
        parts.append(rust_numbers(f"{name}_MASK", "u8", list(data), f"The {name.lower()} mask's vmsk block, as psd-tools writes it.", 24, str))
        parts.append(rust_numbers(f"{name}_COVERAGE", "u32", bits, f"psd-tools' coverage of {name}_MASK, row by row, as float32 bits.", 8, lambda v: f"0x{v:08x}"))
        box = VectorMask(reread).bbox
        parts.append(rust_numbers(f"{name}_BBOX", "f64", [float(v) for v in box], f"psd-tools' bbox of {name}_MASK: left, top, right and bottom, as fractions of the canvas.", 4, repr))
        print(f"wrote {name}: {len(data)} bytes, coverage from {covered.min():.4f} to {covered.max():.4f}, total {covered.sum():.4f}", flush=True)
    with open(out_path, "w", newline="\n") as handle:
        handle.write("\n\n".join(parts) + "\n")
    print(f"WROTE {out_path}", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
