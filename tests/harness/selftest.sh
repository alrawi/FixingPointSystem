#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Component test of the namespace harness. It brings the harness up in several
# configurations, sends HTTP through the routing namespace with curl, checks
# the pcapng capture, and proves that down restores the root namespace.
#
# Usage: selftest.sh
#   FPS_REQUIRE_VLAN=1 fails instead of skipping the VLAN case when the kernel
#   cannot create 802.1Q interfaces. CI sets it.
set -euo pipefail

HARNESS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly HARNESS_DIR
readonly NETNS="${HARNESS_DIR}/netns.sh"
readonly PORT=8080
PASSED=0
SKIPPED=0

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

pass() {
  echo "ok: $*"
  PASSED=$((PASSED + 1))
}

skip() {
  echo "skip: $*"
  SKIPPED=$((SKIPPED + 1))
}

require() {
  local c
  for c in "$@"; do
    command -v "${c}" >/dev/null 2>&1 || fail "missing command ${c}"
  done
}

# root_snapshot prints the namespaces, links, and routes of the root namespace
# in a stable order, so two snapshots compare with diff.
root_snapshot() {
  echo "# netns"
  ip netns list | awk '{print $1}' | sort
  echo "# links"
  ip -o link show | awk -F': ' '{print $2}' | sed 's/@.*//' | sort
  echo "# routes"
  ip route show table all | sort
}

# state_value KEY prints one value of the harness state.
state_value() {
  "${NETNS}" info | sed -n "s/^$1=//p"
}

# start_server serves WORK/www from the server namespace and logs each request
# with the peer address. The server skips the reverse lookup of its own
# address that http.server performs, which would wait for a DNS timeout.
start_server() {
  local addr
  addr="$(state_value SERVER_ADDR)"
  "${NETNS}" exec server python3 -I -c '
import functools, http.server, socketserver, sys

class Server(http.server.ThreadingHTTPServer):
    server_bind = socketserver.TCPServer.server_bind

class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print("peer=%s %s" % (self.client_address[0], fmt % args), file=sys.stderr, flush=True)

handler = functools.partial(Handler, directory=sys.argv[3])
Server((sys.argv[1], int(sys.argv[2])), handler).serve_forever()
' "${addr}" "${PORT}" "${WORK}/www" 2>"${WORK}/server.log" &
  # down kills the listener, and a disowned job dies without a shell notice.
  disown
  local i
  for ((i = 0; i < 50; i++)); do
    if "${NETNS}" exec server ss -Hltn "sport = :${PORT}" | grep -q LISTEN; then
      return 0
    fi
    sleep 0.1
  done
  fail "the server listener did not start"
}

# fetch runs curl in the client namespace and checks status and size.
fetch() {
  local addr out
  addr="$(state_value SERVER_ADDR)"
  out="$("${NETNS}" exec client curl -sS --max-time 10 -o /dev/null \
    -w '%{http_code} %{size_download}' "http://${addr}:${PORT}/blob")"
  [[ "${out}" == "200 ${BLOB_SIZE}" ]] || fail "curl returned '${out}', want '200 ${BLOB_SIZE}'"
}

# stats_value FILE KEY prints one field of the pcapng summary.
stats_value() {
  python3 -I "${HARNESS_DIR}/pcapng.py" stats "$1" |
    python3 -I -c 'import json, sys; v = json.load(sys.stdin)[sys.argv[1]]; print(v if not isinstance(v, list) else ",".join(map(str, v)))' "$2"
}

# check_capture FILE MAX_FRAME VLAN_MIN checks that the file is pcapng with
# Ethernet frames, that no frame exceeds MAX_FRAME, and that at least VLAN_MIN
# frames carry an 802.1Q tag.
check_capture() {
  local file="$1" max_frame="$2" vlan_min="$3" packets largest tagged
  [[ "$(head -c 4 "${file}" | od -An -tx1 | tr -d ' \n')" == 0a0d0d0a ]] ||
    fail "${file} does not start with a pcapng section header"
  [[ "$(stats_value "${file}" linktypes)" == 1 ]] || fail "${file} is not Ethernet"
  packets="$(stats_value "${file}" packets)"
  largest="$(stats_value "${file}" max_orig_len)"
  tagged="$(stats_value "${file}" vlan_tagged)"
  ((packets >= 10)) || fail "${file} holds only ${packets} packets"
  ((largest <= max_frame)) || fail "${file} has a ${largest} byte frame above ${max_frame}"
  ((tagged >= vlan_min)) || fail "${file} has ${tagged} tagged frames, want at least ${vlan_min}"
}

# run_case NAME MAX_FRAME VLAN_MIN WANT_PEER runs one configuration from the
# environment and verifies up, curl, capture, and down.
run_case() {
  local name="$1" max_frame="$2" vlan_min="$3" want_peer="$4" before after peer
  before="$(root_snapshot)"
  "${NETNS}" up
  start_server
  "${NETNS}" capture start "${WORK}/${name}.pcapng" client
  fetch
  "${NETNS}" capture stop
  peer="$(sed -n 's/^peer=\([^ ]*\) .*/\1/p' "${WORK}/server.log" | tail -1)"
  [[ "${peer}" == "$(state_value "${want_peer}")" ]] ||
    fail "${name} server saw peer ${peer}, want ${want_peer}"
  check_capture "${WORK}/${name}.pcapng" "${max_frame}" "${vlan_min}"
  "${NETNS}" down
  after="$(root_snapshot)"
  diff <(echo "${before}") <(echo "${after}") >&2 ||
    fail "${name} down left the root namespace changed"
  [[ ! -e "${FPS_STATE_DIR}/${FPS_NETNS}.env" ]] || fail "${name} down left its state file"
  pass "${name}"
}

case_default() {
  run_case default 1514 0 CLIENT_ADDR
}

case_mtu_nat_prefixes() {
  FPS_MTU=1400 FPS_NAT=1 FPS_CLIENT_NET=192.168.77.0/26 FPS_SERVER_NET=172.31.9.0/29 \
    run_case mtu-nat 1414 0 SERVER_GW
}

vlan_supported() {
  local ns="fpstprobe$$"
  ip netns add "${ns}" || return 1
  local ok=1
  if ip -n "${ns}" link add d0 type dummy 2>/dev/null &&
    ip -n "${ns}" link add link d0 name d0.5 type vlan id 5 2>/dev/null; then
    ok=0
  fi
  ip netns del "${ns}"
  return "${ok}"
}

case_vlan() {
  if ! vlan_supported; then
    [[ "${FPS_REQUIRE_VLAN:-0}" != 1 ]] || fail "the kernel cannot create VLAN interfaces"
    skip "vlan, the kernel cannot create 802.1Q interfaces"
    return 0
  fi
  FPS_CLIENT_VLAN=100 FPS_SERVER_VLAN=200 run_case vlan 1518 10 CLIENT_ADDR
}

# expect_refusal NAME CMD... checks that the command exits with status 1 and
# leaves no harness behind.
expect_refusal() {
  local name="$1" status=0
  shift
  "$@" 2>"${WORK}/refusal.log" || status=$?
  ((status == 1)) || fail "${name} exited with ${status}, want 1"
  if ip netns list | grep -q "^${FPS_NETNS}-"; then
    fail "${name} left namespaces behind"
  fi
  pass "${name} is refused"
}

case_refusals() {
  expect_refusal "MTU 100" env FPS_MTU=100 "${NETNS}" up
  expect_refusal "overlapping prefixes" env FPS_CLIENT_NET=10.9.0.0/16 FPS_SERVER_NET=10.9.4.0/24 "${NETNS}" up
  expect_refusal "VLAN 4095" env FPS_CLIENT_VLAN=4095 "${NETNS}" up
  expect_refusal "exec before up" "${NETNS}" exec client true
  "${NETNS}" up
  local status=0
  "${NETNS}" up 2>/dev/null || status=$?
  ((status == 1)) || fail "a second up exited with ${status}, want 1"
  "${NETNS}" exec client true || fail "a second up damaged the running harness"
  "${NETNS}" down
  "${NETNS}" down
  pass "a second up is refused and down is idempotent"
  if command -v setpriv >/dev/null 2>&1; then
    status=0
    setpriv --reuid=65534 --regid=65534 --clear-groups "${NETNS}" up 2>/dev/null || status=$?
    ((status == 1)) || fail "up without root exited with ${status}, want 1"
    pass "up without root is refused"
  fi
}

cleanup() {
  "${NETNS}" down >/dev/null 2>&1 || true
  [[ -n "${WORK:-}" && -d "${WORK}" ]] && find "${WORK}" -mindepth 1 -delete && rmdir "${WORK}"
  return 0
}

main() {
  [[ "${EUID}" -eq 0 ]] || fail "the self test needs root, run it as root or with sudo"
  require ip curl python3 ss od
  WORK="$(mktemp -d)"
  trap cleanup EXIT
  chmod 755 "${WORK}"
  export FPS_NETNS="fpst$$"
  export FPS_STATE_DIR="${WORK}/state"
  mkdir -p "${WORK}/www"
  BLOB_SIZE=400000
  head -c "${BLOB_SIZE}" /dev/urandom >"${WORK}/www/blob"
  case_default
  case_mtu_nat_prefixes
  case_vlan
  case_refusals
  echo "selftest passed, ${PASSED} checks passed and ${SKIPPED} skipped"
}

main "$@"
