# SPDX-License-Identifier: GPL-3.0-only
"""Protocol conformance check of the realism runner.

Conformance is the share of labelled sessions of an application that a classifier
names as the intended application. The specification sets the v1 acceptance
target at 99% of sessions per application.
"""

from __future__ import annotations

from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass, field
from fractions import Fraction
from pathlib import Path
from typing import Any

from realism.capture import Session
from realism.classifier import UNKNOWN, Classifier, matches
from realism.labels import Labels

CHECK_NAME = "conformance"
METRIC = "realism.conformance_ratio"
DEFAULT_THRESHOLD = 0.99
MAX_MISMATCHES = 100


@dataclass
class ApplicationResult:
    """Conformance of the sessions of one intended application."""

    application: str
    accepted: tuple[str, ...]
    sessions: int = 0
    conforming: int = 0
    observed: Counter[str] = field(default_factory=Counter)

    @property
    def ratio(self) -> float:
        """The conforming share of sessions, 0 when the application has none."""
        return self.conforming / self.sessions if self.sessions else 0.0

    def passes(self, threshold: float) -> bool:
        """Compare the exact ratio with the threshold, so 99 of 100 meets 0.99."""
        if not self.sessions or not self.accepted:
            return False
        return Fraction(self.conforming, self.sessions) >= Fraction(str(threshold))


@dataclass
class ConformanceResult:
    """The outcome of one conformance check for one classifier."""

    classifier: str
    classifier_version: str
    threshold: float
    applications: list[ApplicationResult]
    unlabelled_sessions: int
    mismatches: list[dict[str, Any]]
    mismatches_total: int

    @property
    def passed(self) -> bool:
        """True when at least one application was scored and every one passes."""
        return bool(self.applications) and all(
            app.passes(self.threshold) for app in self.applications
        )

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        return {
            "check": CHECK_NAME,
            "passed": self.passed,
            "classifier": self.classifier,
            "classifier_version": self.classifier_version,
            "threshold": self.threshold,
            "metrics": [
                {
                    "name": METRIC,
                    "labels": {"application": app.application, "classifier": self.classifier},
                    "value": round(app.ratio, 6),
                }
                for app in self.applications
            ],
            "applications": [
                {
                    "application": app.application,
                    "accepted": list(app.accepted),
                    "sessions": app.sessions,
                    "conforming": app.conforming,
                    "ratio": round(app.ratio, 6),
                    "passed": app.passes(self.threshold),
                    "observed": dict(sorted(app.observed.items())),
                }
                for app in self.applications
            ],
            "unlabelled_sessions": self.unlabelled_sessions,
            "mismatches": self.mismatches,
            "mismatches_total": self.mismatches_total,
        }


def check(
    capture: Path,
    sessions: Sequence[Session],
    labels: Labels,
    classifier: Classifier,
    threshold: float,
) -> ConformanceResult:
    """Classify the sessions of a capture and score them against their labels."""
    verdicts = classifier.classify(capture, sessions)
    by_app: dict[str, ApplicationResult] = {}
    unlabelled = 0
    mismatches: list[dict[str, Any]] = []
    mismatches_total = 0
    for session in sessions:
        application = labels.application_of(session)
        if application is None:
            unlabelled += 1
            continue
        result = by_app.get(application)
        if result is None:
            result = ApplicationResult(
                application, labels.accepted_names(application, classifier.name)
            )
            by_app[application] = result
        observed = verdicts.get(session.key, UNKNOWN)
        result.sessions += 1
        result.observed[observed] += 1
        if matches(observed, result.accepted):
            result.conforming += 1
            continue
        mismatches_total += 1
        if len(mismatches) < MAX_MISMATCHES:
            mismatches.append(
                {
                    "session": session.id,
                    "transport": session.transport,
                    "client": {"ip": session.client_ip, "port": session.client_port},
                    "server": {"ip": session.server_ip, "port": session.server_port},
                    "application": application,
                    "observed": observed,
                }
            )
    return ConformanceResult(
        classifier=classifier.name,
        classifier_version=classifier.version(),
        threshold=threshold,
        applications=sorted(by_app.values(), key=lambda app: app.application),
        unlabelled_sessions=unlabelled,
        mismatches=mismatches,
        mismatches_total=mismatches_total,
    )
