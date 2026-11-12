#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Negative proofs of the Gate 2 suite. A gate that cannot fail proves nothing, so
# this script takes the files of a passing run and shows that the gate fails when
# the traffic or the report is worse than the thresholds allow.
#
#   tests/gates/gate2/prove-failure.sh [--only report|mix] OUT_DIR
#
# Each proof works on a copy of the results and expects exit code 1 from the gate.
#
# 1. Conformance (report). The report is written again with a threshold just above
#    the lowest conformance ratio that the run measured, which no capture can meet.
# 2. Module (report). The realism report of one module gets a failed parser error
#    check, which is what a module that emits a malformed message would produce.
#    The gate must fail and name the check.
# 3. Mix (mix). The mix capture is scored again with a tolerance of 0 points, which
#    the Poisson arrivals and the payload sizes of a real run never meet. This
#    proof needs mix/mix.pcapng, ndpiReader, tshark, and Zeek.
#
# The report proofs need the files of all three stages, and the mix proof needs
# the mix stage only, which is how the jobs of the workflow use them: --only mix
# runs in the mix job, and --only report in the report job. Without --only the
# script runs all three. It exits with 0 when every proof made the gate fail as
# required, and with 1 when a proof passed the gate or could not run.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
readonly here root

die() {
  echo "prove-failure.sh: $*" >&2
  exit 1
}

only=all
if [[ ${1:-} == --only ]]; then
  [[ ${2:-} == report || ${2:-} == mix ]] || die "--only takes report or mix"
  only=$2
  shift 2
fi
[[ $# -eq 1 && -d $1 ]] || die "usage: prove-failure.sh [--only report|mix] OUT_DIR"
out=$(cd "$1" && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

waived=$(python3 -I "$here/mix_config.py" waived --requested "${FPS_GATE2_MIX_WAIVE:-}" \
  --cargo "$root/engine/fps-agent/Cargo.toml") || die "cannot decide the waivers"
mix_modules=$(python3 -I "$here/mix_config.py" names --waived "$waived") || die "cannot list the modules"
mix_modules=${mix_modules// /,}
stages=modules,mix,image
[[ $only != mix ]] || stages=mix

# report DIR [ARGS...] runs the gate on the files of DIR without writing a report.
report() {
  local dir=$1
  shift
  local args=(--out-dir "$dir" --no-write --stages "$stages" --mix-modules "$mix_modules" "$@")
  # Either waiver (a module left out of the mix, or a basis other than bytes)
  # lets the verdict be PASS WITH WAIVERS.
  [[ -z ${FPS_GATE2_MIX_WAIVE:-} && ${FPS_GATE2_BASIS:-bytes} == bytes ]] || args+=(--allow-waivers)
  python3 -I "$here/gate2_report.py" "${args[@]}"
}

# The gate passes on the real results first, or the proofs would show nothing.
rc=0
report "$out" >/dev/null || rc=$?
[[ $rc -eq 0 ]] || die "the gate does not pass on $out (exit $rc), so there is nothing to disprove"

# The results of the run without the captures, which the proofs change.
mkdir "$scratch/copy"
tar -C "$out" --exclude='*.pcapng' -cf - . | tar -C "$scratch/copy" -xf -

if [[ $only != mix ]]; then
  echo "prove-failure.sh: proof 1, a conformance threshold above every measured ratio" >&2
  raised=$(python3 -I "$here/gate2_report.py" --out-dir "$out" --raised-threshold)
  rc=0
  report "$out" --threshold "$raised" >/dev/null || rc=$?
  [[ $rc -eq 1 ]] || die "the gate exited with $rc at the threshold $raised, want 1"
  echo "prove-failure.sh: the gate failed at the threshold $raised" >&2

  echo "prove-failure.sh: proof 2, a module whose parser error check failed" >&2
  python3 -I -c 'import json, sys
path = sys.argv[1]
report = json.load(open(path))
for check in report["checks"]:
    if check["check"] == "parser_errors":
        check["passed"] = False
        check["errors_total"] = 1
report["passed"] = False
json.dump(report, open(path, "w"))' "$scratch/copy/modules/http1/http1.realism.json"
  rc=0
  report "$scratch/copy" >"$scratch/module-proof.txt" || rc=$?
  [[ $rc -eq 1 ]] || die "the gate exited with $rc for a failed http1 module, want 1"
  grep -q "Module http1 parser errors" "$scratch/module-proof.txt" || die "the report does not name the failed http1 check"
  echo "prove-failure.sh: the gate failed on the broken http1 module" >&2
fi

if [[ $only != report ]]; then
  echo "prove-failure.sh: proof 3, the mix scored with a tolerance of 0 points" >&2
  [[ -s $out/mix/mix.pcapng ]] || die "the run has no mix capture, so the mix proof cannot run"
  ln -s "$out/mix/mix.pcapng" "$scratch/copy/mix/mix.pcapng"
  rc=0
  FPS_GATE2_TOLERANCE=0 "$here/mix.sh" --rescore "$scratch/copy" >/dev/null 2>&1 || rc=$?
  [[ $rc -eq 1 ]] || die "the mix stage exited with $rc at a tolerance of 0 points, want 1"
  echo "prove-failure.sh: the mix stage failed at a tolerance of 0 points" >&2
fi
echo "prove-failure.sh: every proof made the gate fail as required" >&2
