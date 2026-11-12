# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of the Gate 2 report generator. They need no root and no network."""

from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Any

import gate2_report as g

PARSERS = [{"parser": "tshark", "version": "4.2"}, {"parser": "zeek", "version": "9"}]


def realism_report(
    app: str, sessions: int = 40, extra: tuple[str, ...] = ()
) -> dict[str, Any]:
    """Build a passing realism report with a conformance and a parser error check."""
    checks: list[dict[str, Any]] = [
        {
            "check": "conformance",
            "passed": True,
            "threshold": 0.99,
            "applications": [
                {
                    "application": app,
                    "sessions": sessions,
                    "conforming": sessions,
                    "ratio": 1.0,
                }
            ],
        },
        {
            "check": "parser_errors",
            "passed": True,
            "parsers": PARSERS,
            "missing_parsers": [],
            "skipped_parsers": [],
            "errors_total": 0,
            "applications": [],
        },
    ]
    for kind in extra:
        entry: dict[str, Any] = {"check": kind, "passed": True}
        if kind == "encryption":
            entry.update(
                sampled_sessions=20, decrypted_sessions=20, ja4_matched_sessions=20
            )
        checks.append(entry)
    return {"passed": True, "checks": checks}


def write(path: Path, doc: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc), encoding="utf-8")


def write_modules(out: Path) -> None:
    for name in g.MODULES:
        extra = ("encryption",) if name in ("http1", "http2") else ()
        write(
            out / "modules" / name / f"{name}.realism.json",
            realism_report(name, extra=extra),
        )


def mix_realism(delta: float = 0.5) -> dict[str, Any]:
    rows = [
        {
            "application": "http",
            "configured_pct": 10.0,
            "metrics_pct": 10.0 + delta,
            "ndpi_pct": 10.0 - delta,
            "metrics_delta_pp": delta,
            "ndpi_delta_pp": -delta,
            "ndpi_count": 1000,
        },
        {
            "application": "dns",
            "configured_pct": 0.5,
            "metrics_pct": 0.5,
            "ndpi_pct": 0.5,
            "metrics_delta_pp": 0.0,
            "ndpi_delta_pp": 0.0,
            "ndpi_count": 5,
        },
    ]
    failed = delta > 2
    report = realism_report("http")
    report["checks"].append(
        {
            "check": "mix_accuracy",
            "passed": not failed,
            "tolerance_points": 2.0,
            "test_config": {"file": "test-config.yaml", "basis": "bytes"},
            "failures": ["too far"] if failed else [],
            "bases": [
                {
                    "basis": "bytes",
                    "applications": rows,
                    "unmatched": {"ndpi_pct": 0.1},
                }
            ],
        }
    )
    return report


def write_mix(out: Path, delta: float = 0.5, verdict: bool = True) -> None:
    write(
        out / "mix" / "run-report.json",
        {
            "run": {"state": "completed", "verdict": {"passed": verdict}},
            "scenarios": [
                {"application": m, "scenario": "s", "sessions_attempted": 12}
                for m in g.MODULES
            ],
        },
    )
    write(out / "mix" / "realism.json", mix_realism(delta))
    write(out / "mix" / "context.json", {"rate_mbps": 4})


def write_image(out: Path, result: str = "PASS (43 checks)", ok: bool = True) -> None:
    image = out / "image"
    image.mkdir(parents=True, exist_ok=True)
    lines = ["PASS unit active", "PASS run: completed"]
    if not ok:
        lines.append("FAIL agent online")
    (image / "boot-test.log").write_text("\n".join(lines) + f"\nRESULT: {result}\n")
    write(
        image / "run" / "run-summary.json",
        {"state": "completed", "verdict": {"passed": True}},
    )
    write(
        image / "run" / "run-metrics.json",
        {
            "samples": [
                {"key": "http1", "values": {"sessions.succeeded": 40}},
                {"key": "dns", "values": {"sessions.succeeded": 25}},
            ]
        },
    )
    write(image / "context.json", {"accelerator": "kvm"})


def full(tmp_path: Path) -> Path:
    write_modules(tmp_path)
    write_mix(tmp_path)
    write_image(tmp_path)
    return tmp_path


def run(out: Path, *args: str) -> int:
    return g.main(["--out-dir", str(out), *args])


def test_full_pass_writes_report_and_summary(tmp_path: Path) -> None:
    out = full(tmp_path)
    assert run(out) == 0
    summary = json.loads((out / "summary.json").read_text())
    assert summary["passed"] is True
    assert summary["partial"] is False
    report = (out / "report.md").read_text()
    assert "**Verdict: PASS**" in report
    assert "Application SIP with RTP" in report
    assert "Byte shares" in report


def test_fifteen_applications_are_checked(tmp_path: Path) -> None:
    checks, _ = g.modules_stage(full(tmp_path), 0.99, 10)
    names = [c.name for c in checks if c.name.startswith("Application ")]
    assert len(names) == 15
    assert len(g.MODULES) == 14


def test_missing_module_report_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    (out / "modules" / "ssh" / "ssh.realism.json").unlink()
    assert run(out) == 1
    summary = json.loads((out / "summary.json").read_text())
    failed = {c["name"] for c in summary["checks"] if not c["passed"]}
    assert "Module ssh realism report" in failed
    assert "Application SSH" in failed


def test_failed_check_in_a_module_report_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    path = out / "modules" / "dns" / "dns.realism.json"
    report = json.loads(path.read_text())
    report["checks"][0]["passed"] = False
    report["passed"] = False
    write(path, report)
    assert run(out) == 1


def test_low_conformance_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    path = out / "modules" / "ntp" / "ntp.realism.json"
    report = json.loads(path.read_text())
    app = report["checks"][0]["applications"][0]
    app["conforming"], app["ratio"] = 38, 0.95
    write(path, report)
    assert run(out) == 1


def test_raised_threshold_fails_a_passing_run(tmp_path: Path) -> None:
    out = full(tmp_path)
    assert run(out) == 0
    assert run(out, "--threshold", "1.001", "--no-write") == 1


def test_skipped_parser_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    path = out / "modules" / "ldap" / "ldap.realism.json"
    report = json.loads(path.read_text())
    parsers = report["checks"][1]
    parsers["parsers"] = [PARSERS[0]]
    parsers["skipped_parsers"] = [{"parser": "zeek", "reason": "not installed"}]
    write(path, report)
    assert run(out) == 1


def test_missing_encryption_check_fails_https(tmp_path: Path) -> None:
    out = full(tmp_path)
    path = out / "modules" / "http2" / "http2.realism.json"
    report = json.loads(path.read_text())
    report["checks"] = [c for c in report["checks"] if c["check"] != "encryption"]
    write(path, report)
    checks, _ = g.modules_stage(out, 0.99, 10)
    https = next(c for c in checks if c.name == "Application HTTPS")
    assert not https.passed
    assert "http2 has no encryption check" in https.measured


def test_too_few_sessions_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    write(
        out / "modules" / "mqtt" / "mqtt.realism.json",
        realism_report("mqtt", sessions=3),
    )
    assert run(out) == 1


def test_mix_beyond_tolerance_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    write_mix(out, delta=2.5)
    assert run(out) == 1
    summary = json.loads((out / "summary.json").read_text())
    failed = {c["name"] for c in summary["checks"] if not c["passed"]}
    assert "Mix accuracy from the run metrics" in failed
    assert "Mix accuracy from nDPI" in failed


def test_mix_at_the_tolerance_passes(tmp_path: Path) -> None:
    out = full(tmp_path)
    write_mix(out, delta=2.0)
    assert run(out) == 0


def test_mix_without_a_session_of_a_module_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    report = json.loads((out / "mix" / "run-report.json").read_text())
    report["scenarios"] = [s for s in report["scenarios"] if s["application"] != "hls"]
    write(out / "mix" / "run-report.json", report)
    assert run(out) == 1


def test_mix_run_that_did_not_pass_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    write_mix(out, verdict=False)
    assert run(out) == 1


def test_mix_must_use_the_bytes_basis(tmp_path: Path) -> None:
    out = full(tmp_path)
    report = json.loads((out / "mix" / "realism.json").read_text())
    report["checks"][-1]["test_config"]["basis"] = "sessions"
    write(out / "mix" / "realism.json", report)
    assert run(out) == 1


def test_absent_stage_fails_instead_of_passing(tmp_path: Path) -> None:
    write_modules(tmp_path)
    assert run(tmp_path) == 1
    summary = json.loads((tmp_path / "summary.json").read_text())
    failed = {c["name"] for c in summary["checks"] if not c["passed"]}
    assert {"Mix run files", "Image boot test log"} <= failed


def test_partial_report_is_marked(tmp_path: Path) -> None:
    write_modules(tmp_path)
    assert run(tmp_path, "--stages", "modules") == 0
    summary = json.loads((tmp_path / "summary.json").read_text())
    assert summary["partial"] is True
    assert "partial, stages modules only" in (tmp_path / "report.md").read_text()


def test_image_boot_failure_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    write_image(out, result="FAIL (1 of 43 checks failed)", ok=False)
    assert run(out) == 1


def test_image_run_without_sessions_fails(tmp_path: Path) -> None:
    out = full(tmp_path)
    write(
        out / "image" / "run" / "run-metrics.json",
        {"samples": [{"key": "http1", "values": {"sessions.succeeded": 0}}]},
    )
    assert run(out) == 1


def test_parse_boot_log() -> None:
    parsed = g.parse_boot_log(
        "PASS a\nFAIL b\nNOTE c\nRESULT: FAIL (1 of 2 checks failed)\n"
    )
    assert parsed == {
        "pass_lines": 1,
        "fail_lines": 1,
        "result": "FAIL",
        "result_detail": "(1 of 2 checks failed)",
    }


def test_unreadable_json_exits_with_2(tmp_path: Path) -> None:
    out = full(tmp_path)
    (out / "mix" / "realism.json").write_text("{not json")
    assert run(out) == 2


def test_unknown_stage_exits_with_2(tmp_path: Path) -> None:
    assert run(tmp_path, "--stages", "bogus") == 2


def test_raised_threshold_is_above_the_lowest_ratio(tmp_path: Path) -> None:
    out = full(tmp_path)
    report = copy.deepcopy(realism_report("dns"))
    report["checks"][0]["applications"][0]["ratio"] = 0.995
    write(out / "modules" / "dns" / "dns.realism.json", report)
    assert g.lowest_ratio(out) == 0.995
    assert g.raised_threshold([0.995]) == 0.996


def write_waived_mix(out: Path, waived: list[str]) -> None:
    write_mix(out)
    write(out / "mix" / "context.json", {"waived": " ".join(waived) or "none"})
    report = json.loads((out / "mix" / "run-report.json").read_text())
    report["scenarios"] = [
        s for s in report["scenarios"] if s["application"] not in waived
    ]
    write(out / "mix" / "run-report.json", report)


def test_waived_modules_fail_unless_allowed(tmp_path: Path) -> None:
    out = full(tmp_path)
    waived = ["hls", "ldap", "mqtt", "ntp"]
    write_waived_mix(out, waived)
    mix = [m for m in g.MODULES if m not in waived]
    base = ["--mix-modules", ",".join(mix)]
    assert run(out, *base) == 1
    assert run(out, *base, "--allow-waivers") == 0
    report = (out / "report.md").read_text()
    assert "**Verdict: PASS WITH WAIVERS**" in report
    assert "WAIVED" in report
    summary = json.loads((out / "summary.json").read_text())
    assert summary["facts"]["mix"]["waived"] == waived


def test_a_module_missing_from_the_run_fails_even_with_waivers(
    tmp_path: Path,
) -> None:
    out = full(tmp_path)
    write_waived_mix(out, ["hls"])
    # hls is not in the module list of this report, but ntp is and has sessions;
    # drop the sessions of ntp to show that an unwaived module still has to run.
    report = json.loads((out / "mix" / "run-report.json").read_text())
    report["scenarios"] = [s for s in report["scenarios"] if s["application"] != "ntp"]
    write(out / "mix" / "run-report.json", report)
    mix = [m for m in g.MODULES if m != "hls"]
    assert run(out, "--mix-modules", ",".join(mix), "--allow-waivers") == 1


def test_the_session_basis_fails_unless_waived(tmp_path: Path) -> None:
    out = full(tmp_path)
    write(out / "mix" / "context.json", {"basis": "sessions"})
    assert run(out) == 1
    assert run(out, "--allow-waivers") == 0
    report = (out / "report.md").read_text()
    assert "**Verdict: PASS WITH WAIVERS**" in report
    assert "expected_bytes" in report
