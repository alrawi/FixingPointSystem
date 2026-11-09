#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Writes the job summary of the image workflow (IMG-04) as Markdown on standard output.
#
#   image/ci-summary.sh ACCEL BOOT_LOG ARTIFACT_DIR
#
#   ACCEL         kvm or tcg, the accelerator of the build and the boot test
#   BOOT_LOG      the output of image/boot-test.sh --run
#   ARTIFACT_DIR  the directory that FPS_TEST_ARTIFACT_DIR named, with run/ inside
#
# The summary has the accelerator, the result line, every FAIL line, the counts of
# PASS lines, and the figures of the HTTP and DNS run. It never fails on a missing
# file, because it runs after a failed test as well.
set -uo pipefail

accel="${1:?usage: ci-summary.sh ACCEL BOOT_LOG ARTIFACT_DIR}"
log="${2:?usage: ci-summary.sh ACCEL BOOT_LOG ARTIFACT_DIR}"
dir="${3:?usage: ci-summary.sh ACCEL BOOT_LOG ARTIFACT_DIR}"

echo "## Image boot test"
echo
echo "| | |"
echo "|---|---|"
echo "| Accelerator | $(echo "$accel" | tr '[:lower:]' '[:upper:]') |"
if [[ -f "$log" ]]; then
	result="$(grep '^RESULT:' "$log" | tail -n 1)"
	echo "| Result | \`${result:-no RESULT line}\` |"
	echo "| PASS lines | $(grep -c '^PASS' "$log") |"
	echo "| FAIL lines | $(grep -c '^FAIL' "$log") |"
else
	echo "| Result | no boot log, the build or the boot did not run |"
fi
echo

if [[ -f "$log" ]] && grep -q '^FAIL' "$log"; then
	echo "### Failed checks"
	echo
	echo '```'
	grep '^FAIL' "$log"
	echo '```'
	echo
fi

if [[ -f "$log" ]]; then
	echo "### HTTP and DNS run"
	echo
	echo '```'
	grep '^PASS run:\|^FAIL run:\|^NOTE run ' "$log" || echo "the run did not start"
	echo '```'
	echo
fi

if [[ -s "$dir/run/run-summary.json" ]]; then
	python3 -I - "$dir/run/run-summary.json" "$dir/run/run-metrics.json" <<'EOF'
import json
import sys

try:
    s = json.load(open(sys.argv[1]))
except ValueError:
    sys.exit(0)
print("| Run | State | Verdict |")
print("|---|---|---|")
print("| `%s` | %s | %s |" % (s.get("id", "?"), s.get("state", "?"),
      "passed" if (s.get("verdict") or {}).get("passed") else "not passed"))
print()
try:
    m = json.load(open(sys.argv[2]))
except (OSError, ValueError):
    sys.exit(0)
apps = {}
for x in m.get("samples", []):
    v = x.get("values", {})
    a = apps.setdefault(x.get("key", "?"), [0, 0])
    a[0] += v.get("sessions.succeeded", 0)
    a[1] += v.get("sessions.failed", 0)
print("| Application | Succeeded sessions | Failed sessions |")
print("|---|---|---|")
for k in sorted(apps):
    print("| %s | %d | %d |" % (k, apps[k][0], apps[k][1]))
EOF
fi
