"""Lists a document's layers, bottom first, with what compositing reads of
each: kind, blend mode, opacity, fill opacity, visibility, clipping, its
box, whether it stores pixels, its masks; then the header and whether the
stored composite is a real merge (version info's hasRealMergedData).

    python -u -I psd_layer_list.py <file>

Ends with `LIST DONE`.
"""

import sys

from psd_tools import PSDImage
from psd_tools.constants import Resource, Tag


def walk(group, depth=0):
    for layer in group:
        yield depth, layer
        if layer.is_group():
            yield from walk(layer, depth + 1)


def main(path):
    sys.stdout.reconfigure(encoding="utf-8")
    psd = PSDImage.open(path)
    info = psd.image_resources.get_data(Resource.VERSION_INFO)
    real = getattr(info, "has_composite", None) if info is not None else None
    print(f"{path}: {psd.width}x{psd.height} {psd.color_mode.name} depth {psd.depth}, "
          f"hasRealMergedData {real}", flush=True)
    for depth, layer in walk(psd):
        fill = layer.tagged_blocks.get_data(Tag.BLEND_FILL_OPACITY) if layer.tagged_blocks else None
        mask = layer.mask
        print(f"{'  ' * depth}{layer.kind} {layer.name!r}: blend {layer.blend_mode.name} opacity {layer.opacity} "
              f"fill {fill} visible {layer.visible} clipping {layer.clipping_layer} bbox {layer.bbox} "
              f"pixels {layer.has_pixels()} mask {None if mask is None else (mask.bbox, mask.background_color, mask.disabled)} "
              f"vector mask {layer.has_vector_mask()}", flush=True)
    print("LIST DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
