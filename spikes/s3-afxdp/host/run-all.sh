#!/usr/bin/env bash
# Spike S3 orchestration, run on the host as root after host/launch-vms.sh.
# Waits for both guests, runs every frame size and mode with the receiver
# started first, and appends one JSON line per test to results/s3.jsonl.
#
#   sudo ./host/run-all.sh            # GA kernel in the guests
#   sudo KERNEL=hwe ./host/run-all.sh # installs the HWE kernel, reboots, reruns
set -euo pipefail

WORK=${WORK:-/var/tmp/fps-s3}
KERNEL=${KERNEL:-ga}
QUEUE_SET=${QUEUE_SET:-"1"}
SIZES=${SIZES:-"64 1500"}
MODES=${MODES:-"default copy"}
DURATION=${DURATION:-30}
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${OUT:-$HERE/../results}
KEY=${KEY:-$WORK/id_ed25519}
TX_PORT=2221
RX_PORT=2222
SSH=(ssh -i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5)

on() { local port=$1; shift; "${SSH[@]}" -p "$port" ubuntu@127.0.0.1 "$@"; }

wait_guest() {
  local port=$1
  for _ in $(seq 1 120); do
    on "$port" true 2>/dev/null && break
    sleep 5
  done
  on "$port" cloud-init status --wait >/dev/null
}

prepare() {
  local port=$1
  wait_guest "$port"
  scp -q -i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    -P "$port" "$HERE/../guest/run-afxdp.sh" ubuntu@127.0.0.1:run-afxdp.sh
  if [[ $KERNEL == hwe ]] && ! on "$port" "dpkg -s linux-generic-hwe-24.04" >/dev/null 2>&1; then
    on "$port" "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y linux-generic-hwe-24.04 >/dev/null"
    on "$port" "sudo systemctl reboot" || true
    sleep 20
    wait_guest "$port"
  fi
}

mkdir -p "$OUT"
prepare "$TX_PORT"
prepare "$RX_PORT"
echo "guest kernels: tx $(on $TX_PORT uname -r), rx $(on $RX_PORT uname -r)"

for queues in $QUEUE_SET; do
  for size in $SIZES; do
    for mode in $MODES; do
      echo "== frames ${size} B, mode ${mode}, queues ${queues}"
      on "$RX_PORT" "sudo QUEUES=$queues ./run-afxdp.sh rx $size $mode $((DURATION + 10))" \
        > "$OUT/.rx.json" &
      rx_pid=$!
      sleep 3
      on "$TX_PORT" "sudo QUEUES=$queues ./run-afxdp.sh tx $size $mode $DURATION" | tee -a "$OUT/s3.jsonl"
      wait "$rx_pid"
      tee -a "$OUT/s3.jsonl" < "$OUT/.rx.json"
      rm -f "$OUT/.rx.json"
    done
  done
done

for port in $TX_PORT $RX_PORT; do
  on "$port" "tar -czf - results" > "$OUT/logs-$port-$(on "$port" uname -r).tgz"
done
{
  echo "host_cpu: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
  echo "host_threads: $(nproc)"
  echo "host_kernel: $(uname -r)"
  echo "qemu: $(qemu-system-x86_64 --version | head -1)"
} > "$OUT/host.txt"
echo "results in $OUT/s3.jsonl"
