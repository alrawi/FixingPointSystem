#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the strike fixture inside the QA-01 namespace harness.
#
# The capture has two phases under one pcap. The slowloris phase runs the
# scenario on its default port 8080. The churn phase runs conn_churn on its
# own ports, so the structure check of the realism runner gives every
# scenario a session pool of its own (modules/strike/fixtures/expected.yaml).
# The server side runs in the server namespace and the client side in the
# client namespace, and the router's client leg records the traffic between
# them.
#
# Requires root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/strike/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/strike/fixtures, where the recording lands
# as strike.pcapng next to the realism report strike.report.json. The script
# exits with the realism runner's exit code, so the report is printed and
# checked, never ignored.

set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-20}
# The churn phase moves conn_churn off the slowloris ports: its keeper holds
# 8081 and its churned children dial 8082.
churn_vars=(--var server_port=8081 --var churn_port=8082)

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh must run as root (it needs the namespaces, the raw sockets, and tcpdump)" >&2
  exit 1
fi

peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  echo "building the fixture peer"
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-strike --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpsstrikefix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"

mkdir -p "$out"
cleanup() {
  "$harness" down >/dev/null 2>&1 || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

echo "bringing up $FPS_NETNS"
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

# Waits until the server namespace listens on a port.
wait_ready() {
  for _ in $(seq 100); do
    if "$harness" exec server ss -Hltn "sport = :$1" | grep -q LISTEN; then
      return 0
    fi
    sleep 0.1
  done
  echo "capture.sh: nothing listens on port $1" >&2
  return 1
}

echo "recording $out/strike.pcapng ($sessions sessions per scenario)"
"$harness" capture start "$out/strike.pcapng" client

# The slowloris phase: trickle sessions on the scenario's default port 8080.
"$harness" exec server "$peer" server --listen "$server_addr" --scenario slowloris --sessions "$sessions" &
server_pid=$!
wait_ready 8080
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" --scenario slowloris --sessions "$sessions"
wait "$server_pid"

# The churn phase: keeper connections on 8081, churned children on 8082.
"$harness" exec server "$peer" server --listen "$server_addr" --scenario conn_churn --sessions "$sessions" "${churn_vars[@]}" &
server_pid=$!
wait_ready 8081
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" --scenario conn_churn --sessions "$sessions" "${churn_vars[@]}"
wait "$server_pid"

"$harness" capture stop

cd "$root"
python3 -m realism score --pcap "$out/strike.pcapng" --labels modules/strike/fixtures/expected.yaml --parsers tshark --output "$out/strike.report.json"
