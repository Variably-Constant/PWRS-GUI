"""Copies the corpus documents a merge summary lists as off from their
stored composite by at least a floor, color or alpha, into a batch folder's
written/ for ps_composite_resave.jsx, so Photoshop can save each again with
its own composite. Paths come from the probe log's JSON lines; a name's
"~2" style suffix becomes "-v2", and .psb files are left out, since the
re-save takes .psd only.

    python -u -I psd_resave_pick.py <summary log> <probe log> <work folder> <batch folder> <floor>

Prints a line per file copied or left out, then `PICK DONE <copied>`.
"""

import json
import os
import re
import shutil
import sys

LINE = re.compile(r"^(?P<name>[^|]+?) \| stored: color (?P<color>[\d.]+) \(\d+ px\) alpha (?P<alpha>[\d.]+) ")


def main(summary, probe_log, root, batch, floor):
    paths = {}
    with open(probe_log, encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("{"):
                entry = json.loads(line)
                paths[entry["name"]] = entry["path"]
    out = os.path.join(batch, "written")
    os.makedirs(out)
    copied = 0
    with open(summary, encoding="utf-8") as handle:
        for line in handle:
            match = LINE.match(line)
            if not match or max(float(match["color"]), float(match["alpha"])) < floor:
                continue
            name = match["name"]
            path = paths.get(name)
            if path is None:
                print(f"{name}: not in the probe log", flush=True)
                continue
            if path.lower().endswith(".psb"):
                print(f"{name}: a .psb, left out", flush=True)
                continue
            target = os.path.join(out, re.sub(r"\.psd~(\d+)$", r"-v\1.psd", name))
            shutil.copyfile(os.path.join(root, path), target)
            copied += 1
            print(f"{name}: copied", flush=True)
    print(f"PICK DONE {copied}", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 6:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], float(sys.argv[5]))
