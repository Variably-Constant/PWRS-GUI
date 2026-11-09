"""Writes PSD documents of pixel layers whose every pixel is one compositing
case, for Photoshop to open and save with its own composite
(ps_composite_resave.jsx), so that composite shows Photoshop's arithmetic
case by case. Also writes cases.json, each document's layers as written.

    python -u -I ps_composite_batch.py <output folder> [modes | nonsep]

With `modes`, the documents tabulate each separable blend mode instead:
mode-<name> has an opaque backdrop whose channels hold the column's number
under an opaque source whose channels hold the row's number, in that blend
mode; mode-<name>-a128 is the same with the source's alpha 128. With
`nonsep`, each non-separable mode's document pairs an opaque backdrop of
random colors (seed 1) with an opaque source of random colors (seed 2),
one pair a pixel. With `clearmodes`, the `modes` documents again but over a
backdrop whose alpha is 128 everywhere. With `randmodes`, normal and each
separable mode's document pairs a backdrop of random colors (seed 1) and
random alphas (seed 3) with a source in that mode of random colors (seed 2)
and random alphas (seed 4). With `randfill`, the same random layers for
normal, multiply, screen and the eight modes in SPECIAL, the source at
opacity 200 and fill opacity 77. With `fillmodes`, normal and each separable
mode's opaque table (the `modes` layout) at fill opacity 128 (-f128) and 77
(-f77), and at fill 128 with the source's alpha 128 (-f128-a128), opacity
255 throughout. With `fillspecial`, the opaque tables of the eight modes in
SPECIAL at fill opacity 1, 64, 127, 129, 192 and 254 (-f<fill>). With
`fillalpha`, difference and color burn in the `clear` layout (the backdrop's
alpha the column's number, the source's the row's) at opacity 255 with fill
77 and 128, and at opacity 200 with fill 77 (-op<o>-f<f>). With
`testset`, small documents for
Wgpu's tests in panels of 32 columns by 32 rows (levels in 32 even steps;
no cases.json): t-<mode>, 128 wide, for normal and each separable mode,
the backdrop's level the column's and the source's the row's, opaque over
opaque, then the source at alpha 128 over opaque, then opaque over the
backdrop at alpha 128, then random colors and alphas on both layers (the
`randmodes` seeds); t-<mode>-fill, 128 wide, for multiply and the eight
modes in SPECIAL with the source at opacity 200 and fill 77: an opaque gray
source over the opaque gray backdrop, the same at alpha 128, random colors
with random source alphas over an opaque backdrop of random colors, and
random colors with random alphas over nothing; t-normal-op77 and
t-normal-fill128-op128, 96 wide, the
panels' sources the three source colors with the row's level as alpha over
the opaque columns; t-clear, the clear case with those three sources.

Otherwise every document is 256 by 256, RGB with transparency, 8 bits,
normal blending, its layers bottom first:
- src<k>-op<o>: an opaque backdrop whose three channels hold the column's
  number (0-255), under a source layer of one color whose alpha is the
  row's number (0-255), at layer opacity o; three source colors.
- src0-fill<f>-op<o>: the first source at fill opacity f and opacity o.
- clear: a backdrop of one color whose alpha is the column's number,
  under the first source color with the row's number as alpha, over
  nothing.
"""

import json
import os
import sys

import numpy as np
from PIL import Image
from psd_tools import PSDImage
from psd_tools.api.layers import PixelLayer
from psd_tools.constants import BlendMode

SIZE = 256
SEPARABLE = ["DARKEN", "MULTIPLY", "COLOR_BURN", "LINEAR_BURN", "LIGHTEN", "SCREEN", "COLOR_DODGE",
             "LINEAR_DODGE", "OVERLAY", "SOFT_LIGHT", "HARD_LIGHT", "VIVID_LIGHT", "LINEAR_LIGHT", "PIN_LIGHT",
             "HARD_MIX", "DIFFERENCE", "EXCLUSION", "SUBTRACT", "DIVIDE"]
# The eight modes whose fill opacity acts inside the mode.
SPECIAL = ["COLOR_BURN", "LINEAR_BURN", "COLOR_DODGE", "LINEAR_DODGE", "VIVID_LIGHT", "LINEAR_LIGHT", "HARD_MIX", "DIFFERENCE"]
SOURCES = [(255, 128, 0), (200, 77, 33), (1, 254, 140)]
BACKDROP = (60, 160, 220)
PLANES = {
    "columns": np.tile(np.arange(SIZE, dtype=np.uint8), (SIZE, 1)),
    "rows": np.tile(np.arange(SIZE, dtype=np.uint8)[:, None], (1, SIZE)),
    "opaque": np.full((SIZE, SIZE), 255, dtype=np.uint8),
}


def image(color, alpha):
    """An RGBA image: `color` a triple of levels or "columns" for the
    column's number in every channel; `alpha` a key of PLANES."""
    if color in ("columns", "rows"):
        planes = [PLANES[color]] * 3
    elif isinstance(color, str) and color.startswith("random"):
        values = np.random.default_rng(int(color[6:])).integers(0, 256, (SIZE, SIZE, 3), dtype=np.uint8)
        planes = [values[..., k] for k in range(3)]
    else:
        planes = [np.full((SIZE, SIZE), c, dtype=np.uint8) for c in color]
    return Image.fromarray(np.dstack(planes + [PLANES[alpha]]), "RGBA")


def testset(out):
    """The `testset` documents: panels of 32 by 32, levels stepping 0 to 255
    in 32 even steps across each panel and down; a mode's document has a
    fourth panel of random colors and alphas on both layers."""
    size = 32
    levels = np.round(np.arange(size) * 255 / (size - 1)).astype(np.uint8)

    def panels(count):
        width = size * count
        return (width, np.tile(levels, (size, count)), np.tile(levels[:, None], (1, width)),
                np.tile(np.repeat(np.arange(count), size), (size, 1)))

    def gray(plane):
        return np.dstack([plane] * 3)

    def random(seed, shape):
        return np.random.default_rng(seed).integers(0, 256, shape, dtype=np.uint8)

    width, columns, rows, panel = panels(4)
    mode_backdrop, mode_source = gray(columns), gray(rows)
    mode_backdrop_alpha = np.where(panel == 2, 128, 255).astype(np.uint8)
    mode_source_alpha = np.where(panel == 1, 128, 255).astype(np.uint8)
    for plane, seed in ((mode_backdrop, 1), (mode_source, 2), (mode_backdrop_alpha, 3), (mode_source_alpha, 4)):
        plane[:, 3 * size:] = random(seed, plane[:, 3 * size:].shape)
    batch = []
    for mode in ["NORMAL"] + SEPARABLE:
        batch.append((f"t-{mode.lower()}", width, [("backdrop", mode_backdrop, mode_backdrop_alpha, 255, 255, "NORMAL"),
                                                   ("source", mode_source, mode_source_alpha, 255, 255, mode)]))
    # Fill documents: an opaque gray source over the opaque gray backdrop,
    # the same at alpha 128, random colors over an opaque backdrop of random
    # colors with the source's alpha random, and random colors with random
    # alphas over nothing.
    fill_backdrop, fill_source = gray(columns), gray(rows)
    fill_backdrop_alpha = np.where(panel == 3, 0, 255).astype(np.uint8)
    fill_source_alpha = np.where(panel == 1, 128, 255).astype(np.uint8)
    for plane, seed in ((fill_backdrop, 1), (fill_source, 2)):
        plane[:, 2 * size:] = random(seed, plane[:, 2 * size:].shape)
    fill_source_alpha[:, 2 * size:] = random(4, fill_source_alpha[:, 2 * size:].shape)
    for mode in ["MULTIPLY"] + SPECIAL:
        batch.append((f"t-{mode.lower()}-fill", width, [("backdrop", fill_backdrop, fill_backdrop_alpha, 255, 255, "NORMAL"),
                                                        ("source", fill_source, fill_source_alpha, 200, 77, mode)]))
    width, columns, rows, panel = panels(3)
    opaque = np.full((size, width), 255, dtype=np.uint8)
    source_colors = np.array(SOURCES, dtype=np.uint8)[panel]
    backdrop_color = np.broadcast_to(np.array(BACKDROP, dtype=np.uint8), (size, width, 3))
    for name, opacity, fill in (("t-normal-op77", 77, 255), ("t-normal-fill128-op128", 128, 128)):
        batch.append((name, width, [("backdrop", gray(columns), opaque, 255, 255, "NORMAL"),
                                    ("source", source_colors, rows, opacity, fill, "NORMAL")]))
    batch.append(("t-clear", width, [("backdrop", backdrop_color, columns, 255, 255, "NORMAL"),
                                     ("source", source_colors, rows, 255, 255, "NORMAL")]))
    os.makedirs(out, exist_ok=True)
    for n, (name, width, layers) in enumerate(batch, 1):
        psd = PSDImage.new("RGBA", (width, size), depth=8)
        for layer_name, color, alpha, opacity, fill, mode in layers:
            layer = PixelLayer.frompil(Image.fromarray(np.dstack([color, alpha]), "RGBA"), psd, layer_name)
            layer.opacity = opacity
            layer.fill_opacity = fill
            layer.blend_mode = BlendMode[mode]
        psd.save(os.path.join(out, name + ".psd"))
        print(f"done {n} of {len(batch)}: {name}", flush=True)
    print("BATCH DONE", flush=True)


def main(out, modes=False):
    if modes == "testset":
        testset(out)
        return
    os.makedirs(out, exist_ok=True)
    PLANES["half"] = np.full((SIZE, SIZE), 128, dtype=np.uint8)
    batch = []
    if modes == "fillmodes":
        for mode in ["NORMAL"] + SEPARABLE:
            for alpha, fill, suffix in (("opaque", 128, "-f128"), ("opaque", 77, "-f77"), ("half", 128, "-f128-a128")):
                batch.append((f"mode-{mode.lower()}{suffix}", [("backdrop", "columns", "opaque", 255, 255, "NORMAL"),
                                                              ("source", "rows", alpha, 255, fill, mode)]))
    elif modes == "fillalpha":
        for mode in ("DIFFERENCE", "COLOR_BURN"):
            for opacity, fill in ((255, 77), (255, 128), (200, 77)):
                batch.append((f"mode-{mode.lower()}-op{opacity}-f{fill}", [("backdrop", BACKDROP, "columns", 255, 255, "NORMAL"),
                                                                          ("source", SOURCES[0], "rows", opacity, fill, mode)]))
    elif modes == "fillspecial":
        for mode in SPECIAL:
            for fill in (1, 64, 127, 129, 192, 254):
                batch.append((f"mode-{mode.lower()}-f{fill}", [("backdrop", "columns", "opaque", 255, 255, "NORMAL"),
                                                              ("source", "rows", "opaque", 255, fill, mode)]))
    elif modes in ("randmodes", "randfill"):
        PLANES["random3"] = np.random.default_rng(3).integers(0, 256, (SIZE, SIZE), dtype=np.uint8)
        PLANES["random4"] = np.random.default_rng(4).integers(0, 256, (SIZE, SIZE), dtype=np.uint8)
        chosen = ["NORMAL"] + SEPARABLE if modes == "randmodes" else ["NORMAL", "MULTIPLY", "SCREEN"] + SPECIAL
        opacity, fill = (255, 255) if modes == "randmodes" else (200, 77)
        for mode in chosen:
            batch.append((f"mode-{mode.lower()}", [("backdrop", "random1", "random3", 255, 255, "NORMAL"),
                                                  ("source", "random2", "random4", opacity, fill, mode)]))
    elif modes == "clearmodes":
        for mode in SEPARABLE:
            for alpha, suffix in (("opaque", ""), ("half", "-a128")):
                batch.append((f"mode-{mode.lower()}{suffix}", [("backdrop", "columns", "half", 255, 255, "NORMAL"),
                                                              ("source", "rows", alpha, 255, 255, mode)]))
    elif modes == "nonsep":
        for mode in ("HUE", "SATURATION", "COLOR", "LUMINOSITY", "DARKER_COLOR", "LIGHTER_COLOR"):
            batch.append((f"mode-{mode.lower()}", [("backdrop", "random1", "opaque", 255, 255, "NORMAL"),
                                                  ("source", "random2", "opaque", 255, 255, mode)]))
    elif modes:
        for mode in SEPARABLE:
            for alpha, suffix in (("opaque", ""), ("half", "-a128")):
                batch.append((f"mode-{mode.lower()}{suffix}", [("backdrop", "columns", "opaque", 255, 255, "NORMAL"),
                                                              ("source", "rows", alpha, 255, 255, mode)]))
    for k, source in enumerate(SOURCES if not modes else []):
        for opacity in (255, 128, 77):
            batch.append((f"src{k}-op{opacity}", [("backdrop", "columns", "opaque", 255, 255, "NORMAL"),
                                                 ("source", source, "rows", opacity, 255, "NORMAL")]))
    if not modes:
        for fill, opacity in ((128, 255), (128, 128)):
            batch.append((f"src0-fill{fill}-op{opacity}", [("backdrop", "columns", "opaque", 255, 255, "NORMAL"),
                                                          ("source", SOURCES[0], "rows", opacity, fill, "NORMAL")]))
        batch.append(("clear", [("backdrop", BACKDROP, "columns", 255, 255, "NORMAL"),
                                ("source", SOURCES[0], "rows", 255, 255, "NORMAL")]))
    cases = []
    for n, (name, layers) in enumerate(batch, 1):
        path = os.path.abspath(os.path.join(out, name + ".psd")).replace("\\", "/")
        psd = PSDImage.new("RGBA", (SIZE, SIZE), depth=8)
        for layer_name, color, alpha, opacity, fill, mode in layers:
            layer = PixelLayer.frompil(image(color, alpha), psd, layer_name)
            layer.opacity = opacity
            layer.fill_opacity = fill
            layer.blend_mode = BlendMode[mode]
        psd.save(path)
        cases.append({"name": name, "path": path,
                      "layers": [{"name": layer_name, "color": color, "alpha": alpha, "opacity": opacity, "fill": fill,
                                  "mode": mode}
                                 for layer_name, color, alpha, opacity, fill, mode in layers]})
        print(f"done {n} of {len(batch)}: {name}", flush=True)
    with open(os.path.join(out, "cases.json"), "w", encoding="utf-8") as f:
        json.dump(cases, f, indent=1)
    print("BATCH DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] not in ("modes", "nonsep", "clearmodes", "randmodes", "randfill", "fillmodes", "fillspecial", "fillalpha", "testset")):
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2] if len(sys.argv) == 3 else False)
