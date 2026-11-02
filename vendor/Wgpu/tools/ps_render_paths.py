"""The exact paths of Photoshop render documents, for Wgpu's tests of its
vector rasterizer against Photoshop 2026's own pixels.

A render document holds layers Photoshop drew from a script, each a shape
layer whose stored pixels are Photoshop's render of its path. The
document stores the path's points as 8.24 fixed-point fractions of the
canvas, a hair from the coordinates Photoshop drew from, so the tests
draw each layer from those coordinates instead: the cases file the
script was built from, written beside the document, turned into a plain
text file the Rust tests read. Every layer is kept; the tests read a
layer's fill and stroke settings from the document itself.

The output starts with `canvas <width> <height>`. Each layer is a line
`layer <name>`, then per subpath `path closed|open <operation>`, the
operation as a vector mask's records number it (0 exclude, 1 union,
2 subtract, 3 intersect), and per knot `knot <x> <y> <x before>
<y before> <x after> <y after>` in pixels, the handles before and after
the anchor. Every number is written as Python's repr writes it, which
reads back to the same double.

From the repository's folder:
python -u -I tools/ps_render_paths.py <batch>.cases.json tests/ps-render/<batch>.paths
"""

import json
import sys

# The operations a case's subpath names, as a vector mask's subpath
# record numbers them.
OPERATIONS = {"exclude": 0, "add": 1, "subtract": 2, "intersect": 3}


def main(cases_path, out_path):
    with open(cases_path, encoding="utf-8") as f:
        batch = json.load(f)
    lines = [f"canvas {batch['width']} {batch['height']}"]
    for case in batch["cases"]:
        lines.append(f"layer {case['name']}")
        for subpath in case["subpaths"]:
            closed = "closed" if subpath["closed"] else "open"
            lines.append(f"path {closed} {OPERATIONS[subpath['op']]}")
            for knot in subpath["knots"]:
                lines.append("knot " + " ".join(repr(float(v)) for v in knot))
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print(f"wrote {out_path}: {len(batch['cases'])} layers", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2])
