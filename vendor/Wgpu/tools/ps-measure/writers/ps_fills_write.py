"""Writes the FILLS_RGB, FILLS_CMYK and FILLS_GRAY reference documents that
Wgpu's tools/psd_reference.py builds, as .psd files, for Photoshop to open
and save with its own composite (ps_composite_resave.jsx).

    python -u -I ps_fills_write.py <output folder>

Ends with `FILLS DONE`.
"""

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))

import psd_reference as reference  # noqa: E402


def main(out):
    os.makedirs(out, exist_ok=True)
    for name, build in (("fills-rgb", reference.fills_rgb), ("fills-cmyk", reference.fills_cmyk), ("fills-gray", reference.fills_gray)):
        data = build()
        with open(os.path.join(out, name + ".psd"), "wb") as f:
            f.write(data)
        print(f"wrote {name}.psd, {len(data)} bytes", flush=True)
    print("FILLS DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
