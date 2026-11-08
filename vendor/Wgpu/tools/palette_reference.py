"""Writes src/palette_reference.rs: the palette entries hitherdither
(github.com/hbldh/hitherdither, MIT) gives each pixel of a 24 by 16 test
image for an eight-color palette, by nearest color alone, by each of its
error-diffusion kernels, and by each Bayer pattern centered on the pixel and
spread over SPREAD, with the Bayer index matrices hitherdither builds. The
tests of src/palette.rs compare the module's dithering against them.

hitherdither's own Bayer dithering adds its pattern's values, which run
from above 0 to below 1, times the spread, so it brightens what it dithers;
the module subtracts one half from each first, as this script does, and
takes the nearest color with hitherdither's own search. The dithered
pixels of error diffusion are palette colors, and their entries are read
back by the same search.

    python -I -B tools/palette_reference.py <folder holding hitherdither/>
"""

import sys
from pathlib import Path

WIDTH, HEIGHT = 24, 16
PALETTE = [(0, 0, 0), (255, 255, 255), (255, 0, 0), (0, 255, 0), (0, 0, 255), (128, 128, 128), (255, 128, 0), (0, 128, 128)]
SPREAD = 96.0
KERNELS = [
    ("FLOYD_STEINBERG", "floyd-steinberg"),
    ("JARVIS_JUDICE_NINKE", "jarvis-judice-ninke"),
    ("STUCKI", "stucki"),
    ("ATKINSON", "atkinson"),
    ("BURKES", "burkes"),
    ("SIERRA", "sierra3"),
    ("TWO_ROW_SIERRA", "sierra2"),
    ("SIERRA_LITE", "sierra-2-4a"),
]
BAYER_SIZES = [2, 4, 8, 16]


def test_image():
    """The test image's RGB samples: gradients across and down, and a blue
    that cycles, so every kernel and pattern meets every palette color."""
    samples = []
    for y in range(HEIGHT):
        for x in range(WIDTH):
            samples += [round(x * 255 / (WIDTH - 1)), round(y * 255 / (HEIGHT - 1)), (x * 7 + y * 13) % 256]
    return samples


def rust_array(name, kind, values, doc):
    body = ", ".join(str(v) for v in values)
    return f"/// {doc}\npub(crate) const {name}: [{kind}; {len(values)}] = [{body}];\n"


def main():
    if len(sys.argv) != 2:
        raise SystemExit("give the folder that holds hitherdither's package, hitherdither/")
    sys.path.insert(0, sys.argv[1])
    import numpy as np
    from PIL import Image
    from hitherdither.diffusion import error_diffusion_dithering
    from hitherdither.ordered.bayer import B, I
    from hitherdither.palette import Palette

    samples = test_image()
    image = Image.frombytes("RGB", (WIDTH, HEIGHT), bytes(samples))
    palette = Palette(PALETTE)
    palette.create_PIL_png_from_rgb_array = lambda array: palette.image_closest_colour(array, order=2)
    parts = [
        "//! Written by tools/palette_reference.py from hitherdither: a test image,\n",
        "//! an eight-color palette, and the palette entry hitherdither gives each\n",
        "//! pixel by nearest color, by each error-diffusion kernel, and by each\n",
        "//! Bayer pattern centered and spread over SPREAD, with its Bayer index\n",
        "//! matrices.\n\n",
        f"pub(crate) const WIDTH: usize = {WIDTH};\n",
        f"pub(crate) const HEIGHT: usize = {HEIGHT};\n",
        f"/// The spread of the ordered patterns, in sample values.\npub(crate) const SPREAD: f64 = {SPREAD};\n",
        rust_array("IMAGE", "u8", samples, "The test image's RGB samples, rows from the top."),
        rust_array("PALETTE", "u8", [v for color in PALETTE for v in color], "The palette's RGB colors."),
        rust_array("NEAREST", "u8", palette.image_closest_colour(np.array(image, "float"), order=2).ravel(), "Each pixel's nearest palette entry."),
    ]
    for name, method in KERNELS:
        indices = error_diffusion_dithering(image, palette, method=method, order=2)
        parts.append(rust_array(name, "u8", np.asarray(indices).ravel(), f"The entries hitherdither's {method} error diffusion gives."))
    pixels = np.array(image, "float")
    xx, yy = np.meshgrid(range(WIDTH), range(HEIGHT))
    for n in BAYER_SIZES:
        index = np.asarray(I(n), "int")
        matrix = np.asarray(B(n), "float")
        offsets = (matrix[yy % n, xx % n] - 0.5)[:, :, None] * SPREAD
        indices = palette.image_closest_colour(pixels + offsets, order=2)
        parts.append(rust_array(f"BAYER{n}_INDEX", "u16", index.ravel(), f"hitherdither's {n}x{n} Bayer index matrix, rows from the top."))
        parts.append(rust_array(f"BAYER{n}", "u8", indices.ravel(), f"The entries the {n}x{n} Bayer pattern, centered, gives."))
    out = Path(__file__).resolve().parent.parent / "src" / "palette_reference.rs"
    out.write_text("".join(parts), encoding="utf-8", newline="\n")
    print(f"wrote {out}: {len(KERNELS)} kernels and {len(BAYER_SIZES)} Bayer patterns over {WIDTH}x{HEIGHT} pixels", flush=True)


if __name__ == "__main__":
    main()
