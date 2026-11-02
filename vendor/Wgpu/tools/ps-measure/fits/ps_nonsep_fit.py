"""Fits the non-separable blend modes (hue, saturation, color, luminosity,
darker and lighter color) to Photoshop's composites of ps_composite_batch.py's
`nonsep` batch: random backdrop colors (seed 1) under random source colors
(seed 2), one pair a pixel. Each candidate's misses (samples) and worst miss.

    python -u -I ps_nonsep_fit.py <batch folder>

Ends with `NONSEP DONE`.
"""

import os
import sys

import numpy as np
from psd_tools import PSDImage

SIZE = 256
WEIGHTS = {"0.3/0.59/0.11": (0.3, 0.59, 0.11), "77/151/28 of 256": (77 / 256, 151 / 256, 28 / 256),
           "Rec 709": (0.2126, 0.7152, 0.0722), "30/59/11 whole": (30, 59, 11), "77/150/29 whole": (77, 150, 29),
           "76/150/30 whole": (76, 150, 30), "77/151/28 whole": (77, 151, 28)}


def colors(seed):
    return np.random.default_rng(seed).integers(0, 256, (SIZE, SIZE, 3), dtype=np.uint8).astype(np.float64)


def composite(path):
    psd = PSDImage.open(path)
    header = psd._record.header
    planes = psd._record.image_data.get_data(header)
    return np.dstack([np.frombuffer(p, dtype=np.uint8).reshape(SIZE, SIZE) for p in planes[:3]]).astype(np.int64)


def lum(c, w):
    return c[..., 0] * w[0] + c[..., 1] * w[1] + c[..., 2] * w[2]


def clip_color(c, w):
    l = lum(c, w)[..., None]
    n = c.min(axis=-1, keepdims=True)
    x = c.max(axis=-1, keepdims=True)
    c = np.where(n < 0, l + (c - l) * l / np.where(l - n == 0, 1, l - n), c)
    c = np.where(x > 255, l + (c - l) * (255 - l) / np.where(x - l == 0, 1, x - l), c)
    return c


def set_lum(c, l, w):
    d = l - lum(c, w)
    return clip_color(c + d[..., None], w)


def sat(c):
    return c.max(axis=-1) - c.min(axis=-1)


def set_sat(c, s):
    mx = c.max(axis=-1, keepdims=True)
    mn = c.min(axis=-1, keepdims=True)
    span = mx - mn
    out = np.where(span > 0, (c - mn) * s[..., None] / np.where(span == 0, 1, span), 0.0)
    return out


def rounded(c):
    return np.floor(c + 0.5).astype(np.int64)


def main(folder):
    back, source = colors(1), colors(2)
    for mode in ("luminosity", "color", "hue", "saturation", "darker_color", "lighter_color"):
        got = composite(os.path.join(folder, "photoshop", f"mode-{mode}.psd"))
        scores = []
        for wname, w in WEIGHTS.items():
            if "whole" in wname and mode not in ("darker_color", "lighter_color"):
                continue
            if mode == "luminosity":
                out = set_lum(back, lum(source, w), w)
            elif mode == "color":
                out = set_lum(source, lum(back, w), w)
            elif mode == "hue":
                out = set_lum(set_sat(source, sat(back)), lum(back, w), w)
            elif mode == "saturation":
                out = set_lum(set_sat(back, sat(source)), lum(back, w), w)
            else:
                if wname == "30/59/11 whole":
                    levels = {
                        "level rounded from 30/59/11": lambda c: np.floor((c @ np.array([30.0, 59.0, 11.0])) / 100 + 0.5),
                        "level (77r+151g+28b+128)>>8": lambda c: (c.astype(np.int64) @ np.array([77, 151, 28]) + 128) >> 8,
                        "level cut down from 30/59/11": lambda c: np.floor((c @ np.array([30.0, 59.0, 11.0])) / 100),
                    }
                    for lname, level in levels.items():
                        ls8, lb8 = level(source), level(back)
                        pick_source = (ls8 < lb8) if mode == "darker_color" else (ls8 > lb8)
                        out = np.where(pick_source[..., None], source, back)
                        diff = np.abs(rounded(out) - got)
                        scores.append((int((diff > 0).sum()), int(diff.max()), f"{lname}, backdrop on ties"))
                ls, lb = lum(source, w), lum(back, w)
                for tie in ("source on ties", "backdrop on ties"):
                    pick_source = (ls < lb) if mode == "darker_color" else (ls > lb)
                    if tie == "source on ties":
                        pick_source = pick_source | (ls == lb)
                    out = np.where(pick_source[..., None], source, back)
                    diff = np.abs(rounded(out) - got)
                    scores.append((int((diff > 0).sum()), int(diff.max()), f"{wname}, {tie}"))
                continue
            diff = np.abs(rounded(out) - got)
            scores.append((int((diff > 0).sum()), int(diff.max()), wname))
        scores.sort()
        print(f"{mode}: " + " | ".join(f"{n}: {m} off, worst {x}" for m, x, n in scores), flush=True)
    print("NONSEP DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
