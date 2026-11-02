#!/usr/bin/env bash
# Builds psd-probe against Wgpu's source, reads and merges every Photoshop
# document of the six test corpora into a new output folder, compares both
# modes with psd-tools and the documents' stored composites, and prints a
# summary. The corpora (../corpora.md) are in the work folder
# PS_MEASURE_WORK, where the outputs go too; the probe runs once per corpus,
# from that folder with paths relative to it, to stay within Windows'
# command line. PS_MEASURE_PYTHON names a Python with ../requirements.txt
# installed, python on PATH when it is not set.
#
#     PS_MEASURE_WORK=<folder> bash psd_check.sh <run name>
#
# Writes psd-probe-<run>/ and psd-<mode>-<run>.log in the work folder; ends
# with CHECK DONE, or CHECK FAILED and the step that failed.
HERE="$(cd "$(dirname "$0")" && pwd)"
S="${PS_MEASURE_WORK:-}"
RUN=${1:-}
[ -n "$S" ] || { echo "CHECK FAILED: PS_MEASURE_WORK is not set"; exit 2; }
[ -n "$RUN" ] || { echo "CHECK FAILED: no run name"; exit 2; }
P="$HERE/../psd-probe"
PY="${PS_MEASURE_PYTHON:-python}"
O="$S/psd-probe-$RUN"
CORPORA="psd-real psd-real2 psd-real3 psd-real4 psd-real5 psd-real6"
mkdir "$O" || { echo "CHECK FAILED: $O exists"; exit 1; }
(cd "$P" && cargo build --release) > "$S/psd-build-$RUN.log" 2>&1 || { echo "CHECK FAILED: build, see psd-build-$RUN.log"; exit 1; }
[ -x "$P/target/release/psd-probe.exe" ] || { echo "CHECK FAILED: no probe binary"; exit 1; }
cd "$S" || { echo "CHECK FAILED: no work folder"; exit 1; }
for mode in read merge; do
    : > "$S/psd-probe-$mode-$RUN.log"
    for d in $CORPORA; do
        files=()
        while IFS= read -r f; do files+=("$f"); done < <(find "$d" -type f \( -iname '*.psd' -o -iname '*.psb' \) | sort)
        echo "# $d: ${#files[@]} files" >> "$S/psd-probe-$mode-$RUN.log"
        "$P/target/release/psd-probe.exe" "$mode" "$O" "${files[@]}" >> "$S/psd-probe-$mode-$RUN.log" 2>&1 || { echo "CHECK FAILED: probe $mode $d"; exit 1; }
    done
    PYTHONUNBUFFERED=1 "$PY" -u -I "$HERE/psd_compare.py" "$mode" "$S/psd-probe-$mode-$RUN.log" "$O" $CORPORA > "$S/psd-$mode-$RUN.log" 2>&1 || { echo "CHECK FAILED: compare $mode"; exit 1; }
    PYTHONUNBUFFERED=1 "$PY" -u -I "$HERE/psd_summary.py" "$S/psd-$mode-$RUN.log" "$S/psd-probe-$mode-$RUN.log" > "$S/psd-summary-$mode-$RUN.log" 2>&1 || { echo "CHECK FAILED: summary $mode"; exit 1; }
done
echo "CHECK DONE"
