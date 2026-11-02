#!/usr/bin/env bash
# Spike S3 guest side. Measures AF_XDP packet rates on the test interface with
# DPDK testpmd and the AF_XDP poll mode driver, in zero copy (default) and in
# forced copy mode, for 64 and 1,500 byte frames.
#
#   sudo ./run-afxdp.sh rx     # on the receiver, start first
#   sudo ./run-afxdp.sh tx     # on the sender
#
# Results go to ./results/<host>-<kernel>.jsonl, one JSON object per test.
set -euo pipefail

ROLE=${1:?usage: run-afxdp.sh tx|rx}
IFACE=${IFACE:-test0}
QUEUES=${QUEUES:-1}
SECONDS_PER_TEST=${SECONDS_PER_TEST:-30}
SIZES=${SIZES:-"64 1500"}
MODES=${MODES:-"default copy"}
OUT=${OUT:-results}
# Unicast to the receiver so the bridge does not flood frames.
PEER_MAC=${PEER_MAC:-52:54:00:77:00:02}
TESTPMD=$(command -v dpdk-testpmd || true)

[[ -n $TESTPMD ]] || { echo "dpdk-testpmd not found (apt install dpdk)" >&2; exit 1; }
[[ $ROLE == tx || $ROLE == rx ]] || { echo "role must be tx or rx" >&2; exit 1; }
mkdir -p "$OUT"
kernel=$(uname -r)
driver=$(ethtool -i "$IFACE" | awk '/^driver:/ {print $2}')
results="$OUT/$(hostname)-$kernel.jsonl"

# The AF_XDP PMD needs the queues it uses to exist on the interface.
ethtool -L "$IFACE" combined "$QUEUES" 2>/dev/null || true
ethtool -l "$IFACE" > "$OUT/ethtool-l.txt" 2>&1 || true
ethtool -k "$IFACE" > "$OUT/ethtool-k.txt" 2>&1 || true

for size in $SIZES; do
  for mode in $MODES; do
    vdev="net_af_xdp0,iface=$IFACE,start_queue=0,queue_count=$QUEUES"
    [[ $mode == copy ]] && vdev="$vdev,force_copy=1"
    if [[ $ROLE == tx ]]; then
      fwd=(--forward-mode=txonly "--txpkts=$((size - 4))" "--eth-peer=0,$PEER_MAC")
    else
      fwd=(--forward-mode=rxonly)
    fi
    log="$OUT/$ROLE-$size-$mode.log"
    echo "== $ROLE size=$size mode=$mode ($SECONDS_PER_TEST s)"
    # testpmd prints port statistics every second; SIGINT ends the run cleanly.
    timeout -s INT "$SECONDS_PER_TEST" "$TESTPMD" -l 0-1 --no-pci --vdev "$vdev" \
      --log-level=pmd.net.af_xdp:debug -- \
      "${fwd[@]}" --nb-cores=1 --rxq="$QUEUES" --txq="$QUEUES" --stats-period=1 --auto-start \
      > "$log" 2>&1 || true
    # Take the median of the per second rates, skipping the first five seconds.
    key=$([[ $ROLE == tx ]] && echo "Tx-pps" || echo "Rx-pps")
    pps=$(grep -oE "$key: +[0-9]+" "$log" | awk '{print $2}' | tail -n +6 | sort -n \
      | awk '{a[NR]=$1} END {if (NR) print a[int((NR+1)/2)]; else print 0}')
    zc=$(grep -ciE "zero.?copy" "$log" || true)
    gbps=$(awk -v p="$pps" -v s="$size" 'BEGIN {printf "%.3f", p * (s + 20) * 8 / 1e9}')
    jq -nc --arg role "$ROLE" --arg kernel "$kernel" --arg driver "$driver" --arg mode "$mode" \
      --argjson size "$size" --argjson pps "$pps" --argjson gbps "$gbps" --argjson queues "$QUEUES" \
      --argjson zero_copy_log_lines "$zc" --argjson vcpus "$(nproc)" \
      '{role:$role,kernel:$kernel,driver:$driver,mode:$mode,frame_bytes:$size,queues:$queues,
        vcpus:$vcpus,median_pps:$pps,line_gbps:$gbps,zero_copy_log_lines:$zero_copy_log_lines}' \
      | tee -a "$results"
    [[ $ROLE == rx ]] && sleep 2
  done
done
echo "results appended to $results"
