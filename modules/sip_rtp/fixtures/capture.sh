#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the sip_rtp fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel UDP sockets,
# captures the router's client arm, and checks the capture. It first requires
# one session in the capture for every session that the peers ran (a call is
# one SIP session and one RTP session, a refused call only the SIP session),
# which fails when two sessions share a 5-tuple (MOD-21). The realism runner
# then scores the sessions with nDPI and tshark, which must classify the SIP
# sessions as SIP and the media sessions as RTP. rtp_check.py requires every
# RTP packet gap to match the configured packet interval. When Zeek is
# installed, zeek_check.py also requires one sip.log row per call.
# The script needs root, ip, tcpdump, ss, ndpiReader, and tshark.
#
#   sudo modules/sip_rtp/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/sip_rtp/fixtures, where the capture lands as
# sip_rtp.pcapng and the realism report as sip_rtp.report.json. The script exits
# with the code of the first check that fails. FPS_FIXTURE_SESSIONS sets the
# calls per scenario, and FPS_FIXTURE_PEER names a prebuilt fixture_peer.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-3}
port=5060

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Short calls keep the fixture reviewable. The scenarios keep their structure,
# their codecs, and their packet intervals.
vars=(--var talk_ms=2000 --var ring_ms=300)

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-sip-rtp --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

export FPS_NETNS=${FPS_NETNS:-fpssipfix}
export FPS_STATE_DIR=${FPS_STATE_DIR:-/run/fps-netns-$FPS_NETNS}
harness="$root/tests/harness/netns.sh"
mkdir -p "$out"

cleanup() { "$harness" down >/dev/null 2>&1 || true; }
trap cleanup EXIT
"$harness" up
server_addr=$("$harness" info | sed -n 's/^SERVER_ADDR=//p')
client_addr=$("$harness" info | sed -n 's/^CLIENT_ADDR=//p')

"$harness" capture start "$out/sip_rtp.pcapng" client
"$harness" exec server "$peer" server --listen "$server_addr" --sessions "$sessions" "${vars[@]}" &
server_pid=$!
# The server binds the SIP port before its first call, so the socket shows that
# it is ready. A probe datagram would add a session to the capture.
for _ in $(seq 100); do
  if "$harness" exec server ss -Hlun "sport = :$port" | grep -q .; then
    break
  fi
  sleep 0.1
done
"$harness" exec client "$peer" client --connect "$server_addr" --source "$client_addr" \
  --sessions "$sessions" "${vars[@]}"
wait "$server_pid"
"$harness" capture stop

cd "$root"
# Every session must have its own 5-tuple. The peer assigns a source port per
# flow, and this check names the cause when a capture still holds fewer
# sessions than it ran. Every scenario but the refused call has a media flow.
scenario_count=$(find "$here/../scenarios" -name '*.yaml' | wc -l)
expected=$(( sessions * (2 * (scenario_count - 1) + 1) ))
calls=$(( sessions * scenario_count ))
python3 -I - "$out/sip_rtp.pcapng" "$expected" "$root" <<'PY'
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
python3 -m realism score --pcap "$out/sip_rtp.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/sip_rtp.report.json"

python3 -I "$here/rtp_check.py" "$out/sip_rtp.pcapng"

# Zeek is optional here, because the realism job installs it and scores the
# same capture for parser errors. Where it is present, the check below also
# reads sip.log.
zeek=${FPS_ZEEK:-$(command -v zeek || true)}
if [[ -z $zeek && -x /opt/zeek/bin/zeek ]]; then
  zeek=/opt/zeek/bin/zeek
fi
if [[ -n $zeek ]]; then
  python3 "$here/zeek_check.py" "$out/sip_rtp.pcapng" --zeek "$zeek" --calls "$calls"
else
  echo "capture.sh: skipped the Zeek check because zeek is not installed" >&2
fi
