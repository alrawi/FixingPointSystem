#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the ftp fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and checks the capture three ways. The
# realism runner scores the control connections with nDPI and tshark.
# structure_check.py reads the capture with the CAP-01 session extractor and
# requires one control connection per FTP session and exactly one data
# connection per transfer, to the announced port. When Zeek is installed,
# zeek_check.py also requires that ftp.log holds every RETR and STOR, that
# files.log holds their files without lost bytes, and that weird.log holds no
# ftp entry. The script needs root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/ftp/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/ftp/fixtures, where the capture lands as
# ftp.pcapng and the realism report as ftp.report.json. The script exits with
# the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
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

# Small files keep the fixture reviewable. The scenarios keep their structure,
# and every transfer still draws its own size. The server address becomes the
# variable server_ip on its own, so the 227 replies name the address that the
# data connections reach.
vars=(--var file_min=2000 --var file_max=12000 --var list_entries=16)

# The control connections and transfers per round of the five scenarios, with
# the default of four files for batch_download: one transfer in each of
# list_pasv, retr_epsv, and stor_pasv, three in mixed_session, and four in
# batch_download. File transfers are all but the two listings.
controls=$((sessions * 5))
transfers=$((sessions * 10))
file_transfers=$((sessions * 8))

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-ftp --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpsftpfix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() { "$harness" down >/dev/null 2>&1 || true; }
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/ftp.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" "${vars[@]}" &
server_pid=$!
# The server binds the control port before its first session, so the listener
# shows that it is ready. A probe connection would add a session to the capture.
for _ in $(seq 100); do
  if "$harness" exec server ss -Hltn "sport = :21" | grep -q LISTEN; then
    break
  fi
  sleep 0.1
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --sessions "$sessions" "${vars[@]}"
wait "$server_pid"
"$harness" capture stop

cd "$root"
python3 -m realism score --pcap "$out/ftp.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/ftp.report.json"

python3 "$here/structure_check.py" "$out/ftp.pcapng" \
  --controls "$controls" --transfers "$transfers"

# Zeek is optional here, because the realism job installs it and scores the
# same capture for parser errors. Where it is present, the check below also
# reads ftp.log and files.log.
zeek=${FPS_ZEEK:-$(command -v zeek || true)}
if [[ -z $zeek && -x /opt/zeek/bin/zeek ]]; then
  zeek=/opt/zeek/bin/zeek
fi
if [[ -n $zeek ]]; then
  python3 "$here/zeek_check.py" "$out/ftp.pcapng" --zeek "$zeek" \
    --file-transfers "$file_transfers"
else
  echo "capture.sh: skipped the Zeek check because zeek is not installed" >&2
fi
