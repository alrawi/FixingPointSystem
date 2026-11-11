#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the mqtt fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, and runs every scenario twice, once in MQTT 3.1.1 and once
# in MQTT 5.0. In each round the server side runs in the server namespace and
# the client side in the client namespace, both over kernel sockets, and the
# router's client arm is captured. The script then scores the capture with the
# realism runner, which asks nDPI for the application of every session and
# checks the capture with tshark. It needs root, ip, tcpdump, ss, ndpiReader,
# and tshark.
#
#   sudo modules/mqtt/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/mqtt/fixtures, where the capture lands as
# mqtt.pcapng and the realism report as mqtt.report.json. The script exits
# with the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
# sessions per scenario and round, and FPS_FIXTURE_PEER names a prebuilt
# fixture_peer, for runners where root has no Rust toolchain.
# FPS_REALISM_PARSERS names the parsers that score the capture, default tshark.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-4}
port=1883

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Short gaps keep the fixture small. A keep alive of 2 seconds with 3 pings
# makes the keepalive scenario a session of 6 seconds, which still shows the
# paced PINGREQ packets. Every scenario keeps its structure, and every session
# still draws its own gaps and readings.
common=(--var keepalive=2 --var pings=3 --var messages=6
        --var interval_min_ms=100 --var interval_max_ms=700
        --var push_min_ms=100 --var push_max_ms=700)
# The properties of MQTT 5.0 that the scenarios can set.
v5=(--var session_expiry=300 --var content_type=application/json --var retain_handling=1)

peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-mqtt --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpsmqttfix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() {
  "$harness" down >/dev/null 2>&1 || true
}
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/mqtt.pcapng" client

# One round runs every scenario at one protocol level. The rounds use their own
# seeds, so the client identifiers and the readings differ, and their own range
# of client source ports, so that no two sessions of the capture share a
# 5-tuple (MOD-21). The ranges of the two rounds are 1000 ports apart, which is
# far more than the sessions of a round need.
round() {
  local version=$1 seed=$2 first_port=$3
  shift 3
  local vars=("${common[@]}" --var "version=$version" "$@")
  "$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" \
    --seed "$seed" "${vars[@]}" &
  local server_pid=$!
  # The server binds the port before its first session, so the listener shows
  # that it is ready. A probe connection would add a session to the capture.
  for _ in $(seq 100); do
    if "$harness" exec server ss -Hltn "sport = :$port" | grep -q LISTEN; then
      break
    fi
    sleep 0.1
  done
  "$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
    --sessions "$sessions" --seed "$seed" --first-port "$first_port" "${vars[@]}"
  wait "$server_pid"
}

round 4 42 40000
round 5 43 41000 "${v5[@]}"
"$harness" capture stop

cd "$root"
# Every session must have its own 5-tuple. A source port that comes up twice for
# one destination merges two sessions in the session table and makes Zeek report
# a parser error (MOD-21). The peer assigns a source port per session, and this
# check names the cause when a capture still holds fewer sessions than it ran.
expected=$(( 2 * sessions * $(find "$here/../scenarios" -name '*.yaml' | wc -l) ))
python3 -I - "$out/mqtt.pcapng" "$expected" "$root" <<'PY'
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[3])
from realism.capture import summarize

found = len(summarize(Path(sys.argv[1])).sessions)
expected = int(sys.argv[2])
if found != expected:
    sys.exit(
        f"capture.sh: the capture holds {found} sessions and the peers ran {expected}, "
        "so sessions share a 5-tuple (a reused source port) or lost their packets"
    )
PY
python3 -m realism score --pcap "$out/mqtt.pcapng" --labels "$here/expected.yaml" \
  --parsers "${FPS_REALISM_PARSERS:-tshark}" --output "$out/mqtt.report.json"
