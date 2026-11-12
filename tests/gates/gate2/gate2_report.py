# SPDX-License-Identifier: GPL-3.0-only
"""Gate 2 verdict and report.

The script reads the files that the three stages of the suite leave in an
output directory, decides the Gate 2 verdict, and writes ``report.md`` and
``summary.json``.

    python3 -I gate2_report.py --out-dir OUT [--stages modules,mix,image]
    python3 -I gate2_report.py --out-dir OUT --report-dir DIR

Gate 2 holds when three things hold. All 15 application modules of decision D7
pass the realism checks of module CI, a byte weighted enterprise mix of the
15 applications stays within the tolerance of its weights through the
controller and the agent, and the qcow2 image boots and runs a test unattended.

The output directory has one directory per stage.

    modules/NAME/NAME.realism.json   realism/ci/module-realism.sh, per module
    mix/run-report.json              GET /v1/runs/{id}/report of the client run
    mix/run.json                     GET /v1/runs/{id} of the client run (Run.calibration)
    mix/realism.json                 the realism report of the mix capture
    mix/context.json                 settings and facts of the mix run
    image/boot-test.log              image/boot-test.sh --run
    image/run/run-summary.json       the short run inside the VM
    image/run/run-metrics.json       its application metrics
    image/context.json               accelerator and image facts

The script exits with 0 when every check of every selected stage passes, with
1 when a check fails, and with 2 when an input file is unreadable. A stage that
is selected and has no files fails its checks, so a stage cannot pass by
being absent. ``--stages`` selects a subset, and the report then says that it
is partial.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections.abc import Sequence
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

STAGES = ("modules", "mix", "image")

# The 14 native modules that carry the 15 applications of decision D7. HTTPS
# runs in the http1 and http2 modules, so it has no module of its own.
MODULES = (
    "dns",
    "ftp",
    "hls",
    "http1",
    "http2",
    "imap",
    "ldap",
    "mqtt",
    "ntp",
    "pop3",
    "sip_rtp",
    "smb2",
    "smtp",
    "ssh",
)

# Application of D7, the module that carries it, and the realism check that
# shows it. HTTPS needs the encryption check of both modules that run TLS.
APPLICATIONS: tuple[tuple[str, tuple[str, ...], str], ...] = (
    ("HTTP/1.1", ("http1",), "conformance"),
    ("HTTP/2", ("http2",), "conformance"),
    ("HTTPS", ("http1", "http2"), "encryption"),
    ("DNS", ("dns",), "conformance"),
    ("SMTP", ("smtp",), "conformance"),
    ("IMAP", ("imap",), "conformance"),
    ("POP3", ("pop3",), "conformance"),
    ("FTP", ("ftp",), "conformance"),
    ("SSH", ("ssh",), "conformance"),
    ("SMB2", ("smb2",), "conformance"),
    ("SIP with RTP", ("sip_rtp",), "conformance"),
    ("HLS video", ("hls",), "conformance"),
    ("NTP", ("ntp",), "conformance"),
    ("LDAP", ("ldap",), "conformance"),
    ("MQTT", ("mqtt",), "conformance"),
)

# Checks that every module report must carry.
REQUIRED_CHECKS = ("conformance", "parser_errors")
REQUIRED_PARSERS = ("tshark", "zeek")
DEFAULT_THRESHOLD = 0.99
DEFAULT_TOLERANCE = 2.0
# Sessions of each module that its realism capture must hold.
DEFAULT_MIN_MODULE_SESSIONS = 10
RAISE_STEP = 0.001
WAIVER_CHECK = "Modules waived from the mix"


class GateInputError(Exception):
    """An input file exists and cannot be read."""


@dataclass(frozen=True)
class Check:
    """One gate check with its measured value and the rule it applies."""

    stage: str
    name: str
    passed: bool
    measured: str
    rule: str


def load_json(path: Path) -> Any:
    """Read one JSON file and name the file in every error."""
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise GateInputError(f"{path} is not readable JSON: {e}") from e


def load_optional(path: Path) -> Any | None:
    """Read a JSON file, or return None when the file does not exist."""
    if not path.is_file():
        return None
    return load_json(path)


def fmt_ratio(value: float) -> str:
    """Format a ratio as a percentage with two decimals."""
    return f"{100 * value:.2f}%"


def find_check(report: dict[str, Any], name: str) -> dict[str, Any] | None:
    """Return the check entry of a realism report with the given name."""
    for check in report.get("checks", []):
        if isinstance(check, dict) and check.get("check") == name:
            return check
    return None


def app_entries(check: dict[str, Any] | None) -> list[dict[str, Any]]:
    """Return the per application entries of a realism check."""
    if check is None:
        return []
    return [a for a in check.get("applications", []) if isinstance(a, dict)]


def describe_finding(entry: dict[str, Any]) -> str:
    """Describe one parser finding of a realism report on one line.

    The line names the parser, the kind, the protocol layer, the message, the frame and
    the time of the packet, the session with its application, and the endpoints, so a
    failing job log shows what each parser reported without the run artifacts.
    """
    head = f"{entry.get('parser')} {entry.get('kind')} {entry.get('name')}"
    message = str(entry.get("message") or "").strip()
    if message:
        head += f": {message}"
    parts = [head]
    frame = entry.get("frame")
    stamp = entry.get("timestamp")
    when = ""
    if isinstance(stamp, int | float) and not isinstance(stamp, bool):
        try:
            when = datetime.fromtimestamp(float(stamp), timezone.utc).strftime(
                "%Y-%m-%dT%H:%M:%S.%fZ"
            )
        except (OverflowError, OSError, ValueError):
            when = ""
    if frame is not None and when:
        parts.append(f"frame {frame} at {when}")
    elif frame is not None:
        parts.append(f"frame {frame}")
    elif when:
        parts.append(f"at {when}")
    owner = []
    if entry.get("application"):
        owner.append(f"application {entry['application']}")
    elif entry.get("session") is None:
        owner.append("no session")
    if entry.get("session") is not None:
        owner.append(f"session {entry['session']}")
    if owner:
        parts.append(", ".join(owner))
    client, server = entry.get("client"), entry.get("server")
    if isinstance(client, dict) and isinstance(server, dict):
        parts.append(
            f"{entry.get('transport') or 'ip'} "
            f"{client.get('ip')}:{client.get('port')} -> "
            f"{server.get('ip')}:{server.get('port')}"
        )
    return "; ".join(parts)


def finding_facts(
    report: dict[str, Any], marks: Sequence[str | None] = ()
) -> dict[str, Any]:
    """Return the parser findings of a realism report for the gate report.

    ``total`` is the count of every parser. ``lines`` describes the findings that the
    realism report lists, which are the first 100 in the order of the parsers, and says
    of each port collision (``marks``, one per finding) that it does not count.
    """
    entry = find_check(report, "parser_errors") or {}
    listed = [e for e in entry.get("errors", []) if isinstance(e, dict)]
    total = int(entry.get("errors_total", 0))
    lines = []
    for index, found in enumerate(listed):
        line = describe_finding(found)
        if index < len(marks) and marks[index]:
            line += f"; port collision with {marks[index]}, not counted"
        lines.append(line)
    return {"total": total, "lines": lines, "unlisted": max(0, total - len(listed))}


def finding_lines(facts: dict[str, Any], heading: str) -> list[str]:
    """Render the findings of one realism report as a Markdown list, or nothing."""
    if not facts or not facts.get("total"):
        return []
    lines = ["", heading, ""]
    lines += [f"- {line}" for line in facts.get("lines", [])]
    if facts.get("unlisted"):
        lines.append(
            f"- and {facts['unlisted']} more that the realism report does not list"
        )
    return lines


def collision_marks(path: Path, errors_total: int) -> list[str | None]:
    """Return the port collision of each finding of the mix, or nothing when it is unproven.

    ``mix_findings.py`` marks a tshark finding with the dissector that the client port of
    its session is bound to (see its documentation). The marks hold only when the file
    belongs to the realism report, which means that it lists every one of its findings:
    with an unlisted finding, or a file of another total, the list is empty and every
    finding counts.
    """
    doc = load_optional(path)
    if not isinstance(doc, dict) or int(doc.get("errors_total", -1)) != errors_total:
        return []
    listed = [f for f in doc.get("findings", []) if isinstance(f, dict)]
    if len(listed) != errors_total:
        return []
    return [
        str(f["port_collision"]) if f.get("port_collision") else None for f in listed
    ]


def port_collisions(marks: Sequence[str | None]) -> int:
    """Return the number of findings that are port collisions."""
    return sum(1 for m in marks if m)


def module_checks(
    name: str,
    report: dict[str, Any] | None,
    threshold: float,
    min_sessions: int,
) -> list[Check]:
    """Judge the realism report of one module."""
    stage = "modules"
    label = f"Module {name}"
    if report is None:
        return [
            Check(
                stage,
                f"{label} realism report",
                False,
                "absent",
                f"modules/{name}/{name}.realism.json",
            )
        ]
    out: list[Check] = []
    kinds = [
        str(c.get("check")) for c in report.get("checks", []) if isinstance(c, dict)
    ]
    failed = [
        str(c.get("check"))
        for c in report.get("checks", [])
        if isinstance(c, dict) and c.get("passed") is not True
    ]
    missing = [k for k in REQUIRED_CHECKS if k not in kinds]
    out.append(
        Check(
            stage,
            f"{label} realism checks",
            report.get("passed") is True and not failed and not missing,
            f"{len(kinds)} checks ({', '.join(kinds)})"
            + (f", failed {', '.join(failed)}" if failed else "")
            + (f", missing {', '.join(missing)}" if missing else ""),
            "every check passed, and conformance and parser_errors ran",
        )
    )
    apps = app_entries(find_check(report, "conformance"))
    sessions = sum(int(a.get("sessions", 0)) for a in apps)
    ratios = [float(a.get("ratio", 0.0)) for a in apps if int(a.get("sessions", 0)) > 0]
    lowest = min(ratios) if ratios else 0.0
    out.append(
        Check(
            stage,
            f"{label} conformance",
            bool(ratios) and lowest >= threshold and sessions >= min_sessions,
            f"lowest ratio {fmt_ratio(lowest)} over {sessions} sessions"
            if ratios
            else "no scored session",
            f">= {fmt_ratio(threshold)} with at least {min_sessions} sessions",
        )
    )
    parsers = find_check(report, "parser_errors") or {}
    ran = [
        str(p.get("parser")) for p in parsers.get("parsers", []) if isinstance(p, dict)
    ]
    skipped = [
        str(s.get("parser"))
        for s in parsers.get("skipped_parsers", [])
        if isinstance(s, dict)
    ]
    skipped += [
        str(m) for m in parsers.get("missing_parsers", []) if str(m) not in skipped
    ]
    errors = int(parsers.get("errors_total", 0))
    out.append(
        Check(
            stage,
            f"{label} parser errors",
            not skipped
            and errors == 0
            and all(p in ran for p in REQUIRED_PARSERS)
            and parsers.get("passed") is True,
            f"{errors} findings from {', '.join(ran) or 'no parser'}"
            + (f", skipped {', '.join(skipped)}" if skipped else ""),
            f"0 findings from {' and '.join(REQUIRED_PARSERS)}, none skipped",
        )
    )
    return out


def application_checks(reports: dict[str, dict[str, Any] | None]) -> list[Check]:
    """Check that each of the 15 applications has its evidence."""
    out: list[Check] = []
    for application, modules, kind in APPLICATIONS:
        problems: list[str] = []
        for module in modules:
            report = reports.get(module)
            entry = find_check(report, kind) if report is not None else None
            if entry is None:
                problems.append(f"{module} has no {kind} check")
            elif entry.get("passed") is not True:
                problems.append(f"{module} {kind} failed")
        detail = ""
        if not problems and kind == "encryption":
            parts = []
            for module in modules:
                entry = find_check(reports[module] or {}, "encryption") or {}
                parts.append(
                    f"{module} {entry.get('decrypted_sessions', 0)} of "
                    f"{entry.get('sampled_sessions', 0)} decrypted, "
                    f"{entry.get('ja4_matched_sessions', 0)} JA4 matches"
                )
            detail = "; ".join(parts)
        out.append(
            Check(
                "modules",
                f"Application {application}",
                not problems,
                "; ".join(problems)
                if problems
                else (detail or f"{'+'.join(modules)} {kind} passed"),
                f"the {kind} check of {' and '.join(modules)} passed",
            )
        )
    return out


def modules_stage(
    out_dir: Path, threshold: float, min_sessions: int
) -> tuple[list[Check], dict[str, Any]]:
    """Judge the module stage."""
    reports: dict[str, dict[str, Any] | None] = {}
    for name in MODULES:
        loaded = load_optional(out_dir / "modules" / name / f"{name}.realism.json")
        if loaded is not None and not isinstance(loaded, dict):
            raise GateInputError(f"modules/{name}/{name}.realism.json is not an object")
        reports[name] = loaded
    checks: list[Check] = []
    for name in MODULES:
        checks += module_checks(name, reports[name], threshold, min_sessions)
    checks += application_checks(reports)
    facts: dict[str, Any] = {}
    for name, report in reports.items():
        if report is None:
            continue
        apps = app_entries(find_check(report, "conformance"))
        facts[name] = {
            "checks": [
                c.get("check") for c in report.get("checks", []) if isinstance(c, dict)
            ],
            "applications": {
                str(a.get("application")): {
                    "sessions": int(a.get("sessions", 0)),
                    "ratio": float(a.get("ratio", 0.0)),
                }
                for a in apps
            },
            "findings": finding_facts(report),
        }
    return checks, facts


def mix_rows(entry: dict[str, Any], basis: str) -> list[dict[str, Any]]:
    """Return the application rows of one basis of a mix accuracy entry."""
    for item in entry.get("bases", []):
        if isinstance(item, dict) and item.get("basis") == basis:
            return [r for r in item.get("applications", []) if isinstance(r, dict)]
    return []


def worst_delta(rows: Sequence[dict[str, Any]], key: str) -> float:
    """Return the largest absolute delta of a column, or infinity when a row has none."""
    worst = 0.0
    for row in rows:
        value = row.get(key)
        if not isinstance(value, int | float):
            return float("inf")
        worst = max(worst, abs(float(value)))
    return worst


def calibration_facts(out_dir: Path, run: dict[str, Any]) -> dict[str, Any]:
    """Return the last calibration step of the client run and where it came from.

    ``mix/run.json`` holds ``GET /v1/runs/{id}``. An older output directory has
    none, and the run object of the report carries the same ``calibration``.
    The steps are oldest first, so the last one is the state at the end of the
    run. A run without steps has none: the controller records them only for a
    ``throughput_bps`` objective, and none while the factor stays at one.
    """
    source = "run.json"
    doc = load_optional(out_dir / "mix" / "run.json")
    if doc is None:
        source, doc = "run-report.json", run
    if not isinstance(doc, dict):
        raise GateInputError(f"mix/{source} is not a run object")
    steps = doc.get("calibration")
    if steps is None:
        return {"source": source, "steps": 0, "last": None}
    if not isinstance(steps, list) or not all(isinstance(x, dict) for x in steps):
        raise GateInputError(
            f"mix/{source} has a calibration that is not a list of steps"
        )
    return {"source": source, "steps": len(steps), "last": steps[-1] if steps else None}


def calibration_lines(facts: dict[str, Any], basis: str) -> list[str]:
    """Render the calibration of the mix run as the lines of a Markdown section."""
    lines = ["", "### Calibration", ""]
    last = facts.get("last")
    if not isinstance(last, dict):
        none = (
            f"The client run has no calibration (`{facts.get('source')}` has no `calibration` step). "
            "The controller records a step only for a `throughput_bps` objective and only once "
            "the factor leaves one, so the mix control did not move this run."
        )
        return [*lines, none]

    def num(value: Any) -> str:
        return f"{value:.4g}" if isinstance(value, int | float) else "n/a"

    summary = (
        f"{facts['steps']} steps, the last at {last.get('time', 'n/a')}: factor {num(last.get('factor'))}, "
        f"target {num(last.get('target'))}, {num(last.get('sessions_per_second'))} sessions per second "
        f"(`{facts.get('source')}`)."
    )
    lines += [summary, ""]
    entries = [e for e in last.get("entries") or [] if isinstance(e, dict)]
    multipliers = last.get("multipliers")
    shares = multipliers if isinstance(multipliers, dict) else {}
    lines += table(
        ("Mix entry", "Bytes per session", "Measured", "Multiplier"),
        [
            (
                str(e.get("id")),
                num(e.get("bytes_per_session")),
                "yes" if e.get("measured") is True else "no",
                num(shares.get(e.get("id"))) if shares else "none",
            )
            for e in sorted(entries, key=lambda e: str(e.get("id")))
        ],
    )
    if not shares:
        lines += [
            "",
            f"The last step has no byte mix multipliers, as is the case for a run whose profile basis is not `bytes` (this run: `{basis}`).",
        ]
    return lines


def mix_stage(
    out_dir: Path,
    threshold: float,
    tolerance: float,
    mix_modules: Sequence[str],
    allow_waivers: bool = False,
) -> tuple[list[Check], dict[str, Any]]:
    """Judge the byte weighted enterprise mix."""
    stage = "mix"
    run_report = load_optional(out_dir / "mix" / "run-report.json")
    realism = load_optional(out_dir / "mix" / "realism.json")
    context = load_optional(out_dir / "mix" / "context.json") or {}
    if run_report is None or realism is None:
        absent = [
            f"mix/{n}"
            for n, v in (("run-report.json", run_report), ("realism.json", realism))
            if v is None
        ]
        return [
            Check(
                stage, "Mix run files", False, f"absent: {', '.join(absent)}", "present"
            )
        ], {}
    checks: list[Check] = []
    run = run_report.get("run")
    if not isinstance(run, dict):
        raise GateInputError("mix/run-report.json has no run object")
    verdict = run.get("verdict") or {}
    state = str(run.get("state", ""))
    checks.append(
        Check(
            stage,
            "Mix run verdict",
            state == "completed" and verdict.get("passed") is True,
            f"state {state}, verdict {'passed' if verdict.get('passed') else 'not passed'}",
            "completed with a passed verdict",
        )
    )
    waived = [
        w
        for w in re.split(r"[\s,]+", str(context.get("waived", "none")))
        if w and w != "none"
    ]
    checks.append(
        Check(
            stage,
            WAIVER_CHECK,
            not waived or allow_waivers,
            ", ".join(waived) + " (waived, the agent has no crate for them yet)"
            if waived
            else "none",
            "none, because the agent runs every module of the mix",
        )
    )
    scenarios = [s for s in run_report.get("scenarios", []) if isinstance(s, dict)]
    attempted: dict[str, int] = {}
    for s in scenarios:
        module = str(s.get("application"))
        attempted[module] = attempted.get(module, 0) + int(
            s.get("sessions_attempted", 0)
        )
    missing = [m for m in mix_modules if attempted.get(m, 0) == 0]
    checks.append(
        Check(
            stage,
            "Modules in the mix run",
            not missing,
            ", ".join(f"{m} {attempted.get(m, 0)}" for m in mix_modules)
            if not missing
            else "no session of " + ", ".join(missing),
            f"every one of the {len(mix_modules)} modules attempted a session",
        )
    )
    entry = find_check(realism, "mix_accuracy")
    if entry is None:
        checks.append(
            Check(stage, "Mix accuracy check", False, "absent", "present and passed")
        )
        return checks, {"modules": attempted}
    scored = (entry.get("test_config") or {}).get("basis")
    run_basis = str(context.get("basis", "bytes"))
    basis_waived = run_basis == "sessions"
    checks.append(
        Check(
            stage,
            "Mix basis",
            scored == "bytes"
            and [b.get("basis") for b in entry.get("bases", [])] == ["bytes"]
            and (run_basis == "bytes" or (basis_waived and allow_waivers)),
            f"profile.basis {run_basis}, scored by bytes"
            + (
                " (waived, the controller does not read expected_bytes yet)"
                if basis_waived
                else ""
            ),
            "profile.basis bytes, scored by bytes",
        )
    )
    rows = mix_rows(entry, "bytes")
    metrics_worst = worst_delta(rows, "metrics_delta_pp")
    ndpi_worst = worst_delta(rows, "ndpi_delta_pp")
    unmatched = next(
        (
            b.get("unmatched") or {}
            for b in entry.get("bases", [])
            if b.get("basis") == "bytes"
        ),
        {},
    )
    unmatched_pct = float(unmatched.get("ndpi_pct") or 0.0)
    checks.append(
        Check(
            stage,
            "Mix accuracy from the run metrics",
            bool(rows) and metrics_worst <= tolerance,
            f"largest difference {metrics_worst:.3f} points over {len(rows)} applications",
            f"<= {tolerance} points of the byte weights",
        )
    )
    checks.append(
        Check(
            stage,
            "Mix accuracy from nDPI",
            bool(rows) and ndpi_worst <= tolerance and unmatched_pct <= tolerance,
            f"largest difference {ndpi_worst:.3f} points, unmatched {unmatched_pct:.3f}%",
            f"<= {tolerance} points of the byte weights",
        )
    )
    checks.append(
        Check(
            stage,
            "Realism mix accuracy check",
            entry.get("passed") is True,
            "passed"
            if entry.get("passed")
            else f"failed: {'; '.join(entry.get('failures', [])[:2])}",
            f"passed at {entry.get('tolerance_points')} points",
        )
    )
    conformance = find_check(realism, "conformance")
    apps = app_entries(conformance)
    ratios = [float(a.get("ratio", 0.0)) for a in apps if int(a.get("sessions", 0)) > 0]
    lowest = min(ratios) if ratios else 0.0
    checks.append(
        Check(
            stage,
            "Mix conformance",
            conformance is not None
            and conformance.get("passed") is True
            and bool(ratios)
            and lowest >= threshold,
            f"lowest ratio {fmt_ratio(lowest)} over {len(ratios)} applications"
            if ratios
            else "no scored session",
            f">= {fmt_ratio(threshold)} for every application",
        )
    )
    parsers = find_check(realism, "parser_errors")
    ran = [
        str(p.get("parser"))
        for p in (parsers or {}).get("parsers", [])
        if isinstance(p, dict)
    ]
    errors = int((parsers or {}).get("errors_total", 0))
    marks = collision_marks(out_dir / "mix" / "parser-findings.json", errors)
    collisions = port_collisions(marks)
    checks.append(
        Check(
            stage,
            "Mix parser errors",
            parsers is not None
            and (parsers.get("passed") is True or (errors > 0 and collisions == errors))
            and all(p in ran for p in REQUIRED_PARSERS),
            f"{errors} findings from {', '.join(ran) or 'no parser'}"
            + (f", {collisions} of them port collisions" if collisions else ""),
            f"0 findings from {' and '.join(REQUIRED_PARSERS)}, not counting port collisions",
        )
    )
    facts = {
        "modules": attempted,
        "waived": waived,
        "basis_waived": basis_waived,
        "scenarios": [
            {
                "scenario": f"{x.get('application')}/{x.get('scenario')}",
                "attempted": int(x.get("sessions_attempted", 0)),
                "failed": int(x.get("sessions_failed", 0)),
                "bytes": int(x.get("bytes_tx", 0)) + int(x.get("bytes_rx", 0)),
            }
            for x in scenarios
        ],
        "rows": rows,
        "calibration": calibration_facts(out_dir, run),
        "unmatched_pct": unmatched_pct,
        "findings": finding_facts(realism, marks),
        "metrics_worst_pp": metrics_worst,
        "ndpi_worst_pp": ndpi_worst,
        "context": context,
    }
    return checks, facts


def parse_boot_log(text: str) -> dict[str, Any]:
    """Count the PASS and FAIL lines of the boot test and read its result line."""
    passes = len(re.findall(r"^PASS", text, flags=re.MULTILINE))
    fails = len(re.findall(r"^FAIL", text, flags=re.MULTILINE))
    result = re.findall(r"^RESULT: (PASS|FAIL)(.*)$", text, flags=re.MULTILINE)
    last = result[-1] if result else ("", "")
    return {
        "pass_lines": passes,
        "fail_lines": fails,
        "result": last[0],
        "result_detail": last[1].strip(),
    }


def image_stage(out_dir: Path) -> tuple[list[Check], dict[str, Any]]:
    """Judge the image stage."""
    stage = "image"
    log_path = out_dir / "image" / "boot-test.log"
    if not log_path.is_file():
        return [
            Check(stage, "Image boot test log", False, "absent", "image/boot-test.log")
        ], {}
    try:
        text = log_path.read_text(encoding="utf-8", errors="replace")
    except OSError as e:
        raise GateInputError(f"{log_path} is not readable: {e}") from e
    boot = parse_boot_log(text)
    context = load_optional(out_dir / "image" / "context.json") or {}
    checks = [
        Check(
            stage,
            "Image boots and passes its checks",
            boot["result"] == "PASS"
            and boot["fail_lines"] == 0
            and boot["pass_lines"] > 0,
            f"RESULT {boot['result'] or 'missing'} {boot['result_detail']}, "
            f"{boot['pass_lines']} PASS lines, {boot['fail_lines']} FAIL lines",
            "RESULT PASS with no FAIL line",
        )
    ]
    summary = load_optional(out_dir / "image" / "run" / "run-summary.json")
    metrics = load_optional(out_dir / "image" / "run" / "run-metrics.json") or {}
    if not isinstance(summary, dict):
        checks.append(
            Check(
                stage,
                "Unattended run in the image",
                False,
                "absent",
                "run/run-summary.json",
            )
        )
        return checks, {"boot": boot, "context": context}
    state = str(summary.get("state", ""))
    verdict = (summary.get("verdict") or {}).get("passed") is True
    succeeded: dict[str, int] = {}
    for sample in metrics.get("samples", []):
        if isinstance(sample, dict):
            values = sample.get("values") or {}
            key = str(sample.get("key", ""))
            succeeded[key] = max(
                succeeded.get(key, 0), int(values.get("sessions.succeeded", 0))
            )
    checks.append(
        Check(
            stage,
            "Unattended run in the image",
            state == "completed"
            and verdict
            and bool(succeeded)
            and all(v > 0 for v in succeeded.values()),
            f"state {state}, verdict {'passed' if verdict else 'not passed'}, succeeded sessions "
            + (", ".join(f"{k} {v}" for k, v in sorted(succeeded.items())) or "none"),
            "completed, passed, and every application succeeded at least once",
        )
    )
    return checks, {"boot": boot, "context": context, "succeeded": succeeded}


def raised_threshold(ratios: Sequence[float]) -> float:
    """Return a threshold just above the lowest measured ratio."""
    if not ratios:
        raise GateInputError("no measured conformance ratio")
    return round(min(ratios) + RAISE_STEP, 6)


def table(header: Sequence[str], rows: Sequence[Sequence[str]]) -> list[str]:
    """Render one Markdown table."""
    out = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    out += [
        "| " + " | ".join(c.replace("|", "\\|") for c in row) + " |" for row in rows
    ]
    return out


def render(
    checks: Sequence[Check],
    facts: dict[str, Any],
    stages: Sequence[str],
    threshold: float,
    tolerance: float,
) -> str:
    """Render the gate report in Markdown."""
    passed = all(c.passed for c in checks)
    partial = tuple(stages) != STAGES
    waived = list((facts.get("mix") or {}).get("waived") or [])
    basis_waived = bool((facts.get("mix") or {}).get("basis_waived"))
    word = "PASS" if passed else "FAIL"
    if passed and (waived or basis_waived):
        word = "PASS WITH WAIVERS"
    lines = [
        "# Gate 2 report",
        "",
        (
            "Gate 2 holds when all 15 application modules pass the realism checks in module CI, "
            f"a byte weighted enterprise mix stays within {tolerance:g} points of its weights "
            "through the controller and the agent, and the qcow2 image boots and runs a test "
            "unattended."
        ),
        "",
        f"**Verdict: {word}**"
        + (f" (partial, stages {', '.join(stages)} only)" if partial else ""),
        "",
        f"Conformance threshold {fmt_ratio(threshold)}, mix tolerance {tolerance:g} points.",
    ]
    if basis_waived:
        lines += [
            "",
            (
                "Gate 2 does not hold in full while the controller runs the mix on the session "
                "basis. It does not read `expected_bytes` from the module manifests yet, so the "
                "suite sets the session shares from the byte weights itself, and the byte weighted "
                "mix control of CTL-12 does not correct the run. The capture is scored against the "
                "byte weights all the same."
            ),
        ]
    if waived:
        lines += [
            "",
            "Gate 2 does not hold in full while the mix leaves out "
            + ", ".join(waived)
            + ". The agent has no native crate for these modules yet, and the waiver "
            "ends by itself when it gains one.",
        ]
    for stage in stages:
        stage_checks = [c for c in checks if c.stage == stage]
        title = {
            "modules": "Modules",
            "mix": "Byte weighted enterprise mix",
            "image": "Image",
        }[stage]
        lines += ["", f"## {title}", ""]
        lines += table(
            ("Check", "Measured", "Rule", "Result"),
            [
                (
                    c.name,
                    c.measured,
                    c.rule,
                    (
                        "WAIVED"
                        if (waived and c.name == WAIVER_CHECK)
                        or (basis_waived and c.name == "Mix basis")
                        else "pass"
                    )
                    if c.passed
                    else "FAIL",
                )
                for c in stage_checks
            ],
        )
        if stage == "modules" and "modules" in facts:
            rows = []
            for name, fact in sorted(facts["modules"].items()):
                for app, entry in sorted(fact["applications"].items()):
                    rows.append(
                        (name, app, str(entry["sessions"]), fmt_ratio(entry["ratio"]))
                    )
            lines += ["", "### Conformance by module", ""]
            lines += table(("Module", "Application", "Sessions", "Ratio"), rows)
            lines += ["", "### Checks by module", ""]
            lines += table(
                ("Module", "Realism checks"),
                [
                    (n, ", ".join(f["checks"]))
                    for n, f in sorted(facts["modules"].items())
                ],
            )
            for n, f in sorted(facts["modules"].items()):
                lines += finding_lines(
                    f.get("findings", {}), f"### Parser findings of module {n}"
                )
        if stage == "mix":
            lines += finding_lines(
                (facts.get("mix") or {}).get("findings", {}), "### Parser findings"
            )
        if stage == "mix" and facts.get("mix", {}).get("rows"):
            mix = facts["mix"]
            lines += ["", "### Byte shares", ""]
            lines += table(
                (
                    "Application",
                    "Weight %",
                    "Run metrics %",
                    "Delta pp",
                    "nDPI %",
                    "Delta pp",
                    "nDPI bytes",
                ),
                [
                    (
                        str(r.get("application")),
                        f"{r.get('configured_pct')}",
                        f"{r.get('metrics_pct')}",
                        f"{r.get('metrics_delta_pp')}",
                        f"{r.get('ndpi_pct')}",
                        f"{r.get('ndpi_delta_pp')}",
                        f"{r.get('ndpi_count')}",
                    )
                    for r in mix["rows"]
                ],
            )
            lines += calibration_lines(
                mix.get("calibration") or {},
                str((mix.get("context") or {}).get("basis", "bytes")),
            )
            lines += ["", "### Scenario totals", ""]
            lines += table(
                ("Scenario", "Sessions", "Failed", "Bytes", "Bytes per session"),
                [
                    (
                        str(x["scenario"]),
                        str(x["attempted"]),
                        str(x["failed"]),
                        str(x["bytes"]),
                        str(round(x["bytes"] / x["attempted"]))
                        if x["attempted"]
                        else "n/a",
                    )
                    for x in mix.get("scenarios", [])
                ],
            )
            lines += [
                "",
                "Sessions attempted per module: "
                + ", ".join(f"{k} {v}" for k, v in sorted(mix["modules"].items()))
                + ".",
            ]
            context = mix.get("context") or {}
            if context:
                lines += [
                    "",
                    "Run context: "
                    + ", ".join(f"{k} {v}" for k, v in sorted(context.items()))
                    + ".",
                ]
        if stage == "image" and facts.get("image"):
            image = facts["image"]
            lines += [
                "",
                "Image context: "
                + (
                    ", ".join(
                        f"{k} {v}"
                        for k, v in sorted((image.get("context") or {}).items())
                    )
                    or "none recorded"
                )
                + ".",
            ]
    lines.append("")
    return "\n".join(lines)


def evaluate(
    out_dir: Path,
    stages: Sequence[str],
    threshold: float,
    tolerance: float,
    min_module_sessions: int,
    mix_modules: Sequence[str],
    allow_waivers: bool = False,
) -> tuple[list[Check], dict[str, Any]]:
    """Run the checks of the selected stages."""
    checks: list[Check] = []
    facts: dict[str, Any] = {}
    if "modules" in stages:
        found, facts["modules"] = modules_stage(out_dir, threshold, min_module_sessions)
        checks += found
    if "mix" in stages:
        found, facts["mix"] = mix_stage(
            out_dir, threshold, tolerance, mix_modules, allow_waivers
        )
        checks += found
    if "image" in stages:
        found, facts["image"] = image_stage(out_dir)
        checks += found
    return checks, facts


def lowest_ratio(out_dir: Path) -> float:
    """Return the lowest conformance ratio of all module reports and the mix."""
    ratios: list[float] = []
    paths = [out_dir / "modules" / m / f"{m}.realism.json" for m in MODULES]
    paths.append(out_dir / "mix" / "realism.json")
    for path in paths:
        report = load_optional(path)
        if isinstance(report, dict):
            ratios += [
                float(a.get("ratio", 0.0))
                for a in app_entries(find_check(report, "conformance"))
                if int(a.get("sessions", 0)) > 0
            ]
    return min(ratios) if ratios else 0.0


def main(argv: Sequence[str] | None = None) -> int:
    """Write the gate report of an output directory and return the exit code."""
    parser = argparse.ArgumentParser(description="Gate 2 verdict and report")
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--stages", default=",".join(STAGES))
    parser.add_argument("--threshold", type=float, default=DEFAULT_THRESHOLD)
    parser.add_argument("--tolerance", type=float, default=DEFAULT_TOLERANCE)
    parser.add_argument(
        "--min-module-sessions", type=int, default=DEFAULT_MIN_MODULE_SESSIONS
    )
    parser.add_argument(
        "--mix-modules",
        default=",".join(m for m in MODULES),
        help="modules that the mix run must have attempted",
    )
    parser.add_argument(
        "--raised-threshold",
        action="store_true",
        help="print a threshold just above the lowest measured ratio and exit",
    )
    parser.add_argument(
        "--allow-waivers",
        action="store_true",
        help="let modules that the agent cannot run yet leave the mix; the verdict then says so",
    )
    parser.add_argument(
        "--no-write", action="store_true", help="print the verdict only"
    )
    args = parser.parse_args(argv)
    stages = [s for s in args.stages.split(",") if s]
    unknown = [s for s in stages if s not in STAGES]
    if unknown or not stages:
        print(
            f"gate2_report.py: unknown stage {', '.join(unknown) or '(none)'}",
            file=sys.stderr,
        )
        return 2
    try:
        if args.raised_threshold:
            print(raised_threshold([lowest_ratio(args.out_dir)]))
            return 0
        checks, facts = evaluate(
            args.out_dir,
            stages,
            args.threshold,
            args.tolerance,
            args.min_module_sessions,
            [m for m in args.mix_modules.split(",") if m],
            args.allow_waivers,
        )
    except GateInputError as e:
        print(f"gate2_report.py: {e}", file=sys.stderr)
        return 2
    report = render(checks, facts, stages, args.threshold, args.tolerance)
    passed = all(c.passed for c in checks)
    if not args.no_write:
        (args.out_dir / "report.md").write_text(report, encoding="utf-8")
        summary = {
            "passed": passed,
            "partial": tuple(stages) != STAGES,
            "stages": stages,
            "threshold": args.threshold,
            "tolerance_points": args.tolerance,
            "checks": [
                {
                    "stage": c.stage,
                    "name": c.name,
                    "passed": c.passed,
                    "measured": c.measured,
                    "rule": c.rule,
                }
                for c in checks
            ],
            "facts": facts,
        }
        (args.out_dir / "summary.json").write_text(
            json.dumps(summary, indent=2, default=str) + "\n", encoding="utf-8"
        )
    print(report)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
