#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# tshark that reads the https ports of the http2 fixture as TLS.
#
# capture.sh passes this script to `python3 -m realism decrypted --tshark`. The
# decrypted check lists the TLS sessions of the capture as the streams with a
# ClientHello, and tshark finds a ClientHello on the ports 8443 and 8444 only by
# its heuristic, which takes a stream for TLS when the first TCP segment of the
# client holds at least a record header. A ClientHello whose first segment is
# shorter, for example a write that the kernel or a middlebox splits, is no TLS
# to the heuristic, so the check counted 39 of 40 sessions (MOD-23). The Decode
# As option `-d tcp.port==P,tls`, which the encryption check of `realism score`
# passes as well, makes the port TLS whatever its first segment holds.
#
# The script adds the option for the ports in FPS_TLS_PORTS (blank separated)
# when tshark reads the file FPS_TLS_DECODE_PCAP, and runs tshark unchanged for
# every other file, because the decrypted view keeps the ports of the https
# sessions and holds HTTP/2 in the clear on them. FPS_TSHARK_REAL names the
# tshark to run and defaults to `tshark`.
set -euo pipefail

real=${FPS_TSHARK_REAL:-tshark}
extra=()
if [[ -n ${FPS_TLS_DECODE_PCAP:-} ]]; then
  prev=
  for arg in "$@"; do
    if [[ $prev == -r && $arg == "$FPS_TLS_DECODE_PCAP" ]]; then
      for port in ${FPS_TLS_PORTS:-}; do
        extra+=(-d "tcp.port==$port,tls")
      done
    fi
    prev=$arg
  done
fi
exec "$real" ${extra[@]+"${extra[@]}"} "$@"
