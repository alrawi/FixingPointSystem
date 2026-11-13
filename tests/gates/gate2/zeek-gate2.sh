#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Zeek of the Gate 2 mix stage (QA-13). mix.sh sets FPS_ZEEK to this script, so the
# realism runner starts it in place of Zeek, and the script starts the real Zeek with one
# more setting when it reads a capture:
#
#   max_event_queue_size=0
#
# Zeek counts the events that wait for the script layer, and when more than
# max_event_queue_size are queued it drains the queue at once and raises the weird
# event_queue_overflow_drain (src/Event.cc). A weird has no connection, so the weird log
# carries no endpoints and the parser error check counts it as unattributed. A mix capture
# holds thousands of connections that end together when the run stops, so the queue can
# overflow while Zeek reads the last packets, and a clean run then fails by chance (CTL-49
# saw it at +141.023 s of a capture of 141.023 s). The limit is a resource guard of a
# live sensor. A value of 0 switches it off, so Zeek reads the whole trace and the log holds
# only what the traffic raised. The setting changes no weird of a connection.
#
# The real Zeek is FPS_GATE2_ZEEK, which mix.sh sets from FPS_ZEEK, from the search path,
# or from /opt/zeek/bin/zeek. Any other call, such as --version, passes unchanged.
set -euo pipefail

real=${FPS_GATE2_ZEEK:-}
if [[ -z $real ]]; then
  real=$(command -v zeek || true)
  [[ -n $real ]] || { [[ -x /opt/zeek/bin/zeek ]] && real=/opt/zeek/bin/zeek; } || true
fi
if [[ -z $real ]]; then
  echo "zeek-gate2.sh: zeek not found, set FPS_GATE2_ZEEK" >&2
  exit 127
fi

reads=false
for arg in "$@"; do
  [[ $arg == -r ]] && reads=true
done
if [[ $reads == true ]]; then
  exec "$real" "$@" max_event_queue_size=0
fi
exec "$real" "$@"
