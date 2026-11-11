#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the ntp fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel UDP
# sockets, captures the router's client arm, and checks the capture two ways.
# It first requires one session in the capture for every session that the
# peers ran, which fails when two sessions share a 5-tuple (MOD-21). The
# realism runner then scores the sessions with nDPI and tshark, which must
# classify every session as NTP and report no parser error. The script needs
# root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/ntp/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/ntp/fixtures, where the capture lands as
# ntp.pcapng and the realism report as ntp.report.json. The script exits with
# the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
# sessions per scenario, and FPS_FIXTURE_PEER names a prebuilt fixture_peer.
#
# The server listens on port 123, the port that nDPI needs to classify NTP.
# The namespace harness gives the server its own network namespace, so the
# privileged port is free, and root may bind it. The pauses of the poll
# scenarios shrink with --poll-ms from 64 and 128 seconds to 1 and 2 seconds,
# so that the capture takes about a minute, and the burst keeps its real gap
# of 2 seconds. The poll exponents in the packets stay 6 and 7.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-6}
port=123

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-ntp --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpsntpfix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() { "$harness" down >/dev/null 2>&1 || true; }
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/ntp.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --port "$port" --sessions "$sessions" \
  --poll-ms 1000 &
server_pid=$!
# The server binds its UDP socket before its first session, so the socket
# shows that it is ready. A probe datagram would add a session to the capture.
for _ in $(seq 100); do
  if "$harness" exec server ss -Hlun "sport = :$port" | grep -q .; then
    break
  fi
  sleep 0.1
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --port "$port" --sessions "$sessions" --poll-ms 1000
wait "$server_pid"
"$harness" capture stop

cd "$root"
# Every session must have its own 5-tuple. A source port that comes up twice for
# one destination merges two sessions in the session table (MOD-21). The peer
# assigns a source port per session, and this check names the cause when a
# capture still holds fewer sessions than it ran.
expected=$(( sessions * $(find "$here/../scenarios" -name '*.yaml' | wc -l) ))
python3 -I - "$out/ntp.pcapng" "$expected" "$root" <<'PY'
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
python3 -m realism score --pcap "$out/ntp.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/ntp.report.json"
