# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the temporal distribution check: arrivals, sizes, targets, and the report seam.

Every capture is built in code with ``pcapgen``. Arrival times and sizes come from seeded
generators, so a run is repeatable, and the shifted captures differ from the targets by a
change that a user would notice, such as a 75% longer mean gap.
"""

from __future__ import annotations

import json
import math
import random
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import Any

import jsonschema
import pytest
import yaml

from realism import temporal
from realism.capture import CaptureError, summarize
from realism.classifier import StubClassifier
from realism.cli import EXIT_ERROR, EXIT_FAIL, EXIT_PASS, main
from realism.labels import Labels, LabelsError, parse
from realism.report import LABELS_SCHEMA_PATH, SCHEMA_PATH, score
from realism.tests import pcapgen
from realism.tests.conftest import NDPI_42_HEADER

CATALOG = Path(__file__).resolve().parents[2] / "contracts" / "metrics" / "catalog.yaml"
HTTP_SERVER = ("10.0.1.80", 80)
DNS_SERVER = ("10.0.1.53", 53)
BULK_SERVER = ("10.0.1.90", 9000)
SESSIONS = [
    {"application": "http1", "match": {"transport": "tcp", "server_port": 80}},
    {"application": "dns", "match": {"transport": "udp", "server_port": 53}},
    {"application": "bulk", "match": {"transport": "tcp", "server_port": 9000}},
]


def _labels(temporal_section: dict[str, Any] | None, **extra: Any) -> Labels:
    doc: dict[str, Any] = {"labels": "fps-realism-labels", "version": 1, "sessions": SESSIONS}
    if temporal_section is not None:
        doc["temporal"] = temporal_section
    return parse(doc | extra)


def _poisson(rng: random.Random, rate: float, count: int) -> list[float]:
    """Arrival times of a Poisson process of ``rate`` sessions per second."""
    t, out = 0.0, []
    for _ in range(count):
        t += rng.expovariate(rate)
        out.append(t)
    return out


def _cumulative(gaps: Sequence[float]) -> list[float]:
    t, out = 0.0, []
    for gap in gaps:
        t += gap
        out.append(t)
    return out


def _at(http: Sequence[float] = (), dns: Sequence[float] = ()) -> pcapgen.Capture:
    """A capture whose HTTP sessions and DNS lookups start at the given seconds."""
    cap = pcapgen.Capture()
    base = cap.clock
    for i, t in enumerate(http):
        cap.clock = base + t - 0.001
        cap.http_get(("10.0.0.10", 20000 + i), HTTP_SERVER)
    for i, t in enumerate(dns):
        cap.clock = base + t - 0.001
        cap.dns_lookup(("10.0.0.11", 20000 + i), DNS_SERVER, i & 0xFFFF)
    cap.frames.sort(key=lambda frame: frame[0])
    return cap


def _bulk(server: Sequence[int], client: Sequence[int] | None = None) -> pcapgen.Capture:
    """TCP sessions to port 9000 whose request and response have the given sizes."""
    cap = pcapgen.Capture()
    for i, size in enumerate(server):
        request = b"q" * (client[i] if client is not None else 120)
        cap.tcp_session(("10.0.0.10", 30000 + i), BULK_SERVER, request, b"r" * size)
    return cap


def _run(
    tmp_path: Path, cap: pcapgen.Capture, labels: Labels, **options: Any
) -> temporal.TemporalResult:
    pcap = cap.write_pcap(tmp_path / "t.pcap")
    return temporal.check(pcap, summarize(pcap).sessions, labels, **options)


def _gaps_exp(seed: int, mean_s: float, count: int) -> list[float]:
    return _poisson(random.Random(seed), 1.0 / mean_s, count)


def _sizes(seed: int, count: int, draw: Callable[[random.Random], float]) -> list[int]:
    rng = random.Random(seed)
    return [min(60000, max(1, round(draw(rng)))) for _ in range(count)]


# --- acceptance: samples from the target pass, shifted samples fail --------------------------

EXPONENTIAL_200MS = {
    "arrivals": {"applications": {"http1": {"dist": "exponential", "mean_ms": 200}}}
}


def test_poisson_arrivals_from_the_target_pass(tmp_path: Path) -> None:
    result = _run(tmp_path, _at(http=_gaps_exp(11, 0.2, 201)), _labels(EXPONENTIAL_200MS))
    assert result.passed, result.failures
    (comparison,) = result.comparisons
    assert comparison["property"] == "inter_arrival"
    assert comparison["application"] == "http1"
    assert comparison["n"] == 200
    assert comparison["test"] == "ks_one_sample"
    assert comparison["status"] == "pass"
    assert comparison["p_value"] > 0.05
    assert comparison["reason"] is None
    assert comparison["observed"]["mean"] == pytest.approx(0.2, rel=0.2)


def test_arrivals_with_a_longer_mean_gap_fail_with_a_reason(tmp_path: Path) -> None:
    result = _run(tmp_path, _at(http=_gaps_exp(11, 0.35, 201)), _labels(EXPONENTIAL_200MS))
    assert not result.passed
    (comparison,) = result.comparisons
    assert comparison["status"] == "fail"
    assert comparison["p_value"] < 0.01
    reason = result.failures[0]
    assert reason.startswith("inter-arrival times of application http1: the KS test rejects")
    assert "exponential (mean 0.2)" in reason
    assert "alpha 0.01" in reason
    assert f"statistic {comparison['statistic']:.4f}" in reason
    assert "n 200" in reason
    assert "p value" in reason
    assert "the target median is 0.138629 s" in reason


def test_arrivals_at_a_fixed_pace_fail_an_exponential_target(tmp_path: Path) -> None:
    """A generator that opens sessions on a metronome is not a Poisson process."""
    result = _run(
        tmp_path, _at(http=_cumulative([0.2] * 200)), _labels(EXPONENTIAL_200MS), min_samples=30
    )
    assert not result.passed
    assert result.comparisons[0]["statistic"] > 0.3


def test_per_application_and_overall_arrivals(tmp_path: Path) -> None:
    rng = random.Random(3)
    http, dns = _poisson(rng, 5.0, 150), _poisson(rng, 2.0, 80)
    labels = _labels(
        {
            "arrivals": {
                "overall": {"dist": "exponential", "mean_ms": 1000 / 7},
                "applications": {
                    "http1": {"dist": "exponential", "mean_ms": 200},
                    "dns": {"dist": "exponential", "mean_ms": 500},
                },
            }
        }
    )
    result = _run(tmp_path, _at(http, dns), labels)
    assert result.passed, result.failures
    by_app = {c["application"]: c for c in result.comparisons}
    assert by_app[None]["n"] == 229
    assert by_app["http1"]["n"] == 149
    assert by_app["dns"]["n"] == 79
    # A DNS stream at twice the target gap fails on its own while the others still pass.
    slow = _run(tmp_path, _at(http, _poisson(random.Random(4), 1.0, 80)), labels)
    failed = [c["application"] for c in slow.comparisons if c["status"] == "fail"]
    assert "dns" in failed
    assert "http1" not in failed


@pytest.mark.parametrize(
    ("spec", "gap"),
    [
        ({"dist": "uniform", "min_ms": 50, "max_ms": 350}, lambda r: r.uniform(0.05, 0.35)),
        ({"dist": "lognormal", "mu": -1.8, "sigma": 0.5}, lambda r: r.lognormvariate(-1.8, 0.5)),
        (
            {"dist": "lognormal", "mu": -1.8 + math.log(1000), "sigma": 0.5, "unit": "ms"},
            lambda r: r.lognormvariate(-1.8, 0.5),
        ),
        (
            {"dist": "pareto", "alpha": 2.0, "scale_ms": 100},
            lambda r: 0.1 * r.paretovariate(2.0),
        ),
        ({"dist": "constant", "ms": 100, "tolerance_ms": 0.5}, lambda r: 0.1),
        ({"ms": 100, "tolerance_ms": 0.5}, lambda r: 0.1),
    ],
    ids=["uniform", "lognormal-s", "lognormal-ms", "pareto", "constant", "constant-delay-form"],
)
def test_every_distribution_passes_on_its_own_samples(
    tmp_path: Path, spec: dict[str, Any], gap: Callable[[random.Random], float]
) -> None:
    rng = random.Random(8)
    times = _cumulative([gap(rng) for _ in range(120)])
    labels = _labels({"arrivals": {"applications": {"http1": spec}}})
    result = _run(tmp_path, _at(http=times), labels)
    assert result.passed, result.failures
    assert result.comparisons[0]["status"] == "pass"


def test_a_shifted_lognormal_and_pareto_fail(tmp_path: Path) -> None:
    rng = random.Random(8)
    shifted = _cumulative([rng.lognormvariate(-1.3, 0.5) for _ in range(120)])
    labels = _labels(
        {"arrivals": {"applications": {"http1": {"dist": "lognormal", "mu": -1.8, "sigma": 0.5}}}}
    )
    assert not _run(tmp_path, _at(http=shifted), labels).passed
    pareto = _cumulative([0.2 * rng.paretovariate(2.0) for _ in range(120)])
    target = _labels(
        {"arrivals": {"applications": {"http1": {"dist": "pareto", "alpha": 2.0, "scale_ms": 100}}}}
    )
    assert not _run(tmp_path, _at(http=pareto), target).passed


def test_a_constant_gap_off_the_target_reports_the_share_that_misses(tmp_path: Path) -> None:
    labels = _labels({"arrivals": {"applications": {"http1": {"ms": 150, "tolerance_ms": 1}}}})
    result = _run(tmp_path, _at(http=_cumulative([0.1] * 60)), labels)
    (comparison,) = result.comparisons
    assert comparison["test"] == "constant"
    assert comparison["method"] == "constant"
    assert comparison["statistic"] == 1.0
    assert comparison["p_value"] == 0.0
    assert (
        "100.0% of 59 observations are further than 0.001 s from the constant 0.15 s"
        in (result.failures[0])
    )


# --- sizes --------------------------------------------------------------------------------

SERVER_SIZES = {
    "sizes": {
        "applications": {
            "bulk": {"direction": "server", "dist": "lognormal", "mu": 8.0, "sigma": 0.7}
        }
    }
}


def test_sizes_from_the_target_pass_and_shifted_sizes_fail(tmp_path: Path) -> None:
    good = _sizes(1, 200, lambda r: r.lognormvariate(8.0, 0.7))
    result = _run(tmp_path, _bulk(good), _labels(SERVER_SIZES))
    assert result.passed, result.failures
    (comparison,) = result.comparisons
    assert comparison["property"] == "payload_size"
    assert (comparison["direction"], comparison["unit"], comparison["n"]) == (
        "server",
        "bytes",
        200,
    )
    shifted = _sizes(1, 200, lambda r: r.lognormvariate(8.5, 0.7))
    failed = _run(tmp_path, _bulk(shifted), _labels(SERVER_SIZES))
    assert not failed.passed
    assert failed.failures[0].startswith(
        "payload sizes (server) of application bulk: the KS test rejects lognormal"
    )
    assert "the target median is 2980.96 bytes" in failed.failures[0]


def test_pareto_exponential_and_uniform_sizes(tmp_path: Path) -> None:
    pareto = _sizes(2, 200, lambda r: 500 * r.paretovariate(1.8))
    labels = _labels(
        {
            "sizes": {
                "applications": {
                    "bulk": {"direction": "server", "dist": "pareto", "alpha": 1.8, "scale": 500}
                }
            }
        }
    )
    assert _run(tmp_path, _bulk(pareto), labels).passed
    # The same data against a smaller scale is a shift of the whole distribution.
    wrong = _labels(
        {
            "sizes": {
                "applications": {
                    "bulk": {"direction": "server", "dist": "pareto", "alpha": 1.8, "scale": 300}
                }
            }
        }
    )
    assert not _run(tmp_path, _bulk(pareto), wrong).passed
    expo = _sizes(3, 200, lambda r: r.expovariate(1 / 4000))
    labels = _labels({"sizes": {"overall": {"dist": "exponential", "mean": 4000}}})
    result = _run(tmp_path, _bulk(expo), labels)
    # The overall target reads the request of 120 bytes of every session as well.
    assert not result.passed
    labels = _labels(
        {"sizes": {"overall": {"direction": "server", "dist": "exponential", "mean": 4000}}}
    )
    assert _run(tmp_path, _bulk(expo), labels).passed
    uniform = _sizes(4, 150, lambda r: r.uniform(1000, 9000))
    labels = _labels(
        {
            "sizes": {
                "applications": {
                    "bulk": {"direction": "server", "dist": "uniform", "min": 1000, "max": 9000}
                }
            }
        }
    )
    assert _run(tmp_path, _bulk(uniform), labels).passed


def test_each_direction_of_an_application_has_its_own_target(tmp_path: Path) -> None:
    server = _sizes(5, 100, lambda r: r.lognormvariate(8.0, 0.7))
    client = _sizes(6, 100, lambda r: r.lognormvariate(5.0, 0.4))
    labels = _labels(
        {
            "sizes": {
                "applications": {
                    "bulk": [
                        {"direction": "client", "dist": "lognormal", "mu": 5.0, "sigma": 0.4},
                        {"direction": "server", "dist": "lognormal", "mu": 8.0, "sigma": 0.7},
                    ]
                }
            }
        }
    )
    both = _run(tmp_path, _bulk(server, client), labels)
    assert both.passed, both.failures
    assert [c["direction"] for c in both.comparisons] == ["client", "server"]
    # A shifted request distribution fails the client comparison only.
    big = _sizes(6, 100, lambda r: r.lognormvariate(5.6, 0.4))
    result = _run(tmp_path, _bulk(server, big), labels)
    assert [c["status"] for c in result.comparisons] == ["fail", "pass"]


def test_a_constant_size_target_and_the_http_message_size(tmp_path: Path) -> None:
    labels = _labels(
        {
            "sizes": {
                "applications": {"bulk": {"direction": "client", "dist": "constant", "value": 120}}
            }
        }
    )
    assert _run(tmp_path, _bulk([500] * 40), labels).passed
    off = _run(tmp_path, _bulk([500] * 40, [130] * 40), labels)
    assert not off.passed
    assert (
        "of 40 observations are further than 0 bytes from the constant 120 bytes"
        in (off.failures[0])
    )
    # An HTTP unit is the whole message, headers included.
    cap = pcapgen.Capture()
    body = b"x" * 900
    response = b"HTTP/1.1 200 OK\r\nContent-Length: 900\r\n\r\n" + body
    for i in range(40):
        request = b"GET /a HTTP/1.1\r\nHost: h\r\n\r\n"
        cap.tcp_session(("10.0.0.10", 31000 + i), HTTP_SERVER, request, response)
    http = _labels(
        {
            "sizes": {
                "applications": {
                    "http1": {"direction": "server", "dist": "constant", "value": len(response)}
                }
            }
        }
    )
    assert _run(tmp_path, cap, http).passed


def test_tls_sessions_are_skipped_and_counted(tmp_path: Path) -> None:
    cap = _bulk(_sizes(1, 60, lambda r: r.lognormvariate(8.0, 0.7)))
    for i in range(5):
        hello = b"\x16\x03\x01\x00\x10" + b"h" * 16
        cap.tcp_session(("10.0.0.10", 33000 + i), BULK_SERVER, hello, b"\x16\x03\x03\x00\x04abcd")
    result = _run(tmp_path, cap, _labels(SERVER_SIZES))
    assert result.passed, result.failures
    assert result.to_dict()["skipped_encrypted_sessions"] == 5
    assert result.comparisons[0]["n"] == 60


# --- insufficient data ----------------------------------------------------------------------


def test_a_small_sample_reports_insufficient_data_and_fails(tmp_path: Path) -> None:
    result = _run(tmp_path, _at(http=_gaps_exp(11, 0.2, 11)), _labels(EXPONENTIAL_200MS))
    assert not result.passed
    (comparison,) = result.comparisons
    assert comparison["status"] == "insufficient_data"
    assert (comparison["statistic"], comparison["p_value"], comparison["method"]) == (
        None,
        None,
        None,
    )
    assert comparison["n"] == 10
    assert (
        "insufficient data, 10 observations in the capture and at least 30 are needed"
        in (result.failures[0])
    )
    entry = result.to_dict()
    assert entry["insufficient_comparisons"] == 1
    assert entry["metrics"] == []


def test_the_minimum_is_a_parameter_and_n_at_the_minimum_is_judged(tmp_path: Path) -> None:
    cap = _at(http=_gaps_exp(11, 0.2, 11))
    assert _run(tmp_path, cap, _labels(EXPONENTIAL_200MS), min_samples=10).comparisons[0][
        "status"
    ] in ("pass", "fail")
    assert (
        _run(tmp_path, cap, _labels(EXPONENTIAL_200MS), min_samples=11).comparisons[0]["status"]
        == "insufficient_data"
    )
    section = {"min_samples": 5, **EXPONENTIAL_200MS}
    assert _run(tmp_path, cap, _labels(section)).comparisons[0]["status"] != "insufficient_data"
    # The option beats the labels file.
    assert _run(tmp_path, cap, _labels(section), min_samples=50).comparisons[0]["status"] == (
        "insufficient_data"
    )


def test_an_application_without_sessions_is_insufficient_not_a_pass(tmp_path: Path) -> None:
    result = _run(
        tmp_path,
        _at(http=_gaps_exp(1, 0.2, 100)),
        _labels({"arrivals": {"applications": {"dns": {"dist": "exponential", "mean_ms": 500}}}}),
    )
    assert not result.passed
    assert result.comparisons[0]["n"] == 0
    assert result.comparisons[0]["observed"] is None
    assert "application dns: insufficient data, 0 observations" in result.failures[0]


def test_the_check_fails_when_the_labels_name_no_target(tmp_path: Path) -> None:
    result = _run(tmp_path, _at(http=_gaps_exp(1, 0.2, 50)), _labels({}))
    assert not result.passed
    assert result.comparisons == []
    assert "no target distribution" in result.failures[0]
    assert not _run(tmp_path, _at(http=_gaps_exp(1, 0.2, 50)), _labels(None)).passed


# --- alpha ------------------------------------------------------------------------------------


def test_alpha_is_a_parameter_with_a_default_of_one_percent(tmp_path: Path) -> None:
    cap = _at(http=_gaps_exp(21, 0.25, 101))
    labels = _labels(EXPONENTIAL_200MS)
    base = _run(tmp_path, cap, labels)
    assert base.alpha == 0.01
    p = base.comparisons[0]["p_value"]
    assert 0 < p < 1
    assert _run(tmp_path, cap, labels, alpha=p * 0.99).comparisons[0]["status"] == "pass"
    assert _run(tmp_path, cap, labels, alpha=min(0.99, p * 1.01)).comparisons[0]["status"] == "fail"
    from_labels = _run(tmp_path, cap, _labels({"alpha": min(0.99, p * 1.01), **EXPONENTIAL_200MS}))
    assert from_labels.comparisons[0]["status"] == "fail"
    assert from_labels.alpha == pytest.approx(min(0.99, p * 1.01))
    assert _run(tmp_path, cap, labels, alpha=0.5).comparisons[0]["alpha"] == 0.5


# --- empirical targets ------------------------------------------------------------------------


def test_an_empirical_sample_is_a_two_sample_target(tmp_path: Path) -> None:
    rng = random.Random(9)
    sample = [round(rng.expovariate(5.0), 6) for _ in range(150)]
    labels = _labels(
        {"arrivals": {"applications": {"http1": {"dist": "empirical", "sample": sample}}}}
    )
    result = _run(tmp_path, _at(http=_gaps_exp(12, 0.2, 121)), labels)
    assert result.passed, result.failures
    (comparison,) = result.comparisons
    assert (comparison["test"], comparison["reference_n"], comparison["n"]) == (
        "ks_two_sample",
        150,
        120,
    )
    assert comparison["target"] == {
        "dist": "empirical",
        "parameters": {},
        "median": None,
        "source": "sample",
    }
    failed = _run(tmp_path, _at(http=_gaps_exp(12, 0.4, 121)), labels)
    assert not failed.passed
    assert "the empirical target sample" in failed.failures[0]
    assert "against 150 reference observations" in failed.failures[0]


def test_an_empirical_sample_file(tmp_path: Path) -> None:
    rng = random.Random(10)
    (tmp_path / "gaps.txt").write_text(
        "\n".join(f"{rng.expovariate(5.0):.6f}" for _ in range(100)) + "\n"
    )
    labels = _labels(
        {"arrivals": {"applications": {"http1": {"dist": "empirical", "sample_file": "gaps.txt"}}}}
    )
    labels.base_dir = tmp_path
    result = _run(tmp_path, _at(http=_gaps_exp(12, 0.2, 101)), labels)
    assert result.passed, result.failures
    assert result.comparisons[0]["target"]["source"] == "sample_file"
    (tmp_path / "bad.txt").write_text("0.1 0.2 later\n")
    bad = _labels(
        {"arrivals": {"applications": {"http1": {"dist": "empirical", "sample_file": "bad.txt"}}}}
    )
    bad.base_dir = tmp_path
    with pytest.raises(LabelsError, match="holds 'later', not a number"):
        _run(tmp_path, _at(http=_gaps_exp(12, 0.2, 101)), bad)
    missing = _labels(
        {"arrivals": {"applications": {"http1": {"dist": "empirical", "sample_file": "none.txt"}}}}
    )
    missing.base_dir = tmp_path
    with pytest.raises(LabelsError, match="read sample file"):
        _run(tmp_path, _at(http=_gaps_exp(12, 0.2, 101)), missing)


def test_a_reference_capture_is_the_target(tmp_path: Path) -> None:
    _at(http=_gaps_exp(31, 0.2, 151)).write_pcap(tmp_path / "ref.pcap")
    labels = _labels(
        {
            "arrivals": {
                "applications": {"http1": {"dist": "empirical", "reference": {"pcap": "ref.pcap"}}}
            }
        }
    )
    labels.base_dir = tmp_path
    same = _run(tmp_path, _at(http=_gaps_exp(32, 0.2, 121)), labels)
    assert same.passed, same.failures
    assert same.comparisons[0]["target"]["source"] == "reference"
    assert same.comparisons[0]["reference_n"] == 150
    assert not _run(tmp_path, _at(http=_gaps_exp(32, 0.35, 121)), labels).passed
    missing = _labels(
        {
            "arrivals": {
                "applications": {"http1": {"dist": "empirical", "reference": {"pcap": "x.pcap"}}}
            }
        }
    )
    missing.base_dir = tmp_path
    with pytest.raises(CaptureError):
        _run(tmp_path, _at(http=_gaps_exp(32, 0.2, 121)), missing)


def test_a_reference_capture_with_its_own_labels_and_sizes(tmp_path: Path) -> None:
    (tmp_path / "ref.labels.json").write_text(
        json.dumps(
            {
                "labels": "fps-realism-labels",
                "version": 1,
                "sessions": [{"application": "bulk", "match": {"server_port": 9000}}],
            }
        )
    )
    _bulk(_sizes(7, 150, lambda r: r.lognormvariate(8.0, 0.7))).write_pcap(tmp_path / "ref.pcap")
    target = {
        "dist": "empirical",
        "direction": "server",
        "reference": {"pcap": "ref.pcap", "labels": "ref.labels.json"},
    }
    labels = _labels({"sizes": {"applications": {"bulk": target}}})
    labels.base_dir = tmp_path
    ok = _run(tmp_path, _bulk(_sizes(8, 150, lambda r: r.lognormvariate(8.0, 0.7))), labels)
    assert ok.passed, ok.failures
    bad = _run(tmp_path, _bulk(_sizes(8, 150, lambda r: r.lognormvariate(8.6, 0.7))), labels)
    assert not bad.passed
    # A reference with too few observations is not judged either.
    _bulk([500] * 5).write_pcap(tmp_path / "ref.pcap")
    short = _run(tmp_path, _bulk(_sizes(8, 150, lambda r: r.lognormvariate(8.0, 0.7))), labels)
    assert short.comparisons[0]["status"] == "insufficient_data"
    assert "in the reference" in short.failures[0]


# --- metrics -----------------------------------------------------------------------------------


def test_metrics_carry_the_catalog_labels_for_applications_only(tmp_path: Path) -> None:
    rng = random.Random(3)
    labels = _labels(
        {
            "arrivals": {
                "overall": {"dist": "exponential", "mean_ms": 1000 / 5},
                "applications": {"http1": {"dist": "exponential", "mean_ms": 200}},
            },
            "sizes": {
                "applications": {
                    "http1": [
                        {"direction": "server", "dist": "constant", "value": 1},
                        {"direction": "both", "dist": "constant", "value": 1},
                    ]
                }
            },
        }
    )
    result = _run(tmp_path, _at(http=_poisson(rng, 5.0, 100)), labels)
    seen = {(m["name"], m["labels"]["property"]) for m in result.metrics()}
    assert seen == {
        ("realism.ks_statistic", "inter_arrival"),
        ("realism.ks_p_value", "inter_arrival"),
        ("realism.ks_statistic", "response_bytes"),
        ("realism.ks_p_value", "response_bytes"),
    }
    catalog = yaml.safe_load(CATALOG.read_text())
    metrics = {m["name"]: m for m in catalog["metrics"]}
    allowed = set(catalog["labels"]["property"]["values"])
    for metric in result.metrics():
        assert set(metric["labels"]) == set(metrics[metric["name"]]["labels"])
        assert metric["labels"]["property"] in allowed
    request = _labels(
        {
            "sizes": {
                "applications": {"bulk": {"direction": "client", "dist": "constant", "value": 120}}
            }
        }
    )
    names = {m["labels"]["property"] for m in _run(tmp_path, _bulk([500] * 40), request).metrics()}
    assert names == {"request_bytes"}


# --- labels ---------------------------------------------------------------------------------


def test_the_temporal_section_normalizes_units() -> None:
    labels = _labels(
        {
            "arrivals": {
                "overall": {"dist": "lognormal", "mu": 5.0, "sigma": 0.3, "unit": "ms"},
                "applications": {
                    "dns": {"dist": "uniform", "min_ms": 10, "max_ms": 30},
                    "http1": {"ms": 250},
                },
            },
            "sizes": {"applications": {"bulk": {"dist": "pareto", "alpha": 1.5, "scale": 400}}},
        }
    )
    assert labels.temporal is not None
    assert (labels.temporal.alpha, labels.temporal.min_samples) == (0.01, 30)
    overall, dns, http, bulk = (t.distribution for t in labels.temporal.targets)
    assert overall.params["mu"] == pytest.approx(5.0 - math.log(1000))
    assert dns.params == {"min": 0.01, "max": 0.03}
    assert http.kind == "constant"
    assert http.params == {"value": 0.25}
    assert bulk.params == {"alpha": 1.5, "scale": 400.0}
    assert labels.temporal.targets[3].direction == "both"
    assert labels.temporal.targets[0].application is None


BAD_SECTIONS = [
    ({"alpha": 0}, "temporal.alpha"),
    ({"alpha": 1}, "temporal.alpha"),
    ({"alpha": True}, "temporal.alpha"),
    ({"min_samples": 1}, "temporal.min_samples"),
    ({"min_samples": 2.5}, "temporal.min_samples"),
    ({"bogus": 1}, "temporal: must be a mapping"),
    ({"arrivals": {"bogus": {}}}, "temporal.arrivals: must be a mapping"),
    ({"arrivals": {"overall": {"dist": "weibull"}}}, "dist: must be one of"),
    ({"arrivals": {"overall": {"dist": "exponential", "mean": 5}}}, "takes only dist and mean_ms"),
    (
        {"arrivals": {"overall": {"dist": "exponential", "mean_ms": 0}}},
        "mean_ms: must be a number above 0",
    ),
    (
        {"arrivals": {"overall": {"dist": "uniform", "min_ms": 5, "max_ms": 5}}},
        "max_ms must be above",
    ),
    (
        {"arrivals": {"overall": {"dist": "lognormal", "mu": 0, "sigma": 1, "unit": "us"}}},
        "unit: must be",
    ),
    (
        {"arrivals": {"overall": {"dist": "lognormal", "mu": "x", "sigma": 1}}},
        "mu: must be a number",
    ),
    ({"arrivals": {"overall": {"dist": "pareto", "alpha": 1, "scale_ms": -1}}}, "scale_ms"),
    ({"arrivals": {"overall": {"dist": "constant", "ms": float("nan")}}}, "ms: must be"),
    ({"arrivals": {"overall": {"dist": "empirical"}}}, "exactly one of"),
    ({"arrivals": {"overall": {"dist": "empirical", "sample": []}}}, "non empty list"),
    (
        {"arrivals": {"overall": {"dist": "empirical", "sample": [1], "sample_file": "f"}}},
        "exactly one of",
    ),
    ({"arrivals": {"overall": {"dist": "empirical", "reference": {"x": 1}}}}, "reference: must be"),
    (
        {"arrivals": {"overall": {"dist": "exponential", "mean_ms": 1, "direction": "client"}}},
        "takes only",
    ),
    (
        {"arrivals": {"applications": {"http1": [{"dist": "exponential", "mean_ms": 1}]}}},
        "must be a mapping",
    ),
    (
        {"sizes": {"overall": {"direction": "up", "dist": "exponential", "mean": 5}}},
        "direction: must be",
    ),
    ({"sizes": {"overall": []}}, "must not be an empty list"),
    (
        {
            "sizes": {
                "applications": {
                    "a": [{"dist": "exponential", "mean": 5}, {"dist": "constant", "value": 5}]
                }
            }
        },
        "has two targets",
    ),
]


@pytest.mark.parametrize(("section", "message"), BAD_SECTIONS)
def test_bad_temporal_sections_are_rejected(section: Any, message: str) -> None:
    doc = {"labels": "fps-realism-labels", "version": 1, "temporal": section}
    with pytest.raises(LabelsError, match=message):
        parse(doc)


def test_the_labels_schema_agrees_with_the_parser() -> None:
    schema = jsonschema.Draft202012Validator(json.loads(LABELS_SCHEMA_PATH.read_text()))
    base = {"labels": "fps-realism-labels", "version": 1}
    good: list[dict[str, Any]] = [
        {"alpha": 0.05, "min_samples": 20},
        {"arrivals": {"overall": {"ms": 250}}},
        {"arrivals": {"overall": {"dist": "constant", "ms": 250, "tolerance_ms": 2}}},
        {
            "arrivals": {
                "applications": {"a": {"dist": "lognormal", "mu": 1, "sigma": 1, "unit": "ms"}}
            }
        },
        {
            "arrivals": {
                "applications": {"a": {"dist": "empirical", "reference": {"pcap": "r.pcap"}}}
            }
        },
        {"sizes": {"overall": {"dist": "pareto", "alpha": 1.2, "scale": 100}}},
        {
            "sizes": {
                "applications": {
                    "a": [
                        {"direction": "client", "dist": "uniform", "min": 1, "max": 2},
                        {"direction": "server", "dist": "empirical", "sample": [1, 2, 3]},
                    ]
                }
            }
        },
    ]
    for section in good:
        schema.validate(base | {"temporal": section})
        parse(base | {"temporal": section})
    rejected: list[dict[str, Any]] = [
        {"alpha": 1},
        {"min_samples": 1},
        {"bogus": 1},
        {"arrivals": {"overall": {"dist": "weibull"}}},
        {"arrivals": {"overall": {"dist": "exponential", "mean": 5}}},
        {"arrivals": {"overall": {"dist": "exponential", "mean_ms": 0}}},
        {"arrivals": {"overall": {"dist": "empirical"}}},
        {"arrivals": {"overall": {"dist": "empirical", "sample": [1], "sample_file": "f"}}},
        {"arrivals": {"overall": {"dist": "empirical", "direction": "client", "sample": [1]}}},
        {"arrivals": {"applications": {"a": [{"dist": "exponential", "mean_ms": 1}]}}},
        {"sizes": {"overall": {"direction": "up", "dist": "exponential", "mean": 5}}},
        {"sizes": {"overall": {"dist": "exponential", "mean_ms": 5}}},
        {"sizes": {"overall": []}},
    ]
    for section in rejected:
        assert not schema.is_valid(base | {"temporal": section}), section
        with pytest.raises(LabelsError):
            parse(base | {"temporal": section})


# --- report and command line ----------------------------------------------------------------------


def _labels_file(tmp_path: Path, section: dict[str, Any] | None) -> Path:
    doc: dict[str, Any] = {"labels": "fps-realism-labels", "version": 1, "sessions": SESSIONS}
    if section is not None:
        doc["temporal"] = section
    path = tmp_path / "labels.json"
    path.write_text(json.dumps(doc))
    return path


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(report)


STUB = StubClassifier.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi")


def test_the_report_has_the_temporal_entry_and_validates(tmp_path: Path) -> None:
    good = _at(http=_gaps_exp(11, 0.2, 201)).write_pcap(tmp_path / "good.pcap")
    labels = _labels_file(tmp_path, EXPONENTIAL_200MS)
    report = score(good, labels, STUB).to_dict()
    _validate(report)
    assert report["version"] == "0.11.0"
    assert [c["check"] for c in report["checks"]] == ["conformance", "temporal"]
    assert report["checks"][1]["passed"] is True
    assert report["passed"] is True
    slow = _at(http=_gaps_exp(11, 0.35, 201)).write_pcap(tmp_path / "slow.pcap")
    failed = score(slow, labels, STUB).to_dict()
    _validate(failed)
    assert failed["checks"][1]["passed"] is False
    assert failed["passed"] is False
    assert failed["checks"][1]["comparisons"][0]["status"] == "fail"
    short = _at(http=_gaps_exp(11, 0.2, 11)).write_pcap(tmp_path / "short.pcap")
    insufficient = score(short, labels, STUB).to_dict()
    _validate(insufficient)
    assert insufficient["checks"][1]["comparisons"][0]["status"] == "insufficient_data"


def test_the_report_with_sizes_and_empirical_targets_validates(tmp_path: Path) -> None:
    section = {
        **SERVER_SIZES,
        "arrivals": {
            "overall": {"dist": "empirical", "sample": [0.1 * (i + 1) for i in range(40)]},
        },
    }
    pcap = _bulk(_sizes(1, 100, lambda r: r.lognormvariate(8.0, 0.7))).write_pcap(
        tmp_path / "b.pcap"
    )
    report = score(pcap, _labels_file(tmp_path, section), STUB).to_dict()
    _validate(report)
    assert [c["application"] for c in report["checks"][1]["comparisons"]] == [None, "bulk"]


def test_the_check_is_off_without_a_section_or_an_option(tmp_path: Path) -> None:
    pcap = _at(http=_gaps_exp(11, 0.2, 51)).write_pcap(tmp_path / "t.pcap")
    off = score(pcap, _labels_file(tmp_path, None), STUB).to_dict()
    assert [c["check"] for c in off["checks"]] == ["conformance"]
    on = score(pcap, _labels_file(tmp_path, None), STUB, ks_alpha=0.05).to_dict()
    assert [c["check"] for c in on["checks"]] == ["conformance", "temporal"]
    assert on["checks"][1]["alpha"] == 0.05
    assert on["checks"][1]["passed"] is False  # no target distribution
    on_min = score(pcap, _labels_file(tmp_path, EXPONENTIAL_200MS), STUB, ks_min_samples=500)
    assert on_min.to_dict()["checks"][1]["comparisons"][0]["status"] == "insufficient_data"


def test_the_command_line_options_add_and_override_the_check(
    tmp_path: Path,
    fake_reader: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    csv = NDPI_42_HEADER + "\n"
    for i in range(201):
        csv += f"{i},6,1,1,0,10.0.0.10,{20000 + i},10.0.1.80,80,7,HTTP,,6,414\n"
    pcap = _at(http=_gaps_exp(11, 0.25, 201)).write_pcap(tmp_path / "t.pcap")
    base = ["score", "--pcap", str(pcap), "--parsers", "none"]
    base += ["--ndpi-reader", str(fake_reader(csv))]
    with_section = [*base, "--labels", str(_labels_file(tmp_path, EXPONENTIAL_200MS))]
    assert main(with_section) in (EXIT_PASS, EXIT_FAIL)
    report = json.loads(capsys.readouterr().out)
    assert report["checks"][1]["alpha"] == 0.01
    p = report["checks"][1]["comparisons"][0]["p_value"]
    assert main([*with_section, "--ks-alpha", str(min(0.99, p * 1.01))]) == EXIT_FAIL
    assert json.loads(capsys.readouterr().out)["checks"][1]["comparisons"][0]["status"] == "fail"
    assert main([*with_section, "--ks-alpha", str(p * 0.99)]) == EXIT_PASS
    capsys.readouterr()
    assert main([*with_section, "--ks-min-samples", "1000"]) == EXIT_FAIL
    out = json.loads(capsys.readouterr().out)
    assert out["checks"][1]["comparisons"][0]["status"] == "insufficient_data"
    no_section = [*base, "--labels", str(_labels_file(tmp_path, None))]
    assert main(no_section) == EXIT_PASS
    capsys.readouterr()
    assert main([*no_section, "--ks-alpha", "0.05"]) == EXIT_FAIL
    assert "no target distribution" in capsys.readouterr().out
    for bad in (["--ks-alpha", "0"], ["--ks-alpha", "1"], ["--ks-min-samples", "1"]):
        with pytest.raises(SystemExit) as exc:
            main([*no_section, *bad])
        assert exc.value.code == EXIT_ERROR
    capsys.readouterr()


def test_a_bad_sample_file_is_a_usage_error(
    tmp_path: Path, fake_reader: Callable[..., Path], capsys: pytest.CaptureFixture[str]
) -> None:
    section = {"arrivals": {"overall": {"dist": "empirical", "sample_file": "missing.txt"}}}
    pcap = _at(http=_gaps_exp(11, 0.2, 51)).write_pcap(tmp_path / "t.pcap")
    args = ["score", "--pcap", str(pcap), "--parsers", "none", "--labels"]
    args += [str(_labels_file(tmp_path, section)), "--ndpi-reader", str(fake_reader())]
    assert main(args) == EXIT_ERROR
    assert "read sample file" in capsys.readouterr().err
