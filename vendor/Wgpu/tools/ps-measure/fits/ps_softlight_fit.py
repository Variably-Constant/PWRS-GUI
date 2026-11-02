"""Fits soft light's 8-bit arithmetic to Photoshop's table (out-composite2,
mode-soft_light): integer forms of the W3C formula's two halves, each
combination's misses and worst miss over the 65536 pairs, best first.

    python -u -I ps_softlight_fit.py <batch folder>

Ends with `SOFT DONE`.
"""

import os
import sys

import numpy as np
from psd_tools import PSDImage

S = np.tile(np.arange(256, dtype=np.int64)[:, None], (1, 256))
D = np.tile(np.arange(256, dtype=np.int64), (256, 1))


def r(v):
    return np.where(v >= 0, (v + 127) // 255, -((-v + 127) // 255))


def div_round(a, b):
    return (2 * a + b) // (2 * b)


def curve(d):
    """W3C soft light's D(d), held at 8 bits: the polynomial to a quarter,
    the square root past it."""
    x = d / 255.0
    g = np.where(x <= 0.25, ((16 * x - 12) * x + 4) * x, np.sqrt(x))
    return np.floor(g * 255 + 0.5).astype(np.int64)


def main(folder):
    psd = PSDImage.open(os.path.join(folder, "photoshop", "mode-soft_light.psd"))
    header = psd._record.header
    got = np.frombuffer(psd._record.image_data.get_data(header)[0], dtype=np.uint8).reshape(256, 256).astype(np.int64)
    g8 = curve(D)
    low_factor = {"255-2s": 255 - 2 * S, "255-L": 255 - ((510 * S + 128) >> 8)}
    up_factor = {"2s-255": 2 * S - 255, "2s-256": 2 * S - 256, "U": div_round(np.maximum(S - 128, 0) * 255, 127)}
    lows = {}
    for name, k in low_factor.items():
        lows[f"{name}, d(255-d) first"] = D - r(k * r(D * (255 - D)))
        lows[f"{name}, k d first"] = D - r(r(k * D) * (255 - D))
        lows[f"{name}, at once"] = D - r(k * D * (255 - D) // 255)
    ups = {}
    for name, k in up_factor.items():
        ups[f"{name}, rounded"] = D + r(k * (g8 - D))
        ups[f"{name}, cut down"] = D + (k * (g8 - D)) // 255
        ups[f"{name}, exact curve"] = D + np.floor(k * (np.where(D / 255.0 <= 0.25, ((16 * D / 255.0 - 12) * D / 255.0 + 4) * D / 255.0, np.sqrt(D / 255.0)) * 255 - D) / 255.0 + 0.5).astype(np.int64)
    low_rows, up_rows = S < 128, S >= 128
    low_scores = sorted((int((np.abs(v - got)[low_rows] > 0).sum()), int(np.abs(v - got)[low_rows].max()), n) for n, v in lows.items())
    up_scores = sorted((int((np.abs(v - got)[up_rows] > 0).sum()), int(np.abs(v - got)[up_rows].max()), n) for n, v in ups.items())
    print("lower half (s < 128): " + " | ".join(f"{n}: {m} off, worst {w}" for m, w, n in low_scores), flush=True)
    print("upper half (s >= 128): " + " | ".join(f"{n}: {m} off, worst {w}" for m, w, n in up_scores), flush=True)
    print("SOFT DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
