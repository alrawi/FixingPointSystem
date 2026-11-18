#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Records the http1 module on the AF_XDP backend and scores the capture (TRN-12).
# It is module-afxdp.sh for http1, which TRN-36 generalized to every first party
# module, and it keeps the name and the arguments of the TRN-12 script.
#
#   sudo engine/fps-datapath/realism/http1-afxdp.sh AGENT_BINARY OUTPUT_DIR
set -euo pipefail
[[ $# -eq 2 ]] || {
  echo "usage: http1-afxdp.sh AGENT_BINARY OUTPUT_DIR" >&2
  exit 2
}
exec "$(dirname "${BASH_SOURCE[0]}")/module-afxdp.sh" http1 "$1" "$2"
