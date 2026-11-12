#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the http2 fixture capture in the QA-01 namespace harness.
#
# The script builds the fixture_peer example, brings up a harness under its
# own name prefix, runs the server side of every scenario in the server
# namespace and the client side in the client namespace over kernel sockets,
# captures the router's client arm, and scores the capture with the realism
# runner. It needs root, ip, tcpdump, ss, ndpiReader, tshark, and editcap.
#
#   sudo modules/http2/fixtures/capture.sh [OUTPUT_DIR]
#
# OUTPUT_DIR defaults to modules/http2/fixtures, where the capture lands as
# http2.pcapng and the realism report as http2.report.json. The script exits
# with the code of the realism runner, so a capture that fails a check fails.
#
# The two https scenarios run HTTP/2 over TLS 1.3 with ALPN h2 and the other
# two run it in the clear with prior knowledge. The client peer writes the
# secrets of every handshake to a key log, and the script injects the key log
# into the capture as a decryption secrets block, so the realism runner and
# tshark decrypt the capture without a sidecar file. FPS_FIXTURE_KEYLOG names a
# file that keeps the key log as well, for a reader that wants the sidecar. The
# script then runs `python3 -m realism decrypted`, which decrypts the capture
# with tshark and scores the HTTP/2 inside TLS on a decrypted view with nDPI,
# with the labels realism/examples/decrypted/http2.labels.yaml. The result lands
# as http2.decrypted.json. FPS_FIXTURE_DECRYPTED_DIR keeps the decrypted view, a
# capture of the plaintext streams, as http2.decrypted.pcap.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
out=${1:-$here}
sessions=${FPS_FIXTURE_SESSIONS:-20}
port=8082

if [[ $(id -u) -ne 0 ]]; then
  echo "capture.sh: needs root" >&2
  exit 2
fi

# Small bodies keep the fixture reviewable. Every scenario keeps its structure.
# The https scenarios get smaller bodies still, because the TLS session of the
# TLS module fails when a receiver holds more than 16 KiB of plaintext that it
# has not read, see the README.
vars=(--var page_size=12000 --var chunk_size=2048 --var rounds=2
      --var tail_min=200 --var tail_max=2000
      --var window=16384 --var frame_size=4096 --var round_frames=4
      --var window_waits=1 --var tail_size=8000
      --var-tls page_size=2000 --var-tls chunk_size=512 --var-tls rounds=1
      --var-tls tail_min=100 --var-tls tail_max=400
      --var-tls window=2048 --var-tls frame_size=512 --var-tls round_frames=4
      --var-tls window_waits=1 --var-tls tail_size=1000)

# FPS_FIXTURE_PEER names a prebuilt fixture_peer, for runners where root has no
# Rust toolchain.
peer=${FPS_FIXTURE_PEER:-}
if [[ -z $peer ]]; then
  cargo build --quiet --manifest-path "$root/Cargo.toml" -p fps-module-http2 --example fixture_peer
  peer="${CARGO_TARGET_DIR:-$root/target}/debug/examples/fixture_peer"
fi

work=$(mktemp -d)
ca="$work/ca.pem"
keys=${FPS_FIXTURE_KEYLOG:-$work/keys.log}
: > "$keys"

export FPS_NETNS=${FPS_NETNS:-fpsh2fix}
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

"$harness" capture start "$out/http2.pcapng" client
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
editcap --inject-secrets "tls,$keys" "$out/http2.pcapng" "$work/keyed.pcapng"
mv "$work/keyed.pcapng" "$out/http2.pcapng"

cd "$root"
score_rc=0
python3 -m realism score --pcap "$out/http2.pcapng" --labels "$here/expected.yaml" \
  --parsers tshark --output "$out/http2.report.json" || score_rc=$?
# The two https scenarios run over TLS, so the capture holds two TLS sessions
# per session of the peers.
view_args=()
if [[ -n ${FPS_FIXTURE_DECRYPTED_DIR:-} ]]; then
  mkdir -p "$FPS_FIXTURE_DECRYPTED_DIR"
  view_args=(--view "$FPS_FIXTURE_DECRYPTED_DIR/http2.decrypted.pcap")
fi
# The decrypted check lists the TLS sessions as the streams with a ClientHello.
# tshark finds one on the ports 8443 and 8444 by its heuristic only, which a
# ClientHello whose first segment is shorter than a record header escapes (MOD-23),
# so the check reads the capture through a tshark that decodes those ports as TLS.
# It leaves the decrypted view, which holds HTTP/2 in the clear on them, alone.
tls_ports=$(sed -n 's/^  server_port: *\(84[0-9][0-9]\)$/\1/p' "$here"/../scenarios/https_*.yaml | sort -u | tr '\n' ' ')
tls_ports=${tls_ports% }
if [[ -z $tls_ports ]]; then
  echo "capture.sh: the https scenarios name no server port" >&2
  exit 2
fi
real_tshark=${FPS_TSHARK:-tshark}
check_rc=0
FPS_TSHARK_REAL=$real_tshark FPS_TLS_PORTS=$tls_ports FPS_TLS_DECODE_PCAP="$out/http2.pcapng" \
  python3 -m realism decrypted --pcap "$out/http2.pcapng" \
  --labels "$root/realism/examples/decrypted/http2.labels.yaml" \
  --sessions $(( sessions * 2 )) --parsers tshark --tshark "$here/tshark-tls-ports.sh" \
  --output "$out/http2.decrypted.json" "${view_args[@]}" || check_rc=$?
if [[ $check_rc -ne 0 ]]; then
  # Say how many connections the capture holds on the TLS ports, so a count
  # that is off names its cause: a SYN count that differs from the 5-tuple
  # count is a reused source port, and fewer ClientHellos than SYNs is a
  # connection that never started TLS.
  syn_filter='tcp.flags.syn == 1 && tcp.flags.ack == 0'
  # shellcheck disable=SC2086  # one printf round per port
  port_filter=$(printf 'tcp.dstport == %s || ' $tls_ports)
  port_filter="(${port_filter% || })"
  syns=$("$real_tshark" -r "$out/http2.pcapng" -Y "$syn_filter && $port_filter" -T fields \
    -e tcp.srcport -e tcp.dstport 2>/dev/null || true)
  hello_args=()
  for port in $tls_ports; do hello_args+=(-d "tcp.port==$port,tls"); done
  hellos=$("$real_tshark" -r "$out/http2.pcapng" "${hello_args[@]}" -Y 'tls.handshake.type == 1' \
    -T fields -e tcp.stream 2>/dev/null || true)
  echo "capture.sh: the TLS ports $tls_ports hold $(grep -c . <<<"$syns") SYNs from" \
    "$(sort -u <<<"$syns" | grep -c .) distinct 5-tuples and" \
    "$(sort -u <<<"$hellos" | grep -c .) streams with a ClientHello, $(( sessions * 2 )) expected" >&2
fi
exit $(( score_rc != 0 ? score_rc : check_rc ))
