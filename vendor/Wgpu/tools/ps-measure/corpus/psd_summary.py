"""Summarizes a psd_compare.py log: how many files match the stored
composite (Photoshop's merge, or psd-tools' reading of it) exactly, within
1 level, or worse, and for each worse file the differences against the
stored composite and against psd-tools' merge, with the probe's warnings.

    python -u -I psd_summary.py <psd_compare.py log> <psd-probe log>

Prints the counts, a line per file worse than 1 level, then `DONE`.
"""

import json
import sys


def main(compare_log, probe_log):
    sys.stdout.reconfigure(encoding="utf-8")
    warnings = {}
    with open(probe_log, encoding="utf-8") as handle:
        for line in handle:
            if line.startswith("{"):
                entry = json.loads(line)
                warnings[entry["name"]] = entry.get("warnings", [])
    rows = []
    with open(compare_log, encoding="utf-8") as handle:
        rows = [json.loads(line) for line in handle if line.startswith("{")]
    exact = within = 0
    worse = []
    for row in rows:
        name = row["name"]
        if "stored" not in row:
            worse.append(f"{name} | {row.get('ours_error') or row.get('oracle_error')}")
            continue
        s, p = row["stored"], row.get("psd_tools_merge")
        if s["color_over_1"] == 0 and s["alpha_over_1"] == 0:
            if s["color_max"] == 0 and s["alpha_max"] == 0:
                exact += 1
            else:
                within += 1
            continue
        line = f"{name} | stored: color {s['color_max']:.1f} ({s['color_over_1']} px) alpha {s['alpha_max']:.1f} ({s['alpha_over_1']} px)"
        if p:
            line += f" | psd-tools merge: color {p['color_max']:.1f} ({p['color_over_1']} px) alpha {p['alpha_max']:.1f} ({p['alpha_over_1']} px)"
        if warnings.get(name):
            line += " | " + "; ".join(warnings[name])
        worse.append(line)
    print(f"files {len(rows)}, exact {exact}, within 1 level {within}, worse {len(worse)}", flush=True)
    for line in worse:
        print(line, flush=True)
    print("DONE", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    main(sys.argv[1], sys.argv[2])
