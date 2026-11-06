# SPDX-License-Identifier: GPL-3.0-only
"""Compare handshake rates of the TLS module with spike S2 on the same machine.

Usage: compare_s2.py --spike SPIKE.json [SPIKE.json ...] --module MODULE.json [MODULE.json ...]
                     [--phase0 PHASE0.json] [--limit 0.10]

Each JSON file is the output of `fps-spike-tls --json` or of `handshake_bench --json`
in its default config mode. The script takes the median of each rate over the
files of a side and prints the ratio module / spike. Exit status 1 means that
a median ratio fell more than `--limit` below 1.
"""

import argparse
import json
import statistics
import sys

FIELDS = (
    ("client", "client_handshakes_per_core_s"),
    ("server", "server_handshakes_per_core_s"),
    ("both", "both_sides_handshakes_per_core_s"),
)


def key(row):
    return (row["version"], row["cert"], row["group"], row["mode"])


def load(paths):
    out = {}
    for path in paths:
        with open(path) as f:
            for row in json.load(f):
                out.setdefault(key(row), []).append(row)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--spike", nargs="+", required=True)
    ap.add_argument("--module", nargs="+", required=True)
    ap.add_argument("--phase0")
    ap.add_argument("--limit", type=float, default=0.10)
    args = ap.parse_args()
    spike, module = load(args.spike), load(args.module)
    phase0 = {}
    if args.phase0:
        with open(args.phase0) as f:
            phase0 = {key(r): r for r in json.load(f)}
    print(
        f"{len(args.spike)} spike files, {len(args.module)} module files, medians in handshakes per second"
    )
    print(
        f"{'tls':3} {'cert':10} {'group':9} {'mode':7} {'side':6} {'phase0':>7} {'spike':>8} {'module':>8} {'ratio':>6} {'worst':>6}"
    )
    worst = 1.0
    for k in sorted(spike):
        for side, field in FIELDS:
            s = statistics.median(r[field] for r in spike[k])
            m = statistics.median(r[field] for r in module[k])
            low = min(r[field] for r in module[k]) / s
            worst = min(worst, m / s)
            base = f"{phase0[k][field]:7.0f}" if k in phase0 else f"{'-':>7}"
            print(
                f"{k[0]:3} {k[1]:10} {k[2]:9} {k[3]:7} {side:6} {base} {s:8.0f} {m:8.0f} {m / s:6.3f} {low:6.3f}"
            )
    print(f"worst median ratio {worst:.3f}, limit {1 - args.limit:.3f}")
    return 0 if worst >= 1 - args.limit else 1


if __name__ == "__main__":
    sys.exit(main())
