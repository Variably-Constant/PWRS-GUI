"""Shows where darker color and lighter color miss the whole-color pick by
0.3/0.59/0.11 luminance (out-composite3): per missed pixel the source, the
backdrop, Photoshop's result, and both luminances; then how many misses
Photoshop's result equals the source, the backdrop, or neither.

    python -u -I ps_darker_misses.py <batch folder>

Ends with `MISSES DONE`.
"""

import os
import sys

import numpy as np
from psd_tools import PSDImage

SIZE = 256


def colors(seed):
    return np.random.default_rng(seed).integers(0, 256, (SIZE, SIZE, 3), dtype=np.uint8).astype(np.int64)


def composite(path):
    psd = PSDImage.open(path)
    header = psd._record.header
    planes = psd._record.image_data.get_data(header)
    return np.dstack([np.frombuffer(p, dtype=np.uint8).reshape(SIZE, SIZE) for p in planes[:3]]).astype(np.int64)


def main(folder):
    back, source = colors(1), colors(2)
    for mode in ("darker_color", "lighter_color"):
        got = composite(os.path.join(folder, "photoshop", f"mode-{mode}.psd"))
        ls = source @ np.array([30, 59, 11])
        lb = back @ np.array([30, 59, 11])
        forms = {
            "Rec 601 299/587/114, half up": lambda c: (c @ np.array([299, 587, 114]) + 500) // 1000,
            "Rec 601 fixed 19595/38470/7471 >> 16": lambda c: (c @ np.array([19595, 38470, 7471]) + 32768) >> 16,
            "30/59/11 times 256, >> 8": lambda c: (c @ np.array([77, 151, 28]) + 128) >> 8,
            "30/59/11 in 1/65536": lambda c: (c @ np.array([19661, 38666, 7209]) + 32768) >> 16,
            "30/59/11 in 1/32768": lambda c: (c @ np.array([9830, 19333, 3604]) + 16384) >> 15,
        }
        for fname, level in forms.items():
            pick_f = (level(source) < level(back)) if mode == "darker_color" else (level(source) > level(back))
            out_f = np.where(pick_f[..., None], source, back)
            print(f"{mode}, luminance {fname}, backdrop on ties: {int((out_f != got).any(axis=-1).sum())} pixels off", flush=True)
        for kind in ("float64", "float32"):
            t = np.float64 if kind == "float64" else np.float32
            w = [t(0.3), t(0.59), t(0.11)]
            level = lambda c: np.floor((c[..., 0].astype(t) * w[0] + c[..., 1].astype(t) * w[1] + c[..., 2].astype(t) * w[2]).astype(np.float64) + 0.5)  # noqa: E731
            pick_k = (level(source) < level(back)) if mode == "darker_color" else (level(source) > level(back))
            out_k = np.where(pick_k[..., None], source, back)
            print(f"{mode}, luminance {kind} 0.3/0.59/0.11 rounded half up, backdrop on ties: "
                  f"{int((out_k != got).any(axis=-1).sum())} pixels off", flush=True)
        ls8, lb8 = (ls + 49) // 100, (lb + 49) // 100
        pick = (ls8 < lb8) if mode == "darker_color" else (ls8 > lb8)
        model = np.where(pick[..., None], source, back)
        missed = np.argwhere((model != got).any(axis=-1))
        is_source = sum(1 for y, x in missed if (got[y, x] == source[y, x]).all())
        is_back = sum(1 for y, x in missed if (got[y, x] == back[y, x]).all())
        print(f"{mode}: {len(missed)} pixels missed; Photoshop gave the source at {is_source}, the backdrop at "
              f"{is_back}, neither at {len(missed) - is_source - is_back}", flush=True)
        for y, x in missed[:8]:
            print(f"  ({x},{y}) source {source[y, x].tolist()} lum {ls[y, x]} | backdrop {back[y, x].tolist()} "
                  f"lum {lb[y, x]} | photoshop {got[y, x].tolist()}", flush=True)
    print("MISSES DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
