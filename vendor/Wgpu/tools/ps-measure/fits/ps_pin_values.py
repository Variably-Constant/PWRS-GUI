"""Reads pin light's upper half from Photoshop's table (out-composite2):
for each source level s from 128, the levels Photoshop gives where its
result is not the backdrop d, split by the backdrop's half, beside
2s - 255 and 2s - 256.

    python -u -I ps_pin_values.py <batch folder>

Ends with `PIN DONE`.
"""

import os
import sys

import numpy as np
from psd_tools import PSDImage


def main(folder):
    psd = PSDImage.open(os.path.join(folder, "photoshop", "mode-pin_light.psd"))
    header = psd._record.header
    got = np.frombuffer(psd._record.image_data.get_data(header)[0], dtype=np.uint8).reshape(256, 256).astype(np.int64)
    d = np.arange(256)
    for s in list(range(128, 140)) + list(range(180, 186)) + list(range(250, 256)):
        row = got[s]
        low = sorted(set(int(v) for v, dd in zip(row, d) if v != dd and dd < 128))
        high = sorted(set(int(v) for v, dd in zip(row, d) if v != dd and dd >= 128))
        print(f"s {s}: d<128 -> {low}, d>=128 -> {high}; 2s-255 {2 * s - 255}, 2s-256 {2 * s - 256}", flush=True)
    print("PIN DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
