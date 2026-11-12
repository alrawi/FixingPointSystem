#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Builds the fixture peer of every native module, as the calling user.
#
#   tests/gates/gate2/build-peers.sh PEERS_DIR
#
# modules.sh runs as root and records each module with the peer that this script
# built, so root needs no Rust toolchain. The script lists the modules with
# `python3 -m realism modules`, the same list that the module realism job of the
# realism workflow uses, builds `cargo build -p CRATE --example fixture_peer` for
# each, and copies the program to PEERS_DIR/MODULE. Every module has an example
# of the same name, so the copy follows each build.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: build-peers.sh PEERS_DIR" >&2
  exit 2
fi
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
mkdir -p "$1"
peers=$(cd "$1" && pwd)
target=${CARGO_TARGET_DIR:-$root/target}

cd "$root"
python3 -I -c 'import json, subprocess, sys
raw = subprocess.run([sys.executable, "-m", "realism", "modules"], check=True, capture_output=True, text=True).stdout
for entry in json.loads(raw)["include"]:
    print(entry["module"], entry["crate"])' >"$peers/modules.txt"

while read -r module crate; do
  if [[ -n ${FPS_GATE2_MODULES:-} && ,${FPS_GATE2_MODULES}, != *,"$module",* ]]; then
    continue
  fi
  echo "build-peers.sh: building the fixture peer of $module ($crate)" >&2
  cargo build --quiet -p "$crate" --example fixture_peer
  cp "$target/debug/examples/fixture_peer" "$peers/$module"
done <"$peers/modules.txt"
echo "build-peers.sh: built $(wc -l <"$peers/modules.txt") fixture peers in $peers" >&2
