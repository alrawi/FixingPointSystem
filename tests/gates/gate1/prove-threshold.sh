#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Negative proof of the Gate 1 suite. The script copies the capture and the run
# files of a passing run of run.sh to OUT_DIR/raised, raises the threshold just
# above the lowest measured conformance ratio, and scores the copy again. It
# exits with 0 when the gate then fails with code 1, and with 1 otherwise, so
# CI shows that the threshold decides the verdict. The script then runs
# prove-corruption.sh, which shows that a corrupted HTTP message fails the gate.
#
# Usage: tests/gates/gate1/prove-threshold.sh OUT_DIR
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=${1:?usage: prove-threshold.sh OUT_DIR}
[[ -s $out/summary.json ]] || { echo "prove-threshold.sh: $out holds no summary.json" >&2; exit 2; }
python3 -I -c 'import json, sys; sys.exit(0 if json.load(open(sys.argv[1]))["passed"] else 1)' \
  "$out/summary.json" || { echo "prove-threshold.sh: the run in $out did not pass" >&2; exit 1; }

raised=$out/raised
rm -rf "$raised"
mkdir -p "$raised"
for f in gate1.pcapng run-report.json run-metrics.json context.json; do
  cp "$out/$f" "$raised/"
done
threshold=$(python3 -I "$here/gate1_report.py" --raised-threshold "$out")
rc=0
FPS_GATE1_THRESHOLD=$threshold "$here/run.sh" --rescore "$raised" >/dev/null || rc=$?
if [[ $rc -ne 1 ]]; then
  echo "prove-threshold.sh: the gate exited with $rc at threshold $threshold, want 1" >&2
  exit 1
fi
echo "prove-threshold.sh: the gate fails at threshold $threshold, above the measured ratios, see $raised/report.md"

# The second negative proof corrupts one HTTP message of the same capture.
"$here/prove-corruption.sh" "$out"
