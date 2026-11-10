#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Second negative proof of the Gate 1 suite. The script copies the capture and
# the run files of a passing run of run.sh to OUT_DIR/corrupted, overwrites one
# chunk size line of one HTTP response in the capture with a chunk size that no
# parser reads (corrupt_capture.py), and scores the copy again. It exits with 0
# when the gate then fails with code 1, the parser error check is the failed
# check, and parser-findings.json names the corrupted frame, so CI shows that
# the suite still fails on a capture with a corrupted HTTP message.
#
# Usage: tests/gates/gate1/prove-corruption.sh OUT_DIR
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=${1:?usage: prove-corruption.sh OUT_DIR}
[[ -s $out/summary.json ]] || { echo "prove-corruption.sh: $out holds no summary.json" >&2; exit 2; }
python3 -I -c 'import json, sys; sys.exit(0 if json.load(open(sys.argv[1]))["passed"] else 1)' \
  "$out/summary.json" || { echo "prove-corruption.sh: the run in $out did not pass" >&2; exit 1; }

corrupted=$out/corrupted
rm -rf "$corrupted"
mkdir -p "$corrupted"
for f in run-report.json run-metrics.json context.json; do
  cp "$out/$f" "$corrupted/"
done
frame=$(python3 -I "$here/corrupt_capture.py" "$out/gate1.pcapng" "$corrupted/gate1.pcapng") || {
  echo "prove-corruption.sh: the capture could not be corrupted" >&2
  exit 1
}
rc=0
"$here/run.sh" --rescore "$corrupted" >/dev/null || rc=$?
if [[ $rc -ne 1 ]]; then
  echo "prove-corruption.sh: the gate exited with $rc after the corruption of frame $frame, want 1" >&2
  exit 1
fi
python3 -I - "$corrupted" "$frame" <<'EOF' || exit 1
import json
import sys

out, frame = sys.argv[1], int(sys.argv[2])
summary = json.load(open(out + "/summary.json"))
failed = [c["name"] for c in summary["checks"] if not c["passed"]]
if "Parser error check" not in failed:
    sys.exit(f"prove-corruption.sh: the failed checks are {failed}, the parser error check is not among them")
findings = json.load(open(out + "/parser-findings.json"))["findings"]
if not any(f["frame"] == frame for f in findings):
    sys.exit(f"prove-corruption.sh: parser-findings.json names no finding of frame {frame}")
EOF
echo "prove-corruption.sh: the gate fails on the corrupted frame $frame, see $corrupted/report.md and $corrupted/parser-findings.json"
