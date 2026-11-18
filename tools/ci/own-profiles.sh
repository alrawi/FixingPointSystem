#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Scores the OS profile captures of the own stack (TRN-28) with the realism
# runner, as engine/fps-backend-kernel/tests/netns/profiles.sh does for the
# kernel backend. Needs no root. Each profile must pass its own
# transport_behavior check and fail as the next profile (negative control).
# The runner exits 1 because nDPI does not know the protocol; 2 is an error.
set -euo pipefail
OUT="${1:?usage: own-profiles.sh OUTDIR}"
mkdir -p "$OUT"
cargo run -q -p fps-tcp-own --example os_profile_capture -- "$OUT"
printf '{"labels": "fps-realism-labels", "version": 1, "default": "probe"}\n' >"$OUT/labels.json"

verdict() {
  python3 -I - "$1" <<'PY'
import json, sys
report = json.load(open(sys.argv[1], encoding="utf-8"))
v = [c["passed"] for c in report["checks"] if c["check"] == "transport_behavior"]
print("none" if not v else "pass" if all(v) else "fail")
PY
}

score() {
  local capture="$1" as="$2" report="$3" status=0
  python3 -m realism score --pcap "$OUT/$capture.pcapng" --labels "$OUT/labels.json" \
    --parsers none --os-profile "$as" --client-hops 0 --server-hops 0 --transport-strict \
    --output "$report" >/dev/null || status=$?
  if ((status == 2)); then echo "realism runner failed on $capture as $as" >&2; exit 2; fi
  verdict "$report"
}

rc=0
profiles=(windows11 linux6 macos14)
for i in 0 1 2; do
  p=${profiles[$i]}
  q=${profiles[$(((i + 1) % 3))]}
  got=$(score "$p" "$p" "$OUT/$p.json")
  echo "$p as $p: $got (want pass)"
  [[ "$got" == pass ]] || rc=1
  got=$(score "$p" "$q" "$OUT/$p-as-$q.json")
  echo "$p as $q: $got (want fail)"
  [[ "$got" == fail ]] || rc=1
done
exit "$rc"
