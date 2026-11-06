#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Repeats spike S2 and the handshake benchmark of the TLS module in alternation
# on this machine and prints the comparison. Alternating rounds spread drift of
# a shared or noisy host over both sides. Run it from the repository root on a
# host with no other load, for example the owner host.
#
# Usage: modules/tls/bench/run_s2.sh [ROUNDS] [SECONDS_PER_PROFILE] [OUTPUT_DIR]
set -euo pipefail

rounds="${1:-3}"
seconds="${2:-3}"
out="${3:-$(mktemp -d)}"
target="${CARGO_TARGET_DIR:-target}"

cargo build --release -p fps-spike-tls
cargo build --release -p fps-module-tls --example handshake_bench

mkdir -p "$out"
for i in $(seq 1 "$rounds"); do
  "$target/release/fps-spike-tls" --duration "$seconds" --json "$out/spike-$i.json" >/dev/null
  "$target/release/examples/handshake_bench" --duration "$seconds" --json "$out/module-$i.json" >/dev/null
  echo "round $i of $rounds done"
done

python3 modules/tls/bench/compare_s2.py \
  --spike "$out"/spike-*.json \
  --module "$out"/module-*.json \
  --phase0 spikes/s2-tls/results/phase0-container/handshakes.json
echo "raw results are in $out"
