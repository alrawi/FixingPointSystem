#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the dns fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and scores the capture with the realism
# runner. It needs root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/dns/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/dns/fixtures, where the capture lands as
# dns.pcapng and the realism report as dns.report.json. The script exits
# with the code of the realism runner, so a capture that fails a check fails.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-20}
port=53

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-dns --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpsdnsfix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() { "$harness" down >/dev/null 2>&1 || true; }
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/dns.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --port "$port" --sessions "$sessions" &
server_pid=$!
# The server binds its UDP socket and TCP listener together, so the TCP
# listener shows that both are ready.
for _ in $(seq 100); do
  if "$harness" exec server ss -Hltn "sport = :$port" | grep -q LISTEN; then
    break
  fi
  sleep 0.1
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --port "$port" --sessions "$sessions"
wait "$server_pid"
"$harness" capture stop

cd "$root"
python3 -m realism score --pcap "$out/dns.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/dns.report.json"
