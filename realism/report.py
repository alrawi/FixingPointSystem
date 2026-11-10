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

from realism import (
    __version__,
    conformance,
    content,
    encryption,
    mix,
    parser_errors,
    structure,
    temporal,
    transport,
)
from realism.capture import Session, SessionKey, summarize
from realism.classifier import Classifier
from realism.labels import load
from realism.parsers import ParserRunner

REPORT_FORMAT = "fps-realism-report"
REPORT_VERSION = "0.9.0"
SCHEMA_PATH = Path(__file__).resolve().parent / "schema" / "realism-report.schema.json"
LABELS_SCHEMA_PATH = Path(__file__).resolve().parent / "schema" / "labels.schema.json"


class _ClassifyOnce:
    """Run the classifier once for the checks that share its verdicts on one capture."""

    def __init__(self, inner: Classifier) -> None:
        self._inner = inner
        self.name = inner.name
        self._verdicts: dict[SessionKey, str] | None = None

    def version(self) -> str:
        return self._inner.version()

    def classify(self, capture: Path, sessions: Sequence[Session]) -> Mapping[SessionKey, str]:
        if self._verdicts is None:
            self._verdicts = dict(self._inner.classify(capture, sessions))
        return self._verdicts


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
    tls_probe: encryption.TlsProbe | None = None,
    keylog: Path | None = None,
    max_duplicate_ratio: float | None = None,
    scenario_files: Sequence[Path] = (),
    ks_alpha: float | None = None,
    ks_min_samples: int | None = None,
    test_config: Path | None = None,
    run_report: Path | None = None,
    mix_tolerance: float | None = None,
    mix_basis: str | None = None,
    os_profile: str | None = None,
    os_profiles_dir: Path | None = None,
    transport_strict: bool | None = None,
    client_hops: int | None = None,
    server_hops: int | None = None,
) -> Report:
    """Score a capture against its labels and return the realism report.

    The threshold argument overrides the threshold of the labels file, which
    overrides the default of 0.99. With ``parsers``, even an empty sequence, the
    report also carries the parser error check, and ``missing_parsers`` names the
    requested parsers that were not installed. The check lists each of them under
    ``skipped_parsers`` with its reason from ``skip_reasons``.

    Labels with a ``tls`` section add the encryption check, which runs tshark as
    ``tls_probe`` (by default the tshark of the search path) with the keys of the
    capture and with ``keylog``, a key log in the NSS format.

    Labels with a ``content`` section, or a ``max_duplicate_ratio`` argument, add the
    content variety check. The argument overrides the ratio of the labels, which
    defaults to 0.01.

    Labels with a ``structure`` section, or ``scenario_files`` (paths of scenario files),
    add the session structure check, which compares the connections of each scenario
    instance with the flow graph of the scenario.

    Labels with a ``temporal`` section, or a ``ks_alpha`` or ``ks_min_samples`` argument,
    add the temporal distribution check, which compares inter arrival times and payload
    sizes with the target distributions of the labels through the KS test. The arguments
    override the significance level (default 0.01) and the fewest observations (default 30).

    Labels with a ``mix`` section, or any of ``test_config``, ``run_report``, ``mix_tolerance``,
    and ``mix_basis``, add the mix accuracy check, which compares the achieved mix from the
    metrics of a run (``run_report``) and from nDPI with the weights of ``test_config`` within
    ``mix_tolerance`` percentage points (default 2) on ``mix_basis`` (default the basis of the
    test configuration). The arguments override the labels.

    Labels with a ``transport`` section, or any of ``os_profile``, ``os_profiles_dir``,
    ``transport_strict``, ``client_hops``, and ``server_hops``, add the transport behavior check,
    which compares the TTL and the TCP options of the SYN and SYN-ACK of every connection with
    the operating system profile ``os_profile`` (read from ``os_profiles_dir``). The option order
    and the SYN window decide only when ``transport_strict`` is true, and ``client_hops`` and
    ``server_hops`` give the routers between the hosts and the capture point.
    """
    labels = load(labels_path)
    summary = summarize(pcap)
    if threshold is None:
        threshold = (
            labels.threshold if labels.threshold is not None else conformance.DEFAULT_THRESHOLD
        )
    run_mix = labels.mix is not None or any(
        value is not None for value in (test_config, run_report, mix_tolerance, mix_basis)
    )
    if run_mix:
        classifier = _ClassifyOnce(classifier)
    checks = [conformance.check(pcap, summary.sessions, labels, classifier, threshold).to_dict()]
    if parsers is not None:
        checks.append(
            parser_errors.check(
                pcap, summary.sessions, labels, parsers, missing_parsers, skip_reasons
            ).to_dict()
        )
    if labels.tls is not None:
        probe = tls_probe if tls_probe is not None else encryption.TsharkTlsProbe()
        checks.append(encryption.check(pcap, summary.sessions, labels, probe, keylog).to_dict())
    if labels.content is not None or max_duplicate_ratio is not None:
        checks.append(content.check(pcap, summary.sessions, labels, max_duplicate_ratio).to_dict())
    if labels.structure is not None or scenario_files:
        checks.append(structure.check(pcap, summary.sessions, labels, scenario_files).to_dict())
    if labels.temporal is not None or ks_alpha is not None or ks_min_samples is not None:
        checks.append(
            temporal.check(pcap, summary.sessions, labels, ks_alpha, ks_min_samples).to_dict()
        )
    if run_mix:
        checks.append(
            mix.check(
                pcap,
                summary.sessions,
                labels,
                classifier,
                config_path=test_config,
                run_report=run_report,
                tolerance_points=mix_tolerance,
                basis=mix_basis,
            ).to_dict()
        )
    run_transport = labels.transport is not None or any(
        value is not None
        for value in (os_profile, os_profiles_dir, transport_strict, client_hops, server_hops)
    )
    if run_transport:
        checks.append(
            transport.check(
                pcap,
                labels,
                os_profile=os_profile,
                profiles_dir=os_profiles_dir,
                strict=transport_strict,
                client_hops=client_hops,
                server_hops=server_hops,
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
