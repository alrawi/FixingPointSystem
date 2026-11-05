# SPDX-License-Identifier: GPL-3.0-only
"""Build the realism report of a capture.

The report follows ``realism/schema/realism-report.schema.json``. It holds one
entry per check in ``checks``, and later packages add checks with their own
``check`` names next to the conformance check of this package.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from realism import __version__, conformance, parser_errors
from realism.capture import summarize
from realism.classifier import Classifier
from realism.labels import load
from realism.parsers import ParserRunner

REPORT_FORMAT = "fps-realism-report"
REPORT_VERSION = "0.3.0"
SCHEMA_PATH = Path(__file__).resolve().parent / "schema" / "realism-report.schema.json"
LABELS_SCHEMA_PATH = Path(__file__).resolve().parent / "schema" / "labels.schema.json"


@dataclass
class Report:
    """A realism report with its capture summary and check results."""

    capture: dict[str, Any]
    labels: dict[str, Any]
    checks: list[dict[str, Any]] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        """True when the report has checks and every check passed."""
        return bool(self.checks) and all(entry["passed"] for entry in self.checks)

    def to_dict(self) -> dict[str, Any]:
        """Return the report as a JSON compatible mapping."""
        return {
            "report": REPORT_FORMAT,
            "version": REPORT_VERSION,
            "generator": f"fps-realism {__version__}",
            "passed": self.passed,
            "capture": self.capture,
            "labels": self.labels,
            "checks": self.checks,
        }

    def to_json(self) -> str:
        """Return the report as indented JSON with a trailing newline."""
        return json.dumps(self.to_dict(), indent=2, sort_keys=False) + "\n"


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def score(
    pcap: Path,
    labels_path: Path,
    classifier: Classifier,
    threshold: float | None = None,
    parsers: Sequence[ParserRunner] | None = None,
    missing_parsers: Sequence[str] = (),
    skip_reasons: Mapping[str, str] | None = None,
) -> Report:
    """Score a capture against its labels and return the realism report.

    The threshold argument overrides the threshold of the labels file, which
    overrides the default of 0.99. With ``parsers``, even an empty sequence, the
    report also carries the parser error check, and ``missing_parsers`` names the
    requested parsers that were not installed. The check lists each of them under
    ``skipped_parsers`` with its reason from ``skip_reasons``.
    """
    labels = load(labels_path)
    summary = summarize(pcap)
    if threshold is None:
        threshold = (
            labels.threshold if labels.threshold is not None else conformance.DEFAULT_THRESHOLD
        )
    checks = [conformance.check(pcap, summary.sessions, labels, classifier, threshold).to_dict()]
    if parsers is not None:
        checks.append(
            parser_errors.check(
                pcap, summary.sessions, labels, parsers, missing_parsers, skip_reasons
            ).to_dict()
        )
    return Report(
        capture={
            "file": pcap.name,
            "sha256": _sha256(pcap),
            "format": summary.format,
            "link_types": summary.link_types,
            "frames": summary.frames,
            "skipped_frames": summary.skipped_frames,
            "sessions": len(summary.sessions),
        },
        labels={"file": labels_path.name, "sha256": _sha256(labels_path)},
        checks=checks,
    )
