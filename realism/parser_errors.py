# SPDX-License-Identifier: GPL-3.0-only
"""Parser error check of the realism runner.

The specification requires zero parser errors in a reference implementation. The
check runs each reference parser on the capture, joins its findings to the
session table, and counts parser errors and malformed packets per intended
application. An application counts the largest number that any one parser
reports, so two parsers that see the same fault do not count it twice.
"""

from __future__ import annotations

from collections import Counter
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from realism.capture import Session, SessionKey
from realism.labels import Labels
from realism.parsers import KIND_MALFORMED, Finding, ParserRunner
from realism.series import failures_series

CHECK_NAME = "parser_errors"
METRIC = "realism.parser_errors"
ALL_APPLICATIONS = "all"
MAX_ERRORS = 100


@dataclass
class ApplicationErrors:
    """Parser errors of the sessions of one intended application."""

    application: str
    sessions: int = 0
    errors: Counter[str] = field(default_factory=Counter)
    malformed: Counter[str] = field(default_factory=Counter)

    def to_dict(self, parsers: Sequence[str]) -> dict[str, Any]:
        """Return the application entry with a count per parser."""
        return {
            "application": self.application,
            "sessions": self.sessions,
            "parser_errors": max((self.errors[p] for p in parsers), default=0),
            "malformed_packets": max((self.malformed[p] for p in parsers), default=0),
            "by_parser": {
                p: {"parser_errors": self.errors[p], "malformed_packets": self.malformed[p]}
                for p in parsers
            },
        }


@dataclass
class ParserErrorsResult:
    """The outcome of the parser error check over every parser that ran."""

    parsers: list[dict[str, str]]
    missing: list[str]
    skipped: list[dict[str, str]]
    applications: list[ApplicationErrors]
    unlabelled: Counter[str]
    unattributed: Counter[str]
    errors: list[dict[str, Any]]
    errors_total: int

    @property
    def parser_names(self) -> list[str]:
        """The names of the parsers that ran, in run order."""
        return [p["parser"] for p in self.parsers]

    @property
    def passed(self) -> bool:
        """True when at least one parser ran and no parser reported an error."""
        return bool(self.parsers) and self.errors_total == 0

    @property
    def failures_total(self) -> int:
        """The findings of every parser, and 1 more when no parser ran."""
        return self.errors_total + (0 if self.parsers else 1)

    def metrics(self) -> list[dict[str, Any]]:
        """Return ``realism.parser_errors`` per application and parser, and the failures series.

        Each parser that ran gives one series for each labelled application and one with the
        application ``all`` that totals every finding of the parser: the per application values
        plus its unlabelled and unattributed findings. When no parser ran, only the failures
        series is there.
        """
        out: list[dict[str, Any]] = []
        for parser in self.parser_names:
            for app in self.applications:
                labels = {"application": app.application, "parser": parser}
                out.append({"name": METRIC, "labels": labels, "value": app.errors[parser]})
            total = (
                sum(app.errors[parser] for app in self.applications)
                + self.unlabelled[parser]
                + self.unattributed[parser]
            )
            labels = {"application": ALL_APPLICATIONS, "parser": parser}
            out.append({"name": METRIC, "labels": labels, "value": total})
        out.append(failures_series(CHECK_NAME, self.failures_total, self.passed))
        return out

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        names = self.parser_names
        return {
            "check": CHECK_NAME,
            "passed": self.passed,
            "metrics": self.metrics(),
            "parsers": self.parsers,
            "missing_parsers": self.missing,
            "skipped_parsers": self.skipped,
            "applications": [app.to_dict(names) for app in self.applications],
            "unlabelled_errors": {p: self.unlabelled[p] for p in names},
            "unattributed_errors": {p: self.unattributed[p] for p in names},
            "errors": self.errors,
            "errors_total": self.errors_total,
        }


def _endpoint(ip: str | None, port: int | None) -> dict[str, Any] | None:
    return None if ip is None or port is None else {"ip": ip, "port": port}


def check(
    capture: Path,
    sessions: Sequence[Session],
    labels: Labels,
    parsers: Sequence[ParserRunner],
    missing: Sequence[str] = (),
    skip_reasons: Mapping[str, str] | None = None,
) -> ParserErrorsResult:
    """Run every parser on the capture and count its findings per application.

    ``missing`` names the parsers that were requested and not installed, and the
    result lists each as skipped with its reason from ``skip_reasons``.
    """
    reasons = skip_reasons or {}
    by_key: dict[SessionKey, Session] = {s.key: s for s in sessions}
    by_app: dict[str, ApplicationErrors] = {}
    for session in sessions:
        application = labels.application_of(session)
        if application is not None:
            by_app.setdefault(application, ApplicationErrors(application)).sessions += 1
    unlabelled: Counter[str] = Counter()
    unattributed: Counter[str] = Counter()
    errors: list[dict[str, Any]] = []
    errors_total = 0
    ran: list[dict[str, str]] = []
    for parser in parsers:
        ran.append({"parser": parser.name, "version": parser.version()})
        for finding in parser.findings(capture):
            errors_total += 1
            key = finding.key
            owner = by_key.get(key) if key is not None else None
            application = labels.application_of(owner) if owner is not None else None
            if owner is None:
                unattributed[parser.name] += 1
            elif application is None:
                unlabelled[parser.name] += 1
            else:
                app = by_app[application]
                app.errors[parser.name] += 1
                if finding.kind == KIND_MALFORMED:
                    app.malformed[parser.name] += 1
            if len(errors) < MAX_ERRORS:
                errors.append(_error_entry(parser.name, finding, owner, application))
    return ParserErrorsResult(
        parsers=ran,
        missing=list(missing),
        skipped=[
            {"parser": name, "reason": reasons.get(name, "not installed")} for name in missing
        ],
        applications=sorted(by_app.values(), key=lambda app: app.application),
        unlabelled=unlabelled,
        unattributed=unattributed,
        errors=errors,
        errors_total=errors_total,
    )


def _error_entry(
    parser: str, finding: Finding, session: Session | None, application: str | None
) -> dict[str, Any]:
    if session is not None:
        client = _endpoint(session.client_ip, session.client_port)
        server = _endpoint(session.server_ip, session.server_port)
        transport: str | None = session.transport
    else:
        client = _endpoint(finding.src_ip, finding.src_port)
        server = _endpoint(finding.dst_ip, finding.dst_port)
        transport = finding.transport
    return {
        "parser": parser,
        "kind": finding.kind,
        "name": finding.name,
        "message": finding.message,
        "frame": finding.frame,
        "timestamp": finding.timestamp,
        "session": session.id if session is not None else None,
        "application": application,
        "transport": transport,
        "client": client,
        "server": server,
    }
