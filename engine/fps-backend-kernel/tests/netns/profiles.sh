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
# Usage: profiles.sh [PROFILE...]
#   FPS_PROBE names the probe binary, by default target/debug/examples/os_profile_probe.
#   Build it first with cargo build -p fps-backend-kernel --example os_profile_probe.
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
  if [[ -n "${FPS_KEEP_CAPTURES:-}" ]]; then
    mkdir -p "${FPS_KEEP_CAPTURES}"
    cp "${WORK}/${profile}.pcapng" "${FPS_KEEP_CAPTURES}/"
  fi
  echo "ok: ${profile}"
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
  require ip tcpdump python3
  PROBE="${FPS_PROBE:-${REPO_DIR}/target/debug/examples/os_profile_probe}"
  [[ -x "${PROBE}" ]] ||
    fail "no probe at ${PROBE}, build it with cargo build -p fps-backend-kernel --example os_profile_probe"
  local profiles=("$@")
  ((${#profiles[@]} > 0)) || profiles=(windows11 linux6 macos14)
  WORK="$(mktemp -d)"
  trap cleanup EXIT
  chmod 755 "${WORK}"
  export FPS_NETNS="fpsp$$"
  export FPS_STATE_DIR="${WORK}/state"
  FPS_MTU="${MTU}" "${NETNS}" up
  local i
  for i in "${!profiles[@]}"; do
    run_profile "${profiles[i]}" $((BASE_PORT + i))
  done
  run_baseline $((BASE_PORT + ${#profiles[@]}))
  echo "profile capture test passed for ${profiles[*]}"
}

main "$@"
