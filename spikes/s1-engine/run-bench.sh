#!/usr/bin/env bash
# Spike S1 benchmark suite. Writes one JSON file per run under results/<label>/.
set -euo pipefail
cd "$(dirname "$0")/../.."
LABEL=${LABEL:-$(hostname)-$(date -u +%Y%m%d)}
DUR=${DUR:-15}
OUT=spikes/s1-engine/results/$LABEL
BIN=target/release/fps-spike-engine
SCN=contracts/examples/modules/http1/scenarios
mkdir -p "$OUT"
ulimit -n 65536 2>/dev/null || ulimit -n "$(ulimit -Hn)"
cargo build --release -q -p fps-spike-engine

{
  echo "cpu: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ //')"
  echo "vcpus: $(nproc)"
  echo "kernel: $(uname -r)"
  echo "rustc: $(rustc --version)"
  echo "date: $(date -u +%FT%TZ)"
} > "$OUT/host.txt"

run() {
  local name=$1; shift
  echo "== $name"
  "$BIN" bench --duration "$DUR" --json "$OUT/$name.json" "$@" | sed 's/^/   /'
}

for rep in 1 2; do
  run "keepalive-interp-r$rep"   --scenario $SCN/web_get.yaml --concurrency 64
  run "keepalive-baseline-r$rep" --scenario $SCN/web_get.yaml --concurrency 64 --mode baseline
done
run cps-interp      --scenario $SCN/web_get.yaml --concurrency 64 --var requests=1 --var body_size=0
run cps-baseline    --scenario $SCN/web_get.yaml --concurrency 64 --var requests=1 --var body_size=0 --mode baseline
run cps-interp-1core --scenario $SCN/web_get.yaml --concurrency 64 --var requests=1 --var body_size=0 --threads 1
run throughput-interp   --scenario $SCN/web_get.yaml --concurrency 16 --var body_size=1048576
run throughput-baseline --scenario $SCN/web_get.yaml --concurrency 16 --var body_size=1048576 --mode baseline
run concurrency-browse  --scenario $SCN/web_browse.yaml --concurrency 4000
echo "results in $OUT"
