# SPDX-License-Identifier: GPL-3.0-only
"""Compare a benchmark result with a baseline and fail on regressions.

Both files use the format ``fps-bench/v1``, which ``bench/README.md`` defines.
The baseline owns the policy: each of its metrics names a direction, either
``higher`` or ``lower`` is better, and a relative tolerance. A metric regresses
when the result is worse than the baseline by more than the allowed amount,
which is ``max(tolerance * abs(baseline), abs_tolerance)``.

Exit codes are 0 when every metric holds, 1 when at least one metric regressed
or is missing from the result, and 2 when an input file is invalid.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

FORMAT = "fps-bench/v1"
DEFAULT_TOLERANCE = 0.10
DIRECTIONS = ("higher", "lower")
HOST_KEYS = ("cpu", "vcpus")


class FormatError(ValueError):
    """An input file does not follow the ``fps-bench/v1`` format."""


@dataclass(frozen=True)
class Metric:
    """One metric of a baseline or a result."""

    name: str
    value: float
    better: str
    tolerance: float
    abs_tolerance: float
    unit: str


@dataclass(frozen=True)
class BenchFile:
    """A parsed ``fps-bench/v1`` document."""

    suite: str
    host: dict[str, Any]
    metrics: dict[str, Metric]


@dataclass(frozen=True)
class Verdict:
    """The comparison of one baseline metric with the result."""

    name: str
    status: str
    baseline: float
    result: float | None
    better: str
    tolerance: float
    abs_tolerance: float
    unit: str

    @property
    def worse_by(self) -> float | None:
        """Return the worsening relative to the baseline, positive when worse."""
        if self.result is None or self.baseline == 0:
            return None
        delta = (
            self.baseline - self.result
            if self.better == "higher"
            else self.result - self.baseline
        )
        return delta / abs(self.baseline)

    def to_dict(self) -> dict[str, Any]:
        """Return the verdict as a JSON ready mapping."""
        return {
            "name": self.name,
            "status": self.status,
            "baseline": self.baseline,
            "result": self.result,
            "better": self.better,
            "tolerance": self.tolerance,
            "abs_tolerance": self.abs_tolerance,
            "unit": self.unit,
            "worse_by": self.worse_by,
        }


@dataclass(frozen=True)
class Report:
    """The outcome of a comparison."""

    suite: str
    verdicts: list[Verdict]
    extra: list[str]
    host_notes: list[str]

    @property
    def passed(self) -> bool:
        """Return true when no metric regressed or went missing."""
        return all(v.status in ("ok", "improved") for v in self.verdicts)

    def to_dict(self) -> dict[str, Any]:
        """Return the report as a JSON ready mapping."""
        return {
            "suite": self.suite,
            "passed": self.passed,
            "verdicts": [v.to_dict() for v in self.verdicts],
            "extra_metrics": self.extra,
            "host_notes": self.host_notes,
        }


def _number(value: Any, where: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise FormatError(f"{where} must be a number, got {value!r}")
    number = float(value)
    if not math.isfinite(number):
        raise FormatError(f"{where} must be finite, got {value!r}")
    return number


def parse(doc: Any, default_tolerance: float = DEFAULT_TOLERANCE) -> BenchFile:
    """Validate a decoded ``fps-bench/v1`` document and return its metrics.

    A metric may omit ``better`` only in a result file; the comparison always
    takes the direction and the tolerances from the baseline.
    """
    if not isinstance(doc, dict):
        raise FormatError("the document must be a JSON object")
    if doc.get("format") != FORMAT:
        raise FormatError(f"format must be {FORMAT!r}, got {doc.get('format')!r}")
    suite = doc.get("suite")
    if not isinstance(suite, str) or not suite:
        raise FormatError("suite must be a non empty string")
    host = doc.get("host", {})
    if not isinstance(host, dict):
        raise FormatError("host must be an object")
    raw = doc.get("metrics")
    if not isinstance(raw, dict) or not raw:
        raise FormatError("metrics must be a non empty object")
    metrics: dict[str, Metric] = {}
    for name, entry in raw.items():
        where = f"metrics[{name!r}]"
        if isinstance(entry, (int, float)) and not isinstance(entry, bool):
            entry = {"value": entry}
        if not isinstance(entry, dict):
            raise FormatError(f"{where} must be an object or a number")
        better = entry.get("better", "higher")
        if better not in DIRECTIONS:
            raise FormatError(
                f"{where}.better must be 'higher' or 'lower', got {better!r}"
            )
        tolerance = _number(
            entry.get("tolerance", default_tolerance), f"{where}.tolerance"
        )
        abs_tolerance = _number(entry.get("abs_tolerance", 0), f"{where}.abs_tolerance")
        if tolerance < 0 or abs_tolerance < 0:
            raise FormatError(f"{where} tolerances must not be negative")
        unit = entry.get("unit", "")
        if not isinstance(unit, str):
            raise FormatError(f"{where}.unit must be a string")
        metrics[name] = Metric(
            name=name,
            value=_number(entry.get("value"), f"{where}.value"),
            better=better,
            tolerance=tolerance,
            abs_tolerance=abs_tolerance,
            unit=unit,
        )
    return BenchFile(suite=suite, host=host, metrics=metrics)


def load(path: Path, default_tolerance: float = DEFAULT_TOLERANCE) -> BenchFile:
    """Read and validate one ``fps-bench/v1`` file."""
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        raise FormatError(f"{path}: not valid JSON: {err}") from err
    try:
        return parse(doc, default_tolerance)
    except FormatError as err:
        raise FormatError(f"{path}: {err}") from err


def judge(base: Metric, result: float | None) -> Verdict:
    """Compare one result value with its baseline metric."""
    if result is None:
        status = "missing"
    else:
        delta = base.value - result if base.better == "higher" else result - base.value
        allowed = max(base.tolerance * abs(base.value), base.abs_tolerance)
        # A tiny epsilon keeps a result exactly at the limit from failing on
        # floating point rounding.
        if delta > allowed * (1 + 1e-9) + 1e-12:
            status = "regressed"
        elif delta < 0:
            status = "improved"
        else:
            status = "ok"
    return Verdict(
        name=base.name,
        status=status,
        baseline=base.value,
        result=result,
        better=base.better,
        tolerance=base.tolerance,
        abs_tolerance=base.abs_tolerance,
        unit=base.unit,
    )


def compare(result: BenchFile, baseline: BenchFile) -> Report:
    """Compare every baseline metric with the result file."""
    if result.suite != baseline.suite:
        raise FormatError(
            f"suite mismatch: result {result.suite!r}, baseline {baseline.suite!r}"
        )
    verdicts = []
    for name in sorted(baseline.metrics):
        got = result.metrics.get(name)
        verdicts.append(
            judge(baseline.metrics[name], None if got is None else got.value)
        )
    extra = sorted(set(result.metrics) - set(baseline.metrics))
    host_notes = [
        f"host {key} differs: result {result.host.get(key)!r}, baseline {baseline.host.get(key)!r}"
        for key in HOST_KEYS
        if result.host.get(key) != baseline.host.get(key)
    ]
    return Report(
        suite=baseline.suite, verdicts=verdicts, extra=extra, host_notes=host_notes
    )


def _fmt(value: float | None) -> str:
    if value is None:
        return "-"
    return f"{value:.6g}"


def render(report: Report) -> str:
    """Return a plain text table of the report."""
    lines = [f"suite {report.suite}"]
    lines += [f"note: {note}" for note in report.host_notes]
    lines.append(
        f"{'status':<10} {'metric':<48} {'baseline':>12} {'result':>12} {'worse by':>9} {'limit':>7}"
    )
    for v in report.verdicts:
        worse = "-" if v.worse_by is None else f"{v.worse_by * 100:+.1f}%"
        limit = f"{v.tolerance * 100:.0f}%"
        if v.abs_tolerance:
            limit += f"|{_fmt(v.abs_tolerance)}"
        lines.append(
            f"{v.status:<10} {v.name:<48} {_fmt(v.baseline):>12} {_fmt(v.result):>12} {worse:>9} {limit:>7}"
        )
    for name in report.extra:
        lines.append(f"{'extra':<10} {name}")
    lines.append("PASS" if report.passed else "FAIL")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    """Run the comparison from the command line and return the exit code."""
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("result", type=Path, help="result file in fps-bench/v1 format")
    parser.add_argument(
        "baseline", type=Path, help="baseline file in fps-bench/v1 format"
    )
    parser.add_argument(
        "--tolerance",
        type=float,
        default=DEFAULT_TOLERANCE,
        help="relative tolerance for baseline metrics that state none (default 0.10)",
    )
    parser.add_argument(
        "--json", type=Path, help="also write the report as JSON to this path"
    )
    parser.add_argument(
        "--require-same-host",
        action="store_true",
        help="fail when the cpu or vcpus of the two host blocks differ",
    )
    args = parser.parse_args(argv)
    try:
        baseline = load(args.baseline, args.tolerance)
        result = load(args.result, args.tolerance)
        report = compare(result, baseline)
    except (OSError, FormatError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    print(render(report))
    if args.json is not None:
        args.json.write_text(
            json.dumps(report.to_dict(), indent=2) + "\n", encoding="utf-8"
        )
    if args.require_same_host and report.host_notes:
        print("FAIL because the hosts differ and --require-same-host is set")
        return 1
    return 0 if report.passed else 1


if __name__ == "__main__":
    sys.exit(main())
