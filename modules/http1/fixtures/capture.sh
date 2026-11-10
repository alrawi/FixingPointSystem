#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the http1 fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and scores the capture with the realism
# runner. It needs root, ip, tcpdump, ss, ndpiReader, tshark, and editcap.
#
#   sudo modules/http1/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/http1/fixtures, where the capture lands as
# http1.pcapng and the realism report as http1.report.json. The script exits
# with the code of the realism runner, so a capture that fails a check fails.
#
# The four https scenarios (MOD-04) run over TLS. The client peer writes the
# secrets of every handshake to a key log, and the script injects the key log
# into the capture as a decryption secrets block, so the realism runner and
# tshark decrypt the capture without a sidecar file. FPS_FIXTURE_KEYLOG names a
# file that keeps the key log as well, for a reader that wants the sidecar. The
# script then runs `python3 -m realism decrypted`, which decrypts the capture
# with tshark and scores the HTTP inside TLS on a decrypted view with nDPI, with
# the labels realism/examples/decrypted/http1.labels.yaml. The result lands as
# http1.decrypted.json. FPS_FIXTURE_DECRYPTED_DIR keeps the decrypted view, a
# capture of the plaintext streams, as http1.decrypted.pcap.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-20}
port=8080

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Small bodies keep the fixture reviewable. Every scenario keeps its structure.
vars=(--var body_size=1200 --var page_size=3000 --var chunk_size=1024
      --var objects=3 --var object_size=900 --var upload_size=2500)

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-http1 --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

work=$(mktemp -d)
ca="$work/ca.pem"
keys=${FPS_FIXTURE_KEYLOG:-$work/keys.log}
: > "$keys"

export FPS_NETNS=${FPS_NETNS:-fpsh1fix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() {
  "$harness" down >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/http1.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" \
  --ca "$ca" "${vars[@]}" &
server_pid=$!
for _ in $(seq 100); do
  if "$harness" exec server ss -Hltn "sport = :$port" | grep -q LISTEN; then
    break
  fi
  sleep 0.1
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --sessions "$sessions" --ca "$ca" --keylog "$keys" "${vars[@]}"
wait "$server_pid"
"$harness" capture stop

# Embed the key log in the capture, so every reader finds the secrets with it.
editcap --inject-secrets "tls,$keys" "$out/http1.pcapng" "$work/keyed.pcapng"
mv "$work/keyed.pcapng" "$out/http1.pcapng"

cd "$root"
# Every session must have its own 5-tuple. A source port that comes up twice for
# one destination merges two sessions in the session table and makes Zeek report
# a parser error (MOD-21). The peer assigns a source port per session, and this
# check names the cause when a capture still holds fewer sessions than it ran.
expected=$(( sessions * $(find "$here/../scenarios" -name '*.yaml' | wc -l) ))
python3 -I - "$out/http1.pcapng" "$expected" "$root" <<'PY'
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
score_rc=0
python3 -m realism score --pcap "$out/http1.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/http1.report.json" || score_rc=$?
# The four https scenarios run over TLS, so the capture holds four TLS sessions
# per session of the peers.
view_args=()
if [[ -n ${FPS_FIXTURE_DECRYPTED_DIR:-} ]]; then
  mkdir -p "$FPS_FIXTURE_DECRYPTED_DIR"
  view_args=(--view "$FPS_FIXTURE_DECRYPTED_DIR/http1.decrypted.pcap")
fi
check_rc=0
python3 -m realism decrypted --pcap "$out/http1.pcapng" \
  --labels "$root/realism/examples/decrypted/http1.labels.yaml" \
  --sessions $(( sessions * 4 )) --parsers tshark \
  --output "$out/http1.decrypted.json" "${view_args[@]}" || check_rc=$?
exit $(( score_rc != 0 ? score_rc : check_rc ))
