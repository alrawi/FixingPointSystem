# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of gate1_report.py, which need no root and no tools."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import gate1_report as g
import pytest


def run_report(state: str = "completed", passed: bool = True) -> dict[str, Any]:
    return {
        "run": {
            "id": "run_1",
            "config_id": "cfg_1",
            "state": state,
            "summary": {"sessions_attempted": 300, "sessions_failed": 0},
            "verdict": {
                "passed": passed,
                "criteria": [
                    {
                        "metric": "sessions.failed_ratio",
                        "op": "<=",
                        "value": 0.01,
                        "observed": 0,
                        "passed": passed,
                    }
                ],
            },
        },
        "realism": [],
    }


def app(name: str, sessions: int, conforming: int, accepted: str) -> dict[str, Any]:
    return {
        "application": name,
        "accepted": [accepted],
        "sessions": sessions,
        "conforming": conforming,
        "ratio": conforming / sessions if sessions else 0.0,
        "passed": True,
        "observed": {accepted: conforming, "Unknown": sessions - conforming},
    }


def realism(
    http: tuple[int, int] = (200, 200),
    dns: tuple[int, int] = (100, 100),
    parsers_ok: bool = True,
) -> dict[str, Any]:
    return {
        "report": "fps-realism-report",
        "passed": True,
        "capture": {
            "file": "gate1.pcapng",
            "sha256": "ab",
            "frames": 10,
            "sessions": http[0] + dns[0],
        },
        "checks": [
            {
                "check": "conformance",
                "passed": True,
                "classifier": "ndpi",
                "classifier_version": "4.2.0",
                "threshold": 0.99,
                "applications": [app("http1", *http, "HTTP"), app("dns", *dns, "DNS")],
                "unlabelled_sessions": 0,
                "mismatches_total": 0,
            },
            {
                "check": "parser_errors",
                "passed": parsers_ok,
                "parsers": [
                    {"parser": "tshark", "version": "4.2.2"},
                    {"parser": "zeek", "version": "9.0.1"},
                ],
                "missing_parsers": [],
                "applications": [
                    {
                        "application": "http1",
                        "sessions": http[0],
                        "parser_errors": 0,
                        "malformed_packets": 0,
                    },
                    {
                        "application": "dns",
                        "sessions": dns[0],
                        "parser_errors": 0 if parsers_ok else 1,
                        "malformed_packets": 0,
                    },
                ],
                "errors_total": 0 if parsers_ok else 1,
            },
        ],
    }


def metrics(http: int = 200, dns: int = 100) -> dict[str, Any]:
    def sample(key: str, n: int) -> dict[str, Any]:
        return {
            "ts": "t",
            "view": "application",
            "key": key,
            "values": {"sessions.attempted": n},
        }

    return {
        "view": "application",
        "samples": [
            sample("http1", http // 2),
            sample("dns", dns // 2),
            sample("http1", http),
            sample("dns", dns),
        ],
    }


def failed(checks: list[g.Check]) -> list[str]:
    return [c.name for c in checks if not c.passed]


def test_passing_run_passes_every_check() -> None:
    checks, ratios = g.evaluate(run_report(), realism(), metrics(), 0.99, 100)
    assert failed(checks) == []
    assert ratios == {"http1": 1.0, "dns": 1.0}


def test_threshold_above_measured_ratio_fails() -> None:
    checks, ratios = g.evaluate(
        run_report(), realism(http=(200, 199)), metrics(), 0.99, 100
    )
    assert failed(checks) == []
    raised = g.raised_threshold(ratios)
    assert raised > ratios["http1"]
    checks, _ = g.evaluate(
        run_report(), realism(http=(200, 199)), metrics(), raised, 100
    )
    assert failed(checks) == ["Conformance of http1"]


def test_full_conformance_fails_above_one() -> None:
    _, ratios = g.evaluate(run_report(), realism(), metrics(), 0.99, 100)
    raised = g.raised_threshold(ratios)
    assert raised == pytest.approx(1.001)
    checks, _ = g.evaluate(run_report(), realism(), metrics(), raised, 100)
    assert failed(checks) == ["Conformance of http1", "Conformance of dns"]


def test_low_conformance_fails() -> None:
    checks, _ = g.evaluate(run_report(), realism(dns=(100, 95)), metrics(), 0.99, 100)
    assert failed(checks) == ["Conformance of dns"]


def test_failed_run_verdict_fails() -> None:
    checks, _ = g.evaluate(run_report("failed", False), realism(), metrics(), 0.99, 100)
    assert failed(checks) == ["Client run verdict"]


def test_too_few_sessions_fail() -> None:
    checks, _ = g.evaluate(
        run_report(), realism(dns=(20, 20)), metrics(dns=20), 0.99, 100
    )
    assert failed(checks) == ["Sessions of dns in the capture"]


def test_missing_application_fails() -> None:
    r = realism()
    r["checks"][0]["applications"] = r["checks"][0]["applications"][:1]
    checks, ratios = g.evaluate(run_report(), r, metrics(), 0.99, 100)
    assert "Conformance of dns" in failed(checks)
    assert "dns" not in ratios


def test_parser_errors_fail() -> None:
    checks, _ = g.evaluate(
        run_report(), realism(parsers_ok=False), metrics(), 0.99, 100
    )
    assert failed(checks) == ["Parser error check"]


def findings_doc(*entries: dict[str, Any]) -> dict[str, Any]:
    return {
        "errors_total": len(entries),
        "listed": len(entries),
        "findings": list(entries),
    }


def finding(kind: str, reused: bool, session: int | None = 7) -> dict[str, Any]:
    return {
        "parser": "zeek" if kind == "weird" else "tshark",
        "kind": kind,
        "name": "premature_connection_reuse" if kind == "weird" else "http",
        "frame": 12,
        "session": session,
        "application": "http1",
        "reused_tuple": reused,
    }


def test_zeek_weird_in_a_reused_tuple_is_not_counted() -> None:
    r = realism(parsers_ok=False)
    docs = findings_doc(finding("weird", True))
    checks, _ = g.evaluate(run_report(), r, metrics(), 0.99, 100, docs)
    assert failed(checks) == []
    [parser] = [c for c in checks if c.name == "Parser error check"]
    assert parser.measured.startswith(
        "0 findings from tshark, zeek, 1 premature_connection_reuse weirds not counted"
    )


@pytest.mark.parametrize(
    "entries",
    [
        # A malformed packet of tshark counts even in a reused tuple.
        [finding("malformed", True)],
        # A weird of another name counts, also when Zeek merges it with the reuse weird.
        [{**finding("weird", True), "name": "HTTP_bad_chunk_size"}],
        [
            {
                **finding("weird", True),
                "name": "premature_connection_reuse,bad_HTTP_reply",
            }
        ],
        # A weird in a session of one connection counts.
        [finding("weird", False)],
        # A weird that no session holds counts.
        [{**finding("weird", True), "session": None}],
        # One counted finding beside an uncounted one fails the gate.
        [finding("weird", True), finding("malformed", True)],
    ],
)
def test_other_findings_still_fail(entries: list[dict[str, Any]]) -> None:
    r = realism(parsers_ok=False)
    r["checks"][1]["errors_total"] = len(entries)
    checks, _ = g.evaluate(
        run_report(), r, metrics(), 0.99, 100, findings_doc(*entries)
    )
    assert failed(checks) == ["Parser error check"]


def test_findings_that_the_realism_report_cut_off_count() -> None:
    r = realism(parsers_ok=False)
    r["checks"][1]["errors_total"] = 150
    docs = findings_doc(finding("weird", True))
    checks, _ = g.evaluate(run_report(), r, metrics(), 0.99, 100, docs)
    assert failed(checks) == ["Parser error check"]


def test_report_lists_the_findings() -> None:
    r = realism(parsers_ok=False)
    docs = findings_doc(finding("weird", True))
    checks, _ = g.evaluate(run_report(), r, metrics(), 0.99, 100, docs)
    text = g.render(checks, run_report(), r, {}, 0.99, docs)
    assert "### Findings" in text
    assert (
        "| zeek | weird | premature_connection_reuse | 12 | 7 | http1 | yes |" in text
    )


def test_missing_parser_fails() -> None:
    r = realism()
    r["checks"][1]["missing_parsers"] = ["zeek"]
    checks, _ = g.evaluate(run_report(), r, metrics(), 0.99, 100)
    assert failed(checks) == ["Parser error check"]


def test_capture_that_misses_sessions_fails() -> None:
    checks, _ = g.evaluate(run_report(), realism(), metrics(http=210), 0.99, 100)
    assert failed(checks) == ["Captured sessions of http1"]


def test_capture_with_merged_five_tuple_passes() -> None:
    checks, _ = g.evaluate(
        run_report(), realism(dns=(99, 99)), metrics(dns=100), 0.99, 99
    )
    assert failed(checks) == []


def test_capture_with_foreign_sessions_fails() -> None:
    checks, _ = g.evaluate(
        run_report(), realism(dns=(101, 101)), metrics(dns=100), 0.99, 100
    )
    assert failed(checks) == ["Captured sessions of dns"]


def test_raised_threshold_needs_a_ratio() -> None:
    with pytest.raises(g.GateInputError):
        g.raised_threshold({})


def write_inputs(out: Path, rr: dict[str, Any], re: dict[str, Any]) -> None:
    (out / "run-report.json").write_text(json.dumps(rr))
    (out / "realism.json").write_text(json.dumps(re))
    (out / "run-metrics.json").write_text(json.dumps(metrics()))
    (out / "context.json").write_text(
        json.dumps({"client_run": "run_1", "rate": "100"})
    )


def test_main_writes_report_and_summary(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    write_inputs(tmp_path, run_report(), realism())
    assert g.main(["--out-dir", str(tmp_path)]) == 0
    md = (tmp_path / "report.md").read_text()
    assert "**Verdict: PASS**" in md
    assert "| Conformance of dns | 100.00% (100 of 100) | >= 99.00% | pass |" in md
    assert "| Client run | run_1 |" in md
    summary = json.loads((tmp_path / "summary.json").read_text())
    assert summary["passed"] is True
    assert g.main(["--raised-threshold", str(tmp_path)]) == 0
    assert capsys.readouterr().out.strip() == "1.001"
    assert g.main(["--out-dir", str(tmp_path), "--threshold", "1.001"]) == 1
    assert "**Verdict: FAIL**" in (tmp_path / "report.md").read_text()


def test_main_reports_missing_inputs(tmp_path: Path) -> None:
    assert g.main(["--out-dir", str(tmp_path)]) == 2
    (tmp_path / "run-report.json").write_text("{not json")
    assert g.main(["--out-dir", str(tmp_path)]) == 2


def test_report_escapes_table_cells() -> None:
    lines = g.table(("A",), [("x|y",)])
    assert lines[-1] == "| x\\|y |"
