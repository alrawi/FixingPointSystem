#!/usr/bin/env bash
# Stop the spike S3 guests and remove their tap devices.
set -euo pipefail
WORK=${WORK:-/var/tmp/fps-s3}
for role in tx rx; do
  if [[ -f $WORK/$role.pid ]]; then
    kill "$(cat "$WORK/$role.pid")" 2>/dev/null || true
    rm -f "$WORK/$role.pid"
  fi
  ip link del "fps-$role" 2>/dev/null || true
done
