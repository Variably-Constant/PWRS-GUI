"""Reads every .psd and .psb file in a folder with psd-tools and records,
for each, what psd-tools makes of its composite image: the header, the
image data's compression, the layer count, the merged-transparency keys,
the alpha identifiers, the version info's real-merged-data flag, which
channel psd-tools takes as transparency and whether it removes the white
the composite is stored over, the ICC profile and EXIF sizes, and the
composite itself as psd-tools' numpy reader gives it (every stored channel,
0 to 1, white removed where psd-tools removes it).

    python -u -I psd_oracle.py <folder of PSD files> <new output folder>

Writes <output>/<file>.f32, the composite as little-endian float32 in
height, width, channel order, and prints one JSON line per file, then
`DONE <count>`. A file psd-tools cannot read prints its error instead.
"""

import json
import os
import sys

import numpy as np
from psd_tools import PSDImage
from psd_tools.api import numpy_io, utils
from psd_tools.constants import Resource, Tag


def record(path):
    psd = PSDImage.open(path)
    rec = psd._record
    header = rec.header
    layer_info = rec.layer_and_mask_information.layer_info
    blocks = psd.tagged_blocks
    merged_keys = [
        tag.value.decode()
        for tag in (Tag.SAVING_MERGED_TRANSPARENCY, Tag.SAVING_MERGED_TRANSPARENCY16, Tag.SAVING_MERGED_TRANSPARENCY32)
        if blocks and tag in blocks
    ]
    resources = psd.image_resources
    alpha_ids = resources.get_data(Resource.ALPHA_IDENTIFIERS)
    version_info = resources.get_data(Resource.VERSION_INFO)
    icc = resources.get_data(Resource.ICC_PROFILE)
    exif = resources.get_data(Resource.EXIF_DATA_1)
    color_planes = utils.get_color_channels(psd)
    array = numpy_io.get_image_data(psd, None)
    return {
        "mode": header.color_mode.name,
        "depth": header.depth,
        "channels": header.channels,
        "width": header.width,
        "height": header.height,
        "version": header.version,
        "compression": rec.image_data.compression.name,
        "layer_count": None if layer_info is None else layer_info.layer_count,
        "merged_keys": merged_keys,
        "alpha_ids": None if alpha_ids is None else list(alpha_ids),
        "has_composite": None if version_info is None else bool(version_info.has_composite),
        "has_transparency": bool(utils.has_transparency(psd)),
        "transparency_index": utils.get_transparency_index(psd),
        "slot": numpy_io._transparency_slot(psd, color_planes),
        "background_index": numpy_io._background_index(psd),
        "icc": 0 if icc is None else len(icc),
        "exif": 0 if exif is None else len(exif),
        "shape": list(array.shape),
    }, array


def main(source, out):
    os.makedirs(out)
    count = 0
    for name in sorted(os.listdir(source)):
        if not name.lower().endswith((".psd", ".psb")):
            continue
        try:
            info, array = record(os.path.join(source, name))
        except Exception as e:
            print(json.dumps({"name": name, "error": f"{type(e).__name__}: {e}"}), flush=True)
            continue
        np.ascontiguousarray(array, dtype="<f4").tofile(os.path.join(out, name + ".f32"))
        print(json.dumps({"name": name, **info}), flush=True)
        count += 1
    print(f"DONE {count}", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2])
