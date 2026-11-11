#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the ldap fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and checks the capture two ways. It first
# requires one session in the capture for every session that the peers ran,
# which fails when two sessions share a 5-tuple (MOD-21). The realism runner
# then scores the sessions with nDPI and tshark, which must classify every
# session as LDAP and report no parser error. The peers fail on any session
# whose messages do not decode or whose result codes differ from the scenario.
# The script needs root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/ldap/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/ldap/fixtures, where the capture lands as
# ldap.pcapng and the realism report as ldap.report.json. The script exits with
# the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
# sessions per scenario, FPS_FIXTURE_PEER names a prebuilt fixture_peer, and
# FPS_REALISM_PARSERS names the parsers that score the capture, default tshark.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-6}
port=389

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# The sessions are short, so the scenarios run with their defaults. Every
# client session binds as another account of the directory and every listing
# returns the whole directory, which keeps the fixture small and varied.
vars=()

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-ldap --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpsldapfix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() { "$harness" down >/dev/null 2>&1 || true; }
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/ldap.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" "${vars[@]}" &
server_pid=$!
# The server binds the port before its first session, so the listener shows
# that it is ready. A probe connection would add a session to the capture.
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
python3 -I - "$out/ldap.pcapng" "$expected" "$root" <<'PY'
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
python3 -m realism score --pcap "$out/ldap.pcapng" --labels "$here/expected.yaml" \
  --parsers "${FPS_REALISM_PARSERS:-tshark}" --output "$out/ldap.report.json"
