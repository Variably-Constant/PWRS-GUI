"""Prints what a saved document's raw composite holds: per plane its
smallest and largest values and where it is 0, and the planes' values at
the given pixels (x,y pairs).

    python -u -I ps_composite_peek.py <document> [x,y ...]

Ends with `PEEK DONE`.
"""

import sys

import numpy as np
from psd_tools import PSDImage


def main(path, points):
    psd = PSDImage.open(path)
    header = psd._record.header
    channels = psd._record.image_data.get_data(header)
    planes = np.stack([np.frombuffer(c, dtype=np.uint8).reshape(header.height, header.width) for c in channels]).astype(np.int64)
    print(f"{path}: {header.channels} channels, {header.width}x{header.height}, compression "
          f"{psd._record.image_data.compression}", flush=True)
    for k, plane in enumerate(planes):
        zeros = np.argwhere(plane == 0)
        where = f"first zeros (y,x) {zeros[:4].tolist()}" if len(zeros) else "no zeros"
        print(f"  plane {k}: min {int(plane.min())} max {int(plane.max())}, {len(zeros)} zeros, {where}", flush=True)
    for point in points:
        x, y = map(int, point.split(","))
        print(f"  ({x},{y}): {[int(p[y, x]) for p in planes]}", flush=True)
    print("PEEK DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2:])
