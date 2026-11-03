#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Runs contracts/examples/plans/http_dns_smoke.json unchanged in the QA-01
# namespace harness. The server side runs in the server namespace with
# --side server and the client side in the client namespace with --side
# client, so every session crosses the routing namespace. The test passes
# when both processes exit with the pass code and the final snapshot of each
# side reports successful http1 and dns sessions.
#
# The harness links use 10.201.0.0/24 and 10.202.0.0/24. The plan pools
# 16.0.0.0/24 and 48.0.0.0/24 become local routes on the loopback of their
# namespace (AnyIP), so the agent binds any pool address, and the router
# routes each pool to its arm.
#
# Usage: sudo engine/fps-agent/tests/netns_smoke.sh AGENT_BINARY [OUT_DIR]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
readonly ROOT
AGENT="${1:?usage: netns_smoke.sh AGENT_BINARY [OUT_DIR]}"
AGENT="$(cd "$(dirname "${AGENT}")" && pwd)/$(basename "${AGENT}")"
OUT="${2:-$(mktemp -d)}"
mkdir -p "${OUT}"
OUT="$(cd "${OUT}" && pwd)"
readonly AGENT OUT
PLAN="${ROOT}/contracts/examples/plans/http_dns_smoke.json"
MODULES="${ROOT}/contracts/examples/modules"
HARNESS="${ROOT}/tests/harness/netns.sh"

export FPS_NETNS="${FPS_NETNS:-agt01}"
export FPS_STATE_DIR="${FPS_STATE_DIR:-${OUT}/state}"

die() {
  echo "netns_smoke.sh: $*" >&2
  exit 1
}

[[ "${EUID}" -eq 0 ]] || die "this test needs root"
[[ -x "${AGENT}" ]] || die "agent binary ${AGENT} is not executable"

cleanup() {
  "${HARNESS}" down >/dev/null 2>&1 || true
}
trap cleanup EXIT

"${HARNESS}" up
info() {
  "${HARNESS}" info | sed -n "s/^$1=//p"
}
CLIENT_ADDR="$(info CLIENT_ADDR)"
SERVER_ADDR="$(info SERVER_ADDR)"
ROUTER_IN="ip netns exec ${FPS_NETNS}-router"

# The pools of the plan, read from the plan so the test follows it.
pool() {
  python3 -I -c 'import json,sys; p=json.load(open(sys.argv[1])); print(next(h["ipv4Prefix"] for h in p["hostPools"] if h["role"]==sys.argv[2]))' "${PLAN}" "$1"
}
CLIENT_POOL="$(pool HOST_ROLE_CLIENT)"
SERVER_POOL="$(pool HOST_ROLE_SERVER)"
${ROUTER_IN} ip route add "${CLIENT_POOL}" via "${CLIENT_ADDR}"
${ROUTER_IN} ip route add "${SERVER_POOL}" via "${SERVER_ADDR}"
"${HARNESS}" exec client ip route add local "${CLIENT_POOL}" dev lo
"${HARNESS}" exec server ip route add local "${SERVER_POOL}" dev lo

"${HARNESS}" exec server "${AGENT}" run --plan "${PLAN}" --modules "${MODULES}" \
  --side server --linger 5 >"${OUT}/server.jsonl" 2>"${OUT}/server.log" &
SERVER_PID=$!

# The server side is ready when its first TCP listener is bound.
FIRST_SERVER="$(python3 -I -c 'import ipaddress,sys; print(next(ipaddress.ip_network(sys.argv[1]).hosts()))' "${SERVER_POOL}")"
for ((i = 0; i < 100; i++)); do
  if "${HARNESS}" exec server ss -Hltn src "${FIRST_SERVER}" | grep -q .; then
    break
  fi
  kill -0 "${SERVER_PID}" 2>/dev/null || { cat "${OUT}/server.log" >&2; die "the server side ended early"; }
  sleep 0.1
done

CLIENT_RC=0
"${HARNESS}" exec client "${AGENT}" run --plan "${PLAN}" --modules "${MODULES}" \
  --side client >"${OUT}/client.jsonl" 2>"${OUT}/client.log" || CLIENT_RC=$?
SERVER_RC=0
wait "${SERVER_PID}" || SERVER_RC=$?

echo "--- client log (exit ${CLIENT_RC})"
cat "${OUT}/client.log"
echo "--- server log (exit ${SERVER_RC})"
cat "${OUT}/server.log"
[[ "${CLIENT_RC}" -eq 0 ]] || die "the client side exited with ${CLIENT_RC}, not the pass code 0"
[[ "${SERVER_RC}" -eq 0 ]] || die "the server side exited with ${SERVER_RC}, not the pass code 0"

python3 -I - "${OUT}/client.jsonl" "${OUT}/server.jsonl" <<'EOF'
import json
import sys


def totals(path):
    lines = [json.loads(line) for line in open(path) if line.strip()]
    if not lines or not lines[-1].get("isFinal"):
        sys.exit(f"{path}: the last line is not the final snapshot")
    for i, line in enumerate(lines, 1):
        if line.get("sequence") != str(i):
            sys.exit(f"{path}: line {i} has sequence {line.get('sequence')}")
    out = {}
    for c in lines[-1].get("counters", []):
        if c["metric"] in ("sessions.succeeded", "sessions.attempted"):
            app = c["labels"]["application"]
            key = (c["metric"], app)
            out[key] = out.get(key, 0) + int(c.get("count", "0"))
    return len(lines), out


for side, path in zip(("client", "server"), sys.argv[1:]):
    n, t = totals(path)
    print(f"{side}: {n} snapshot lines, {t}")
    for app in ("http1", "dns"):
        if t.get(("sessions.succeeded", app), 0) == 0:
            sys.exit(f"{side}: no successful {app} sessions")
    if n < 40:
        sys.exit(f"{side}: only {n} snapshot lines for a 45 second plan")
EOF
echo "netns_smoke.sh: the example plan passed on both arms"
