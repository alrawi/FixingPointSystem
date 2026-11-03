# SPDX-License-Identifier: GPL-3.0-only
"""Convert a Spike S1 results directory into one ``fps-bench/v1`` file.

``spikes/s1-engine/run-bench.sh`` writes one JSON file per run and a
``host.txt`` file into ``spikes/s1-engine/results/<label>/``. This adapter
reads that directory and prints a document that ``bench/compare.py`` accepts,
either as a new result or, after review, as a baseline.

Runs named ``<name>-r<N>`` are repetitions, and the adapter reports their mean
under ``<name>``. For every workload that has both an ``-interp`` and a
``-baseline`` run, it adds the ratio of their session rates, which depends less
on the host than the absolute rates do.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

FORMAT = "fps-bench/v1"
SUITE = "s1-engine"

# Metric name suffix, path into the run JSON, direction, relative tolerance,
# absolute tolerance, and unit. Run to run variation of the phase 0 runs was
# about 2% for rates and 5% for p99 latency, so latency gets a wider margin.
FIELDS: tuple[tuple[str, tuple[str, ...], str, float, float, str], ...] = (
    ("sessions_per_s", ("sessions_per_s",), "higher", 0.10, 0.0, "sessions/s"),
    ("l7_bits_per_s", ("l7_bits_per_s",), "higher", 0.10, 0.0, "bit/s"),
    ("latency_p50_us", ("session_latency_us", "p50"), "lower", 0.15, 0.0, "us"),
    ("latency_p99_us", ("session_latency_us", "p99"), "lower", 0.25, 0.0, "us"),
    ("sessions_failed", ("sessions_failed",), "lower", 0.0, 0.0, "sessions"),
)
RATIO_TOLERANCE = 0.10
REPEAT = re.compile(r"^(?P<name>.+)-r(?P<rep>\d+)$")


class AdapterError(ValueError):
    """The results directory cannot be converted."""


def parse_host(text: str) -> dict[str, Any]:
    """Parse the ``key: value`` lines of ``host.txt``."""
    host: dict[str, Any] = {}
    for line in text.splitlines():
        key, sep, value = line.partition(":")
        if not sep:
            continue
        value = value.strip()
        host[key.strip()] = int(value) if value.isdigit() else value
    return host


def _lookup(run: dict[str, Any], path: tuple[str, ...], where: str) -> float:
    value: Any = run
    for key in path:
        if not isinstance(value, dict) or key not in value:
            raise AdapterError(f"{where}: missing field {'.'.join(path)}")
        value = value[key]
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise AdapterError(f"{where}: field {'.'.join(path)} is not a number")
    return float(value)


def _metric(
    value: float, better: str, tolerance: float, abs_tolerance: float, unit: str
) -> dict[str, Any]:
    entry: dict[str, Any] = {
        "value": round(value, 6),
        "better": better,
        "tolerance": tolerance,
    }
    if abs_tolerance:
        entry["abs_tolerance"] = abs_tolerance
    entry["unit"] = unit
    return entry


def convert(results_dir: Path) -> dict[str, Any]:
    """Return the ``fps-bench/v1`` document for one results directory."""
    host_file = results_dir / "host.txt"
    if not host_file.is_file():
        raise AdapterError(f"{results_dir}: no host.txt")
    host = parse_host(host_file.read_text(encoding="utf-8"))
    host["label"] = results_dir.name
    runs: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for path in sorted(results_dir.glob("*.json")):
        try:
            run = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as err:
            raise AdapterError(f"{path}: not valid JSON: {err}") from err
        if not isinstance(run, dict):
            raise AdapterError(f"{path}: a run file must hold a JSON object")
        match = REPEAT.match(path.stem)
        runs[match.group("name") if match else path.stem].append(
            {"file": path.name, "run": run}
        )
    if not runs:
        raise AdapterError(f"{results_dir}: no run files")
    metrics: dict[str, dict[str, Any]] = {}
    rates: dict[str, float] = {}
    for name in sorted(runs):
        for suffix, field, better, tolerance, abs_tolerance, unit in FIELDS:
            values = [
                _lookup(r["run"], field, f"{results_dir / r['file']}")
                for r in runs[name]
            ]
            mean = sum(values) / len(values)
            metrics[f"{name}.{suffix}"] = _metric(
                mean, better, tolerance, abs_tolerance, unit
            )
            if suffix == "sessions_per_s":
                rates[name] = mean
    for name in sorted(rates):
        if not name.endswith("-interp"):
            continue
        workload = name.removesuffix("-interp")
        reference = rates.get(f"{workload}-baseline")
        if reference:
            ratio = rates[name] / reference
            metrics[f"{workload}.interp_over_baseline"] = _metric(
                ratio, "higher", RATIO_TOLERANCE, 0.0, "ratio"
            )
    return {
        "format": FORMAT,
        "suite": SUITE,
        "recorded": host.get("date", ""),
        "source": f"spikes/s1-engine/results/{results_dir.name}",
        "host": host,
        "metrics": dict(sorted(metrics.items())),
    }


def dumps(doc: dict[str, Any]) -> str:
    """Serialize a document the same way every time, so baselines diff cleanly."""
    return json.dumps(doc, indent=2) + "\n"


def main(argv: list[str] | None = None) -> int:
    """Convert a results directory and print or write the document."""
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "results_dir", type=Path, help="a spikes/s1-engine/results/<label> directory"
    )
    parser.add_argument(
        "-o",
        "--output",
        type=Path,
        help="write to this file instead of standard output",
    )
    args = parser.parse_args(argv)
    try:
        text = dumps(convert(args.results_dir))
    except (OSError, AdapterError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    if args.output is None:
        sys.stdout.write(text)
    else:
        args.output.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
