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
# that smtp.log holds every message, that files.log holds every attachment,
# and that weird.log holds no smtp entry. The script needs root, ip, tcpdump,
# ss, ndpiReader, and tshark.
#
#   sudo modules/smtp/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/smtp/fixtures, where the capture lands as
# smtp.pcapng and the realism report as smtp.report.json. The script exits
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

# Messages per session of each scenario, which zeek_check.py needs: five
# scenarios send one message and submission_batch sends its default of three.
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

export FPS_NETNS=${FPS_NETNS:-fpssmtpfix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() { "$harness" down >/dev/null 2>&1 || true; }
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/smtp.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" "${vars[@]}" &
server_pid=$!
# The server binds the ports 25 and 587 before its first session, so the
# listeners show that it is ready.
for port in 25 587; do
  for _ in $(seq 100); do
    if "$harness" exec server ss -Hltn "sport = :$port" | grep -q LISTEN; then
      break
    fi
    sleep 0.1
  done
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --sessions "$sessions" "${vars[@]}"
wait "$server_pid"
"$harness" capture stop

cd "$root"
python3 -m realism score --pcap "$out/smtp.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/smtp.report.json"

# Zeek is optional here, because the realism job installs it and scores the
# same capture for parser errors. Where it is present, the check below also
# reads smtp.log and files.log.
zeek=${FPS_ZEEK:-$(command -v zeek || true)}
if [[ -z $zeek && -x /opt/zeek/bin/zeek ]]; then
  zeek=/opt/zeek/bin/zeek
fi
if [[ -n $zeek ]]; then
  python3 "$here/zeek_check.py" "$out/smtp.pcapng" --zeek "$zeek" \
    --messages "$messages" --attachments "$attachments"
else
  echo "capture.sh: skipped the Zeek check because zeek is not installed" >&2
fi
