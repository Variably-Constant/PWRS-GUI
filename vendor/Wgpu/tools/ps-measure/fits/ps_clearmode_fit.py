"""Fits how a blend mode composites over a half-transparent backdrop: the
`clearmodes` batch (out-composite4: backdrop level = column at alpha 128,
source level = row at alpha 255 or 128) against models built on Photoshop's
own opaque mode tables (out-composite2), comparing the stored composite
(color matted with white, and alpha).

    python -u -I ps_clearmode_fit.py <root folder holding out-composite2 and out-composite4>
    python -u -I ps_clearmode_fit.py <root folder holding out-composite2 and out-composite5> random

Ends with `CLEARMODE DONE`. With `random`, fits the `randmodes` batch
instead (random colors and alphas on both layers) and ends with
`RANDOM DONE`.
"""

import os
import sys

import numpy as np
from psd_tools import PSDImage

SIZE = 256
S = np.tile(np.arange(SIZE, dtype=np.int64)[:, None], (1, SIZE))
D = np.tile(np.arange(SIZE, dtype=np.int64), (SIZE, 1))
MODES = ["darken", "multiply", "color_burn", "linear_burn", "lighten", "screen", "color_dodge", "linear_dodge",
         "overlay", "soft_light", "hard_light", "vivid_light", "linear_light", "pin_light", "hard_mix",
         "difference", "exclusion", "subtract", "divide"]


def r(v):
    return (v + 127) // 255


def planes(path):
    psd = PSDImage.open(path)
    header = psd._record.header
    return [np.frombuffer(c, dtype=np.uint8).reshape(SIZE, SIZE).astype(np.int64)
            for c in psd._record.image_data.get_data(header)]


def composite(source_color, a, b):
    """The normal rule over a backdrop of level D at alpha b, the source of
    color source_color at alpha a: color matted with white, and alpha."""
    t = b + a - r(b * a)
    w = np.minimum((255 * a + t // 2) // np.maximum(t, 1), 255)
    c = r(source_color * w + D * (255 - w))
    return np.where(t > 0, r(c * t + 255 * (255 - t)), 255), t


def main(root):
    b = 128
    totals = {}
    for mode in MODES:
        table = planes(os.path.join(root, "out-composite2", "photoshop", f"mode-{mode}.psd"))[0]
        for a, suffix in ((255, ""), (128, "-a128")):
            got = planes(os.path.join(root, "out-composite4", "photoshop", f"mode-{mode}{suffix}.psd"))
            # Five planes: color, transparency, and the documents' extra
            # alpha channel; four: an opaque composite, no transparency.
            stored_alpha = got[3] if len(got) == 5 else np.full((SIZE, SIZE), 255, dtype=np.int64)
            got = [got[0], got[1], got[2], stored_alpha]
            models = {
                "pdf: B mixed with s by backdrop alpha": r(table * b + S * (255 - b)),
                "B as the source": table,
            }
            line = []
            # One rounding: the premultiplied sum of the source alone, the
            # mode's result and the backdrop alone, over the result alpha.
            t1 = b + a - r(b * a)
            single = (2 * (S * a * (255 - b) + table * a * b + D * b * (255 - a)) + 255 * t1) // (2 * 255 * np.maximum(t1, 1))
            m1 = np.where(t1 > 0, r(single * t1 + 255 * (255 - t1)), 255)
            off1 = int((np.abs(m1 - got[0]) > 0).sum()) + int((np.abs(t1 - got[3]) > 0).sum())
            totals["single rounding"] = totals.get("single rounding", 0) + off1
            line.append(f"single rounding: {off1} off, worst {int(np.abs(m1 - got[0]).max())}")
            # The same PDF steps on Photoshop's 15-bit scale (0 to 32768),
            # each level widened, then the color brought back to 8 bits.
            def wide(v):
                return (v * 32768 + 127) // 255

            def mul15(x, y):
                return (x * y + 16384) >> 15

            a15, b15 = wide(a), wide(b)
            t15 = b15 + a15 - mul15(b15, a15)
            w15 = (a15 * 32768 + t15 // 2) // t15
            mixed15 = mul15(wide(table), b15) + mul15(wide(S), 32768 - b15)
            c15 = mul15(mixed15, w15) + mul15(wide(D), 32768 - w15)
            c8 = (c15 * 255 + 16384) >> 15
            t8 = b + a - r(b * a)
            m4 = np.where(t8 > 0, r(c8 * t8 + 255 * (255 - t8)), 255)
            off4 = int((np.abs(m4 - got[0]) > 0).sum()) + int((np.abs(t8 - got[3]) > 0).sum())
            totals["15-bit steps"] = totals.get("15-bit steps", 0) + off4
            line.append(f"15-bit steps: {off4} off")
            # Premultiplied in levels: the source alone, the backdrop alone
            # and the mode's result over the overlap, then divided by the
            # result alpha.
            t3 = b + a - r(b * a)
            premultiplied = r(r(S * a) * (255 - b)) + r(r(D * b) * (255 - a)) + r(table * r(a * b))
            for dname, divided in (("rounded", np.minimum((premultiplied * 255 + t3 // 2) // np.maximum(t3, 1), 255)),
                                   ("cut down", np.minimum((premultiplied * 255) // np.maximum(t3, 1), 255))):
                m3 = np.where(t3 > 0, r(divided * t3 + 255 * (255 - t3)), 255)
                off3 = int((np.abs(m3 - got[0]) > 0).sum()) + int((np.abs(t3 - got[3]) > 0).sum())
                key3 = f"premultiplied, {dname}"
                totals[key3] = totals.get(key3, 0) + off3
                line.append(f"{key3}: {off3} off")
            # The normal composite N, then the mode's difference B - s added
            # by the overlap's share of the result alpha, v = round(255 ab/t).
            t2 = b + a - r(b * a)
            w2 = np.minimum((255 * a + t2 // 2) // np.maximum(t2, 1), 255)
            normal = r(S * w2 + D * (255 - w2))
            for vname, overlap in (("r(ab)", r(a * b)), ("ab/255 exact", a * b / 255.0)):
                v = np.floor(255 * overlap / np.maximum(t2, 1) + 0.5).astype(np.int64)
                delta = (table - S) * v
                c2 = normal + np.sign(delta) * ((np.abs(delta) + 127) // 255)
                m2 = np.where(t2 > 0, r(c2 * t2 + 255 * (255 - t2)), 255)
                off2 = int((np.abs(m2 - got[0]) > 0).sum()) + int((np.abs(t2 - got[3]) > 0).sum())
                key = f"normal then overlap {vname}"
                totals[key] = totals.get(key, 0) + off2
                line.append(f"{key}: {off2} off")
            # The result over an opaque backdrop first, the mode's result
            # mixed into the backdrop by the source's alpha; then the part of
            # the source over nothing, a - r(ab) of the result alpha t, mixed
            # in by its share.
            t5 = b + a - r(b * a)
            x5 = r(table * a + D * (255 - a))
            for uname, u5 in (("source share", np.minimum((255 * (t5 - b) + t5 // 2) // np.maximum(t5, 1), 255)),
                              ("backdrop share", 255 - np.minimum((255 * b + t5 // 2) // np.maximum(t5, 1), 255))):
                c5 = r(S * u5 + x5 * (255 - u5))
                m5 = np.where(t5 > 0, r(c5 * t5 + 255 * (255 - t5)), 255)
                off5 = int((np.abs(m5 - got[0]) > 0).sum()) + int((np.abs(t5 - got[3]) > 0).sum())
                key5 = f"opaque result first, {uname}"
                totals[key5] = totals.get(key5, 0) + off5
                line.append(f"{key5}: {off5} off")
            for name, color in models.items():
                m, t = composite(color, a, b)
                off = int((np.abs(m - got[0]) > 0).sum()) + int((np.abs(t - got[3]) > 0).sum())
                worst = max(int(np.abs(m - got[0]).max()), int(np.abs(t - got[3]).max()))
                totals[name] = totals.get(name, 0) + off
                line.append(f"{name}: {off} off, worst {worst}")
            print(f"{mode}{suffix}: " + " | ".join(line), flush=True)
    print("totals: " + " | ".join(f"{k}: {v}" for k, v in totals.items()), flush=True)
    print("CLEARMODE DONE", flush=True)


def random_fit(root):
    """Fits normal and every mode of the `randmodes` batch (out-composite5):
    random colors and alphas on both layers, regenerated from the batch's
    seeds, the mode's result read from Photoshop's own opaque tables
    (out-composite2); normal's result is the source."""
    d = np.random.default_rng(1).integers(0, 256, (SIZE, SIZE, 3), dtype=np.uint8).astype(np.int64)
    s = np.random.default_rng(2).integers(0, 256, (SIZE, SIZE, 3), dtype=np.uint8).astype(np.int64)
    b = np.random.default_rng(3).integers(0, 256, (SIZE, SIZE), dtype=np.uint8).astype(np.int64)[..., None]
    a = np.random.default_rng(4).integers(0, 256, (SIZE, SIZE), dtype=np.uint8).astype(np.int64)[..., None]
    t = b + a - r(b * a)
    tt = np.maximum(t, 1)
    w = np.minimum((255 * a + tt // 2) // tt, 255)
    u = np.minimum((255 * (t - b) + tt // 2) // tt, 255)
    v = np.minimum((255 * b + tt // 2) // tt, 255)
    totals = {}
    for mode in ["normal"] + MODES:
        got = planes(os.path.join(root, "out-composite5", "photoshop", f"mode-{mode}.psd"))
        stored_alpha = got[3] if len(got) == 5 else np.full((SIZE, SIZE), 255, dtype=np.int64)
        color = np.dstack(got[:3])
        if mode == "normal":
            blended = s
        else:
            blended = planes(os.path.join(root, "out-composite2", "photoshop", f"mode-{mode}.psd"))[0][s, d]
        x = r(blended * a + d * (255 - a))
        models = {
            "pdf": r(r(blended * b + s * (255 - b)) * w + d * (255 - w)),
            "opaque first, source share": r(s * u + x * (255 - u)),
            "opaque first, backdrop share": r(x * v + s * (255 - v)),
        }
        line = [f"alpha: {int((t[..., 0] != stored_alpha).sum())} off"]
        for name, c in models.items():
            m = np.where(t > 0, r(c * t + 255 * (255 - t)), 255)
            off = int((m != color).sum())
            totals[name] = totals.get(name, 0) + off
            line.append(f"{name}: {off} off, worst {int(np.abs(m - color).max())}")
        print(f"{mode}: " + " | ".join(line), flush=True)
    print("totals: " + " | ".join(f"{k}: {v}" for k, v in totals.items()), flush=True)
    print("RANDOM DONE", flush=True)


SPECIAL = ["color_burn", "linear_burn", "color_dodge", "linear_dodge", "vivid_light", "linear_light", "hard_mix", "difference"]


def random_fill_fit(root):
    """Fits the `randfill` batch (out-composite8): the `randmodes` layers
    with the source at opacity 200 and fill 77. The special eight's result
    with fill inside comes from Photoshop's own fill-77 opaque tables
    (out-composite6), then composites in normal's order with the source's
    alpha scaled by opacity alone, or by opacity and fill; multiply and
    screen, from out-composite2's tables, composite in the other order with
    fill acting as opacity, and normal by its rule."""
    opacity, fill = 200, 77
    d = np.random.default_rng(1).integers(0, 256, (SIZE, SIZE, 3), dtype=np.uint8).astype(np.int64)
    s = np.random.default_rng(2).integers(0, 256, (SIZE, SIZE, 3), dtype=np.uint8).astype(np.int64)
    b = np.random.default_rng(3).integers(0, 256, (SIZE, SIZE), dtype=np.uint8).astype(np.int64)[..., None]
    alpha = np.random.default_rng(4).integers(0, 256, (SIZE, SIZE), dtype=np.uint8).astype(np.int64)[..., None]
    for mode in ["normal", "multiply", "screen"] + SPECIAL:
        got = planes(os.path.join(root, "out-composite8", "photoshop", f"mode-{mode}.psd"))
        stored_alpha = got[3] if len(got) == 5 else np.full((SIZE, SIZE), 255, dtype=np.int64)
        color = np.dstack(got[:3])
        models = {}
        if mode in SPECIAL:
            blended = planes(os.path.join(root, "out-composite6", "photoshop", f"mode-{mode}-f77.psd"))[0][s, d]
            for name, k in (("alpha by opacity", opacity), ("alpha by opacity and fill", r(opacity * fill))):
                a = r(alpha * k)
                t = b + a - r(b * a)
                tt = np.maximum(t, 1)
                w = np.minimum((255 * a + tt // 2) // tt, 255)
                models[name] = (r(r(blended * b + s * (255 - b)) * w + d * (255 - w)), t)
            # Two alphas: the result mixes into the backdrop by the alpha
            # scaled by opacity, the source over nothing covers the alpha
            # scaled by opacity and fill, rounded in one of three orders.
            a_o = r(alpha * opacity)
            for name, a_of in (("k first", r(alpha * r(opacity * fill))), ("opacity then fill", r(r(alpha * opacity) * fill)),
                               ("fill then opacity", r(r(alpha * fill) * opacity))):
                t = b + a_of - r(b * a_of)
                tt = np.maximum(t, 1)
                u = np.minimum((255 * (t - b) + tt // 2) // tt, 255)
                x = r(blended * a_o + d * (255 - a_o))
                models[f"two alphas, {name}"] = (r(s * u + x * (255 - u)), t)
                # Normal's order with the two coverages: the mode's result
                # over the overlap r(a_o b) and the source over nothing
                # c - r(c b) mix first, then into the backdrop by their
                # share of t.
                overlap = r(a_o * b)
                covered = overlap + a_of - r(a_of * b)
                safe = np.maximum(covered, 1)
                v = np.minimum((255 * overlap + safe // 2) // safe, 255)
                share = np.minimum((255 * covered + tt // 2) // tt, 255)
                models[f"normal's order, two alphas, {name}"] = (r(r(blended * v + s * (255 - v)) * share + d * (255 - share)), t)
        else:
            a = r(alpha * r(opacity * fill))
            t = b + a - r(b * a)
            tt = np.maximum(t, 1)
            if mode == "normal":
                w = np.minimum((255 * a + tt // 2) // tt, 255)
                models["normal rule"] = (r(s * w + d * (255 - w)), t)
            else:
                blended = planes(os.path.join(root, "out-composite2", "photoshop", f"mode-{mode}.psd"))[0][s, d]
                u = np.minimum((255 * (t - b) + tt // 2) // tt, 255)
                x = r(blended * a + d * (255 - a))
                models["result into backdrop first"] = (r(s * u + x * (255 - u)), t)
        line = []
        for name, (c, t) in models.items():
            m = np.where(t > 0, r(c * t + 255 * (255 - t)), 255)
            line.append(f"{name}: color {int((m != color).sum())} off, worst {int(np.abs(m - color).max())}, "
                        f"alpha {int((t[..., 0] != stored_alpha).sum())} off")
        print(f"{mode}: " + " | ".join(line), flush=True)
        if mode == "color_burn":
            alpha_fit(alpha[..., 0], b[..., 0], stored_alpha, opacity, fill)
    print("RANDOM FILL DONE", flush=True)


def alpha_fit(a, b, got, opacity, fill):
    """Fits the result alpha of a special-eight layer at `opacity` and
    `fill` over a partly transparent backdrop: candidates for the alpha
    the source covers over nothing, then for the union, each with its
    misses, and the misses' (a, b, Photoshop, candidate) for the best."""
    a_o = r(a * opacity)
    alone = {
        "r(a r(o f))": r(a * r(opacity * fill)),
        "r(r(a o) f)": r(a_o * fill),
        "r(r(a f) o)": r(r(a * fill) * opacity),
        "a o f / 65025 rounded": (2 * a * opacity * fill + 65025) // (2 * 65025),
    }
    zero = b == 0
    for name, cover in alone.items():
        print(f"    over nothing, {name}: {int((cover[zero] != got[zero]).sum())} of {int(zero.sum())} off", flush=True)
    unions = {
        "b + c - r(b c), c = r(r(a o) f)": b + r(a_o * fill) - r(b * r(a_o * fill)),
        "b + r((a_o - r(a_o b)) f)": b + r((a_o - r(a_o * b)) * fill),
        "b + r(r(a_o (255 - b)) f)": b + r(r(a_o * (255 - b)) * fill),
        "b + a_o (255 - b) f / 65025 rounded": b + (2 * a_o * (255 - b) * fill + 65025) // (2 * 65025),
        "r(t_o f + b (255 - f)), t_o = b + a_o - r(b a_o)": r((b + a_o - r(b * a_o)) * fill + b * (255 - fill)),
    }
    scores = []
    for name, t in unions.items():
        scores.append((int((t != got).sum()), name))
        print(f"    union, {name}: {scores[-1][0]} off", flush=True)
    best = unions[min(scores)[1]]
    misses = np.flatnonzero(best != got)[:8]
    print("    best's misses (a, b, photoshop, candidate): " + ", ".join(
        f"({int(a.flat[i])}, {int(b.flat[i])}, {int(got.flat[i])}, {int(best.flat[i])})" for i in misses), flush=True)


def fill_alpha_fit(root):
    """Fits the result alpha of the `fillalpha` batch (out-composite9): a
    special-eight source whose alpha is the row's number at opacity o and
    fill f, over a backdrop whose alpha is the column's number; candidates
    for the union, each one's misses, and Photoshop's alpha less the best
    candidate's along a few rows."""
    a = S
    b = D
    for name in sorted(os.listdir(os.path.join(root, "out-composite9", "photoshop"))):
        opacity, fill = (int(part[2:]) if part.startswith("op") else int(part[1:])
                         for part in name[:-4].split("-")[-2:])
        got = planes(os.path.join(root, "out-composite9", "photoshop", name))
        stored = got[3] if len(got) == 5 else np.full((SIZE, SIZE), 255, dtype=np.int64)
        a_o = r(a * opacity)
        c = r(a_o * fill)
        t_o = b + a_o - r(b * a_o)
        unions = {
            "b + c - r(b c)": b + c - r(b * c),
            "b + r((a_o - r(a_o b)) f)": b + r((a_o - r(a_o * b)) * fill),
            "r(t_o f + b (255 - f))": r(t_o * fill + b * (255 - fill)),
            "t_o - r((t_o - b)(255 - f))": t_o - r((t_o - b) * (255 - fill)),
            "r(t_o f) + r(b (255 - f))": r(t_o * fill) + r(b * (255 - fill)),
        }
        scores = sorted((int((t != stored).sum()), key) for key, t in unions.items())
        print(f"{name}: " + " | ".join(f"{key}: {off} off" for off, key in scores), flush=True)
        # Whether the stored alpha is a function of each key alone: the keys
        # that give one alpha to two pixels with different alphas.
        keys = {
            "(a, b)": a * 256 + b,
            "(b, t_o)": b * 512 + t_o,
            "(b, c)": b * 512 + c,
            "(b, a_o)": b * 512 + a_o,
            "(b, a_o - r(a_o b))": b * 512 + (a_o - r(a_o * b)),
            "(t_o, c)": t_o * 512 + c,
            "(b >> 1, t_o)": (b >> 1) * 512 + t_o,
        }
        clashes = []
        for key_name, key in keys.items():
            flat_key, flat_alpha = key.ravel(), stored.ravel()
            order = np.lexsort((flat_alpha, flat_key))
            k_sorted, t_sorted = flat_key[order], flat_alpha[order]
            same_key = k_sorted[1:] == k_sorted[:-1]
            clashes.append(f"{key_name}: {int((same_key & (t_sorted[1:] != t_sorted[:-1])).sum())}")
        print("    keys whose pixels disagree: " + ", ".join(clashes), flush=True)
        if opacity == 255:
            for row in (64, 128):
                exact = b[row] + row * fill * (255 - b[row]) / 65025
                print(f"    a {row}, b 0..15, photoshop less b (exact less b): " + " ".join(
                    f"{int(stored[row, col] - col)}({float(exact[col] - col):.2f})" for col in range(16)), flush=True)
        best = unions[scores[0][1]]
        for row in (64, 128, 200, 255):
            diff = stored[row] - best[row]
            cols = np.flatnonzero(diff)
            if cols.size:
                print(f"    a {row}: {cols.size} off, at b {int(cols[0])}..{int(cols[-1])}, values {sorted(set(int(v) for v in diff[cols]))}, "
                      f"first b {', '.join(str(int(x)) for x in cols[:8])}", flush=True)
    print("FILL ALPHA DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[2] == "random":
        random_fit(sys.argv[1])
    elif len(sys.argv) == 3 and sys.argv[2] == "fillalpha":
        fill_alpha_fit(sys.argv[1])
    elif len(sys.argv) == 3 and sys.argv[2] == "randfill":
        random_fill_fit(sys.argv[1])
    elif len(sys.argv) == 2:
        main(sys.argv[1])
    else:
        raise SystemExit(__doc__)
