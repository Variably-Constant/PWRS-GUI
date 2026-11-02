#!/usr/bin/env bash
# The gate for a PSD change, against the working tree of the Wgpu checkout
# this script sits in: the unit tests and clippy without and with the
# psd-merge feature, clippy as the library and as the module without avif,
# jxl, svg and pdf, the unit tests only a build without them has, a module build (without
# psd-merge, as it ships) checked by its DLL's time against the PSD sources, the
# Pester tests of Photoshop documents on that build, the plain-wording check,
# and corpus/psd_check.sh over the six corpora. cargo-pwrs must be on PATH;
# PS_MEASURE_WORK names the work folder holding the corpora (corpora.md).
# Each step logs to psd-gate-<run>-<step>.log there and ends with
# `STEP <name> exit=<code>`; the last line counts the steps that failed.
#
#   PS_MEASURE_WORK=<folder> bash gate.sh <run name>
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
RUN="${1:-}"
W="${PS_MEASURE_WORK:-}"
[ -n "$RUN" ] || { echo "GATE no run name"; exit 2; }
[ -n "$W" ] || { echo "GATE PS_MEASURE_WORK is not set"; exit 2; }
cd "$REPO" || { echo "GATE no checkout"; exit 99; }
failed=0
log() { echo "$W/psd-gate-$RUN-$1.log"; }
step() {
    echo "STEP $1 exit=$2"
    if [ "$2" -ne 0 ]; then failed=$((failed + 1)); fi
}

echo "GATE $RUN $(git -C "$REPO" rev-parse --short HEAD) $(git -C "$REPO" status --short | wc -l | tr -d ' ') changed paths $(date +%H:%M:%S)"

cargo test -p wgpu-pwsh --lib > "$(log unit)" 2>&1
step unit "$?"
cargo test -p wgpu-pwsh --lib --features psd-merge > "$(log unit-psd-merge)" 2>&1
step unit-psd-merge "$?"

cargo clippy -p wgpu-pwsh --no-deps --all-targets -- -D warnings > "$(log clippy)" 2>&1
step clippy "$?"
cargo clippy -p wgpu-pwsh --no-deps --all-targets --features psd-merge -- -D warnings > "$(log clippy-psd-merge)" 2>&1
step clippy-psd-merge "$?"
cargo clippy -p wgpu-pwsh --no-deps --all-targets --no-default-features -- -D warnings > "$(log clippy-library)" 2>&1
step clippy-library "$?"
cargo clippy -p wgpu-pwsh --no-deps --all-targets --no-default-features --features module,static-dxc -- -D warnings > "$(log clippy-without-avif)" 2>&1
step clippy-module-without-avif "$?"
cargo test -p wgpu-pwsh --lib --no-default-features file_without_the > "$(log unit-without-formats)" 2>&1
code=$?
[[ "$(< "$(log unit-without-formats)")" == *"test result: ok. 4 passed"* ]] || code=96
step unit-without-formats "$code"

cargo pwrs build --release > "$(log build)" 2>&1
code=$?
DLL="$REPO/target/pwrs/Wgpu/runtimes/win-x64/native/wgpu_pwsh.dll"
newest=""
for f in src/psd*.rs src/image_decode.rs src/profile_table.rs src/lib.rs; do
    if [ -z "$newest" ] || [ "$f" -nt "$newest" ]; then newest=$f; fi
done
if [ "$code" -eq 0 ] && [ ! "$DLL" -nt "$newest" ]; then
    echo "the module DLL is not newer than $newest"
    code=98
fi
echo "module DLL $(stat -c '%y' "$DLL"), newest PSD source $newest $(stat -c '%y' "$newest")"
step build "$code"

pwsh -NoProfile -Command "& '$(cygpath -w "$HERE/pester_filter.ps1")' -Path '$(cygpath -w "$REPO/tests/Texture.Tests.ps1")' -FullName '*Photoshop documents*'; exit \$LASTEXITCODE" > "$(log pester)" 2>&1
step pester "$?"

pwsh -NoProfile -File tools/Test-PlainWording.ps1 > "$(log wording)" 2>&1
step wording "$?"

bash "$HERE/corpus/psd_check.sh" "$RUN" > "$(log corpus)" 2>&1
code=$?
[[ "$(< "$(log corpus)")" == *"CHECK DONE"* ]] || code=97
step corpus "$code"

echo "GATE $RUN steps failing: $failed $(date +%H:%M:%S)"
exit "$failed"
