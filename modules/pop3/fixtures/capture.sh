#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the pop3 fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and scores the capture with the realism
# runner, which asks nDPI for the application of every session and checks the
# capture with tshark. The script needs root, ip, tcpdump, ss, ndpiReader, and
# tshark.
#
#   sudo modules/pop3/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/pop3/fixtures, where the capture lands as
# pop3.pcapng and the realism report as pop3.report.json. The script exits
# with the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
# sessions per scenario, and FPS_FIXTURE_PEER names a prebuilt fixture_peer.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-8}

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Small attachments keep the fixture reviewable. The scenarios keep their
# structure, and every message still draws its own attachment size.
vars=(--var body_size=900 --var attachment_min=2000 --var attachment_max=12000)

# nDPI 4.2 identifies POP3 only when a PASS command has passed (its other
# condition, four +OK lines, cannot hold in the two bit counter of its flow
# state), so a session that logs in with APOP alone stays unknown. The
# fixture leaves out apop_login for that reason, and apop_fallback covers APOP.
vars+=(--skip apop_login)

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-pop3 --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpspop3fix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() { "$harness" down >/dev/null 2>&1 || true; }
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/pop3.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" "${vars[@]}" &
server_pid=$!
# The server binds port 110 before its first session, so the listener shows
# that it is ready.
for _ in $(seq 100); do
  if "$harness" exec server ss -Hltn "sport = :110" | grep -q LISTEN; then
    break
  fi
  sleep 0.1
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --sessions "$sessions" "${vars[@]}"
wait "$server_pid"
"$harness" capture stop

cd "$root"
python3 -m realism score --pcap "$out/pop3.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/pop3.report.json"
