"""Compares the composites Photoshop saved for ps_composite_batch.py's
documents (out-composite1/photoshop) with candidate arithmetic for normal
blending, case by case, and prints per document and candidate how many
samples miss and by how much.

    python -u -I ps_composite_fit.py <batch folder>

The composite is read raw, from the document's merged image data: its
color as stored (matted with white where the document is transparent) and
its alpha. Ends with `FIT DONE`.
"""

import json
import os
import sys

import numpy as np
from psd_tools import PSDImage

SIZE = 256


def stored_composite(path):
    psd = PSDImage.open(path)
    header = psd._record.header
    channels = psd._record.image_data.get_data(header)
    return np.stack([np.frombuffer(c, dtype=np.uint8).reshape(header.height, header.width) for c in channels]).astype(np.int64)


def mul8(a, b):
    """a * b / 255 rounded, by the shift form (t = a b + 128, (t + t >> 8) >> 8)."""
    t = a * b + 128
    return (t + (t >> 8)) >> 8


def div255_round(v):
    """v / 255 rounded half up, for v >= 0."""
    return (v + 127) // 255


CANDIDATES = {}


def candidate(name):
    def register(f):
        CANDIDATES[name] = f
        return f
    return register


@candidate("float once")
def float_once(src, dst, a, o, f):
    alpha = (a / 255.0) * (o / 255.0) * (f / 255.0)
    return np.floor(dst + (src - dst) * alpha + 0.5).astype(np.int64)


@candidate("float32 once (psd-tools)")
def float32_once(src, dst, a, o, f):
    alpha = (a.astype(np.float32) / np.float32(255)) * np.float32(o / 255.0) * np.float32(f / 255.0)
    out = dst.astype(np.float32) * (np.float32(1) - alpha) + np.float32(src) * alpha
    return np.floor(out.astype(np.float64) + 0.5).astype(np.int64)


@candidate("a8 by mul8, mul8 blend")
def mul8_blend(src, dst, a, o, f):
    a8 = mul8(mul8(a, o), f)
    return mul8(src, a8) + mul8(dst, 255 - a8)


@candidate("a8 by mul8, lerp div255")
def mul8_lerp(src, dst, a, o, f):
    a8 = mul8(mul8(a, o), f)
    return div255_round(src * a8 + dst * (255 - a8))


@candidate("a8 rounded, lerp div255")
def round_lerp(src, dst, a, o, f):
    a8 = div255_round(div255_round(a * o) * f)
    return div255_round(src * a8 + dst * (255 - a8))


@candidate("fill first, then opacity")
def fill_first(src, dst, a, o, f):
    a8 = div255_round(div255_round(a * f) * o)
    return div255_round(src * a8 + dst * (255 - a8))


@candidate("opacity times fill first")
def combined_first(src, dst, a, o, f):
    a8 = div255_round(a * div255_round(o * f))
    return div255_round(src * a8 + dst * (255 - a8))


@candidate("fill blend, then opacity blend")
def two_stage(src, dst, a, o, f):
    inner = div255_round(src * div255_round(a * f) + dst * (255 - div255_round(a * f)))
    return div255_round(inner * o + dst * (255 - o))


@candidate("dst + mul8(src - dst)")
def dst_plus(src, dst, a, o, f):
    a8 = mul8(mul8(a, o), f)
    diff = src - dst
    return dst + np.sign(diff) * mul8(np.abs(diff), a8)


def lerp8(dst, src, a):
    return div255_round(src * a + dst * (255 - a))


def fit_clear(folder, cases, xs, ys):
    """The clear document: a backdrop of one color, alpha the column, under
    a source of one color, alpha the row, over nothing; the composite's
    color is stored matted with white."""
    case = next(c for c in cases if c["name"] == "clear")
    planes = stored_composite(os.path.join(folder, "photoshop", "clear.psd"))
    back, source = case["layers"][0]["color"], case["layers"][1]["color"]
    alphas = {
        "union rounded": xs + ys - div255_round(xs * ys),
        "union by mul8": xs + ys - mul8(xs, ys),
        "union of complements": 255 - div255_round((255 - xs) * (255 - ys)),
    }
    for name, alpha in alphas.items():
        diff = np.abs(alpha - planes[3])
        print(f"clear alpha, {name}: {int((diff > 0).sum())} off, worst {int(diff.max())}", flush=True)
    result = xs + ys - div255_round(xs * ys)
    safe = np.maximum(result, 1)

    def shared(share, c):
        # The source's share of the result alpha, the colors mixed by it,
        # then matted with white by the result alpha.
        mixed = lerp8(back[c], source[c], share)
        return np.where(result > 0, lerp8(255, mixed, result), 255)

    def exact_then_matte(c):
        mixed = (source[c] * ys + back[c] * xs * (255 - ys) / 255.0) / safe
        mixed = np.floor(mixed + 0.5).astype(np.int64)
        return np.where(result > 0, lerp8(255, mixed, result), 255)

    colors = {
        "both onto white": lambda c: lerp8(lerp8(255, back[c], xs), source[c], ys),
        "float onto white": lambda c: np.floor(255 + (back[c] - 255) * xs / 255.0 * (1 - ys / 255.0)
                                               + (source[c] - 255) * ys / 255.0 + 0.5).astype(np.int64),
        "share rounded": lambda c: shared(np.minimum((ys * 255 + safe // 2) // safe, 255), c),
        "share cut down": lambda c: shared(np.minimum((ys * 255) // safe, 255), c),
        "share cut up": lambda c: shared(np.minimum(-((-ys * 255) // safe), 255), c),
        "exact color, then matte": exact_then_matte,
    }
    for name, f in colors.items():
        misses, worst = 0, 0
        for c in range(3):
            diff = np.abs(f(c) - planes[c])
            misses += int((diff > 0).sum())
            worst = max(worst, int(diff.max()))
        print(f"clear color, {name}: {misses} off, worst {worst}", flush=True)


def main(folder):
    with open(os.path.join(folder, "written", "cases.json"), encoding="utf-8") as f:
        cases = json.load(f)
    xs = np.tile(np.arange(SIZE, dtype=np.int64), (SIZE, 1))
    ys = np.tile(np.arange(SIZE, dtype=np.int64)[:, None], (1, SIZE))
    fit_clear(folder, cases, xs, ys)
    for case in cases:
        name = case["name"]
        if name == "clear":
            continue
        planes = stored_composite(os.path.join(folder, "photoshop", name + ".psd"))
        source = case["layers"][1]
        line = []
        for cname, f in CANDIDATES.items():
            misses, worst = 0, 0
            for c in range(3):
                got = f(source["color"][c], xs, ys, source["opacity"], source["fill"])
                diff = np.abs(got - planes[c])
                misses += int((diff > 0).sum())
                worst = max(worst, int(diff.max()))
            line.append(f"{cname}: {misses} off, worst {worst}")
        alpha_note = f"alpha min {int(planes[3].min())}" if planes.shape[0] > 3 else "no alpha"
        print(f"{name} ({alpha_note}): " + " | ".join(line), flush=True)
    print("FIT DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
