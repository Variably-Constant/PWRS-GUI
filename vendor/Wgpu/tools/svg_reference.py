"""Reference renders for Wgpu's tests of its SVG import, made by resvg's own
command line at the version the module links, which the module's drawing
must match pixel for pixel: tests/svg's shapes.svg at its own size, at a
width of 80 and at half its size; text.svg in the bundled Noto Sans subset
and no system font, Noto Sans standing in for every generic family; and
image.svg, an 8 by 8 drawing of the 2 by 2 PNG it embeds, which this script
writes. It also writes shapes.svgz, shapes.svg gzipped.

From the repository's folder, with Pillow installed and resvg's command
line built (cargo install resvg --version 0.49.0):
python -u -I tools/svg_reference.py <resvg> tests/svg tests/fonts/NotoSans-Regular-subset.ttf
"""

import base64
import gzip
import io
import os
import subprocess
import sys

from PIL import Image

# The generic families text.svg's drawing maps to the one font it has.
FAMILIES = ["--font-family", "--serif-family", "--sans-serif-family", "--cursive-family", "--fantasy-family", "--monospace-family"]
FONT_FAMILY = "Noto Sans"


def render(resvg, svg, png, *flags):
    """`svg` drawn by resvg into `png`, with no system font."""
    subprocess.run([resvg, "--skip-system-fonts", *flags, svg, png], check=True)
    print(f"{os.path.basename(png)} from {os.path.basename(svg)} {' '.join(flags)}".rstrip(), flush=True)


def embedded_png():
    """A 2 by 2 PNG of three opaque colors and one half transparent, as
    base64."""
    pixels = Image.new("RGBA", (2, 2))
    pixels.putdata([(204, 51, 0, 255), (0, 153, 51, 255), (51, 102, 204, 128), (255, 255, 255, 255)])
    out = io.BytesIO()
    pixels.save(out, "PNG")
    return base64.b64encode(out.getvalue()).decode("ascii")


def main(resvg, folder, font):
    image = os.path.join(folder, "image.svg")
    with open(image, "w", encoding="utf-8", newline="\n") as f:
        f.write('<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8" viewBox="0 0 8 8">\n')
        f.write(f'  <image width="8" height="8" image-rendering="optimizeSpeed" href="data:image/png;base64,{embedded_png()}"/>\n')
        f.write("</svg>\n")
    shapes = os.path.join(folder, "shapes.svg")
    with open(shapes, "rb") as f:
        source = f.read()
    with open(os.path.join(folder, "shapes.svgz"), "wb") as f:
        f.write(gzip.compress(source, mtime=0))
    render(resvg, shapes, os.path.join(folder, "shapes.png"))
    render(resvg, shapes, os.path.join(folder, "shapes-80.png"), "-w", "80")
    render(resvg, shapes, os.path.join(folder, "shapes-half.png"), "-z", "0.5")
    families = [part for flag in FAMILIES for part in (flag, FONT_FAMILY)]
    render(resvg, os.path.join(folder, "text.svg"), os.path.join(folder, "text.png"), "--use-font-file", font, *families)
    render(resvg, image, os.path.join(folder, "image.png"))
    print("SVG REFERENCE DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit(__doc__)
    main(*sys.argv[1:])
