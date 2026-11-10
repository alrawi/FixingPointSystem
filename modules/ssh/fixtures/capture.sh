#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the ssh fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and checks the capture three ways. It
# first requires one session in the capture for every session that the peers
# ran, which fails when two sessions share a 5-tuple (MOD-21). The realism
# runner then scores the sessions with nDPI and tshark, which must classify
# every session as SSH. When Zeek is installed, zeek_check.py also requires one
# ssh.log row per session with the negotiated algorithms and one host key. The
# peers fail on any session whose key exchange or packet tags do not verify.
# The script needs root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/ssh/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/ssh/fixtures, where the capture lands as
# ssh.pcapng and the realism report as ssh.report.json. The script exits with
# the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
# sessions per scenario, and FPS_FIXTURE_PEER names a prebuilt fixture_peer.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-4}
port=22

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Short sessions and small transfers keep the fixture reviewable. The scenarios
# keep their structure, and every session still draws its own sizes and times.
vars=(--var commands=3 --var typed=6 --var output_bytes=40000 --var file_bytes=60000)

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-ssh --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpssshfix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() { "$harness" down >/dev/null 2>&1 || true; }
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/ssh.pcapng" client
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
python3 -I - "$out/ssh.pcapng" "$expected" "$root" <<'PY'
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
python3 -m realism score --pcap "$out/ssh.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/ssh.report.json"

# Zeek is optional here, because the realism job installs it and scores the
# same capture for parser errors. Where it is present, the check below also
# reads ssh.log.
zeek=${FPS_ZEEK:-$(command -v zeek || true)}
if [[ -z $zeek && -x /opt/zeek/bin/zeek ]]; then
  zeek=/opt/zeek/bin/zeek
fi
if [[ -n $zeek ]]; then
  python3 "$here/zeek_check.py" "$out/ssh.pcapng" --zeek "$zeek" --sessions "$expected"
else
  echo "capture.sh: skipped the Zeek check because zeek is not installed" >&2
fi
