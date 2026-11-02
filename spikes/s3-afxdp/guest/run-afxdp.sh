#!/usr/bin/env bash
# Spike S3 guest side. Runs one AF_XDP measurement with DPDK testpmd and the
# AF_XDP poll mode driver on the test interface, then prints one JSON line.
#
#   sudo ./run-afxdp.sh rx|tx FRAME_BYTES default|copy SECONDS
#
# host/run-all.sh calls this in both guests and starts the receiver first.
set -euo pipefail

ROLE=${1:?usage: run-afxdp.sh rx|tx FRAME_BYTES default|copy SECONDS}
SIZE=${2:?frame size in bytes, including the 4 byte FCS}
MODE=${3:?default or copy}
DURATION=${4:?seconds}
IFACE=${IFACE:-test0}
QUEUES=${QUEUES:-1}
# Unicast to the receiver so the bridge does not flood frames.
PEER_MAC=${PEER_MAC:-52:54:00:77:00:02}
OUT=${OUT:-results}
TESTPMD=$(command -v dpdk-testpmd || true)

[[ -n $TESTPMD ]] || { echo "dpdk-testpmd not found (apt install dpdk-dev librte-net-af-xdp24)" >&2; exit 1; }
[[ $ROLE == tx || $ROLE == rx ]] || { echo "role must be tx or rx" >&2; exit 1; }
[[ $MODE == default || $MODE == copy ]] || { echo "mode must be default or copy" >&2; exit 1; }
mkdir -p "$OUT"

# The AF_XDP driver needs the queues it binds to, so set the channel count.
ethtool -L "$IFACE" combined "$QUEUES" >/dev/null 2>&1 || true
ethtool -l "$IFACE" > "$OUT/ethtool-l.txt" 2>&1 || true
ethtool -k "$IFACE" > "$OUT/ethtool-k.txt" 2>&1 || true

vdev="net_af_xdp0,iface=$IFACE,start_queue=0,queue_count=$QUEUES"
[[ $MODE == copy ]] && vdev="$vdev,force_copy=1"
if [[ $ROLE == tx ]]; then
  fwd=(--forward-mode=txonly "--txpkts=$((SIZE - 4))" "--eth-peer=0,$PEER_MAC")
  key=Tx-pps
else
  fwd=(--forward-mode=rxonly)
  key=Rx-pps
fi
log="$OUT/$ROLE-$SIZE-$MODE-q$QUEUES-$(uname -r).log"

# Lcore 0 runs the main thread; lcores 1..QUEUES forward packets.
timeout -s INT "$DURATION" "$TESTPMD" -l "0-$QUEUES" --no-pci --vdev "$vdev" \
  --log-level=pmd.net.af_xdp:debug -- \
  "${fwd[@]}" --nb-cores="$QUEUES" --rxq="$QUEUES" --txq="$QUEUES" --stats-period=1 --auto-start \
  > "$log" 2>&1 || true

# Median of the non zero per second rates, trimming two samples at each end
# so start up and shut down do not bias the result.
pps=$(grep -oE "$key: +[0-9]+" "$log" | awk '{print $2}' | awk '$1 > 0' \
  | awk '{a[NR]=$1} END {for (i = 3; i <= NR - 2; i++) print a[i]}' | sort -n \
  | awk '{a[NR]=$1} END {if (NR) print a[int((NR + 1) / 2)]; else print 0}')
zc=$(grep -ciE "zero.?copy|zerocopy" "$log" || true)
gbps=$(awk -v p="$pps" -v s="$SIZE" 'BEGIN {printf "%.3f", p * (s + 20) * 8 / 1e9}')
driver=$(ethtool -i "$IFACE" | awk '/^driver:/ {print $2}')
jq -nc --arg role "$ROLE" --arg kernel "$(uname -r)" --arg driver "$driver" --arg mode "$MODE" \
  --argjson size "$SIZE" --argjson pps "$pps" --argjson gbps "$gbps" --argjson queues "$QUEUES" \
  --argjson zero_copy_log_lines "$zc" --argjson vcpus "$(nproc)" \
  '{role:$role,kernel:$kernel,driver:$driver,mode:$mode,frame_bytes:$size,queues:$queues,
    vcpus:$vcpus,median_pps:$pps,line_gbps:$gbps,zero_copy_log_lines:$zero_copy_log_lines}'
