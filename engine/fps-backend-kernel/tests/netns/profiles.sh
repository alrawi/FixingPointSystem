#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Namespace capture test of the OS profiles of the kernel backend. It brings
# the QA-01 harness up with an MTU of 9000, so the route alone would allow an
# MSS of 8960, and opens one session per profile from the client namespace to
# the server namespace with the example os_profile_probe. Before each session
# it writes the sysctls of the profile in both namespaces. A capture of the
# client arm must show the TTL, MSS, window scale, SACK permitted, and
# timestamp settings of the profile in the SYN and the SYN-ACK. A last
# baseline session without a profile must show the kernel defaults, TTL 64 and
# MSS 8960, which proves that the profiles and not the harness set the values.
#
# The realism runner then scores every capture with its transport behavior
# check, which reads the profile files in profiles/ and not the probe, so it
# is a second opinion on the same handshakes. The check runs in strict mode
# for linux6. It stays off for windows11 and macos14, whose option order the
# kernel backend cannot set, and for macos14 also the SYN window. Each capture
# must also fail when scored with another profile, and the baseline capture
# must fail against linux6, so a capture recorded with the wrong profile
# cannot pass. The score needs ndpiReader (the conformance check always runs,
# and this test ignores its verdict) and PyYAML for the profile files, and no
# other tool.
#
# Usage: profiles.sh [PROFILE...]
#   FPS_PROBE names the probe binary, by default target/debug/examples/os_profile_probe.
#   Build it first with cargo build -p fps-backend-kernel --example os_profile_probe.
#   FPS_KEEP_CAPTURES names a directory that receives the captures and the
#   realism reports of the profiles.
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly TEST_DIR
REPO_DIR="$(cd "${TEST_DIR}/../../../.." && pwd)"
readonly REPO_DIR
readonly NETNS="${REPO_DIR}/tests/harness/netns.sh"
readonly CHECK="${TEST_DIR}/handshakes.py"
readonly MTU=9000
readonly BASE_PORT=9100

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

require() {
  local c
  for c in "$@"; do
    command -v "${c}" >/dev/null 2>&1 || fail "missing command ${c}"
  done
}

state_value() {
  "${NETNS}" info | sed -n "s/^$1=//p"
}

# expect_value PROFILE KEY prints one expected handshake value of the profile.
expect_value() {
  "${PROBE}" expect "$1" | sed -n "s/^$2=//p"
}

# apply_sysctls PROFILE writes the sysctls of the profile in the client and
# server namespaces through /proc/sys, so the test needs no sysctl command.
apply_sysctls() {
  local line key value ns
  while IFS= read -r line; do
    key="${line%%=*}"
    value="${line#*=}"
    for ns in client server; do
      # shellcheck disable=SC2016 # the inner shell expands its own arguments
      "${NETNS}" exec "${ns}" sh -c 'printf "%s\n" "$2" >"/proc/sys/$1"' sh \
        "${key//.//}" "${value}"
    done
  done < <("${PROBE}" sysctls "$1")
}

# realism_score NAME CAPTURE REPORT [ARG...] scores the capture with the
# transport behavior check against the profile NAME and writes the report. The
# capture is the client arm, so a SYN has not crossed the router and a SYN-ACK
# has crossed it once. The runner also runs its conformance check, which fails
# on the probe's own protocol that nDPI does not know, so the exit code of the
# runner is not the verdict: the callers read the entry of the transport
# behavior check from the report (transport_verdict) and use the exit code
# only to tell an error of the runner, which is 2, from a scored capture.
realism_score() {
  local name="$1" capture="$2" report="$3" status=0
  shift 3
  (cd "${REPO_DIR}" && python3 -m realism score --pcap "${capture}" --labels "${LABELS}" \
    --parsers none --os-profile "${name}" --client-hops 0 --server-hops 1 \
    --output "${report}" "$@" >/dev/null) || status=$?
  ((status != 2)) || fail "the realism runner failed on ${capture} as ${name}, see its message above"
}

# explain_report REPORT prints the failures and deviations of the transport
# behavior check of one realism report.
explain_report() {
  python3 -I - "$1" <<'EOF' >&2 || true
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    report = json.load(f)
for check in report["checks"]:
    if check["check"] != "transport_behavior":
        continue
    for item in check["failures"]:
        print("  failure:", json.dumps(item, sort_keys=True))
    for item in check["deviations"]:
        print("  deviation:", json.dumps(item, sort_keys=True))
EOF
}

# transport_verdict REPORT prints pass or fail for the transport behavior
# check of a realism report, and none when the report has no such entry, which
# happens when the runner ignored the profile.
transport_verdict() {
  python3 -I - "$1" <<'EOF'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    report = json.load(f)
verdicts = [c["passed"] for c in report["checks"] if c["check"] == "transport_behavior"]
print("none" if not verdicts else "pass" if all(verdicts) else "fail")
EOF
}

# must_pass NAME CAPTURE LABEL [ARG...] scores a capture that has to match its
# profile and keeps the report. The check must have run and passed.
must_pass() {
  local name="$1" capture="$2" label="$3" report verdict=none
  shift 3
  report="${WORK}/${label}-realism.json"
  realism_score "${name}" "${capture}" "${report}" "$@"
  [[ ! -f "${report}" ]] || verdict="$(transport_verdict "${report}")"
  if [[ "${verdict}" != pass ]]; then
    [[ ! -f "${report}" ]] || explain_report "${report}"
    fail "the realism transport check of ${label} against ${name} gave ${verdict}"
  fi
  echo "ok: realism transport check of ${label} against ${name}"
}

# must_fail NAME CAPTURE LABEL scores a capture against a profile that did not
# record it. Only a failed transport check counts, so a runner that ignored the
# profile, which leaves no entry, cannot pass for a detected mismatch.
must_fail() {
  local name="$1" capture="$2" label="$3" report verdict=none
  report="${WORK}/${label}-as-${name}-realism.json"
  realism_score "${name}" "${capture}" "${report}"
  [[ ! -f "${report}" ]] || verdict="$(transport_verdict "${report}")"
  [[ "${verdict}" == fail ]] ||
    fail "the capture of ${label} scored as ${name} gave ${verdict}, expected a failed check"
  echo "ok: the capture of ${label} fails the transport check as ${name}"
}

# wait_listening LOG waits until the probe server reports its address.
wait_listening() {
  local i
  for ((i = 0; i < 100; i++)); do
    grep -qs '^listening on' "$1" && return 0
    sleep 0.1
  done
  cat "$1" >&2
  fail "the probe server did not start"
}

# run_profile PROFILE PORT opens one session with the profile and checks the
# capture of its handshake.
run_profile() {
  local profile="$1" port="$2" client server log pid status=0
  client="$(state_value CLIENT_ADDR)"
  server="$(state_value SERVER_ADDR)"
  log="${WORK}/${profile}-server.log"
  apply_sysctls "${profile}"
  "${NETNS}" exec server "${PROBE}" serve "${profile}" "${server}:${port}" 1 >"${log}" 2>&1 &
  pid=$!
  wait_listening "${log}"
  "${NETNS}" capture start "${WORK}/${profile}.pcapng" client
  "${NETNS}" exec client "${PROBE}" connect "${profile}" "${client}" "${server}:${port}" ||
    fail "${profile} session failed"
  wait "${pid}" || fail "${profile} server failed: $(cat "${log}")"
  "${NETNS}" capture stop
  if grep -q 'needs' "${log}"; then
    cat "${log}" >&2
    fail "${profile} server namespace lacks the sysctls of the profile"
  fi
  python3 -I "${CHECK}" "${WORK}/${profile}.pcapng" --port "${port}" \
    --ttl "$(expect_value "${profile}" TTL)" --mss "$(expect_value "${profile}" MSS)" \
    --wscale "$(expect_value "${profile}" WSCALE)" --sack "$(expect_value "${profile}" SACK)" \
    --ts "$(expect_value "${profile}" TS)" --df 1 \
    --options "$(expect_value "${profile}" OPTIONS)" || status=$?
  ((status == 0)) || fail "${profile} handshake differs from the profile"
  # Linux decides the option order and rounds the SYN window itself, so only
  # linux6, which is the stack of the kernel, can pass the strict check.
  if [[ "${profile}" == linux6 ]]; then
    must_pass "${profile}" "${WORK}/${profile}.pcapng" "${profile}" --transport-strict
  else
    must_pass "${profile}" "${WORK}/${profile}.pcapng" "${profile}"
  fi
  if [[ -n "${FPS_KEEP_CAPTURES:-}" ]]; then
    mkdir -p "${FPS_KEEP_CAPTURES}"
    cp "${WORK}/${profile}.pcapng" "${WORK}/${profile}-realism.json" "${FPS_KEEP_CAPTURES}/"
  fi
  echo "ok: ${profile}"
}

# run_wrong_profiles PROFILE... scores each capture with the next profile of
# the list and the baseline capture with linux6, and every score must fail.
run_wrong_profiles() {
  local profiles=("$@") i next
  for i in "${!profiles[@]}"; do
    next="${profiles[$(((i + 1) % ${#profiles[@]}))]}"
    [[ "${next}" != "${profiles[i]}" ]] || continue
    must_fail "${next}" "${WORK}/${profiles[i]}.pcapng" "${profiles[i]}"
  done
  must_fail linux6 "${WORK}/none.pcapng" none
}

# run_baseline PORT opens one session without a profile and checks that the
# kernel defaults differ from the profiles.
run_baseline() {
  local port="$1" client server log pid
  client="$(state_value CLIENT_ADDR)"
  server="$(state_value SERVER_ADDR)"
  log="${WORK}/none-server.log"
  "${NETNS}" exec server "${PROBE}" serve none "${server}:${port}" 1 >"${log}" 2>&1 &
  pid=$!
  wait_listening "${log}"
  "${NETNS}" capture start "${WORK}/none.pcapng" client
  "${NETNS}" exec client "${PROBE}" connect none "${client}" "${server}:${port}" ||
    fail "baseline session failed"
  wait "${pid}" || fail "baseline server failed: $(cat "${log}")"
  "${NETNS}" capture stop
  python3 -I "${CHECK}" "${WORK}/none.pcapng" --port "${port}" --ttl 64 --mss $((MTU - 40)) ||
    fail "the baseline handshake does not show the kernel defaults"
  echo "ok: baseline without a profile"
}

cleanup() {
  "${NETNS}" down >/dev/null 2>&1 || true
  [[ -n "${WORK:-}" && -d "${WORK}" ]] && find "${WORK}" -mindepth 1 -delete && rmdir "${WORK}"
  return 0
}

main() {
  [[ "${EUID}" -eq 0 ]] || fail "the capture test needs root, run it as root or with sudo"
  require ip tcpdump python3 ndpiReader
  python3 -c 'import yaml' 2>/dev/null ||
    fail "python3 lacks PyYAML, which the realism runner needs for the profile files"
  PROBE="${FPS_PROBE:-${REPO_DIR}/target/debug/examples/os_profile_probe}"
  [[ -x "${PROBE}" ]] ||
    fail "no probe at ${PROBE}, build it with cargo build -p fps-backend-kernel --example os_profile_probe"
  local profiles=("$@")
  ((${#profiles[@]} > 0)) || profiles=(windows11 linux6 macos14)
  WORK="$(mktemp -d)"
  trap cleanup EXIT
  chmod 755 "${WORK}"
  LABELS="${WORK}/labels.json"
  printf '{"labels": "fps-realism-labels", "version": 1, "default": "probe"}\n' >"${LABELS}"
  export FPS_NETNS="fpsp$$"
  export FPS_STATE_DIR="${WORK}/state"
  FPS_MTU="${MTU}" "${NETNS}" up
  local i
  for i in "${!profiles[@]}"; do
    run_profile "${profiles[i]}" $((BASE_PORT + i))
  done
  run_baseline $((BASE_PORT + ${#profiles[@]}))
  run_wrong_profiles "${profiles[@]}"
  echo "profile capture test passed for ${profiles[*]}"
}

main "$@"
