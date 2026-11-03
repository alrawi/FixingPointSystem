#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records a fresh capture of one native module in the QA-01 namespace harness
# and scores it with every realism check.
#
#   sudo realism/ci/module-realism.sh MODULE OUTPUT_DIR
#
# The script runs modules/MODULE/fixtures/capture.sh, which builds or takes the
# fixture peer, records OUTPUT_DIR/MODULE.pcapng, and scores it with tshark. It
# then scores the same capture with nDPI and the parsers in FPS_REALISM_PARSERS,
# tshark and Zeek by default, and writes OUTPUT_DIR/MODULE.realism.json. It
# exits with 0 when every check passes, 1 when a check fails, and 2 when the
# capture or a tool cannot run. FPS_FIXTURE_PEER, FPS_FIXTURE_SESSIONS, and
# FPS_NETNS pass through to capture.sh. When GITHUB_STEP_SUMMARY is set, the
# script appends one table row per check to the job summary.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: module-realism.sh MODULE OUTPUT_DIR" >&2
  exit 2
fi
module=$1
out=$2
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
fixtures="$root/modules/$module/fixtures"
parsers=${FPS_REALISM_PARSERS:-tshark,zeek}

if [[ ! $module =~ ^[a-z0-9][a-z0-9_-]*$ || ! -x $fixtures/capture.sh ]]; then
  echo "module-realism.sh: $module has no executable fixtures/capture.sh" >&2
  exit 2
fi
mkdir -p "$out"
out=$(cd "$out" && pwd)

capture_rc=0
"$fixtures/capture.sh" "$out" || capture_rc=$?
pcap="$out/$module.pcapng"
if [[ ! -s $pcap ]]; then
  echo "module-realism.sh: capture.sh exited with $capture_rc and wrote no $pcap" >&2
  exit 2
fi

score_rc=0
cd "$root"
python3 -m realism score --pcap "$pcap" --labels "$fixtures/expected.yaml" \
  --parsers "$parsers" --output "$out/$module.realism.json" || score_rc=$?
if [[ $score_rc -eq 2 || ! -s $out/$module.realism.json ]]; then
  echo "module-realism.sh: the realism runner could not score $pcap" >&2
  exit 2
fi

python3 -m realism.ci.summary "$module" "$out/$module.realism.json"

if [[ $score_rc -ne 0 ]]; then
  echo "module-realism.sh: $module failed a realism check, see $out/$module.realism.json" >&2
  exit 1
fi
if [[ $capture_rc -ne 0 ]]; then
  echo "module-realism.sh: capture.sh exited with $capture_rc" >&2
  exit "$capture_rc"
fi
echo "module-realism.sh: $module passed every realism check"
