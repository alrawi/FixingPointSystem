#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the imap fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and scores the capture with the realism
# runner, which asks nDPI for the application of every session and checks the
# capture with tshark. The script needs root, ip, tcpdump, ss, ndpiReader, and
# tshark, and editcap.
#
#   sudo modules/imap/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/imap/fixtures, where the capture lands as
# imap.pcapng and the realism report as imap.report.json. The script exits
# with the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
# sessions per scenario, and FPS_FIXTURE_PEER names a prebuilt fixture_peer.
#
# The scenario starttls_mailbox (MOD-20) upgrades its sessions to TLS after
# STARTTLS. It runs on port 1143 in the fixture, so the labels can tell it from
# the plain sessions on port 143, and its client peer writes the secrets of
# every handshake to a key log. The script injects the key log into the capture
# as a decryption secrets block (editcap --inject-secrets), so the realism
# runner and tshark decrypt the capture without a sidecar file.
# FPS_FIXTURE_KEYLOG names a file that keeps the key log as well.
# `python3 -m realism decrypted` then reads the dialogue inside TLS with tshark,
# scores a decrypted view with nDPI, and runs Zeek on the view when Zeek is
# installed, see the README. The result lands as imap.decrypted.json.
# FPS_FIXTURE_DECRYPTED_DIR keeps the view as imap.decrypted.pcap.
# The command gets --starttls-port 1143:imap (RLM-21, MOD-30), so tshark decodes
# the capture port as IMAP and starts TLS after the STARTTLS reply, and does not
# depend on its TLS heuristic. The labels name port 143, the port of the view.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-8}
starttls_port=1143

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Small attachments keep the fixture reviewable. The scenarios keep their
# structure, and every message still draws its own attachment size.
vars=(--var body_size=900 --var attachment_min=2000 --var attachment_max=12000
      --port "starttls_mailbox=$starttls_port")

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-imap --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

work=$(mktemp -d)
ca="$work/ca.pem"
keys=${FPS_FIXTURE_KEYLOG:-$work/keys.log}
: > "$keys"

export FPS_NETNS=${FPS_NETNS:-fpsimapfix}
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

"$harness" capture start "$out/imap.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" \
  --ca "$ca" "${vars[@]}" &
server_pid=$!
# The server binds ports 143 and the STARTTLS port before its first session,
# so the listeners show that it is ready.
for port in 143 "$starttls_port"; do
  for _ in $(seq 100); do
    if "$harness" exec server ss -Hltn "sport = :$port" | grep -q LISTEN; then
      break
    fi
    sleep 0.1
  done
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --sessions "$sessions" --ca "$ca" --keylog "$keys" "${vars[@]}"
wait "$server_pid"
"$harness" capture stop

# Embed the key log in the capture, so every reader finds the secrets with it.
editcap --inject-secrets "tls,$keys" "$out/imap.pcapng" "$work/keyed.pcapng"
mv "$work/keyed.pcapng" "$out/imap.pcapng"

cd "$root"
python3 -m realism score --pcap "$out/imap.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/imap.report.json"

# The sessions inside TLS: tshark decrypts them with the embedded keys, nDPI
# scores the decrypted view, and Zeek reads the view when it is installed.
zeek=${FPS_ZEEK:-$(command -v zeek || true)}
if [[ -z $zeek && -x /opt/zeek/bin/zeek ]]; then
  zeek=/opt/zeek/bin/zeek
fi
parsers=tshark
zeek_args=()
if [[ -n $zeek ]]; then
  parsers=tshark,zeek
  zeek_args=(--zeek "$zeek")
fi
view_args=()
if [[ -n ${FPS_FIXTURE_DECRYPTED_DIR:-} ]]; then
  mkdir -p "$FPS_FIXTURE_DECRYPTED_DIR"
  view_args=(--view "$FPS_FIXTURE_DECRYPTED_DIR/imap.decrypted.pcap")
fi
python3 -m realism decrypted --pcap "$out/imap.pcapng" \
  --labels "$root/realism/examples/decrypted/imap.labels.yaml" \
  --sessions "$sessions" --starttls-port "$starttls_port:imap" --view-port 143 \
  --parsers "$parsers" \
  --output "$out/imap.decrypted.json" "${zeek_args[@]}" "${view_args[@]}"
