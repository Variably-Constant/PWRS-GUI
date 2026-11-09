"""Fits each separable blend mode's 8-bit arithmetic to the tables
Photoshop composited for ps_composite_batch.py's `modes` batch: for every
mode, the source level s (the row) over the backdrop level d (the column),
each candidate's misses and worst miss over all 65536 pairs; then for the
alpha-128 documents, whether the mode's opaque result mixes into the
backdrop by the normal rule, r(B a + d (255 - a)).

    python -u -I ps_blend_fit.py <batch folder>
    python -u -I ps_blend_fit.py <modes batch folder> fill <fillmodes batch folder>

Ends with `BLEND FIT DONE`. With `fill`, fits the `fillmodes` documents
against the `modes` batch's opaque tables and ends with `FILL FIT DONE`.
"""

import os
import sys

import numpy as np
from psd_tools import PSDImage

SIZE = 256
S = np.tile(np.arange(SIZE, dtype=np.int64)[:, None], (1, SIZE))
D = np.tile(np.arange(SIZE, dtype=np.int64), (SIZE, 1))


def r(v):
    return (v + 127) // 255


def div_round(a, b):
    """a / b rounded half up, b > 0, a >= 0."""
    return (2 * a + b) // (2 * b)


def symmetric_r(v):
    """A product of levels over 255, rounded half away from zero."""
    return np.sign(v) * r(np.abs(v))


def table(path):
    psd = PSDImage.open(path)
    header = psd._record.header
    channels = psd._record.image_data.get_data(header)
    planes = [np.frombuffer(c, dtype=np.uint8).reshape(header.height, header.width).astype(np.int64) for c in channels]
    if not (np.array_equal(planes[0], planes[1]) and np.array_equal(planes[0], planes[2])):
        print(f"  {os.path.basename(path)}: channels differ", flush=True)
    return planes[0]


def burn(s, d, rounding):
    q = (255 - d) * 255
    safe = np.maximum(s, 1)
    v = div_round(q, safe) if rounding == "round" else q // safe if rounding == "floor" else -((-q) // safe)
    return np.where(d == 255, 255, np.where(s == 0, 0, np.maximum(0, 255 - v)))


def dodge(s, d, rounding):
    q = d * 255
    safe = np.maximum(255 - s, 1)
    v = div_round(q, safe) if rounding == "round" else q // safe if rounding == "floor" else -((-q) // safe)
    return np.where(d == 0, 0, np.where(s == 255, 255, np.minimum(255, v)))


def soft_light_float(s, d):
    sf, df = s / 255.0, d / 255.0
    low = 2 * sf * df + df * df * (1 - 2 * sf)
    high = 2 * df * (1 - sf) + np.sqrt(df) * (2 * sf - 1)
    return np.floor(np.where(sf < 0.5, low, high) * 255 + 0.5).astype(np.int64)


def soft_light_w3c(s, d):
    sf, df = s / 255.0, d / 255.0
    g = np.where(df <= 0.25, ((16 * df - 12) * df + 4) * df, np.sqrt(df))
    out = np.where(sf <= 0.5, df - (1 - 2 * sf) * df * (1 - df), df + (2 * sf - 1) * (g - df))
    return np.floor(out * 255 + 0.5).astype(np.int64)


CANDIDATES = {
    "darken": {"min": lambda s, d: np.minimum(s, d)},
    "lighten": {"max": lambda s, d: np.maximum(s, d)},
    "multiply": {"r(sd)": lambda s, d: r(s * d), "floor(sd/255)": lambda s, d: (s * d) // 255},
    "screen": {"s+d-r(sd)": lambda s, d: s + d - r(s * d), "255-r((255-s)(255-d))": lambda s, d: 255 - r((255 - s) * (255 - d))},
    "linear_burn": {"max(0,s+d-255)": lambda s, d: np.maximum(0, s + d - 255)},
    "linear_dodge": {"min(255,s+d)": lambda s, d: np.minimum(255, s + d)},
    "difference": {"|s-d|": lambda s, d: np.abs(s - d)},
    "exclusion": {"s+d-2r(sd)": lambda s, d: s + d - 2 * r(s * d), "s+d-r(2sd)": lambda s, d: s + d - r(2 * s * d)},
    "subtract": {"max(0,d-s)": lambda s, d: np.maximum(0, d - s)},
    "divide": {
        "round": lambda s, d: np.where(d == 0, 0, np.where(s == 0, 255, np.minimum(255, div_round(d * 255, np.maximum(s, 1))))),
        "floor": lambda s, d: np.where(d == 0, 0, np.where(s == 0, 255, np.minimum(255, (d * 255) // np.maximum(s, 1)))),
    },
    "color_burn": {k: (lambda s, d, k=k: burn(s, d, k)) for k in ("round", "floor", "ceil")},
    "color_dodge": {k: (lambda s, d, k=k: dodge(s, d, k)) for k in ("round", "floor", "ceil")},
    "overlay": {
        "d<128": lambda s, d: np.where(d < 128, r(2 * s * d), 255 - r(2 * (255 - s) * (255 - d))),
        "d<=128": lambda s, d: np.where(d <= 128, r(2 * s * d), 255 - r(2 * (255 - s) * (255 - d))),
    },
    "hard_light": {
        "s<128": lambda s, d: np.where(s < 128, r(2 * s * d), 255 - r(2 * (255 - s) * (255 - d))),
        "upper 511-2s": lambda s, d: np.where(s < 128, r(2 * s * d), 255 - r((511 - 2 * s) * (255 - d))),
    },
    "soft_light": {"photoshop float": soft_light_float, "w3c float": soft_light_w3c},
    "vivid_light": {
        "upper (s-128)*255/127": lambda s, d: np.where(
            s < 128,
            np.where((510 * s + 128) >> 8 == 0, 0,
                     np.maximum(0, 255 - div_round((255 - d) * 255, np.maximum((510 * s + 128) >> 8, 1)))),
            np.where(div_round(np.maximum(s - 128, 0) * 255, 127) >= 255, 255, np.where(d == 0, 0,
                     np.minimum(255, div_round(d * 255, np.maximum(255 - div_round(np.maximum(s - 128, 0) * 255, 127), 1)))))),
        "upper split by d's half": lambda s, d: np.where(
            s < 128,
            np.where((510 * s + 128) >> 8 == 0, 0,
                     np.maximum(0, 255 - div_round((255 - d) * 255, np.maximum((510 * s + 128) >> 8, 1)))),
            np.where(d == 0, 0, np.minimum(255, div_round(d * 255, np.maximum(255 - np.where(d < 128, 2 * s - 255, 2 * s - 256), 1))))),
        "lower 510s/256, upper 2(s-128)": lambda s, d: np.where(
            s < 128,
            np.where((510 * s + 128) >> 8 == 0, 0,
                     np.maximum(0, 255 - div_round((255 - d) * 255, np.maximum((510 * s + 128) >> 8, 1)))),
            np.where(d == 0, 0, np.where(2 * (s - 128) >= 255, 255,
                     np.minimum(255, div_round(d * 255, np.maximum(255 - 2 * (s - 128), 1)))))),
        "lower 510s/256, upper 510(s-128)/256": lambda s, d: np.where(
            s < 128,
            np.where((510 * s + 128) >> 8 == 0, 0,
                     np.maximum(0, 255 - div_round((255 - d) * 255, np.maximum((510 * s + 128) >> 8, 1)))),
            np.where(d == 0, 0, np.where(((510 * (s - 128) + 128) >> 8) >= 255, 255,
                     np.minimum(255, div_round(d * 255, np.maximum(255 - ((510 * (s - 128) + 128) >> 8), 1)))))),
        "doubled by 510/256": lambda s, d: np.where(
            s < 128,
            np.where((510 * s + 128) >> 8 == 0, 0,
                     np.maximum(0, 255 - div_round((255 - d) * 255, np.maximum((510 * s + 128) >> 8, 1)))),
            np.where(d == 0, 0, np.where((510 * (255 - s) + 128) >> 8 == 0, 255,
                     np.minimum(255, div_round(d * 255, np.maximum((510 * (255 - s) + 128) >> 8, 1)))))),
        "burn 2s-1, dodge 2s-255": lambda s, d: np.where(
            s < 128,
            np.where(s == 0, 0, np.where(d == 255, 255, np.maximum(0, 255 - div_round((255 - d) * 255, np.maximum(2 * s - 1, 1))))),
            np.where(d == 0, 0, np.where(s == 255, 255, np.minimum(255, div_round(d * 255, np.maximum(255 - (2 * s - 255), 1)))))),
        "burn 2s-1 (s=0 first), dodge 2s-255": lambda s, d: np.where(
            s < 128,
            np.where(s == 0, 0, np.maximum(0, 255 - div_round((255 - d) * 255, np.maximum(2 * s - 1, 1)))),
            np.where(s == 255, 255, np.minimum(255, div_round(d * 255, np.maximum(510 - 2 * s, 1))))),
    },
    "linear_light": {"clamp(d+2s-255)": lambda s, d: np.clip(d + 2 * s - 255, 0, 255),
                     "clamp(d+2s-256)": lambda s, d: np.clip(d + 2 * s - 256, 0, 255)},
    "pin_light": {
        "s<128": lambda s, d: np.where(s < 128, np.minimum(d, 2 * s), np.maximum(d, 2 * s - 255)),
        "2s / 2s-256": lambda s, d: np.where(s < 128, np.minimum(d, 2 * s), np.maximum(d, 2 * s - 256)),
        "2s-1 / 2s-256": lambda s, d: np.where(s < 128, np.minimum(d, np.maximum(0, 2 * s - 1)), np.maximum(d, 2 * s - 256)),
        "d<128 decides": lambda s, d: np.where(d < 128, np.minimum(d, 2 * s), np.where(s < 128, np.minimum(d, 2 * s - 1), np.maximum(d, 2 * s - 256))),
        "offset by both halves": lambda s, d: np.where(
            s < 128,
            np.where(d < 128, np.minimum(d, 2 * s), np.minimum(d, np.maximum(0, 2 * s - 1))),
            np.where(d < 128, np.maximum(d, 2 * s - 256), np.maximum(d, 2 * s - 255))),
        "doubled by 510/256": lambda s, d: np.where(
            s < 128, np.minimum(d, (510 * s + 128) >> 8), np.maximum(d, 255 - ((510 * (255 - s) + 128) >> 8))),
        "lower 510s/256, upper 2(s-128)": lambda s, d: np.where(
            s < 128, np.minimum(d, (510 * s + 128) >> 8), np.maximum(d, 2 * (s - 128))),
        "upper split by d's half": lambda s, d: np.where(
            s < 128, np.minimum(d, (510 * s + 128) >> 8), np.maximum(d, np.where(d < 128, 2 * s - 256, 2 * s - 255))),
        "upper (s-128)*255/127": lambda s, d: np.where(
            s < 128, np.minimum(d, (510 * s + 128) >> 8), np.maximum(d, div_round(np.maximum(s - 128, 0) * 255, 127))),
        "lower 510s/256, upper 510(s-128)/256": lambda s, d: np.where(
            s < 128, np.minimum(d, (510 * s + 128) >> 8), np.maximum(d, (510 * (s - 128) + 128) >> 8)),
    },
    "hard_mix": {"s+d>=255": lambda s, d: np.where(s + d >= 255, 255, 0), "s+d>255": lambda s, d: np.where(s + d > 255, 255, 0),
                 "s<128: >=, else >": lambda s, d: np.where(np.where(s < 128, s + d >= 255, s + d > 255), 255, 0)},
}


def main(folder):
    for mode, candidates in CANDIDATES.items():
        got = table(os.path.join(folder, "photoshop", f"mode-{mode}.psd"))
        scores = []
        for name, f in candidates.items():
            diff = np.abs(f(S, D) - got)
            scores.append((int((diff > 0).sum()), int(diff.max()), name))
        scores.sort()
        best = scores[0]
        line = " | ".join(f"{n}: {m} off, worst {w}" for m, w, n in scores)
        print(f"{mode}: {line}", flush=True)
        if best[0] > 0:
            # Where the best candidate misses: per quadrant of s and d
            # (below 128 or from 128), the count and the mean of
            # Photoshop's level less the candidate's, and a few pairs.
            diff = got - candidates[best[2]](S, D)
            for s_hi in (False, True):
                for d_hi in (False, True):
                    q = ((S >= 128) == s_hi) & ((D >= 128) == d_hi) & (diff != 0)
                    if q.any():
                        print(f"    s{'>=' if s_hi else '<'}128 d{'>=' if d_hi else '<'}128: {int(q.sum())} off, "
                              f"mean {float(diff[q].mean()):+.2f}", flush=True)
            pairs = np.argwhere(diff != 0)[:: max(1, int((diff != 0).sum()) // 6)][:6]
            print("    pairs (s, d, photoshop, candidate): " + ", ".join(
                f"({s}, {d}, {int(got[s, d])}, {int(candidates[best[2]](S, D)[s, d])})" for s, d in pairs), flush=True)
        half = table(os.path.join(folder, "photoshop", f"mode-{mode}-a128.psd"))
        mixed = r(got * 128 + D * 127)
        diff = np.abs(mixed - half)
        print(f"  {mode} at alpha 128, Photoshop's own opaque result mixed by the normal rule: "
              f"{int((diff > 0).sum())} off, worst {int(diff.max())} (best formula {best[2]})", flush=True)
    print("BLEND FIT DONE", flush=True)


# The source level each mode leaves the backdrop unchanged at; hard mix has
# none.
NEUTRAL = {"color_burn": 255, "linear_burn": 255, "color_dodge": 0, "linear_dodge": 0, "difference": 0,
           "vivid_light": 128, "linear_light": 128}


# Each fill document's suffix, fill opacity and source alpha: the
# `fillmodes` batch's, then the `fillspecial` batch's.
FILL_CASES = (("-f128", 128, 255), ("-f77", 77, 255), ("-f128-a128", 128, 128), ("-f1", 1, 255), ("-f64", 64, 255),
              ("-f127", 127, 255), ("-f129", 129, 255), ("-f192", 192, 255), ("-f254", 254, 255))


def hard_mix_fill(g):
    """Hard mix at fill g below 255: linear burn at fill g stretched by
    255 / (255 - g), (255 d - g (255 - s)) / (255 - g) rounded, clamped."""
    return np.clip((2 * (255 * D - g * (255 - S)) + (255 - g)) // (2 * (255 - g)), 0, 255)


def linear_light_fill(g):
    """Linear light at fill g, each half's doubled source moved toward its
    own neutral: the burn half's 2s - 1 toward 255, the dodge half's
    2s - 256 toward 0."""
    return np.where(S < 128, np.maximum(0, D - g + symmetric_r((2 * S - 1) * g)), np.minimum(255, D + r((2 * S - 256) * g)))


def hard_mix_steps(burn_fill, stretch_fill):
    """Hard mix at fill in two whole-level steps: linear burn at
    `burn_fill`, d + r(s f) - f, then stretched by 255 / (255 - g) at
    `stretch_fill` g, rounded, clamped."""
    burned = np.maximum(0, D + r(S * burn_fill) - burn_fill)
    return np.minimum(255, div_round(burned * 255, 255 - stretch_fill))


def light_offsets(got):
    """Linear light's offset from the backdrop for every source level, read
    where the result is not clamped: at d 0 where it is above 0, else at d
    255."""
    at_zero, at_top = got[:, 0], got[:, 255] - 255
    return np.where(at_zero > 0, at_zero, at_top)


def where_misses(diff):
    """Prints where `diff`, Photoshop's level less a model's, is not 0: per
    half of s and of d the count and mean, the s and d each spans, and the
    differences along a few rows of s."""
    for s_hi in (False, True):
        for d_hi in (False, True):
            q = ((S >= 128) == s_hi) & ((D >= 128) == d_hi) & (diff != 0)
            if q.any():
                print(f"    s{'>=' if s_hi else '<'}128 d{'>=' if d_hi else '<'}128: {int(q.sum())} off, mean {float(diff[q].mean()):+.2f}, "
                      f"s {int(S[q].min())}..{int(S[q].max())}, d {int(D[q].min())}..{int(D[q].max())}", flush=True)
    for s in (0, 1, 64, 127, 128, 129, 192, 254, 255):
        row = np.flatnonzero(diff[s])
        if row.size:
            print(f"    s {s}: {row.size} off at d {int(row[0])}..{int(row[-1])}, values {sorted(set(int(v) for v in diff[s][row]))}", flush=True)


def best_fill(model, got, alpha):
    """The fill value 0 to 254 at which `model` misses `got` least, mixed by
    the source's alpha, and its misses."""
    return min((int((r(model(g) * alpha + D * (255 - alpha)) != got).sum()), g) for g in range(255))


def fill_fit(tables_folder, fills_folder):
    """Fits the `fillmodes` and `fillspecial` documents found in
    `fills_folder` (fill opacities over an opaque backdrop) against
    Photoshop's own opaque tables in `tables_folder` (the `modes` batch):
    fill acting as opacity, mixing the mode's result into the backdrop, and
    the source moved toward the mode's neutral level by fill before the
    mode; for hard mix and linear light, the fill value that fits best."""
    fitted = 0
    for mode in ["normal"] + list(CANDIDATES):
        opaque = S if mode == "normal" else table(os.path.join(tables_folder, "photoshop", f"mode-{mode}.psd"))
        for suffix, fill, alpha in FILL_CASES:
            path = os.path.join(fills_folder, "photoshop", f"mode-{mode}{suffix}.psd")
            if not os.path.exists(path):
                continue
            fitted += 1
            got = table(path)
            effective = r(alpha * fill)
            models = {"fill as opacity": r(opaque * effective + D * (255 - effective))}
            if mode in NEUTRAL:
                toward = r(S * fill + NEUTRAL[mode] * (255 - fill))
                models["source toward neutral"] = r(opaque[toward, D] * alpha + D * (255 - alpha))
            if mode == "vivid_light":
                # Each half's doubled source toward its own neutral: the burn
                # half's toward 255, the dodge half's toward 0.
                low = r(((510 * S + 128) >> 8) * fill + 255 * (255 - fill))
                high = r(div_round(np.maximum(S - 128, 0) * 255, 127) * fill)
                burned = np.maximum(0, 255 - div_round((255 - D) * 255, np.maximum(low, 1)))
                dodged = np.where(high >= 255, 255, np.where(D == 0, 0, np.minimum(255, div_round(D * 255, np.maximum(255 - high, 1)))))
                models["each half toward its neutral"] = r(np.where(S < 128, burned, dodged) * alpha + D * (255 - alpha))
            if mode == "linear_light":
                halves = np.where(S < 128, np.maximum(0, D - fill + symmetric_r((2 * S - 1) * fill)),
                                  np.minimum(255, D + r((2 * S - 256) * fill)))
                models["each half toward its neutral"] = r(halves * alpha + D * (255 - alpha))
                for name, doubled in (("2s-256", 2 * S - 256), ("2s-255", 2 * S - 255)):
                    models[f"d + ({name}) f"] = r(np.clip(D + symmetric_r(doubled * fill), 0, 255) * alpha + D * (255 - alpha))
                models["d + r((2s-255) f - 255)"] = r(np.clip(D + r((2 * S - 255) * fill - 255), 0, 255) * alpha + D * (255 - alpha))
            if mode == "hard_mix":
                models["stretched burn at the fill"] = r(hard_mix_fill(fill) * alpha + D * (255 - alpha))
            line = []
            for name, model in models.items():
                diff = np.abs(model - got)
                line.append(f"{name}: {int((diff > 0).sum())} off, worst {int(diff.max())}")
            if mode == "hard_mix":
                off, g = best_fill(hard_mix_fill, got, alpha)
                line.append(f"stretched burn at its best fill {g}: {off} off")
                mapped = (255 * fill + 127) >> 8
                for name, model in (("steps at the fill", hard_mix_steps(fill, fill)),
                                    (f"steps, burn at the fill, stretch at {mapped}", hard_mix_steps(fill, mapped)),
                                    (f"steps at {mapped}", hard_mix_steps(mapped, mapped))):
                    line.append(f"{name}: {int((r(model * alpha + D * (255 - alpha)) != got).sum())} off")
                off, g = best_fill(lambda g: hard_mix_steps(fill, g), got, alpha)
                line.append(f"steps, burn at the fill, stretch at its best {g}: {off} off")
            if mode == "linear_light" and alpha == 255:
                offsets = light_offsets(got)
                print(f"    {mode}{suffix} offsets by s: " + " ".join(f"{s}:{int(offsets[s])}" for s in
                      (0, 1, 2, 3, 63, 64, 65, 126, 127, 128, 129, 130, 191, 192, 193, 253, 254, 255)), flush=True)
            if mode == "linear_light":
                off, g = best_fill(linear_light_fill, got, alpha)
                line.append(f"each half toward its neutral at its best fill {g}: {off} off")
            print(f"{mode}{suffix}: " + " | ".join(line), flush=True)
            if mode in ("hard_mix", "linear_light") and suffix in ("-f64", "-f192", "-f254"):
                model = hard_mix_fill(fill) if mode == "hard_mix" else linear_light_fill(fill)
                where_misses(got - r(model * alpha + D * (255 - alpha)))
    print(f"{fitted} documents fitted", flush=True)
    print("FILL FIT DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[2] == "fill":
        fill_fit(sys.argv[1], sys.argv[3])
    elif len(sys.argv) == 2:
        main(sys.argv[1])
    else:
        raise SystemExit(__doc__)
