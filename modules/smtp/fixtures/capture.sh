#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the smtp fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and scores the capture with the realism
# runner. When Zeek is installed it also runs zeek_check.py, which requires
# that smtp.log holds every message of the plain sessions, that files.log holds
# every attachment, and that weird.log holds no smtp entry. The script needs
# root, ip, tcpdump, ss, ndpiReader, tshark, and editcap.
#
#   sudo modules/smtp/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/smtp/fixtures, where the capture lands as
# smtp.pcapng and the realism report as smtp.report.json. The script exits
# with the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
# sessions per scenario, and FPS_FIXTURE_PEER names a prebuilt fixture_peer.
#
# The scenario starttls_submission (MOD-20) upgrades its sessions to TLS after
# STARTTLS. It runs on port 5587 in the fixture, so the labels can tell it from
# the plain submission sessions on port 587, and its client peer writes the
# secrets of every handshake to a key log. The script injects the key log into
# the capture as a decryption secrets block (editcap --inject-secrets), so the
# realism runner and tshark decrypt the capture without a sidecar file.
# FPS_FIXTURE_KEYLOG names a file that keeps the key log as well, for a reader
# that wants the sidecar. The realism runner checks the encryption of those
# sessions with the keys and the JA4 of their profile, and decrypted_check.py
# then reads the dialogue inside TLS with tshark and, where Zeek is installed,
# with Zeek, see the README. FPS_FIXTURE_DECRYPTED_DIR keeps the decrypted
# view that the check builds.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-8}
starttls_port=5587

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Small attachments keep the fixture reviewable. The scenarios keep their
# structure, and every message still draws its own attachment size.
vars=(--var body_size=900 --var attachment_min=2000 --var attachment_max=12000
      --port "starttls_submission=$starttls_port")

# Messages per session of each plain scenario, which zeek_check.py needs: five
# scenarios send one message and submission_batch sends its default of three.
# The STARTTLS sessions come on top, and Zeek cannot read them on the capture.
messages=$((sessions * 8))
# Messages with an attachment: all but text_only, so four single ones and three
# from submission_batch.
attachments=$((sessions * 7))

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-smtp --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

work=$(mktemp -d)
ca="$work/ca.pem"
keys=${FPS_FIXTURE_KEYLOG:-$work/keys.log}
: > "$keys"

export FPS_NETNS=${FPS_NETNS:-fpssmtpfix}
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

"$harness" capture start "$out/smtp.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" \
  --ca "$ca" "${vars[@]}" &
server_pid=$!
# The server binds the ports 25, 587, and the STARTTLS port before its first
# session, so the listeners show that it is ready.
for port in 25 587 "$starttls_port"; do
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
editcap --inject-secrets "tls,$keys" "$out/smtp.pcapng" "$work/keyed.pcapng"
mv "$work/keyed.pcapng" "$out/smtp.pcapng"

cd "$root"
python3 -m realism score --pcap "$out/smtp.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/smtp.report.json"

# Zeek is optional here, because the realism job installs it and scores the
# same capture for parser errors. Where it is present, the checks below also
# read smtp.log and files.log.
zeek=${FPS_ZEEK:-$(command -v zeek || true)}
if [[ -z $zeek && -x /opt/zeek/bin/zeek ]]; then
  zeek=/opt/zeek/bin/zeek
fi
zeek_args=()
if [[ -n $zeek ]]; then
  zeek_args=(--zeek "$zeek")
  python3 "$here/zeek_check.py" "$out/smtp.pcapng" --zeek "$zeek" \
    --messages "$messages" --attachments "$attachments"
else
  echo "capture.sh: skipped the Zeek check of the plain sessions because zeek is not installed" >&2
fi

# The sessions inside TLS: tshark decrypts them with the embedded keys, nDPI
# scores the decrypted view, and Zeek reads the view when it is installed.
python3 "$here/decrypted_check.py" "$out/smtp.pcapng" --sessions "$sessions" \
  --work "${FPS_FIXTURE_DECRYPTED_DIR:-$work}" "${zeek_args[@]}"
