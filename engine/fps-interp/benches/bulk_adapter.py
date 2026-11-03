#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Convert the criterion estimates of benches/bulk.rs into an fps-bench/v1 result.

Usage:
    python3 engine/fps-interp/benches/bulk_adapter.py <criterion dir> -o result.json

The criterion directory is `$CARGO_TARGET_DIR/criterion` after
`cargo bench -p fps-interp --bench bulk`. The adapter needs only the Python
standard library, like the tools in bench/.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import platform
import sys
from pathlib import Path
from typing import Any

SUITE = "fps-interp-bulk"
GROUP = "bulk-1mib"
VARIANTS = ("interp", "handwritten", "handwritten-copy")
SESSIONS_PER_ITERATION = 16
BYTES_PER_SESSION = 4 * (1 << 20)
# The Gate 0 criterion: the interpreter keeps at least 75% of the hand
# written session rate on the same host.
MIN_RATIO = 0.75


def iteration_seconds(variant_dir: Path) -> float:
    """Return criterion's estimate of one iteration in seconds.

    Linear sampling reports the slope as its headline figure, so we prefer it
    and fall back to the mean.
    """
    est = json.loads((variant_dir / "new" / "estimates.json").read_text())
    entry = est.get("slope") or est["mean"]
    return float(entry["point_estimate"]) / 1e9


def host_facts() -> dict[str, Any]:
    cpu = platform.processor() or "unknown"
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    return {"cpu": cpu, "vcpus": os.cpu_count(), "kernel": platform.release()}


def convert(criterion_dir: Path) -> dict[str, Any]:
    metrics: dict[str, Any] = {}
    rates: dict[str, float] = {}
    for variant in VARIANTS:
        seconds = iteration_seconds(criterion_dir / GROUP / variant)
        rate = SESSIONS_PER_ITERATION / seconds
        rates[variant] = rate
        name = f"{GROUP}-{variant}"
        metrics[f"{name}.sessions_per_s"] = {
            "value": round(rate, 2),
            "better": "higher",
            "tolerance": 0.10,
            "unit": "sessions/s",
        }
        metrics[f"{name}.l7_bits_per_s"] = {
            "value": round(rate * BYTES_PER_SESSION * 8, 0),
            "better": "higher",
            "tolerance": 0.10,
            "unit": "bit/s",
        }
    metrics[f"{GROUP}.interp_over_baseline"] = {
        "value": round(rates["interp"] / rates["handwritten"], 4),
        "better": "higher",
        "tolerance": 0.10,
        "unit": "ratio",
    }
    metrics[f"{GROUP}.copy_over_baseline"] = {
        "value": round(rates["handwritten-copy"] / rates["handwritten"], 4),
        "better": "higher",
        "tolerance": 0.10,
        "unit": "ratio",
    }
    return {
        "format": "fps-bench/v1",
        "suite": SUITE,
        "recorded": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "source": "cargo bench -p fps-interp --bench bulk",
        "host": host_facts(),
        "metrics": metrics,
    }


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("criterion_dir", type=Path)
    p.add_argument("-o", "--output", type=Path, required=True)
    args = p.parse_args(argv)
    try:
        doc = convert(args.criterion_dir)
    except (OSError, KeyError, ValueError) as e:
        print(f"bulk_adapter: {e}", file=sys.stderr)
        return 2
    args.output.write_text(json.dumps(doc, indent=2) + "\n")
    ratio = doc["metrics"][f"{GROUP}.interp_over_baseline"]["value"]
    verdict = "meets" if ratio >= MIN_RATIO else "misses"
    print(f"interp over hand written {ratio:.3f}, {verdict} the {MIN_RATIO} criterion")
    return 0 if ratio >= MIN_RATIO else 1


if __name__ == "__main__":
    sys.exit(main())
