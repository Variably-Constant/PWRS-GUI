"""Composites an 8-bit RGB document of normal pixel layers (no groups,
masks, clipping or effects) by Photoshop's measured 8-bit rule (memory
wgpu-photoshop-normal-compositing-8bit) and compares it with the document's
stored composite, raw: color matted with white where there is
transparency, and alpha. Prints the misses per plane and the worst.

    python -u -I ps_composite_check.py <document>

Ends with `CHECK DONE`.
"""

import sys

import numpy as np
from psd_tools import PSDImage
from psd_tools.constants import Tag


def r(v):
    return (v + 127) // 255


def main(path):
    psd = PSDImage.open(path)
    header = psd._record.header
    w, h = psd.width, psd.height
    color = np.full((3, h, w), 255, dtype=np.int64)
    alpha = np.zeros((h, w), dtype=np.int64)
    for layer in psd:
        if layer.is_group() or not layer.visible:
            raise SystemExit(f"{layer.name!r}: groups and hidden layers are not handled here")
        if layer.blend_mode.name != "NORMAL":
            raise SystemExit(f"{layer.name!r}: blend mode {layer.blend_mode.name}")
        left, top, right, bottom = layer.bbox
        pixels = np.asarray(layer.topil().convert("RGBA"), dtype=np.int64)
        fill = layer.tagged_blocks.get_data(Tag.BLEND_FILL_OPACITY, 255) if layer.tagged_blocks else 255
        k = r(layer.opacity * fill)
        x0, y0, x1, y1 = max(left, 0), max(top, 0), min(right, w), min(bottom, h)
        part = pixels[y0 - top:y1 - top, x0 - left:x1 - left]
        a = r(part[..., 3] * k)
        b = alpha[y0:y1, x0:x1]
        t = b + a - r(b * a)
        share = np.where(t > 0, np.minimum((255 * a + t // 2) // np.maximum(t, 1), 255), 0)
        for c in range(3):
            under = color[c, y0:y1, x0:x1]
            color[c, y0:y1, x0:x1] = np.where(t > 0, r(part[..., c] * share + under * (255 - share)), under)
        alpha[y0:y1, x0:x1] = t
    matted = np.where(alpha > 0, r(color * alpha + 255 * (255 - alpha)), 255)
    channels = psd._record.image_data.get_data(header)
    stored = np.stack([np.frombuffer(c, dtype=np.uint8).reshape(h, w) for c in channels]).astype(np.int64)
    for c in range(3):
        diff = np.abs(matted[c] - stored[c])
        print(f"plane {c}: {int((diff > 0).sum())} off, worst {int(diff.max())}", flush=True)
    if stored.shape[0] > 3:
        diff = np.abs(alpha - stored[3])
        print(f"alpha: {int((diff > 0).sum())} off, worst {int(diff.max())}", flush=True)
    print("CHECK DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
