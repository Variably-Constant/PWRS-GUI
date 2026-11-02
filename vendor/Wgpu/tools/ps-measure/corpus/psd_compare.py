"""Compares what Wgpu's psd-probe read with psd-tools 1.26.0 and with the
composite Photoshop stored.

`read` mode: the picture the import loads against psd-tools' numpy reading
of the stored composite, white taken out where psd-tools takes it out, and
its transparency where psd-tools finds one. `merge` mode: the layers Wgpu
merged against the stored composite, which is Photoshop's own merge, and
against psd-tools' merge of the layers.

    python -u -I psd_compare.py <read|merge> <probe log> <probe output folder> <PSD folder>...

Each file is the path its probe line names, relative to the folder the
probe ran in, or for a line without one, the file its name stands for in
the PSD folders.

Prints a JSON line per file: the largest and mean difference in color, over
pixels both sides cover, and in alpha, in 8-bit levels, the pixels more
than 1 level apart, and whether the two sides agree on having alpha. CMYK
compares inks, 0 for none. Then `DONE <count>`.

psd-tools' reading and merge of each file are kept in psd-tools-cache/ in
the work folder (PS_MEASURE_WORK, else the current folder), under the
file's path, size and modification time and psd-tools' version, as numpy
holds them, so each is worked out once.
"""

import hashlib
import json
import os
import sys

import numpy as np
import psd_tools
from psd_tools import PSDImage
from psd_tools.api import numpy_io, utils

CACHE = os.path.join(os.environ.get("PS_MEASURE_WORK") or os.getcwd(), "psd-tools-cache")


def cached(kind, path, compute):
    """compute()'s color and alpha for the file at `path`, kept in CACHE:
    an alpha of None is kept as no alpha."""
    st = os.stat(path)
    key = f"{kind}|{psd_tools.__version__}|{os.path.abspath(path)}|{st.st_size}|{st.st_mtime_ns}"
    file = os.path.join(CACHE, hashlib.sha1(key.encode()).hexdigest() + ".npz")
    if os.path.exists(file):
        with np.load(file) as data:
            return data["color"], (data["alpha"] if "alpha" in data.files else None)
    color, alpha = compute()
    os.makedirs(CACHE, exist_ok=True)
    arrays = {"color": color} if alpha is None else {"color": color, "alpha": alpha}
    np.savez(file, **arrays)
    return color, alpha


def find(name, folders):
    """The file a probe name, <folder>__<file>, stands for."""
    parent, _, file = name.partition("__")
    for folder in folders:
        for root, _, names in os.walk(folder):
            if file in names and os.path.basename(root) == parent:
                return os.path.join(root, file)
    return None


def stored(psd):
    """The stored composite as psd-tools reads it: color, as inks for CMYK,
    and its transparency or None."""
    colors = utils.get_color_channels(psd)
    data = numpy_io.get_image_data(psd, None)
    if psd.color_mode.name == "INDEXED":
        return data[:, :, :3], None
    color = data[:, :, :colors]
    if psd.color_mode.name == "CMYK":
        color = 1.0 - color
    slot = numpy_io._transparency_slot(psd, colors)
    alpha = data[:, :, slot] if slot >= 0 else None
    return color, alpha


def merged_by_psd_tools(psd):
    from psd_tools.composite import composite

    color, _, alpha = composite(psd)
    if psd.color_mode.name == "CMYK":
        color = 1.0 - color
    return color, alpha[:, :, 0]


def difference(ours, theirs):
    ours_color, ours_alpha = ours
    theirs_color, theirs_alpha = theirs
    h, w = ours_color.shape[:2]
    a1 = ours_alpha if ours_alpha is not None else np.ones((h, w))
    a2 = theirs_alpha if theirs_alpha is not None else np.ones((h, w))
    both = (a1 > 0) & (a2 > 0)
    channels = min(ours_color.shape[2], theirs_color.shape[2])
    # Premultiplied: a color under a small alpha counts for as little as it shows.
    premultiplied_ours = ours_color[:, :, :channels] * a1[:, :, None]
    premultiplied_theirs = theirs_color[:, :, :channels] * a2[:, :, None]
    d = np.abs(premultiplied_ours - premultiplied_theirs).max(axis=2) * 255.0
    d = np.where(both, d, 0.0)
    da = np.abs(a1 - a2) * 255.0
    return {
        "color_max": round(float(d.max()), 3),
        "color_mean": round(float(d[both].mean()) if both.any() else 0.0, 4),
        "color_over_1": int((d > 1.0).sum()),
        "alpha_max": round(float(da.max()), 3),
        "alpha_over_1": int((da > 1.0).sum()),
        "alpha_agrees": (ours_alpha is None) == (theirs_alpha is None),
    }


def main(mode, log, out, folders):
    count = 0
    with open(log, encoding="utf-8") as handle:
        lines = [line for line in handle if line.startswith("{")]
    for line in lines:
        entry = json.loads(line)
        name = entry["name"]
        if "error" in entry:
            print(json.dumps({"name": name, "ours_error": entry["error"]}), flush=True)
            continue
        path = entry.get("path") or find(name, folders)
        w, h = entry["width"], entry["height"]
        values = np.fromfile(os.path.join(out, f"{name}.{mode}.f32"), dtype="<f4")
        channels = values.size // (w * h)
        values = values.reshape(h, w, channels)
        colors = channels - (1 if entry["alpha"] else 0)
        ours = (values[:, :, :colors], values[:, :, colors] if entry["alpha"] else None)
        result = {"name": name, "ms": entry["ms"], "warnings": len(entry["warnings"])}
        try:
            result["stored"] = difference(ours, cached("stored", path, lambda: stored(PSDImage.open(path))))
            if mode == "merge":
                result["psd_tools_merge"] = difference(ours, cached("merge", path, lambda: merged_by_psd_tools(PSDImage.open(path))))
        except Exception as e:
            result["oracle_error"] = f"{type(e).__name__}: {e}"
        print(json.dumps(result), flush=True)
        count += 1
    print(f"DONE {count}", flush=True)


if __name__ == "__main__":
    if len(sys.argv) < 5 or sys.argv[1] not in ("read", "merge"):
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:])
