#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records a first party module on the AF_XDP backend and scores the capture with
# the realism runner (TRN-12 for http1, TRN-36 for every module).
#
#   sudo engine/fps-datapath/realism/module-afxdp.sh MODULE AGENT_BINARY OUTPUT_DIR
#
# MODULE is a directory of modules/ with scenarios, manifest.yaml, and
# fixtures/expected.yaml, for example http1, dns, or sip_rtp. The agent runs the
# scenarios of the module as mix entries of plans with the backend
# BACKEND_AF_XDP (make_plan.py), so every session goes through the own TCP and
# UDP stacks and the AF_XDP sockets of the agent. A plan listens once for each
# port of a server host, so the scenarios run in rounds, one run of the agent per
# round, and all rounds go into one capture. The agent needs both arms of the
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
# router's client arm does in the kernel run (modules/MODULE/fixtures/capture.sh).
#
# The capture is scored like the kernel run: the labels of expected.yaml, so the
# conformance check, the parser error check, and the checks that the labels ask
# for (the encryption check with the decryption and the JA4 of the https
# sessions, the temporal and structure checks), then the module inside TLS on the
# decrypted view when the kernel run has one. profiles.yaml holds what capture.sh
# sets for the module: the session counts, the variables, and the ports.
#
# The script exits with 0 when every check passes, 1 when a check or a run of the
# agent fails, and 2 when the run cannot start. It needs root, ip, ethtool,
# tcpdump, editcap, ndpiReader, tshark, and PyYAML (realism/ci/install-tools.sh
# installs them).
#
#   FPS_AFXDP_SESSIONS  sessions per scenario, about; default the profile of the module
#   FPS_AFXDP_PARSERS   reference parsers of the realism runner, default tshark
#   FPS_AFXDP_NETNS     name prefix of the namespaces, default fpsxd
#   FPS_AFXDP_LINGER    seconds that the agent keeps the sockets after a round, default 2
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
readonly here root

die() {
  echo "module-afxdp.sh: $*" >&2
  exit 2
}

[[ $# -eq 3 ]] || die "usage: module-afxdp.sh MODULE AGENT_BINARY OUTPUT_DIR"
module=$1
[[ $module =~ ^[a-z0-9][a-z0-9_-]*$ ]] || die "$module is not a module name"
[[ -f $root/modules/$module/fixtures/expected.yaml && -f $root/modules/$module/manifest.yaml ]] ||
  die "modules/$module has no manifest.yaml and fixtures/expected.yaml"
[[ $EUID -eq 0 ]] || die "this script needs root"
[[ -x $2 ]] || die "$2 is not an executable agent"
agent=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
mkdir -p "$3"
out=$(cd "$3" && pwd)
readonly agent out
parsers=${FPS_AFXDP_PARSERS:-tshark}
prefix=${FPS_AFXDP_NETNS:-fpsxd}
linger=${FPS_AFXDP_LINGER:-2}
ns_agent=$prefix-agent
ns_wire=$prefix-wire
for tool in ip tcpdump ndpiReader tshark editcap; do
  command -v "$tool" >/dev/null || die "$tool is not installed"
done

work=$(mktemp -d)
capture_pid=
# shellcheck disable=SC2317,SC2329 # runs through the EXIT trap (SC2317 on older shellcheck, SC2329 on newer)
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
usage=$("$agent" run --help 2>&1 || true)
if [[ $usage == *"--keylog"* ]]; then
  keys=keylog
else
  echo "module-afxdp.sh: NOTICE this agent has no run --keylog, so the capture cannot be decrypted and the encryption check is not scored" >&2
fi
session_args=()
[[ -z ${FPS_AFXDP_SESSIONS:-} ]] || session_args=(--sessions "$FPS_AFXDP_SESSIONS")
rounds=$(python3 "$here/make_plan.py" "$root" "$out" "$module" "${session_args[@]}" --keys "$keys") ||
  die "make_plan.py could not plan $module"

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

raw=$work/$module-afxdp.pcap
ip netns exec "$ns_wire" tcpdump -n -s 0 -U --immediate-mode -i wc -w "$raw" 2>"$work/tcpdump.log" &
capture_pid=$!
for _ in $(seq 50); do
  grep -q "listening on" "$work/tcpdump.log" && break
  sleep 0.1
done

# The bridge and the veth ports need a moment before the first datagram: a UDP
# session does not retransmit, and the first round lost its first datagrams here.
sleep "${FPS_AFXDP_SETTLE:-1}"
agent_rc=0
: >"$work/keys.log"
for ((round = 0; round < rounds; round++)); do
  keylog_args=()
  [[ $keys != keylog ]] || keylog_args=(--keylog "$work/keys-$round.log")
  rc=0
  ip netns exec "$ns_agent" "$agent" run --plan "$out/round-$round/plan.json" --modules "$out/modules" \
    --linger "$linger" "${keylog_args[@]}" \
    >"$out/agent-$round.jsonl" 2>"$out/agent-$round.log" || rc=$?
  echo "module-afxdp.sh: round $round of $rounds: the agent exited with $rc" >&2
  grep -E "backend af_xdp bound|datapath lane|outcome|error" "$out/agent-$round.log" >&2 || true
  [[ $rc -eq 0 ]] || agent_rc=1
  if [[ $keys == keylog && -f $work/keys-$round.log ]]; then
    cat "$work/keys-$round.log" >>"$work/keys.log"
  fi
done
sleep 1
kill "$capture_pid"
wait "$capture_pid" 2>/dev/null || true
capture_pid=

pcap=$out/$module-afxdp.pcapng
python3 -I "$root/tests/harness/pcapng.py" convert "$raw" "$pcap"
if [[ $keys == keylog ]]; then
  # Embed the key log in the capture, as capture.sh does for the kernel run.
  # A module without TLS leaves the key log empty, and editcap refuses an empty file.
  if [[ -s $work/keys.log ]]; then
    editcap --inject-secrets "tls,$work/keys.log" "$pcap" "$work/keyed.pcapng"
    mv "$work/keyed.pcapng" "$pcap"
  fi
fi

cd "$root"
# The labels of the kernel run count sessions that the agent starts at Poisson
# arrivals, so the counts come from what it reported.
python3 "$here/finalize.py" counts "$root" "$out" "$module"
score_rc=0
python3 -m realism score --pcap "$pcap" --labels "$out/labels.yaml" \
  --parsers "$parsers" --output "$out/$module-afxdp.report.json" || score_rc=$?
check_rc=0
if [[ $keys == keylog && -s $out/decrypted.args ]]; then
  # The module inside the TLS sessions, scored on a decrypted view as in the
  # kernel run. Every labelled TLS session of the capture must be in the view.
  mapfile -t decrypted_args <"$out/decrypted.args"
  tls_sessions=$(python3 -I -c 'import json, sys
r = json.load(open(sys.argv[1]))
print(next(c["labelled_sessions"] for c in r["checks"] if c["check"] == "encryption"))' \
    "$out/$module-afxdp.report.json")
  python3 -m realism decrypted --pcap "$pcap" "${decrypted_args[@]}" \
    --sessions "$tls_sessions" --parsers "$parsers" \
    --view "$out/$module-afxdp.decrypted.pcap" \
    --output "$out/$module-afxdp.decrypted.json" || check_rc=$?
fi
# The scripts that the kernel run runs next to the score.
extra_rc=0
python3 "$here/finalize.py" checks "$root" "$out" "$module" "$pcap" || extra_rc=$?
if [[ $score_rc -ne 0 ]]; then
  echo "module-afxdp.sh: $module failed a realism check, see $out/$module-afxdp.report.json" >&2
elif [[ $check_rc -ne 0 ]]; then
  echo "module-afxdp.sh: $module failed the decrypted check, see $out/$module-afxdp.decrypted.json" >&2
elif [[ $extra_rc -ne 0 ]]; then
  echo "module-afxdp.sh: $module failed a check that its kernel run makes next to the score" >&2
elif [[ $agent_rc -ne 0 ]]; then
  echo "module-afxdp.sh: a run of the agent failed, see $out/agent-*.log" >&2
else
  echo "module-afxdp.sh: $module passed every realism check on AF_XDP" >&2
fi
exit $(( score_rc != 0 ? score_rc : (check_rc != 0 ? check_rc : (extra_rc != 0 ? extra_rc : agent_rc)) ))
