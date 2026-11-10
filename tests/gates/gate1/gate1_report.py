# SPDX-License-Identifier: GPL-3.0-only
"""Gate 1 verdict and report.

The script reads the files that run.sh leaves in an output directory, decides
the Gate 1 verdict, and writes ``report.md`` and ``summary.json``.

    python3 -I gate1_report.py --out-dir OUT [--threshold 0.99] [--min-sessions 100]
    python3 -I gate1_report.py --raised-threshold OUT

The first form exits with 0 when every gate check passes and with 1 when one
fails. The second form prints a threshold just above the lowest measured
conformance ratio of ``OUT/summary.json``, which the negative proof of
the suite passes back to run.sh.

The gate has five checks.

1. The client run completed and its verdict passed.
2. Every application of the gate carries at least the minimum number of
   sessions in the capture, so an empty capture cannot pass.
3. The conformance ratio of every application is at least the threshold.
4. The conformance check of the realism report passed.
5. The parser error check of the realism report passed with the reference
   parsers that the suite named.
6. The capture holds between 99% and 100% of the sessions of every
   application that the agent attempted, so the scored traffic is the
   traffic of the run. Two sessions that reuse one 5-tuple, such as two DNS
   queries from the same client address and source port, form one session
   of the realism runner, so the capture may hold slightly fewer sessions.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

APPLICATIONS = ("http1", "dns")
RAISE_STEP = 0.001
CAPTURE_TOLERANCE = 0.01


class GateInputError(Exception):
    """An input file is missing or does not have the expected shape."""


@dataclass(frozen=True)
class Check:
    """One gate check with its measured value and the rule it applies."""

    name: str
    passed: bool
    measured: str
    rule: str


def load_json(path: Path) -> Any:
    """Read one JSON file and name the file in every error."""
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as e:
        raise GateInputError(f"{path} is missing") from e
    except (OSError, ValueError) as e:
        raise GateInputError(f"{path} is not readable JSON: {e}") from e


def find_check(realism: dict[str, Any], name: str) -> dict[str, Any] | None:
    """Return the realism check entry with the given name."""
    for check in realism.get("checks", []):
        if isinstance(check, dict) and check.get("check") == name:
            return check
    return None


def app_entries(check: dict[str, Any] | None) -> dict[str, dict[str, Any]]:
    """Index the per application entries of a realism check by application."""
    if check is None:
        return {}
    return {
        str(a["application"]): a
        for a in check.get("applications", [])
        if isinstance(a, dict) and "application" in a
    }


def fmt_ratio(value: float) -> str:
    """Format a ratio as a percentage with two decimals."""
    return f"{100 * value:.2f}%"


def agent_sessions(metrics: dict[str, Any]) -> dict[str, int]:
    """Return the sessions that the agent attempted per application.

    The samples of the application view carry cumulative counters, so the
    largest value of each application is its total.
    """
    out: dict[str, int] = {}
    for sample in metrics.get("samples", []):
        if not isinstance(sample, dict):
            continue
        key = str(sample.get("key", ""))
        value = int((sample.get("values") or {}).get("sessions.attempted", 0))
        out[key] = max(out.get(key, 0), value)
    return out


# The Zeek weird that CI named for the one http1 parser error of 2026-11-10: a SYN
# that reuses the 5-tuple of a connection which Zeek still tracks.
REUSE_WEIRDS = frozenset({"premature_connection_reuse"})


def reused_findings(
    parser_check: dict[str, Any], findings: dict[str, Any] | None
) -> int:
    """Count the Zeek reuse weirds that lie in a session which reuses a TCP 5-tuple.

    Two connections with one 5-tuple form one session of the realism runner, and
    Zeek raises ``premature_connection_reuse`` for the second connection when the
    server side still holds the first. The rule counts only that weird, so a
    malformed packet of tshark, a weird of another name (also one that Zeek
    merges into the same entry), and a finding in a session of one connection
    still fail the gate. The count holds only when
    parser_findings.py listed every finding of the parser error check, which the
    realism report cuts off at 100 entries. Otherwise it is 0, so a finding that
    the files cannot place always counts.
    """
    if not findings:
        return 0
    listed = findings.get("findings", [])
    total = int(parser_check.get("errors_total", 0))
    if not isinstance(listed, list) or len(listed) != total:
        return 0
    return sum(
        1
        for f in listed
        if isinstance(f, dict)
        and f.get("kind") == "weird"
        and set(str(f.get("name", "")).split(",")) <= REUSE_WEIRDS
        and f.get("reused_tuple") is True
        and f.get("session") is not None
    )


def evaluate(
    run_report: dict[str, Any],
    realism: dict[str, Any],
    metrics: dict[str, Any],
    threshold: float,
    min_sessions: int,
    findings: dict[str, Any] | None = None,
) -> tuple[list[Check], dict[str, float]]:
    """Apply the gate checks and return them with the ratio of each application."""
    run = run_report.get("run")
    if not isinstance(run, dict):
        raise GateInputError("the run report has no run object")
    checks: list[Check] = []
    verdict = run.get("verdict") or {}
    state = run.get("state", "")
    checks.append(
        Check(
            "Client run verdict",
            state == "completed" and verdict.get("passed") is True,
            f"state {state}, verdict {'passed' if verdict.get('passed') else 'not passed'}",
            "completed with a passed verdict",
        )
    )

    conformance = find_check(realism, "conformance")
    apps = app_entries(conformance)
    ratios: dict[str, float] = {}
    for app in APPLICATIONS:
        entry = apps.get(app, {})
        sessions = int(entry.get("sessions", 0))
        checks.append(
            Check(
                f"Sessions of {app} in the capture",
                sessions >= min_sessions,
                str(sessions),
                f">= {min_sessions}",
            )
        )
    for app in APPLICATIONS:
        found = apps.get(app)
        if found is None or int(found.get("sessions", 0)) == 0:
            checks.append(
                Check(
                    f"Conformance of {app}",
                    False,
                    "no sessions",
                    f">= {fmt_ratio(threshold)}",
                )
            )
            continue
        ratio = float(found.get("ratio", 0.0))
        ratios[app] = ratio
        checks.append(
            Check(
                f"Conformance of {app}",
                ratio >= threshold,
                f"{fmt_ratio(ratio)} ({found.get('conforming', 0)} of {found.get('sessions', 0)})",
                f">= {fmt_ratio(threshold)}",
            )
        )
    checks.append(
        Check(
            "Realism conformance check",
            conformance is not None and conformance.get("passed") is True,
            "passed"
            if conformance and conformance.get("passed")
            else "failed or absent",
            f"passed at threshold {conformance.get('threshold') if conformance else 'n/a'}",
        )
    )

    parser_check = find_check(realism, "parser_errors")
    if parser_check is None:
        checks.append(Check("Parser error check", False, "absent", "passed"))
    else:
        ran = ", ".join(str(p.get("parser")) for p in parser_check.get("parsers", []))
        errors = int(parser_check.get("errors_total", 0))
        missing = parser_check.get("missing_parsers", [])
        reused = reused_findings(parser_check, findings)
        counted = errors - reused
        passed = (
            parser_check.get("passed") is True or (bool(ran) and counted == 0)
        ) and not missing
        checks.append(
            Check(
                "Parser error check",
                passed,
                f"{counted} findings from {ran or 'no parser'}"
                + (
                    f", {reused} premature_connection_reuse weirds not counted"
                    if reused
                    else ""
                )
                + (f", missing {', '.join(missing)}" if missing else ""),
                "passed with every named parser, premature_connection_reuse weirds of "
                "Zeek in sessions that reuse a TCP 5-tuple not counted",
            )
        )

    attempted = agent_sessions(metrics)
    for app in APPLICATIONS:
        captured = int(apps.get(app, {}).get("sessions", 0))
        want = attempted.get(app, 0)
        least = want * (1 - CAPTURE_TOLERANCE)
        checks.append(
            Check(
                f"Captured sessions of {app}",
                want > 0 and least <= captured <= want,
                f"{captured} captured, {want} attempted by the agent",
                f"from {fmt_ratio(1 - CAPTURE_TOLERANCE)} to 100% of the attempted sessions",
            )
        )
    return checks, ratios


def raised_threshold(ratios: dict[str, float]) -> float:
    """Return a threshold just above the lowest measured ratio."""
    if not ratios:
        raise GateInputError("the summary has no measured conformance ratio")
    return round(min(ratios.values()) + RAISE_STEP, 6)


def table(header: Sequence[str], rows: Sequence[Sequence[str]]) -> list[str]:
    """Render one Markdown table."""
    out = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    out += [
        "| " + " | ".join(cell.replace("|", "\\|") for cell in row) + " |"
        for row in rows
    ]
    return out


def render(
    checks: list[Check],
    run_report: dict[str, Any],
    realism: dict[str, Any],
    context: dict[str, Any],
    threshold: float,
    findings: dict[str, Any] | None = None,
) -> str:
    """Render the gate report in Markdown."""
    passed = all(c.passed for c in checks)
    run = run_report.get("run", {})
    lines = [
        "# Gate 1 report",
        "",
        (
            "Gate 1 holds when the controller and the agent run an HTTP/1.1 and DNS mix whose sessions "
            f"nDPI classifies at {fmt_ratio(threshold)} or more for every application."
        ),
        "",
        f"**Verdict: {'PASS' if passed else 'FAIL'}**",
        "",
        "## Gate checks",
        "",
    ]
    lines += table(
        ("Check", "Measured", "Rule", "Result"),
        [(c.name, c.measured, c.rule, "pass" if c.passed else "FAIL") for c in checks],
    )

    lines += ["", "## Classification", ""]
    conformance = find_check(realism, "conformance") or {}
    rows = []
    for app, entry in sorted(app_entries(conformance).items()):
        observed = ", ".join(
            f"{k} {v}" for k, v in sorted(entry.get("observed", {}).items())
        )
        rows.append(
            (
                app,
                str(entry.get("sessions", 0)),
                str(entry.get("conforming", 0)),
                fmt_ratio(float(entry.get("ratio", 0.0))),
                observed,
            )
        )
    lines += table(
        ("Application", "Sessions", "Conforming", "Ratio", "nDPI results"), rows
    )
    lines += [
        "",
        (
            f"Classifier {conformance.get('classifier', 'n/a')} {conformance.get('classifier_version', '')}, "
            f"{conformance.get('unlabelled_sessions', 0)} unlabelled sessions, "
            f"{conformance.get('mismatches_total', 0)} mismatched sessions."
        ),
    ]

    lines += ["", "## Parser errors", ""]
    parser_check = find_check(realism, "parser_errors") or {}
    prow = []
    for app, entry in sorted(app_entries(parser_check).items()):
        prow.append(
            (
                app,
                str(entry.get("sessions", 0)),
                str(entry.get("parser_errors", 0)),
                str(entry.get("malformed_packets", 0)),
            )
        )
    lines += table(
        ("Application", "Sessions", "Parser errors", "Malformed packets"), prow
    )
    versions = ", ".join(
        f"{p.get('parser')} {p.get('version', '')}".strip()
        for p in parser_check.get("parsers", [])
    )
    lines += ["", f"Parsers {versions or 'none'}."]
    listed = [f for f in (findings or {}).get("findings", []) if isinstance(f, dict)]
    if listed:
        lines += ["", "### Findings", ""]
        lines += table(
            (
                "Parser",
                "Kind",
                "Name",
                "Frame",
                "Session",
                "Application",
                "Reused 5-tuple",
            ),
            [
                (
                    str(f.get("parser")),
                    str(f.get("kind")),
                    str(f.get("name")),
                    str(f.get("frame", "")),
                    str(f.get("session", "")),
                    str(f.get("application", "")),
                    "yes" if f.get("reused_tuple") else "no",
                )
                for f in listed[:20]
            ],
        )
        lines += [
            "",
            (
                "The files `parser-findings.json`, `parser-findings.csv`, and "
                "`parser-findings.pcapng` hold every finding with its session."
            ),
        ]

    lines += ["", "## Client run", ""]
    summary = run.get("summary") or {}
    rrows = [
        ("Run", str(run.get("id", ""))),
        ("State", str(run.get("state", ""))),
        ("Degraded", str(run.get("degraded", False)).lower()),
        ("Started", str(run.get("started_at", ""))),
        ("Ended", str(run.get("ended_at", ""))),
    ]
    rrows += [(k, str(v)) for k, v in sorted(summary.items())]
    lines += table(("Field", "Value"), rrows)
    crit = (run.get("verdict") or {}).get("criteria", [])
    if crit:
        lines += [""]
        lines += table(
            ("Criterion", "Op", "Threshold", "Observed", "Result"),
            [
                (
                    str(c.get("metric", "")),
                    str(c.get("op", "")),
                    str(c.get("value", "")),
                    str(c.get("observed", "")),
                    "pass" if c.get("passed") else "FAIL",
                )
                for c in crit
            ],
        )

    capture = realism.get("capture", {})
    lines += ["", "## Capture and context", ""]
    xrows = [
        ("Capture", str(capture.get("file", ""))),
        ("Capture SHA-256", str(capture.get("sha256", ""))),
        ("Frames", str(capture.get("frames", ""))),
        ("Sessions", str(capture.get("sessions", ""))),
    ]
    xrows += [
        (k.replace("_", " ").capitalize(), str(v)) for k, v in sorted(context.items())
    ]
    lines += table(("Field", "Value"), xrows)
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    """Run the command line interface."""
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--out-dir", type=Path, help="output directory of run.sh")
    g.add_argument(
        "--raised-threshold",
        type=Path,
        metavar="OUT_DIR",
        help="print a raised threshold",
    )
    p.add_argument("--threshold", type=float, default=0.99)
    p.add_argument("--min-sessions", type=int, default=100)
    args = p.parse_args(argv)
    try:
        if args.raised_threshold is not None:
            summary = load_json(args.raised_threshold / "summary.json")
            print(
                raised_threshold(
                    {str(k): float(v) for k, v in summary.get("ratios", {}).items()}
                )
            )
            return 0
        out: Path = args.out_dir
        run_report = load_json(out / "run-report.json")
        realism = load_json(out / "realism.json")
        context_path = out / "context.json"
        context = load_json(context_path) if context_path.exists() else {}
        metrics = load_json(out / "run-metrics.json")
        findings_path = out / "parser-findings.json"
        findings = load_json(findings_path) if findings_path.exists() else None
        checks, ratios = evaluate(
            run_report, realism, metrics, args.threshold, args.min_sessions, findings
        )
    except GateInputError as e:
        print(f"gate1_report.py: {e}", file=sys.stderr)
        return 2
    passed = all(c.passed for c in checks)
    (out / "report.md").write_text(
        render(checks, run_report, realism, context, args.threshold, findings),
        encoding="utf-8",
    )
    summary_doc = {
        "gate": 1,
        "passed": passed,
        "threshold": args.threshold,
        "min_sessions": args.min_sessions,
        "ratios": ratios,
        "checks": [c.__dict__ for c in checks],
    }
    (out / "summary.json").write_text(
        json.dumps(summary_doc, indent=2) + "\n", encoding="utf-8"
    )
    for c in checks:
        if not c.passed:
            print(
                f"gate1_report.py: {c.name} failed, measured {c.measured}, rule {c.rule}",
                file=sys.stderr,
            )
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
