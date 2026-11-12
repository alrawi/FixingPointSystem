#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Module stage of the Gate 2 suite. It records a fresh capture of every native
# module in the QA-01 namespace harness and scores it with every realism check,
# which is the realism job of module CI (realism/ci/module-realism.sh) run for
# all modules in one pass.
#
#   sudo tests/gates/gate2/modules.sh PEERS_DIR OUT_DIR
#
# PEERS_DIR holds the fixture peers that build-peers.sh built. The script writes
# OUT_DIR/modules/NAME/NAME.realism.json and the capture of each module, and
# OUT_DIR/modules/results.json with the exit code of each module. A module that
# fails does not stop the others, so the report shows every failure at once. The
# script exits with 0 when every module passed, with 1 when one failed a check,
# and with 2 when one could not run. The verdict of the gate comes from
# gate2_report.py, which reads the realism reports.
#   FPS_REALISM_PARSERS   reference parsers of the realism runner, tshark,zeek
#   FPS_GATE2_MODULES     modules to run, default all native modules
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
readonly here root

die() {
  echo "modules.sh: $*" >&2
  exit 2
}

[[ $# -eq 2 ]] || die "usage: modules.sh PEERS_DIR OUT_DIR"
[[ $EUID -eq 0 ]] || die "this stage needs root for the namespace harness"
[[ -d $1 ]] || die "$1 is not a directory"
peers=$(cd "$1" && pwd)
mkdir -p "$2/modules"
out=$(cd "$2/modules" && pwd)
readonly peers out

if [[ -n ${FPS_GATE2_MODULES:-} ]]; then
  # shellcheck disable=SC2206 # a list of module names without spaces inside
  modules=(${FPS_GATE2_MODULES//,/ })
else
  mapfile -t modules < <(cd "$root" && python3 -m realism modules --format names)
fi
[[ ${#modules[@]} -gt 0 ]] || die "no native module found"

worst=0
codes=""
for module in "${modules[@]}"; do
  [[ -x $peers/$module ]] || die "$peers/$module is missing, run build-peers.sh first"
  echo "modules.sh: recording and scoring $module" >&2
  rc=0
  FPS_FIXTURE_PEER="$peers/$module" "$root/realism/ci/module-realism.sh" "$module" "$out/$module" || rc=$?
  codes+="${codes:+, }\"$module\": $rc"
  [[ $rc -le $worst ]] || worst=$rc
done
printf '{%s}\n' "$codes" >"$out/results.json"
echo "modules.sh: ${#modules[@]} modules done, worst exit code $worst" >&2
exit "$worst"
