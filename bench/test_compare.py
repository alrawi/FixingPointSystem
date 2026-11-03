# SPDX-License-Identifier: GPL-3.0-only
"""Unit and component tests of bench/compare.py."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import compare
import pytest


def doc(
    metrics: dict[str, Any],
    suite: str = "synthetic",
    host: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "format": compare.FORMAT,
        "suite": suite,
        "host": host or {},
        "metrics": metrics,
    }


def write(tmp_path: Path, name: str, content: dict[str, Any]) -> Path:
    path = tmp_path / name
    path.write_text(json.dumps(content), encoding="utf-8")
    return path


def rate_baseline(tolerance: float = 0.10) -> dict[str, Any]:
    return doc(
        {
            "rate": {
                "value": 1000.0,
                "better": "higher",
                "tolerance": tolerance,
                "unit": "sessions/s",
            },
            "p99": {
                "value": 2000.0,
                "better": "lower",
                "tolerance": tolerance,
                "unit": "us",
            },
        }
    )


@pytest.mark.parametrize(
    ("worse_by", "expected"),
    [(0.15, 1), (0.05, 0), (0.0, 0), (0.10, 0), (-0.20, 0)],
)
def test_acceptance_regression_against_ten_percent(
    tmp_path: Path, worse_by: float, expected: int
) -> None:
    # The acceptance case of QA-03: 15% worse fails and 5% worse passes, in
    # both directions.
    base = write(tmp_path, "base.json", rate_baseline())
    result = write(
        tmp_path,
        "result.json",
        doc({"rate": 1000.0 * (1 - worse_by), "p99": 2000.0 * (1 + worse_by)}),
    )
    assert compare.main([str(result), str(base)]) == expected


def test_direction_decides_which_change_is_a_regression() -> None:
    higher = compare.Metric("rate", 100.0, "higher", 0.10, 0.0, "")
    lower = compare.Metric("lat", 100.0, "lower", 0.10, 0.0, "")
    assert compare.judge(higher, 85.0).status == "regressed"
    assert compare.judge(higher, 115.0).status == "improved"
    assert compare.judge(lower, 115.0).status == "regressed"
    assert compare.judge(lower, 85.0).status == "improved"
    assert compare.judge(lower, 95.0).worse_by == pytest.approx(-0.05)


def test_only_one_regressed_metric_fails_the_report() -> None:
    base = compare.parse(rate_baseline())
    result = compare.parse(doc({"rate": 1050.0, "p99": 2300.0}))
    report = compare.compare(result, base)
    assert not report.passed
    assert {v.name: v.status for v in report.verdicts} == {
        "p99": "regressed",
        "rate": "improved",
    }


def test_per_metric_tolerance_overrides_the_default() -> None:
    base = compare.parse(rate_baseline(tolerance=0.20))
    report = compare.compare(compare.parse(doc({"rate": 850.0, "p99": 2300.0})), base)
    assert report.passed


def test_default_tolerance_applies_when_the_baseline_states_none(
    tmp_path: Path,
) -> None:
    base = write(tmp_path, "base.json", doc({"rate": {"value": 100.0}}))
    result = write(tmp_path, "result.json", doc({"rate": 88.0}))
    assert compare.main([str(result), str(base)]) == 1
    assert compare.main([str(result), str(base), "--tolerance", "0.15"]) == 0


def test_result_direction_never_overrides_the_baseline() -> None:
    base = compare.parse(
        doc({"rate": {"value": 100.0, "better": "higher", "tolerance": 0.10}})
    )
    result = compare.parse(
        doc({"rate": {"value": 80.0, "better": "lower", "tolerance": 0.5}})
    )
    assert compare.compare(result, base).verdicts[0].status == "regressed"


def test_absolute_tolerance_handles_a_zero_baseline() -> None:
    strict = compare.Metric("failed", 0.0, "lower", 0.0, 0.0, "")
    slack = compare.Metric("failed", 0.0, "lower", 0.0, 2.0, "")
    assert compare.judge(strict, 0.0).status == "ok"
    assert compare.judge(strict, 1.0).status == "regressed"
    assert compare.judge(slack, 2.0).status == "ok"
    assert compare.judge(slack, 3.0).status == "regressed"
    assert compare.judge(strict, 1.0).worse_by is None


def test_missing_metric_fails_and_extra_metric_is_listed() -> None:
    base = compare.parse(rate_baseline())
    report = compare.compare(compare.parse(doc({"rate": 1000.0, "new": 1.0})), base)
    assert not report.passed
    assert [v.status for v in report.verdicts if v.name == "p99"] == ["missing"]
    assert report.extra == ["new"]


def test_host_difference_is_a_note_unless_required(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    base_doc = rate_baseline()
    base_doc["host"] = {"cpu": "A", "vcpus": 2}
    base = write(tmp_path, "base.json", base_doc)
    result = write(
        tmp_path,
        "result.json",
        doc({"rate": 1000.0, "p99": 2000.0}, host={"cpu": "B", "vcpus": 2}),
    )
    assert compare.main([str(result), str(base)]) == 0
    assert "host cpu differs" in capsys.readouterr().out
    assert compare.main([str(result), str(base), "--require-same-host"]) == 1


def test_json_report_is_written(tmp_path: Path) -> None:
    base = write(tmp_path, "base.json", rate_baseline())
    result = write(tmp_path, "result.json", doc({"rate": 800.0, "p99": 2000.0}))
    out = tmp_path / "report.json"
    assert compare.main([str(result), str(base), "--json", str(out)]) == 1
    report = json.loads(out.read_text(encoding="utf-8"))
    assert report["passed"] is False
    rate = next(v for v in report["verdicts"] if v["name"] == "rate")
    assert rate["status"] == "regressed"
    assert rate["worse_by"] == pytest.approx(0.2)


@pytest.mark.parametrize(
    "bad",
    [
        [],
        {"format": "other", "suite": "s", "metrics": {"a": 1}},
        {"format": compare.FORMAT, "suite": "", "metrics": {"a": 1}},
        {"format": compare.FORMAT, "suite": "s", "metrics": {}},
        {"format": compare.FORMAT, "suite": "s", "metrics": {"a": "fast"}},
        {"format": compare.FORMAT, "suite": "s", "metrics": {"a": {"value": True}}},
        {
            "format": compare.FORMAT,
            "suite": "s",
            "metrics": {"a": {"value": 1, "better": "up"}},
        },
        {
            "format": compare.FORMAT,
            "suite": "s",
            "metrics": {"a": {"value": 1, "tolerance": -0.1}},
        },
        {"format": compare.FORMAT, "suite": "s", "host": [], "metrics": {"a": 1}},
    ],
)
def test_invalid_documents_are_rejected(bad: Any) -> None:
    with pytest.raises(compare.FormatError):
        compare.parse(bad)


def test_invalid_input_exits_with_two(tmp_path: Path) -> None:
    base = write(tmp_path, "base.json", rate_baseline())
    broken = tmp_path / "broken.json"
    broken.write_text("{", encoding="utf-8")
    other_suite = write(tmp_path, "other.json", doc({"rate": 1.0}, suite="other"))
    assert compare.main([str(broken), str(base)]) == 2
    assert compare.main([str(other_suite), str(base)]) == 2
    assert compare.main([str(tmp_path / "absent.json"), str(base)]) == 2
