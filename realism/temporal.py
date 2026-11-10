# SPDX-License-Identifier: GPL-3.0-only
"""Temporal distribution check of the realism runner.

Real users arrive at random, think for a while, and fetch objects of very different
sizes. A generator that opens sessions at a fixed pace, or whose object sizes cluster
around one value, does not look like the population that it imitates. The check
compares two properties of the capture with the target distributions of the labels,
through the Kolmogorov Smirnov (KS) test of :mod:`realism.ks`.

**Properties.**

* ``inter_arrival``: the gaps in seconds between the first packets of consecutive
  sessions, in order of those first packets. The target ``applications`` entry takes the
  sessions that the labels name as that application, and ``overall`` takes every session
  of the capture, labelled or not. A Poisson arrival process of rate ``r`` sessions per
  second has exponential gaps with ``mean_ms`` of ``1000 / r``.
* ``payload_size``: the size in bytes of each payload unit of :mod:`realism.content` (an
  HTTP message, a UDP datagram, or one whole direction of a TCP session), for one
  application or for all, and for the client direction, the server direction, or both.
  TLS sessions have no plaintext here and are skipped as in the content check.

**Targets.** A parametric target is ``constant``, ``exponential``, ``uniform``,
``lognormal``, or ``pareto``, written with the parameter names of a scenario ``delay``
for arrivals (``mean_ms``, ``min_ms``, ``max_ms``, ``mu`` and ``sigma`` with a ``unit``,
``alpha`` and ``scale_ms``) and in bytes for sizes. An ``empirical`` target is a list of
values, a file of values, or a reference capture, and the check then runs the two sample
test against that sample.

**Decision.** With significance level ``alpha`` (default 0.01) a comparison passes when
the p value is at least ``alpha``, and fails with the statistic, the p value, and the
sample size in its reason otherwise. A comparison with fewer than ``min_samples``
observations (default 30) in any of its samples reports ``insufficient_data`` and fails
the check, because a pass on a handful of points would prove nothing. The check also
fails when the labels name no target at all.

A constant target has no spread, so the test is exact: the observations must all lie
within the tolerance of the value, and the statistic is the larger share of observations
below or above that band.

Each comparison is a test at level ``alpha``, so a capture with many targets rejects
about ``alpha`` of the correct ones by chance. Byte sizes are discrete, which makes the
p value against a continuous target conservative. The timestamps of a capture have the
resolution of the file (microseconds for most captures), which creates zero gaps for
sessions that start in the same tick.
"""

from __future__ import annotations

import math
import re
import statistics
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass, field
from itertools import pairwise
from pathlib import Path
from typing import Any

from realism import ks
from realism.capture import Session, summarize
from realism.content import IGNORED_HEADERS, Unit, collect_units, read_flows
from realism.labels import (
    Distribution,
    Labels,
    LabelsError,
    TemporalLabels,
    TemporalTarget,
)
from realism.labels import load as load_labels

CHECK_NAME = "temporal"
PROP_ARRIVAL = "inter_arrival"
PROP_SIZE = "payload_size"
STATUS_PASS = "pass"
STATUS_FAIL = "fail"
STATUS_INSUFFICIENT = "insufficient_data"
# The catalog property label of a size target, by direction. Sizes of both directions
# have no catalog property, so they carry no metric.
_SIZE_METRIC_PROPERTY = {"client": "request_bytes", "server": "response_bytes"}
_NUMBER_SPLIT = re.compile(r"[\s,;]+")


class _Source:
    """A capture with its sessions and labels, which gives the samples of a target."""

    def __init__(self, capture: Path, sessions: Sequence[Session], labels: Labels) -> None:
        self.capture = capture
        self.sessions = sessions
        self.labels = labels
        self._units: list[Unit] | None = None
        self.skipped_encrypted = 0

    def arrivals(self, application: str | None) -> list[float]:
        """Return the gaps between the first packets of consecutive sessions, in seconds."""
        starts = sorted(
            s.first_seen
            for s in self.sessions
            if application is None or self.labels.application_of(s) == application
        )
        return [later - earlier for earlier, later in pairwise(starts)]

    def sizes(self, application: str | None, direction: str) -> list[float]:
        """Return the payload sizes in bytes of one application, or of all."""
        if self._units is None:
            units, stats = collect_units(
                self.sessions,
                read_flows(self.capture, self.sessions),
                self.labels,
                IGNORED_HEADERS,
            )
            self._units = units
            self.skipped_encrypted = stats.skipped_encrypted
        return [
            float(u.size)
            for u in self._units
            if (application is None or u.application == application)
            and direction in ("both", u.direction)
        ]

    def samples(self, target: TemporalTarget) -> list[float]:
        """Return the observations of a target."""
        if target.prop == PROP_ARRIVAL:
            return self.arrivals(target.application)
        return self.sizes(target.application, target.direction)


def cdf_of(dist: Distribution) -> Callable[[float], float]:
    """Return the cumulative distribution function of a parametric distribution."""
    p = dist.params
    if dist.kind == "exponential":
        mean = p["mean"]
        return lambda x: 0.0 if x <= 0 else -math.expm1(-x / mean)
    if dist.kind == "uniform":
        low, high = p["min"], p["max"]
        return lambda x: min(1.0, max(0.0, (x - low) / (high - low)))
    if dist.kind == "lognormal":
        mu, sigma = p["mu"], p["sigma"]
        return lambda x: (
            0.0 if x <= 0 else 0.5 * (1.0 + math.erf((math.log(x) - mu) / (sigma * math.sqrt(2.0))))
        )
    if dist.kind == "pareto":
        alpha, scale = p["alpha"], p["scale"]
        return lambda x: 0.0 if x < scale else 1.0 - (scale / x) ** alpha
    raise ValueError(f"{dist.kind} has no cumulative distribution function")


def target_median(dist: Distribution) -> float | None:
    """Return the median of a parametric distribution, or None for an empirical one."""
    p = dist.params
    if dist.kind == "constant":
        return p["value"]
    if dist.kind == "exponential":
        return p["mean"] * math.log(2.0)
    if dist.kind == "uniform":
        return (p["min"] + p["max"]) / 2.0
    if dist.kind == "lognormal":
        return math.exp(p["mu"])
    if dist.kind == "pareto":
        return float(p["scale"] * 2.0 ** (1.0 / p["alpha"]))
    return None


def _read_sample_file(path: Path, where: str) -> list[float]:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise LabelsError(f"{where}: read sample file {path}: {exc}") from exc
    values: list[float] = []
    for word in _NUMBER_SPLIT.split(text.strip()):
        if not word:
            continue
        try:
            value = float(word)
        except ValueError as exc:
            raise LabelsError(f"{where}: sample file {path} holds {word!r}, not a number") from exc
        if not math.isfinite(value):
            raise LabelsError(f"{where}: sample file {path} holds {word!r}, not a finite number")
        values.append(value)
    return values


def _describe(sample: Sequence[float]) -> dict[str, float] | None:
    if not sample:
        return None
    return {
        "min": round(min(sample), 6),
        "median": round(statistics.median(sample), 6),
        "mean": round(statistics.fmean(sample), 6),
        "max": round(max(sample), 6),
    }


def _name(target: TemporalTarget) -> str:
    who = "all sessions" if target.application is None else f"application {target.application}"
    if target.prop == PROP_ARRIVAL:
        return f"inter-arrival times of {who}"
    side = "" if target.direction == "both" else f" ({target.direction})"
    return f"payload sizes{side} of {who}"


def _unit_text(target: TemporalTarget, value: float) -> str:
    return f"{value:.6g} s" if target.prop == PROP_ARRIVAL else f"{value:.6g} bytes"


def _target_text(target: TemporalTarget) -> str:
    dist = target.distribution
    if dist.kind == "empirical":
        return "the empirical target sample"
    params = ", ".join(f"{k} {v:.6g}" for k, v in dist.params.items())
    return f"{dist.kind} ({params})"


@dataclass
class TemporalResult:
    """The outcome of the temporal distribution check."""

    alpha: float
    min_samples: int
    sessions_total: int
    skipped_encrypted: int
    comparisons: list[dict[str, Any]] = field(default_factory=list)
    failures: list[str] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        """True when there is a comparison and every one of them passed."""
        return bool(self.comparisons) and not self.failures

    def metrics(self) -> list[dict[str, Any]]:
        """Return the KS statistic and p value of each comparison that the catalog names.

        The catalog labels a metric with an application and a property. Comparisons over
        all sessions, and sizes of both directions, have no such label and stay out.
        """
        out: list[dict[str, Any]] = []
        for c in self.comparisons:
            if c["application"] is None or c["statistic"] is None:
                continue
            prop = (
                "inter_arrival"
                if c["property"] == PROP_ARRIVAL
                else _SIZE_METRIC_PROPERTY.get(c["direction"])
            )
            if prop is None:
                continue
            labels = {"application": c["application"], "property": prop}
            out.append({"name": "realism.ks_statistic", "labels": labels, "value": c["statistic"]})
            out.append({"name": "realism.ks_p_value", "labels": labels, "value": c["p_value"]})
        return out

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        return {
            "check": CHECK_NAME,
            "metrics": self.metrics(),
            "passed": self.passed,
            "alpha": self.alpha,
            "min_samples": self.min_samples,
            "sessions_total": self.sessions_total,
            "skipped_encrypted_sessions": self.skipped_encrypted,
            "comparisons": self.comparisons,
            "insufficient_comparisons": sum(
                1 for c in self.comparisons if c["status"] == STATUS_INSUFFICIENT
            ),
            "failures": self.failures,
        }


def check(
    capture: Path,
    sessions: Sequence[Session],
    labels: Labels,
    alpha: float | None = None,
    min_samples: int | None = None,
) -> TemporalResult:
    """Compare inter arrival times and payload sizes with the targets of the labels.

    ``alpha`` and ``min_samples`` override the values of the ``temporal`` section, whose
    defaults are 0.01 and 30. Paths of reference captures and sample files resolve against
    the directory of the labels file.
    """
    settings = labels.temporal or TemporalLabels()
    level = settings.alpha if alpha is None else alpha
    minimum = settings.min_samples if min_samples is None else min_samples
    main = _Source(capture, sessions, labels)
    references: dict[tuple[str, str | None], _Source] = {}
    comparisons = [
        _compare(main, target, level, minimum, references) for target in settings.targets
    ]
    failures = [c["reason"] for c in comparisons if c["status"] != STATUS_PASS]
    if not comparisons:
        failures.append(
            "the labels give no target distribution: add a temporal section with arrivals or sizes"
        )
    return TemporalResult(
        alpha=level,
        min_samples=minimum,
        sessions_total=len(sessions),
        skipped_encrypted=main.skipped_encrypted,
        comparisons=comparisons,
        failures=failures,
    )


def _compare(
    main: _Source,
    target: TemporalTarget,
    alpha: float,
    minimum: int,
    references: dict[tuple[str, str | None], _Source],
) -> dict[str, Any]:
    dist = target.distribution
    observed = main.samples(target)
    name = _name(target)
    reference: list[float] | None = None
    reference_source: str | None = None
    if dist.kind == "empirical":
        reference, reference_source = _reference_sample(main, target, references)
    entry: dict[str, Any] = {
        "property": target.prop,
        "application": target.application,
        "direction": target.direction if target.prop == PROP_SIZE else None,
        "unit": "seconds" if target.prop == PROP_ARRIVAL else "bytes",
        "target": {
            "dist": dist.kind,
            "parameters": dict(dist.params),
            "median": target_median(dist),
            "source": reference_source,
        },
        "test": "ks_two_sample" if reference is not None else "ks_one_sample",
        "method": None,
        "alpha": alpha,
        "n": len(observed),
        "reference_n": None if reference is None else len(reference),
        "statistic": None,
        "p_value": None,
        "status": STATUS_INSUFFICIENT,
        "reason": None,
        "observed": _describe(observed),
    }
    short = None
    if len(observed) < minimum:
        short = f"{len(observed)} observations in the capture"
    elif reference is not None and len(reference) < minimum:
        short = f"{len(reference)} observations in the reference"
    if short is not None:
        entry["reason"] = (
            f"{name}: insufficient data, {short} and at least {minimum} are needed "
            "(temporal.min_samples), so the distribution is not judged"
        )
        return entry
    if reference is not None:
        result = ks.ks_two_sample(observed, reference)
        entry["test"] = "ks_two_sample"
    elif dist.kind == "constant":
        result = ks.ks_one_sample_constant(observed, dist.params["value"], dist.tolerance)
        entry["test"] = "constant"
    else:
        result = ks.ks_one_sample(observed, cdf_of(dist))
    entry["method"] = result.method
    entry["statistic"] = round(result.statistic, 6)
    entry["p_value"] = result.p_value
    passed = result.p_value >= alpha
    entry["status"] = STATUS_PASS if passed else STATUS_FAIL
    if not passed:
        entry["reason"] = _failure_reason(target, entry, result)
    return entry


def _failure_reason(target: TemporalTarget, entry: Mapping[str, Any], result: ks.KsResult) -> str:
    name = _name(target)
    seen = entry["observed"]
    if target.distribution.kind == "constant":
        value = target.distribution.params["value"]
        return (
            f"{name}: {result.statistic:.1%} of {result.n} observations are further than "
            f"{_unit_text(target, target.distribution.tolerance)} from the constant "
            f"{_unit_text(target, value)}; the observed median is "
            f"{_unit_text(target, seen['median'])}"
        )
    reference = ""
    if result.m is not None:
        reference = f" against {result.m} reference observations"
    expected = ""
    median = entry["target"]["median"]
    if median is not None:
        expected = f", the target median is {_unit_text(target, median)}"
    return (
        f"{name}: the KS test rejects {_target_text(target)} at alpha {entry['alpha']:g} "
        f"(statistic {result.statistic:.4f}, p value {result.p_value:.3g}, n {result.n}"
        f"{reference}); the observed median is {_unit_text(target, seen['median'])}{expected}"
    )


def _reference_sample(
    main: _Source,
    target: TemporalTarget,
    references: dict[tuple[str, str | None], _Source],
) -> tuple[list[float], str]:
    """Return the target sample of an empirical distribution and where it came from."""
    dist = target.distribution
    where = f"labels temporal ({_name(target)})"
    if dist.sample is not None:
        return list(dist.sample), "sample"
    base = main.labels.base_dir or Path()
    if dist.sample_file is not None:
        return _read_sample_file(base / dist.sample_file, where), "sample_file"
    assert dist.reference is not None
    key = (dist.reference.pcap, dist.reference.labels)
    source = references.get(key)
    if source is None:
        pcap = base / dist.reference.pcap
        labels = load_labels(base / dist.reference.labels) if dist.reference.labels else main.labels
        source = _Source(pcap, summarize(pcap).sessions, labels)
        references[key] = source
    return source.samples(target), "reference"
