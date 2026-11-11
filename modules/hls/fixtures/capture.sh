#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the hls fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and checks the capture three ways:
#
#   1. The capture holds one session for every TCP connection that the client
#      peer opened. The peer gives every connection its own source port
#      (MOD-21), so a capture with fewer sessions has a reused 5-tuple or lost
#      packets, and the script names that cause.
#   2. The realism runner scores it with nDPI and tshark against expected.yaml.
#      Its temporal check compares the gaps between the starts of the segment
#      sessions of vod_segments with the segment duration.
#   3. hls_check.py reads the playbacks and checks the segment cadence inside
#      each of them and the byte share of each rung against the configured
#      bit rates.
#
# It needs root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/hls/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/hls/fixtures, where the capture lands as
# hls.pcapng, the realism report as hls.report.json, and the playback check
# as hls.playbacks.json. The parameters of the capture, which are the ladder,
# the segment duration, and the length of every playback, are in params.json,
# next to this script. The script exits with the first nonzero code of the
# three checks, so a capture that fails one fails.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
params=${FPS_FIXTURE_PARAMS:-$here/params.json}
log=$(mktemp)

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# The peer takes its variables and session counts from params.json, so the
# capture and the playback check read one set of numbers.
mapfile -t peer_args < <(python3 -I - "$params" <<'PY'
import json
import sys

p = json.load(open(sys.argv[1]))
print("--var")
print("ladder=" + p["ladder"])
print("--var")
print(f"segment_ms={p['segment_ms']}")
for name, n in p["sessions"].items():
    print("--sessions-for")
    print(f"{name}={n}")
for name, values in p["vars"].items():
    for key, value in values.items():
        print("--var")
        print(f"{name}:{key}={value}")
PY
)

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-hls --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpsh15fix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() {
  "$harness" down >/dev/null 2>&1 || true
  rm -f "$log"
}
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/hls.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" "${peer_args[@]}" &
server_pid=$!
# The server binds the ports of every scenario before its first session. The
# last one is the live port, so wait for the playlist port of each scenario.
for port in $(python3 -I -c 'import json,sys; print(*json.load(open(sys.argv[1]))["ports"].values())' "$params"); do
  for _ in $(seq 100); do
    if "$harness" exec server ss -Hltn "sport = :$port" | grep -q LISTEN; then
      break
    fi
    sleep 0.1
  done
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  "${peer_args[@]}" | tee "$log"
wait "$server_pid"
"$harness" capture stop

cd "$root"
# Every connection must have its own 5-tuple. A source port that comes up twice
# for one destination merges two sessions in the session table and makes Zeek
# report a parser error (MOD-21). The peer assigns a source port per
# connection, and this check names the cause when a capture still holds fewer
# sessions than it opened.
expected=$(sed -n 's/^connections=//p' "$log")
python3 -I - "$out/hls.pcapng" "$expected" "$root" <<'PY'
import sys
from pathlib import Path

sys.path.insert(0, sys.argv[3])
from realism.capture import summarize

found = len(summarize(Path(sys.argv[1])).sessions)
expected = int(sys.argv[2])
if found != expected:
    sys.exit(
        f"capture.sh: the capture holds {found} sessions and the client opened {expected} "
        "connections, so sessions share a 5-tuple (a reused source port) or lost their packets"
    )
PY
score_rc=0
python3 -m realism score --pcap "$out/hls.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/hls.report.json" || score_rc=$?
check_rc=0
python3 -I "$here/hls_check.py" "$out/hls.pcapng" --params "$params" \
  --output "$out/hls.playbacks.json" || check_rc=$?
exit $(( score_rc != 0 ? score_rc : check_rc ))
