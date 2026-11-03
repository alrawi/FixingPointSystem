# SPDX-License-Identifier: GPL-3.0-only
"""Component tests of bench/s1_adapter.py against the committed S1 results."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import compare
import pytest
import s1_adapter

ROOT = Path(__file__).resolve().parent.parent
PHASE0 = ROOT / "spikes" / "s1-engine" / "results" / "phase0-container"
BASELINE = ROOT / "bench" / "baselines" / "s1-engine-phase0-container.json"


def run(rate: float, p99: int = 4000, failed: int = 0) -> dict[str, Any]:
    return {
        "sessions_per_s": rate,
        "l7_bits_per_s": rate * 1000,
        "session_latency_us": {"p50": p99 // 2, "p99": p99},
        "sessions_failed": failed,
    }


def make_results(tmp_path: Path, runs: dict[str, dict[str, Any]]) -> Path:
    out = tmp_path / "synthetic-host"
    out.mkdir(parents=True)
    (out / "host.txt").write_text(
        "cpu: Test CPU\nvcpus: 4\ndate: 2026-11-03T00:00:00Z\n", encoding="utf-8"
    )
    for name, content in runs.items():
        (out / f"{name}.json").write_text(json.dumps(content), encoding="utf-8")
    return out


def test_recorded_baseline_matches_the_committed_spike_results() -> None:
    # The baseline must be the adapter output of the raw files, so nobody can
    # edit a number by hand without the test noticing.
    assert BASELINE.read_text(encoding="utf-8") == s1_adapter.dumps(
        s1_adapter.convert(PHASE0)
    )


def test_baseline_carries_host_details_and_valid_metrics() -> None:
    base = compare.load(BASELINE)
    assert base.suite == "s1-engine"
    assert base.host["vcpus"] == 2
    assert base.host["cpu"].startswith("Intel")
    assert {"kernel", "rustc", "date", "label"} <= set(base.host)
    assert base.metrics["throughput.interp_over_baseline"].value == pytest.approx(
        0.739, abs=1e-3
    )
    assert base.metrics["keepalive-interp.sessions_per_s"].value == pytest.approx(
        14840.8, abs=0.1
    )


def test_baseline_passes_against_itself() -> None:
    base = compare.load(BASELINE)
    assert compare.compare(base, base).passed


def test_repetitions_are_averaged_and_ratios_added(tmp_path: Path) -> None:
    results = make_results(
        tmp_path,
        {
            "w-interp-r1": run(90.0),
            "w-interp-r2": run(110.0),
            "w-baseline": run(200.0),
            "solo": run(5.0),
        },
    )
    doc = s1_adapter.convert(results)
    metrics = doc["metrics"]
    assert metrics["w-interp.sessions_per_s"]["value"] == 100.0
    assert metrics["w.interp_over_baseline"]["value"] == 0.5
    assert "solo.interp_over_baseline" not in metrics
    assert doc["host"] == {
        "cpu": "Test CPU",
        "vcpus": 4,
        "date": "2026-11-03T00:00:00Z",
        "label": "synthetic-host",
    }
    compare.parse(doc)


@pytest.mark.parametrize(("factor", "expected"), [(0.85, 1), (0.95, 0)])
def test_synthetic_regression_of_a_converted_run(
    tmp_path: Path, factor: float, expected: int
) -> None:
    base_dir = make_results(tmp_path / "a", {"cps-interp": run(1000.0)})
    new_dir = make_results(tmp_path / "b", {"cps-interp": run(1000.0 * factor)})
    base_file = tmp_path / "base.json"
    new_file = tmp_path / "new.json"
    assert s1_adapter.main([str(base_dir), "-o", str(base_file)]) == 0
    assert s1_adapter.main([str(new_dir), "-o", str(new_file)]) == 0
    assert compare.main([str(new_file), str(base_file)]) == expected


def test_new_failures_regress(tmp_path: Path) -> None:
    base = compare.parse(
        s1_adapter.convert(make_results(tmp_path / "a", {"x": run(10.0)}))
    )
    new = compare.parse(
        s1_adapter.convert(make_results(tmp_path / "b", {"x": run(10.0, failed=1)}))
    )
    statuses = {v.name: v.status for v in compare.compare(new, base).verdicts}
    assert statuses["x.sessions_failed"] == "regressed"


def test_bad_directories_exit_with_two(tmp_path: Path) -> None:
    assert s1_adapter.main([str(tmp_path)]) == 2
    (tmp_path / "host.txt").write_text("cpu: x\n", encoding="utf-8")
    assert s1_adapter.main([str(tmp_path)]) == 2
    (tmp_path / "bad.json").write_text(
        json.dumps({"sessions_per_s": 1}), encoding="utf-8"
    )
    assert s1_adapter.main([str(tmp_path)]) == 2
