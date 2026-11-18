#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the http1 module on the AF_XDP backend and scores the capture with the
# realism runner (TRN-12).
#
#   sudo engine/fps-datapath/realism/http1-afxdp.sh AGENT_BINARY OUTPUT_DIR
#
# The agent runs the ten http1 scenarios as ten mix entries of one plan with the
# backend BACKEND_AF_XDP (make_plan.py), so every session goes through the own
# TCP stack and the AF_XDP sockets of the agent. The agent needs both arms of the
# datapath in one process, so the script builds this wire:
#
#   <p>-agent                           <p>-wire
#   dpc ---------------------------- wc ]
#                                        ] br0 (a bridge, which only forwards)
#   dps ---------------------------- ws ]
#
# AF_XDP takes the frames of dpc and dps before the kernel sees them, so a
# capture on either end sees nothing. The other ends are ordinary veth ports:
# tcpdump on wc sees every frame of both directions, as the capture on the
# router's client arm does in the kernel run (modules/http1/fixtures/capture.sh).
#
# The capture is scored like the kernel run: the labels of expected.yaml with
# the plain scenarios on the ports 8080 to 8085, the conformance check, the
# parser error check, and the encryption check (decryption and JA4) of the https
# sessions. The script exits with 0 when every check passes, 1 when a check
# fails, and 2 when the run cannot start. It needs root, ip, ethtool, tcpdump,
# editcap, ndpiReader, tshark, and PyYAML (realism/ci/install-tools.sh installs
# them).
#
#   FPS_AFXDP_SESSIONS  sessions per scenario, about, default 20
#   FPS_AFXDP_PARSERS   reference parsers of the realism runner, default tshark
#   FPS_AFXDP_NETNS     name prefix of the namespaces, default fpsxd
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
readonly here root

die() {
  echo "http1-afxdp.sh: $*" >&2
  exit 2
}

[[ $# -eq 2 ]] || die "usage: http1-afxdp.sh AGENT_BINARY OUTPUT_DIR"
[[ $EUID -eq 0 ]] || die "this script needs root"
[[ -x $1 ]] || die "$1 is not an executable agent"
agent=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
mkdir -p "$2"
out=$(cd "$2" && pwd)
readonly agent out
sessions=${FPS_AFXDP_SESSIONS:-20}
parsers=${FPS_AFXDP_PARSERS:-tshark}
prefix=${FPS_AFXDP_NETNS:-fpsxd}
ns_agent=$prefix-agent
ns_wire=$prefix-wire
for tool in ip tcpdump ndpiReader tshark editcap; do
  command -v "$tool" >/dev/null || die "$tool is not installed"
done

work=$(mktemp -d)
capture_pid=
# shellcheck disable=SC2329 # runs through the EXIT trap
cleanup() {
  if [[ -n $capture_pid ]]; then
    kill "$capture_pid" 2>/dev/null || true
    wait "$capture_pid" 2>/dev/null || true
  fi
  ip netns del "$ns_agent" 2>/dev/null || true
  ip netns del "$ns_wire" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

# The agent exports the TLS secrets of a standalone run to a key log when it
# knows the flag --keylog. The capture then carries them and the encryption check
# decrypts the https sessions, as in the kernel run. An agent without the flag
# leaves the decryption out of the labels, and the script says so.
keys=none
keylog_args=()
if "$agent" run --help 2>&1 | grep -q -- '--keylog'; then
  keys=keylog
  keylog_args=(--keylog "$work/keys.log")
else
  echo "http1-afxdp.sh: NOTICE this agent has no run --keylog, so the capture cannot be decrypted and the encryption check is not scored" >&2
fi
python3 "$here/make_plan.py" "$root" "$out" --sessions "$sessions" --keys "$keys"

ip netns add "$ns_agent"
ip netns add "$ns_wire"
for arm in c s; do
  ip -n "$ns_agent" link add "dp$arm" type veth peer name "w$arm" netns "$ns_wire"
  ip -n "$ns_agent" link set "dp$arm" up
  # The agent writes whole frames and checks every checksum itself.
  ip netns exec "$ns_agent" ethtool -K "dp$arm" tx off rx off >/dev/null 2>&1 || true
done
ip -n "$ns_wire" link add br0 type bridge
ip -n "$ns_wire" link set br0 type bridge ageing_time 0 2>/dev/null || true
for arm in c s; do
  ip netns exec "$ns_wire" ethtool -K "w$arm" tx off rx off >/dev/null 2>&1 || true
  ip -n "$ns_wire" link set "w$arm" master br0
  ip -n "$ns_wire" link set "w$arm" up
done
ip -n "$ns_wire" link set br0 up
# IPv6 would add router solicitations and multicast reports to the capture.
for ns in "$ns_agent" "$ns_wire"; do
  ip netns exec "$ns" sysctl -qw net.ipv6.conf.all.disable_ipv6=1 net.ipv6.conf.default.disable_ipv6=1 2>/dev/null || true
done

raw=$work/http1-afxdp.pcap
ip netns exec "$ns_wire" tcpdump -n -s 0 -U --immediate-mode -i wc -w "$raw" 2>"$work/tcpdump.log" &
capture_pid=$!
for _ in $(seq 50); do
  grep -q "listening on" "$work/tcpdump.log" && break
  sleep 0.1
done

agent_rc=0
ip netns exec "$ns_agent" "$agent" run --plan "$out/plan.json" --modules "$out/modules" "${keylog_args[@]}" \
  >"$out/agent.jsonl" 2>"$out/agent.log" || agent_rc=$?
sleep 1
kill "$capture_pid"
wait "$capture_pid" 2>/dev/null || true
capture_pid=
echo "http1-afxdp.sh: the agent exited with $agent_rc" >&2
grep -E "backend af_xdp bound|datapath lane|outcome" "$out/agent.log" >&2 || true

python3 -I "$root/tests/harness/pcapng.py" convert "$raw" "$out/http1-afxdp.pcapng"
if [[ $keys == keylog ]]; then
  # Embed the key log in the capture, as capture.sh does for the kernel run.
  editcap --inject-secrets "tls,$work/keys.log" "$out/http1-afxdp.pcapng" "$work/keyed.pcapng"
  mv "$work/keyed.pcapng" "$out/http1-afxdp.pcapng"
fi

cd "$root"
score_rc=0
python3 -m realism score --pcap "$out/http1-afxdp.pcapng" --labels "$out/labels.yaml" \
  --parsers "$parsers" --output "$out/http1-afxdp.report.json" || score_rc=$?
exit $(( score_rc != 0 ? score_rc : agent_rc ))
