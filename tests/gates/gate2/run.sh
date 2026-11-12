#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Gate 2 acceptance suite. The suite runs the three stages of the gate and writes
# the gate report.
#
#   sudo tests/gates/gate2/run.sh PEERS_DIR FPS_CONTROLLER FPS_AGENT OUT_DIR [IMAGE]
#   tests/gates/gate2/run.sh --report OUT_DIR
#
# 1. modules.sh records every native module in the namespace harness and scores
#    it with the realism runner, as the realism job of module CI does.
# 2. mix.sh runs a byte weighted enterprise mix of the 15 applications through the
#    controller and the agent and checks the shares against the weights.
# 3. image.sh builds the qcow2 image, boots it, and runs a test unattended inside.
#
# A stage that fails does not stop the others, so one run shows every failure.
# gate2_report.py then reads the files of all three stages and writes
# OUT_DIR/report.md and OUT_DIR/summary.json with every measured value. The second
# form writes the report again from the files of an earlier run, which is how the
# workflow combines the stages that its jobs ran apart.
#
# Exit codes are 0 when the gate passes, 1 when a gate check fails, and 2 when the
# suite cannot run. FPS_GATE2_STAGES lists the stages to run, default
# modules,mix,image, and tunes the report to the same stages. The environment
# variables of the stage scripts apply as they describe.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
readonly here
stages=${FPS_GATE2_STAGES:-modules,mix,image}
threshold=${FPS_GATE2_THRESHOLD:-0.99}
tolerance=${FPS_GATE2_TOLERANCE:-2}

die() {
  echo "run.sh: $*" >&2
  exit 2
}

# report OUT writes the gate report and returns the verdict.
report() {
  local out=$1 mix_modules rc=0
  local root waived
  root=$(cd "$here/../../.." && pwd)
  waived=$(python3 -I "$here/mix_config.py" waived --requested "${FPS_GATE2_MIX_WAIVE:-}" \
    --cargo "$root/engine/fps-agent/Cargo.toml") || die "cannot decide the waivers"
  mix_modules=$(python3 -I "$here/mix_config.py" names --waived "$waived") || die "cannot list the modules"
  # Either waiver (a module left out of the mix, or a basis other than bytes)
  # lets the verdict be PASS WITH WAIVERS.
  local allow_waivers=
  [[ -z ${FPS_GATE2_MIX_WAIVE:-} && ${FPS_GATE2_BASIS:-bytes} == bytes ]] || allow_waivers=--allow-waivers
  python3 -I "$here/gate2_report.py" --out-dir "$out" --stages "$stages" --threshold "$threshold" \
    --tolerance "$tolerance" --mix-modules "${mix_modules// /,}" ${allow_waivers:+"$allow_waivers"} || rc=$?
  return "$rc"
}

if [[ ${1:-} == --report ]]; then
  [[ $# -eq 2 && -d $2 ]] || die "usage: run.sh --report OUT_DIR"
  report "$2"
  exit $?
fi

[[ $# -ge 4 && $# -le 5 ]] || die "usage: run.sh PEERS_DIR FPS_CONTROLLER FPS_AGENT OUT_DIR [IMAGE]"
[[ $EUID -eq 0 ]] || die "this suite needs root for the namespace harness"
peers=$1 controller=$2 agent=$3 out=$4 image=${5:-}
mkdir -p "$out"

for stage in ${stages//,/ }; do
  rc=0
  case $stage in
    modules) "$here/modules.sh" "$peers" "$out" || rc=$? ;;
    mix) "$here/mix.sh" "$controller" "$agent" "$out" || rc=$? ;;
    image) "$here/image.sh" "$out" ${image:+"$image"} || rc=$? ;;
    *) die "unknown stage $stage" ;;
  esac
  echo "run.sh: stage $stage exited with $rc" >&2
done

rc=0
report "$out" || rc=$?
exit "$rc"
