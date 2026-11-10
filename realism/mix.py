# SPDX-License-Identifier: GPL-3.0-only
"""Mix accuracy check of the realism runner.

A test configures the traffic mix as weights per scenario, by sessions or by bytes
(``profile.basis``). The check measures the mix that the run achieved in two independent
ways and compares each with the configured weights.

* **Run metrics.** The controller counts ``sessions.attempted``, ``bytes.tx``, and
  ``bytes.rx`` per scenario. The check reads them from a run report file (``--run-report``).
* **nDPI.** The runner classifies the sessions of the capture and counts the sessions and the
  application payload bytes of each application that nDPI names.

The two measurements have different faults. A scheduler that sends the wrong mix shows in
both. A generator whose traffic nDPI does not recognise as the intended application shows
only in the nDPI share, and a counter bug of the engine only in the metrics share.

**Series.** The nDPI side also gives the catalog gauges ``mix.share_pct``, ``mix.abs_error_pp``,
and ``mix.max_abs_error_pp`` with ``source`` ``classifier`` and the classifier label, for the
basis that :attr:`MixResult.metrics_basis` names.

**Mapping.** A scenario is not an nDPI protocol, so the labels map each mix scenario to an
application (``mix.scenarios``), and the application gives its nDPI names exactly as the
conformance check reads them (``applications.<name>.ndpi`` of the labels, or the built in
names of ``http1`` and ``dns``). A scenario entry may list its own ``ndpi`` names instead.
Scenarios that map to one application are one row, because nDPI cannot tell them apart, and
two applications that accept the same nDPI name are refused, because a session could not
be assigned.

**Shares.** Per basis, the configured share of an application is the sum of the weights of
its scenarios over the sum of all weights. The achieved share from the metrics is the count of
its scenarios over the count of all scenarios of the run report. The achieved share from nDPI
is the count of the sessions that nDPI named as the application over the count of all labelled
sessions of the capture (all sessions when the labels have no rules and no default). Sessions
that nDPI named as no application of the mix, including ``Unknown``, therefore lower the
shares of the others and form their own row ``(unmatched)``, whose configured share is 0.
Bytes are the application payload bytes of both directions. The metrics side sums ``bytes.tx``
and ``bytes.rx`` of the series it was given, so give it the client side (the default of the
metrics endpoint), which counts each byte once.

**Comparison.** The share of each application, from each source, must lie within
``tolerance_points`` (default 2) of the configured share, inclusive. The comparison is exact:
shares are fractions of the counts and the weights, so a delta of exactly 2.0 points passes.

**Limits.** A scenario that opens several connections per session (a lookup and then a page
fetch) has more sessions in a capture than in the metrics, so the check fits scenarios with
one connection per session. Give such a scenario its own application only if nDPI counts the
same number of sessions for it, or compare the bytes basis, which does not depend on the number
of connections.
"""

from __future__ import annotations

import importlib
import json
from collections import Counter
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from datetime import datetime
from fractions import Fraction
from pathlib import Path
from typing import Any

from realism.capture import Session
from realism.classifier import UNKNOWN, Classifier, matches
from realism.labels import Labels

CHECK_NAME = "mix_accuracy"
DEFAULT_TOLERANCE_POINTS = 2.0
BASES = ("sessions", "bytes")
UNMATCHED = "(unmatched)"
MAX_FAILURES_LISTED = 50
SESSIONS_METRIC = "sessions.attempted"
BYTES_METRICS = ("bytes.tx", "bytes.rx")
RUN_KEY = "*"
SHARE_METRIC = "mix.share_pct"
ERROR_METRIC = "mix.abs_error_pp"
MAX_ERROR_METRIC = "mix.max_abs_error_pp"
# The value of the catalog label ``source`` for what the classifier saw in the capture.
SOURCE_CLASSIFIER = "classifier"


class MixError(Exception):
    """The inputs of the mix check cannot be read or do not fit each other."""


@dataclass(frozen=True)
class MixConfig:
    """The configured mix: weights by scenario and the basis that the test sets."""

    basis: str | None
    weights: Mapping[str, float]
    source: str


@dataclass(frozen=True)
class Counts:
    """Sessions and bytes of one scenario in a run. ``bytes`` is None when not reported."""

    sessions: int
    bytes: int | None = None


@dataclass(frozen=True)
class RunCounts:
    """The achieved counts per scenario reference, from a run report file."""

    scenarios: Mapping[str, Counts]
    source: str
    form: str


def _yaml(path: Path, text: str) -> Any:
    try:
        yaml: Any = importlib.import_module("yaml")
    except ImportError as exc:
        raise MixError(f"parse {path}: YAML needs PyYAML, or write the file as JSON") from exc
    try:
        return yaml.safe_load(text)
    except yaml.YAMLError as exc:
        raise MixError(f"parse {path}: {exc}") from exc


def _read(path: Path, what: str) -> Any:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise MixError(f"read {what} {path}: {exc}") from exc
    if path.suffix.lower() in (".yaml", ".yml"):
        return _yaml(path, text)
    try:
        return json.loads(text)
    except json.JSONDecodeError as exc:
        raise MixError(f"parse {what} {path}: {exc}") from exc


def _is_number(value: Any) -> bool:
    return not isinstance(value, bool) and isinstance(value, int | float)


def load_config(path: Path) -> MixConfig:
    """Read ``profile.basis`` and ``profile.mix`` of a test configuration file.

    A scenario that the mix lists twice, for example with different parameters, counts
    with the sum of its weights.
    """
    doc = _read(path, "test configuration")
    profile = doc.get("profile") if isinstance(doc, dict) else None
    if not isinstance(profile, dict):
        raise MixError(f"test configuration {path}: it has no profile section")
    basis = profile.get("basis")
    if basis not in BASES:
        raise MixError(f"test configuration {path}: profile.basis must be sessions or bytes")
    mix = profile.get("mix")
    if not isinstance(mix, list) or not mix:
        raise MixError(f"test configuration {path}: profile.mix must be a non empty list")
    weights: dict[str, float] = {}
    for index, entry in enumerate(mix):
        scenario = entry.get("scenario") if isinstance(entry, dict) else None
        weight = entry.get("weight") if isinstance(entry, dict) else None
        if (
            not isinstance(scenario, str)
            or not scenario
            or not isinstance(weight, int | float)
            or isinstance(weight, bool)
            or weight <= 0
        ):
            raise MixError(
                f"test configuration {path}: profile.mix[{index}] needs a scenario and a "
                "weight above 0"
            )
        weights[scenario] = _add(weights.get(scenario), weight)
    return MixConfig(basis=basis, weights=weights, source=path.name)


def _add(first: float | None, second: float) -> float:
    return float(second) if first is None else float(Fraction(str(first)) + Fraction(str(second)))


def _stamp(value: Any) -> float | None:
    if not isinstance(value, str):
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def _count(value: Any, where: str) -> int:
    if not _is_number(value) or value < 0 or value != int(value):
        raise MixError(f"{where}: must be a whole number of 0 or more, found {value!r}")
    return int(value)


def _from_series(doc: Mapping[str, Any], where: str) -> dict[str, Counts]:
    """Take the last sample of every scenario key of a scenario view MetricSeriesSet."""
    if doc.get("view") != "scenario":
        raise MixError(
            f"{where}: the metric series has view {doc.get('view')!r}, the mix check needs the "
            "scenario view (GET /v1/runs/{id}/metrics?view=scenario)"
        )
    samples = doc.get("samples")
    if not isinstance(samples, list):
        raise MixError(f"{where}: samples must be a list")
    latest: dict[str, tuple[float, int, Mapping[str, Any]]] = {}
    for index, sample in enumerate(samples):
        if not isinstance(sample, dict) or not isinstance(sample.get("key"), str):
            raise MixError(f"{where}: samples[{index}] needs a key")
        values = sample.get("values")
        if not isinstance(values, dict):
            raise MixError(f"{where}: samples[{index}] needs values")
        key = sample["key"]
        if key == RUN_KEY:
            continue
        stamp = _stamp(sample.get("ts"))
        order = (stamp if stamp is not None else float("-inf"), index)
        if key not in latest or order >= latest[key][:2]:
            latest[key] = (order[0], index, values)
    result: dict[str, Counts] = {}
    for key, (_, index, values) in latest.items():
        here = f"{where}: samples[{index}] ({key})"
        if SESSIONS_METRIC not in values:
            raise MixError(f"{here}: it has no {SESSIONS_METRIC}")
        reported = [m for m in BYTES_METRICS if m in values]
        total = (
            sum(_count(values[m], f"{here} {m}") for m in reported)
            if len(reported) == len(BYTES_METRICS)
            else None
        )
        result[key] = Counts(_count(values[SESSIONS_METRIC], f"{here} {SESSIONS_METRIC}"), total)
    return result


def _from_table(entries: Any, where: str) -> dict[str, Counts]:
    if not isinstance(entries, list):
        raise MixError(f"{where}: scenarios must be a list")
    result: dict[str, Counts] = {}
    for index, entry in enumerate(entries):
        here = f"{where}: scenarios[{index}]"
        if not isinstance(entry, dict) or not isinstance(entry.get("scenario"), str):
            raise MixError(f"{here}: needs a scenario name")
        if entry["scenario"] in result:
            raise MixError(f"{here}: scenario {entry['scenario']} is listed twice")
        result[entry["scenario"]] = Counts(
            _count(entry.get("sessions"), f"{here}.sessions"),
            _count(entry["bytes"], f"{here}.bytes") if entry.get("bytes") is not None else None,
        )
    return result


_REST_KEYS = ("sessions_attempted", "bytes_tx", "bytes_rx")


def _is_rest_entry(entry: Any) -> bool:
    return isinstance(entry, dict) and any(k in entry for k in _REST_KEYS)


def _from_rest(entries: list[Any], where: str) -> dict[str, Counts]:
    """Read ``RunReport.scenarios``, the ``ScenarioTotals`` of the REST report.

    The application and the scenario are apart, so the key is ``<application>/<scenario>``,
    the sessions are ``sessions_attempted``, and the bytes are ``bytes_tx`` plus ``bytes_rx``.
    """
    result: dict[str, Counts] = {}
    for index, entry in enumerate(entries):
        here = f"{where}: scenarios[{index}]"
        if not isinstance(entry, dict):
            raise MixError(f"{here}: must be an object")
        for name in ("application", "scenario"):
            if not isinstance(entry.get(name), str) or not entry[name]:
                raise MixError(f"{here}: needs an {name} name")
        key = f"{entry['application']}/{entry['scenario']}"
        if key in result:
            raise MixError(f"{here}: scenario {key} is listed twice")
        result[key] = Counts(
            _count(entry.get("sessions_attempted"), f"{here}.sessions_attempted"),
            _count(entry.get("bytes_tx"), f"{here}.bytes_tx")
            + _count(entry.get("bytes_rx"), f"{here}.bytes_rx"),
        )
    return result


def load_run_counts(path: Path) -> RunCounts:
    """Read the achieved sessions and bytes per scenario of a run.

    The file is JSON (or YAML) in one of four forms.

    * A ``MetricSeriesSet`` of ``GET /v1/runs/{id}/metrics?view=scenario``, whose last sample per
      key supplies ``sessions.attempted``, ``bytes.tx``, and ``bytes.rx``. Keys are
      ``<application>/<scenario>``.
    * A bundle object with that series under ``metrics``, next to the ``run`` of a run report.
    * A table ``{"scenarios": [{"scenario", "sessions", "bytes"}]}``, with the scenario as
      ``<application>/<scenario>``, which is the file form that the controller writes.
    * The ``RunReport`` of ``GET /v1/runs/{id}/report``, whose ``scenarios`` list (API 0.9.0 and
      later) has the application and the scenario apart, ``sessions_attempted``, ``bytes_tx``,
      and ``bytes_rx``.

    A ``RunReport`` of a controller that predates ``scenarios`` holds only the totals of the
    run, so the check cannot use it and says so.
    """
    doc = _read(path, "run report")
    where = f"run report {path}"
    if not isinstance(doc, dict):
        raise MixError(f"{where}: the document must be an object")
    if isinstance(doc.get("metrics"), dict):
        return RunCounts(_from_series(doc["metrics"], where), path.name, "bundle")
    if "samples" in doc:
        return RunCounts(_from_series(doc, where), path.name, "metric_series")
    scenarios = doc.get("scenarios")
    rest_report = "run" in doc and "realism" in doc
    if isinstance(scenarios, list) and (rest_report or any(_is_rest_entry(e) for e in scenarios)):
        return RunCounts(_from_rest(scenarios, where), path.name, "rest_report")
    if "scenarios" in doc:
        return RunCounts(_from_table(doc["scenarios"], where), path.name, "table")
    raise MixError(
        f"{where}: it has no per scenario counts. A report of GET /v1/runs/{{id}}/report from a "
        "controller before API 0.9.0 holds only the totals of the run. Save "
        'GET /v1/runs/{id}/metrics?view=scenario next to it, under "metrics", or give a table '
        '"scenarios"'
    )


def same_scenario(configured: str, reported: str) -> bool:
    """Return True when two scenario references name the same scenario.

    A reference is ``<module>/<scenario>`` or a bare scenario name. A bare name matches the
    scenario part of the other reference.
    """
    if configured == reported:
        return True
    head, _, tail = configured.rpartition("/")
    other_head, _, other_tail = reported.rpartition("/")
    return tail == other_tail and (not head or not other_head)


@dataclass
class Row:
    """One application of one basis, with both measurements."""

    application: str
    scenarios: list[str]
    configured: Fraction
    metrics: Fraction | None
    ndpi: Fraction | None
    metrics_count: int | None = None
    ndpi_count: int | None = None
    reasons: list[str] = field(default_factory=list)
    metrics_ok: bool = True
    ndpi_ok: bool = True

    @property
    def passed(self) -> bool:
        """True when both measurements are within the tolerance."""
        return self.metrics_ok and self.ndpi_ok


@dataclass
class BasisResult:
    """The outcome of one basis."""

    basis: str
    rows: list[Row]
    metrics_total: int
    ndpi_total: int
    unmatched: Row
    unmatched_observed: Counter[str]
    failures: list[str]

    @property
    def passed(self) -> bool:
        """True when no row and no total failed."""
        return not self.failures


@dataclass
class MixResult:
    """The outcome of the mix accuracy check."""

    classifier: str
    classifier_version: str
    tolerance_points: float
    basis_source: str
    config: MixConfig | None
    run: RunCounts
    mapping: list[dict[str, Any]]
    bases: list[BasisResult]
    unlabelled_sessions: int

    @property
    def failures(self) -> list[str]:
        """The failure sentences of every basis."""
        return [f for b in self.bases for f in b.failures]

    @property
    def passed(self) -> bool:
        """True when every basis passed."""
        return bool(self.bases) and all(b.passed for b in self.bases)

    @property
    def metrics_basis(self) -> str | None:
        """The basis that the metric series describe, or None when it cannot be told.

        The catalog says that ``mix.share_pct`` is by sessions or by bytes according to
        ``profile.basis`` and has no label for the basis, so one report carries one basis. It is
        the ``profile.basis`` of the test configuration when the check ran it, and the only basis
        when the check ran just one. Weights in the labels and both bases leave it open, and the
        report then carries no series, because two values would share one set of labels.
        """
        ran = [b.basis for b in self.bases]
        if self.config is not None and self.config.basis in ran:
            return self.config.basis
        return ran[0] if len(ran) == 1 else None

    def metrics(self) -> list[dict[str, Any]]:
        """Return the catalog series of what the classifier saw, in the form of the report.

        Each application gives ``mix.share_pct`` (the achieved share in percent) and
        ``mix.abs_error_pp`` (its distance from the configured share in percentage points),
        and the run gives ``mix.max_abs_error_pp``, all with ``source`` ``classifier`` and the
        classifier label. An application whose share the capture cannot give (it has no
        sessions or bytes to count) has no series. The row of the unrecognised traffic is not an
        application and has none either: it lowers the shares of the others.
        """
        basis = next((b for b in self.bases if b.basis == self.metrics_basis), None)
        if basis is None:
            return []
        labels = {"source": SOURCE_CLASSIFIER, "classifier": self.classifier}
        rows = [r for r in basis.rows if r.ndpi is not None]
        shares = [
            {
                "name": SHARE_METRIC,
                "labels": {"application": r.application, **labels},
                "value": _pct(r.ndpi),
            }
            for r in rows
        ]
        if not rows:
            return []
        distances = [round(abs(_delta(r.ndpi, r.configured) or 0.0), 6) for r in rows]
        errors = [
            {
                "name": ERROR_METRIC,
                "labels": {"application": r.application, **labels},
                "value": distance,
            }
            for r, distance in zip(rows, distances, strict=True)
        ]
        peak = {"name": MAX_ERROR_METRIC, "labels": labels, "value": max(distances)}
        return [*shares, *errors, peak]

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        failures = self.failures
        return {
            "check": CHECK_NAME,
            "metrics": self.metrics(),
            "metrics_basis": self.metrics_basis,
            "passed": self.passed,
            "classifier": self.classifier,
            "classifier_version": self.classifier_version,
            "tolerance_points": self.tolerance_points,
            "basis_source": self.basis_source,
            "test_config": (
                {"file": self.config.source, "basis": self.config.basis} if self.config else None
            ),
            "run_report": {"file": self.run.source, "form": self.run.form},
            "mapping": self.mapping,
            "unlabelled_sessions": self.unlabelled_sessions,
            "bases": [_basis_entry(b) for b in self.bases],
            "failures_total": len(failures),
            "failures": failures[:MAX_FAILURES_LISTED],
        }


def _pct(value: Fraction | None) -> float | None:
    return None if value is None else round(float(value * 100), 6)


def _delta(value: Fraction | None, configured: Fraction) -> float | None:
    return None if value is None else round(float((value - configured) * 100), 6)


def _row_entry(row: Row) -> dict[str, Any]:
    return {
        "application": row.application,
        "scenarios": row.scenarios,
        "configured_pct": _pct(row.configured),
        "metrics_pct": _pct(row.metrics),
        "ndpi_pct": _pct(row.ndpi),
        "metrics_delta_pp": _delta(row.metrics, row.configured),
        "ndpi_delta_pp": _delta(row.ndpi, row.configured),
        "metrics_count": row.metrics_count,
        "ndpi_count": row.ndpi_count,
        "metrics_passed": row.metrics_ok,
        "ndpi_passed": row.ndpi_ok,
        "passed": row.passed,
        "reasons": row.reasons,
    }


def _basis_entry(result: BasisResult) -> dict[str, Any]:
    return {
        "basis": result.basis,
        "passed": result.passed,
        "metrics_total": result.metrics_total,
        "ndpi_total": result.ndpi_total,
        "applications": [_row_entry(r) for r in result.rows],
        "unmatched": {
            **_row_entry(result.unmatched),
            "observed": dict(sorted(result.unmatched_observed.items())),
        },
        "failures": result.failures,
    }


@dataclass(frozen=True)
class _App:
    name: str
    scenarios: tuple[str, ...]
    accepted: tuple[str, ...]


def _applications(
    labels: Labels, weights: Mapping[str, float], classifier: str
) -> tuple[list[_App], dict[str, str]]:
    """Group the configured scenarios by application and resolve their nDPI names."""
    section = labels.mix
    if section is None or not section.scenarios:
        raise MixError(
            "the labels file has no mix.scenarios section, which must map each scenario of "
            "the mix to its application and so to its nDPI protocols"
        )
    grouped: dict[str, list[str]] = {}
    override: dict[str, tuple[str, ...]] = {}
    for scenario in weights:
        key = next((k for k in section.scenarios if same_scenario(scenario, k)), None)
        if key is None:
            raise MixError(
                f"scenario {scenario} of the mix has no entry in mix.scenarios of the labels"
            )
        entry = section.scenarios[key]
        grouped.setdefault(entry.application, []).append(scenario)
        if entry.ndpi is not None:
            override.setdefault(entry.application, entry.ndpi)
            if override[entry.application] != entry.ndpi:
                raise MixError(
                    f"application {entry.application}: its scenarios list different ndpi names"
                )
    apps: list[_App] = []
    for name, scenarios in grouped.items():
        accepted = override.get(name) or labels.accepted_names(name, classifier)
        if not accepted:
            raise MixError(
                f"application {name} has no {classifier} protocol names: add "
                f"applications.{name}.{classifier} or an ndpi list in mix.scenarios"
            )
        apps.append(_App(name, tuple(scenarios), tuple(accepted)))
    for index, first in enumerate(apps):
        for second in apps[index + 1 :]:
            shared = {n.casefold() for n in first.accepted} & {
                n.casefold() for n in second.accepted
            }
            if shared:
                raise MixError(
                    f"applications {first.name} and {second.name} both accept the "
                    f"{classifier} name {sorted(shared)[0]}, so a session cannot be assigned"
                )
    return apps, {s: a.name for a in apps for s in a.scenarios}


def _scope(labels: Labels, sessions: Sequence[Session]) -> tuple[list[Session], int]:
    if not labels.rules and labels.default is None:
        return list(sessions), 0
    kept = [s for s in sessions if labels.application_of(s) is not None]
    return kept, len(sessions) - len(kept)


def _tolerance(points: float | None, labels: Labels) -> float:
    if points is None and labels.mix is not None:
        points = labels.mix.tolerance_points
    value = DEFAULT_TOLERANCE_POINTS if points is None else points
    if not 0 <= value <= 100:
        raise MixError("the mix tolerance must be from 0 to 100 percentage points")
    return float(value)


def _bases(
    requested: str | None, labels: Labels, config: MixConfig | None
) -> tuple[list[str], str]:
    section = labels.mix
    choice, source = requested, "command line"
    if choice is None and section is not None and section.basis is not None:
        choice, source = section.basis, "labels"
    if choice is None and config is not None and config.basis is not None:
        choice, source = config.basis, "test configuration"
    if choice is None:
        raise MixError(
            "no basis: the test configuration has profile.basis, or give mix.basis in the "
            "labels or --mix-basis"
        )
    if choice == "both":
        return list(BASES), source
    if choice not in BASES:
        raise MixError(f"basis must be sessions, bytes, or both, found {choice!r}")
    return [choice], source


def _weights(
    labels: Labels, config_path: Path | None
) -> tuple[MixConfig | None, Mapping[str, float]]:
    section = labels.mix
    if config_path is None and section is not None and section.test_config is not None:
        config_path = (labels.base_dir or Path.cwd()) / section.test_config
    if config_path is not None:
        config = load_config(config_path)
        return config, config.weights
    if section is not None and section.weights:
        return None, section.weights
    raise MixError(
        "no configured weights: give --test-config, or mix.test_config or mix.weights in the labels"
    )


def _run_path(labels: Labels, given: Path | None) -> Path:
    if given is not None:
        return given
    section = labels.mix
    if section is not None and section.run_report is not None:
        return (labels.base_dir or Path.cwd()) / section.run_report
    raise MixError("no run report: give --run-report, or mix.run_report in the labels")


def _points(value: Fraction) -> str:
    return f"{float(value * 100):.2f}"


def _judge(row: Row, source: str, basis: str, tolerance: Fraction, failures: list[str]) -> bool:
    achieved = row.metrics if source == "metrics" else row.ndpi
    if achieved is None:
        return True
    delta = achieved - row.configured
    if abs(delta) <= tolerance:
        return True
    count = row.metrics_count if source == "metrics" else row.ndpi_count
    origin = "the run metrics" if source == "metrics" else "nDPI"
    direction = "above" if delta > 0 else "below"
    reason = (
        f"{row.application}: the {basis} share from {origin} is {_points(achieved)}% "
        f"({count} {'sessions' if basis == 'sessions' else 'bytes'}), "
        f"{_points(abs(delta))} points {direction} the configured {_points(row.configured)}% "
        f"(tolerance {_points(tolerance)} points)"
    )
    row.reasons.append(reason)
    failures.append(f"{basis} basis, {reason}")
    return False


def check(
    capture: Path,
    sessions: Sequence[Session],
    labels: Labels,
    classifier: Classifier,
    *,
    config_path: Path | None = None,
    run_report: Path | None = None,
    tolerance_points: float | None = None,
    basis: str | None = None,
) -> MixResult:
    """Compare the achieved mix, from run metrics and from nDPI, with the configured weights.

    The weights come from ``config_path`` or from the ``mix`` section of the labels, the achieved
    counts from ``run_report`` or the labels, and the basis from ``basis``, the labels, or the
    test configuration, in that order. An input that cannot be read or does not fit raises
    :class:`MixError`.
    """
    config, weights = _weights(labels, config_path)
    run = load_run_counts(_run_path(labels, run_report))
    tolerance_value = _tolerance(tolerance_points, labels)
    tolerance = Fraction(str(tolerance_value)) / 100
    bases, basis_source = _bases(basis, labels, config)
    apps, owner = _applications(labels, weights, classifier.name)
    total_weight = sum((Fraction(str(w)) for w in weights.values()), Fraction(0))

    for basis_name in bases:
        if basis_name == "bytes":
            missing = [k for k, c in run.scenarios.items() if c.bytes is None]
            if missing:
                raise MixError(
                    f"run report {run.source}: scenario {missing[0]} has no {BYTES_METRICS[0]} "
                    f"and {BYTES_METRICS[1]}, which the bytes basis needs"
                )

    verdicts = classifier.classify(capture, sessions)
    scope, unlabelled = _scope(labels, sessions)
    # Metrics counts per application, and the scenarios that the configuration does not list.
    metrics_by_app: dict[str, Counts] = {a.name: Counts(0, 0) for a in apps}
    extra: list[tuple[str, Counts]] = []
    for key, counts in run.scenarios.items():
        name = next((owner[s] for s in weights if same_scenario(s, key)), None)
        if name is None:
            extra.append((key, counts))
            continue
        held = metrics_by_app[name]
        metrics_by_app[name] = Counts(
            held.sessions + counts.sessions,
            None if held.bytes is None or counts.bytes is None else held.bytes + counts.bytes,
        )
    ndpi_sessions: Counter[str] = Counter()
    ndpi_bytes: Counter[str] = Counter()
    unmatched_observed: Counter[str] = Counter()
    for session in scope:
        observed = verdicts.get(session.key, UNKNOWN)
        name = next((a.name for a in apps if matches(observed, a.accepted)), UNMATCHED)
        ndpi_sessions[name] += 1
        ndpi_bytes[name] += session.payload_bytes
        if name == UNMATCHED:
            unmatched_observed[observed] += 1

    results: list[BasisResult] = []
    for basis_name in bases:
        failures: list[str] = []
        is_bytes = basis_name == "bytes"
        run_total = sum(
            (c.bytes or 0) if is_bytes else c.sessions
            for c in [*metrics_by_app.values(), *(c for _, c in extra)]
        )
        ndpi_counts = ndpi_bytes if is_bytes else ndpi_sessions
        ndpi_total = sum(ndpi_counts.values())
        unit = "bytes" if is_bytes else "sessions"
        if run_total == 0:
            failures.append(f"{basis_name} basis: the run report counts no {unit}")
        if ndpi_total == 0:
            failures.append(
                f"{basis_name} basis: the capture holds no {unit} to count for nDPI"
                + (" among the labelled sessions" if labels.rules or labels.default else "")
            )
        for key, counts in extra:
            unlisted = (counts.bytes or 0) if is_bytes else counts.sessions
            if unlisted:
                failures.append(
                    f"{basis_name} basis, scenario {key}: the run report counts {unlisted} {unit} "
                    "for a scenario that the configured mix does not list"
                )
        rows: list[Row] = []
        for app in apps:
            configured = (
                sum((Fraction(str(weights[s])) for s in app.scenarios), Fraction(0)) / total_weight
            )
            held = metrics_by_app[app.name]
            metrics_count = (held.bytes or 0) if is_bytes else held.sessions
            row = Row(
                application=app.name,
                scenarios=list(app.scenarios),
                configured=configured,
                metrics=Fraction(metrics_count, run_total) if run_total else None,
                ndpi=Fraction(ndpi_counts[app.name], ndpi_total) if ndpi_total else None,
                metrics_count=metrics_count,
                ndpi_count=ndpi_counts[app.name],
            )
            row.metrics_ok = row.metrics is not None
            row.ndpi_ok = row.ndpi is not None
            if row.metrics is not None:
                row.metrics_ok = _judge(row, "metrics", basis_name, tolerance, failures)
            if row.ndpi is not None:
                row.ndpi_ok = _judge(row, "ndpi", basis_name, tolerance, failures)
            rows.append(row)
        other = Row(
            application=UNMATCHED,
            scenarios=[],
            configured=Fraction(0),
            metrics=None,
            ndpi=Fraction(ndpi_counts[UNMATCHED], ndpi_total) if ndpi_total else None,
            metrics_count=None,
            ndpi_count=ndpi_counts[UNMATCHED],
        )
        other.ndpi_ok = other.ndpi is None or _judge(other, "ndpi", basis_name, tolerance, failures)
        if not other.ndpi_ok:
            seen = ", ".join(f"{n} {c}" for n, c in unmatched_observed.most_common(5))
            failures[-1] += f"; nDPI named them {seen}"
        results.append(
            BasisResult(
                basis_name,
                rows,
                run_total,
                ndpi_total,
                other,
                unmatched_observed,
                failures,
            )
        )
    return MixResult(
        classifier=classifier.name,
        classifier_version=classifier.version(),
        tolerance_points=tolerance_value,
        basis_source=basis_source,
        config=config,
        run=run,
        mapping=[
            {"application": a.name, "scenarios": list(a.scenarios), "ndpi": list(a.accepted)}
            for a in apps
        ],
        bases=results,
        unlabelled_sessions=unlabelled,
    )
