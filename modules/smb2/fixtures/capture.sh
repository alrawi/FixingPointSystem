#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the smb2 fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and scores the capture with the realism
# runner, which asks nDPI for the application of every session and checks the
# capture with tshark. It needs root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/smb2/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/smb2/fixtures, where the capture lands as
# smb2.pcapng and the realism report as smb2.report.json. The script exits
# with the code of the realism runner, so a capture that fails a check fails.
# FPS_FIXTURE_SESSIONS sets the sessions per scenario, and FPS_FIXTURE_PEER
# names a prebuilt fixture_peer, for runners where root has no Rust toolchain.
# FPS_REALISM_PARSERS names the parsers that score the capture, default tshark.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-6}
port=445

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Smaller files keep the fixture reviewable. Every scenario keeps its structure,
# which is two reads or two writes with a tail of their own. The tails follow
# from the sizes: 100000 = 65536 + 34464 and 90000 = 65536 + 24464.
vars=(--var file_size=100000 --var read_size=65536 --var tail_size=34464
      --var upload_size=90000 --var write_size=65536 --var upload_tail=24464)

peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-smb2 --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpssmb2fix}
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

"$harness" capture start "$out/smb2.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" \
  "${vars[@]}" &
server_pid=$!
for _ in $(seq 100); do
  if "$harness" exec server ss -Hltn "sport = :$port" | grep -q LISTEN; then
    break
  fi
  sleep 0.1
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --sessions "$sessions" "${vars[@]}"
wait "$server_pid"
"$harness" capture stop

cd "$root"
# Every session must have its own 5-tuple. A source port that comes up twice for
# one destination merges two sessions in the session table and makes Zeek report
# a parser error (MOD-21). The peer assigns a source port per session, and this
# check names the cause when a capture still holds fewer sessions than it ran.
expected=$(( sessions * $(find "$here/../scenarios" -name '*.yaml' | wc -l) ))
python3 -I - "$out/smb2.pcapng" "$expected" "$root" <<'PY'
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
python3 -m realism score --pcap "$out/smb2.pcapng" --labels "$here/expected.yaml" \
  --parsers "${FPS_REALISM_PARSERS:-tshark}" --output "$out/smb2.report.json"
