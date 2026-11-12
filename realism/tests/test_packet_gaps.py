# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the packet gap property of the temporal check (RLM-15).

The check measures the gaps between the packets of one direction of a session, which is the
pacing of an RTP stream or the cadence of a keepalive. The captures are built in code with
``pcapgen`` from seeded generators, and one test reads the committed sip_rtp fixture, which
holds calls with packet times of 20 ms and 30 ms recorded from the real module.
"""

from __future__ import annotations

import json
import random
from pathlib import Path
from typing import Any

import jsonschema
import pytest
import yaml

from realism import temporal
from realism.capture import summarize
from realism.classifier import StubClassifier
from realism.labels import Labels, LabelsError, parse
from realism.report import LABELS_SCHEMA_PATH, REPORT_VERSION, SCHEMA_PATH, score
from realism.tests import pcapgen

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "modules" / "sip_rtp" / "fixtures"
CLIENT = "10.201.0.2"
SERVER = "10.202.0.2"
PAYLOAD = b"\x80\x00" + bytes(158)
PACED = {"dist": "constant", "ms": [20, 30], "tolerance_ms": 8}


def _labels(section: dict[str, Any] | None, **extra: Any) -> Labels:
    doc: dict[str, Any] = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "rtp", "match": {"transport": "udp"}}],
    }
    if section is not None:
        doc["temporal"] = section
    return parse(doc | extra)


def _packets(app: dict[str, Any] | list[dict[str, Any]]) -> dict[str, Any]:
    return {"packets": {"applications": {"rtp": app}}}


def _call(
    cap: pcapgen.Capture,
    call: int,
    ptime_ms: float,
    packets: int,
    jitter_ms: float = 0.0,
    seed: int = 1,
    stall_every: int = 0,
    start: float = 0.0,
) -> None:
    """Append one RTP call whose two directions send in lock step on one pair of ports.

    Packet ``i`` leaves at ``i * ptime`` plus a uniform jitter, so the jitter does not add
    up to a drift. With ``stall_every``, every nth packet waits for a stall of five packet
    times and then leaves with the four packets behind it in one burst.
    """
    rng = random.Random(seed + call)
    base = cap.clock + start
    ports = (40000 + call, 20000 + call)
    for i in range(packets):
        nominal = i * ptime_ms / 1000.0
        if stall_every and i % stall_every >= stall_every - 5:
            nominal = (i - i % stall_every + stall_every - 5) * ptime_ms / 1000.0 + 0.0001 * (
                i % stall_every
            )
        for src, dst in (
            ((CLIENT, ports[0]), (SERVER, ports[1])),
            ((SERVER, ports[1]), (CLIENT, ports[0])),
        ):
            late = rng.uniform(-jitter_ms, jitter_ms) / 1000.0
            at = base + nominal + max(0.0, late) + (0.00001 if src[0] == SERVER else 0.0)
            cap.frames.append((at, pcapgen.ethernet(pcapgen.udp(*src, *dst, PAYLOAD))))
    cap.frames.sort(key=lambda frame: frame[0])


def _check(tmp_path: Path, cap: pcapgen.Capture, labels: Labels) -> temporal.TemporalResult:
    pcap = cap.write_pcap(tmp_path / "rtp.pcap")
    return temporal.check(pcap, summarize(pcap).sessions, labels)


# --- acceptance: 20 ms and 30 ms pass, jitter fails with a named reason ------------------------


def test_a_stream_at_20_ms_passes_a_paced_target(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=2)
    result = _check(
        tmp_path, cap, _labels(_packets({"dist": "constant", "ms": 20, "tolerance_ms": 5}))
    )
    assert result.passed, result.failures
    (comparison,) = result.comparisons
    assert comparison["property"] == "packet_gap"
    assert comparison["application"] == "rtp"
    assert comparison["direction"] == "both"
    assert comparison["unit"] == "seconds"
    assert comparison["n"] == 198  # two streams of 100 packets
    assert comparison["target"]["values"] == [0.02]
    assert comparison["target"]["parameters"] == {"value": 0.02, "tolerance": 0.005, "within": 0.9}
    assert comparison["streams"] == {"judged": 2, "failed": 0, "too_short": 0}
    assert (comparison["test"], comparison["method"], comparison["p_value"]) == (
        "constant",
        "constant",
        1.0,
    )


def test_a_stream_at_30_ms_passes_a_paced_target(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 30, 100, jitter_ms=2)
    result = _check(
        tmp_path, cap, _labels(_packets({"dist": "constant", "ms": 30, "tolerance_ms": 5}))
    )
    assert result.passed, result.failures
    assert result.comparisons[0]["observed"]["median"] == pytest.approx(0.03, abs=0.002)


def test_calls_with_both_packet_times_pass_a_target_that_lists_them(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=2)
    _call(cap, 2, 30, 80, jitter_ms=2)
    _call(cap, 3, 20, 100, jitter_ms=2)
    result = _check(
        tmp_path, cap, _labels(_packets({"dist": "constant", "ms": [20, 30], "tolerance_ms": 5}))
    )
    assert result.passed, result.failures
    comparison = result.comparisons[0]
    assert comparison["target"]["values"] == [0.02, 0.03]
    assert "value" not in comparison["target"]["parameters"]
    assert comparison["target"]["median"] is None
    assert comparison["streams"]["judged"] == 6


def test_a_target_for_one_packet_time_fails_a_call_with_the_other(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=1)
    _call(cap, 2, 30, 100, jitter_ms=1)
    result = _check(
        tmp_path, cap, _labels(_packets({"dist": "constant", "ms": 20, "tolerance_ms": 5}))
    )
    assert not result.passed
    comparison = result.comparisons[0]
    assert comparison["status"] == "fail"
    assert comparison["streams"] == {"judged": 4, "failed": 2, "too_short": 0}
    (reason,) = result.failures
    assert "2 of 4 streams are not paced at 20 ms" in reason
    assert "udp stream" in reason
    assert "further than 5 ms from 20 ms" in reason


def test_a_capture_with_jittered_packets_fails_with_a_named_reason(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=1)
    _call(cap, 2, 20, 100, jitter_ms=14, seed=7)
    result = _check(
        tmp_path, cap, _labels(_packets({"dist": "constant", "ms": 20, "tolerance_ms": 5}))
    )
    assert not result.passed
    comparison = result.comparisons[0]
    assert comparison["status"] == "fail"
    assert comparison["reason"] == result.failures[0]
    assert comparison["streams"]["judged"] == 4
    assert comparison["streams"]["failed"] == 2  # the two directions of the jittered call
    reason = result.failures[0]
    # The reason names the application, the stream with its session and side, the lateness,
    # the packet time, and what was observed.
    assert reason.startswith(
        "packet gaps of application rtp: 2 of 4 streams are not paced at 20 ms"
    )
    assert "(session " in reason and " side)" in reason
    assert "10.201.0.2:40002" in reason
    assert "10.202.0.2:20002" in reason
    assert "gaps from" in reason
    assert "median" in reason


def test_a_burst_after_a_stall_fails_even_though_the_mean_is_right(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 200, stall_every=10)  # 5 of every 10 packets leave in a burst
    result = _check(
        tmp_path, cap, _labels(_packets({"dist": "constant", "ms": 20, "tolerance_ms": 5}))
    )
    assert not result.passed
    comparison = result.comparisons[0]
    assert comparison["observed"]["mean"] == pytest.approx(0.02, rel=0.05)
    assert comparison["observed"]["min"] < 0.001
    assert "not paced at 20 ms" in result.failures[0]


def test_a_late_wake_up_within_the_share_is_tolerated(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100)
    # Move 3 of the 100 packets of every direction 30 ms late, which makes 6% of the gaps late.
    late = {10, 40, 70}
    moved = []
    for i, (at, frame) in enumerate(cap.frames):
        moved.append((at + 0.03 if (i // 2) in late else at, frame))
    cap.frames = sorted(moved, key=lambda f: f[0])
    labels = _labels(_packets({"dist": "constant", "ms": 20, "tolerance_ms": 5}))
    assert _check(tmp_path, cap, labels).passed
    strict = _labels(_packets({"dist": "constant", "ms": 20, "tolerance_ms": 5, "within": 0.99}))
    result = _check(tmp_path, cap, strict)
    assert not result.passed
    assert "more than 1.0% of their gaps" in result.failures[0]


# --- the sip_rtp fixture ------------------------------------------------------------------------


def _fixture_labels(section: dict[str, Any]) -> Labels:
    doc = yaml.safe_load((FIXTURE / "expected.yaml").read_text(encoding="utf-8"))
    doc["temporal"] = section
    return parse(doc)


def test_the_sip_rtp_fixture_passes_with_20_ms_and_30_ms_packet_times() -> None:
    pcap = FIXTURE / "sip_rtp.pcapng"
    sessions = summarize(pcap).sessions
    result = temporal.check(pcap, sessions, _fixture_labels(_packets(PACED)))
    assert result.passed, result.failures
    comparison = result.comparisons[0]
    assert comparison["streams"]["failed"] == 0
    assert comparison["streams"]["judged"] >= 20  # both directions of every call with media


def test_the_sip_rtp_fixture_fails_when_the_target_lists_only_one_packet_time() -> None:
    pcap = FIXTURE / "sip_rtp.pcapng"
    sessions = summarize(pcap).sessions
    for only in (20, 30):
        section = _packets({**PACED, "ms": only})
        result = temporal.check(pcap, sessions, _fixture_labels(section))
        assert not result.passed, only
        assert f"not paced at {only} ms" in result.failures[0]


def test_the_sip_rtp_signalling_sessions_have_no_paced_stream() -> None:
    pcap = FIXTURE / "sip_rtp.pcapng"
    sessions = summarize(pcap).sessions
    section = {"packets": {"applications": {"sip": PACED}}}
    result = temporal.check(pcap, sessions, _fixture_labels(section))
    assert result.comparisons[0]["status"] == "insufficient_data"
    assert "no stream has the 30 gaps" in result.failures[0] or "observations" in result.failures[0]


# --- streams and directions ----------------------------------------------------------------------


def test_a_direction_selects_the_streams_that_a_target_reads(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=1)
    both = _check(tmp_path, cap, _labels(_packets({**PACED, "direction": "both"})))
    client = _check(tmp_path, cap, _labels(_packets({**PACED, "direction": "client"})))
    server = _check(tmp_path, cap, _labels(_packets({**PACED, "direction": "server"})))
    assert [r.comparisons[0]["n"] for r in (both, client, server)] == [198, 99, 99]
    assert [r.comparisons[0]["direction"] for r in (both, client, server)] == [
        "both",
        "client",
        "server",
    ]
    assert all(r.passed for r in (both, client, server))


def test_each_direction_can_have_its_own_target(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=1)
    labels = _labels(
        _packets(
            [
                {"dist": "constant", "ms": 20, "tolerance_ms": 5, "direction": "client"},
                {"dist": "constant", "ms": 30, "tolerance_ms": 5, "direction": "server"},
            ]
        )
    )
    result = _check(tmp_path, cap, labels)
    assert [c["status"] for c in result.comparisons] == ["pass", "fail"]
    assert "server side" in result.failures[0]


def test_a_gap_never_spans_the_two_directions(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    # The server answers 1 ms after every client packet, so the merged gaps would alternate
    # between 1 ms and 19 ms and fail a 20 ms target.
    _call(cap, 1, 20, 100)
    result = _check(tmp_path, cap, _labels(_packets({**PACED, "ms": 20})))
    assert result.passed, result.failures
    assert result.comparisons[0]["observed"]["min"] > 0.019


def test_packets_without_payload_do_not_count(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    client, server = ("10.0.0.10", 31000), ("10.0.1.90", 9000)
    seq = 1000
    for i in range(60):
        at = cap.clock + 1.0 + i * 0.1
        data = pcapgen.tcp(*client, *server, pcapgen.PSH | pcapgen.ACK, seq, 5000, b"x" * 40)
        cap.frames.append((at, pcapgen.ethernet(data)))
        # Pure acknowledgments from both sides, at times that would break the cadence.
        for extra, side in ((0.013, client), (0.071, server)):
            other = server if side == client else client
            ack = pcapgen.tcp(*side, *other, pcapgen.ACK, 5000, seq + 40)
            cap.frames.append((at + extra, pcapgen.ethernet(ack)))
        seq += 40
    cap.frames.sort(key=lambda frame: frame[0])
    section = {
        "packets": {
            "overall": {"dist": "constant", "ms": 100, "tolerance_ms": 1, "direction": "client"}
        }
    }
    labels = _labels(section)
    result = _check(tmp_path, cap, labels)
    assert result.passed, result.failures
    assert result.comparisons[0]["application"] is None
    assert result.comparisons[0]["n"] == 59


def test_short_streams_are_not_judged_alone(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=1)
    _call(cap, 2, 20, 12, jitter_ms=15, seed=5)  # short and jittered, 11 gaps per stream
    result = _check(tmp_path, cap, _labels(_packets({**PACED, "ms": 20})))
    assert result.passed, result.failures
    assert result.comparisons[0]["streams"] == {"judged": 2, "failed": 0, "too_short": 2}


def test_a_capture_without_a_long_stream_has_insufficient_data(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for call in range(1, 6):
        _call(cap, call, 20, 20, jitter_ms=1)  # 5 calls, 19 gaps per stream, 190 gaps
    result = _check(tmp_path, cap, _labels(_packets({**PACED, "ms": 20})))
    assert not result.passed
    comparison = result.comparisons[0]
    assert comparison["status"] == "insufficient_data"
    assert comparison["n"] == 190
    assert comparison["streams"] is None
    assert "no stream has the 30 gaps" in result.failures[0]
    few = _labels(_packets({**PACED, "ms": 20}))
    cap2 = pcapgen.Capture()
    _call(cap2, 1, 20, 10)
    assert _check(tmp_path, cap2, few).comparisons[0]["status"] == "insufficient_data"


# --- the other distributions run the KS test on the gaps of all streams --------------------------


def test_an_exponential_gap_target_passes_a_keepalive_with_random_gaps(tmp_path: Path) -> None:
    rng = random.Random(3)
    cap = pcapgen.Capture()
    t = cap.clock
    for _ in range(150):
        t += rng.expovariate(1 / 0.5)
        cap.frames.append(
            (t, pcapgen.ethernet(pcapgen.udp(CLIENT, 41000, SERVER, 5683, b"keepalive")))
        )
    labels = _labels(_packets({"dist": "exponential", "mean_ms": 500, "direction": "client"}))
    result = _check(tmp_path, cap, labels)
    assert result.passed, result.failures
    comparison = result.comparisons[0]
    assert comparison["test"] == "ks_one_sample"
    assert comparison["streams"] is None
    assert comparison["n"] == 149
    wrong = _labels(_packets({"dist": "exponential", "mean_ms": 200, "direction": "client"}))
    failed = _check(tmp_path, cap, wrong)
    assert not failed.passed
    assert (
        "packet gaps (client) of application rtp: the KS test rejects exponential"
        in (failed.failures[0])
    )


def test_a_paced_stream_fails_an_exponential_target(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=1)
    labels = _labels(_packets({"dist": "exponential", "mean_ms": 20, "direction": "client"}))
    assert not _check(tmp_path, cap, labels).passed


def test_an_empirical_gap_target_uses_the_two_sample_test(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=3)
    sample = [0.02 + random.Random(9).uniform(-0.003, 0.003) * k / 100 for k in range(100)]
    labels = _labels(_packets({"dist": "empirical", "sample": sample, "direction": "client"}))
    result = _check(tmp_path, cap, labels)
    comparison = result.comparisons[0]
    assert comparison["test"] == "ks_two_sample"
    assert comparison["reference_n"] == 100
    assert comparison["target"]["source"] == "sample"


# --- labels --------------------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("entry", "message"),
    [
        ({"dist": "constant", "ms": 20}, "tolerance_ms: a paced target needs"),
        ({"dist": "constant", "ms": 0, "tolerance_ms": 5}, "ms: must be a packet time above 0"),
        ({"dist": "constant", "ms": [], "tolerance_ms": 5}, "ms: must be a packet time above 0"),
        ({"dist": "constant", "ms": [20, -1], "tolerance_ms": 5}, "ms: must be a packet time"),
        ({"dist": "constant", "ms": "20", "tolerance_ms": 5}, "ms: must be a packet time"),
        ({"dist": "constant", "ms": 20, "tolerance_ms": -1}, "tolerance_ms: must be a number"),
        ({"dist": "constant", "ms": 20, "tolerance_ms": 5, "within": 0}, "within: must be"),
        ({"dist": "constant", "ms": 20, "tolerance_ms": 5, "within": 1.5}, "within: must be"),
        ({"dist": "constant", "ms": 20, "tolerance_ms": 5, "direction": "up"}, "direction"),
        ({"dist": "constant", "ms": 20, "tolerance_ms": 5, "mean_ms": 1}, "takes only dist and"),
        ({"dist": "exponential", "mean_ms": 20, "within": 0.9}, "takes only dist and"),
    ],
)
def test_bad_packet_targets_are_rejected(entry: dict[str, Any], message: str) -> None:
    with pytest.raises(LabelsError, match=message):
        _labels(_packets(entry))


def test_two_targets_for_one_direction_are_rejected() -> None:
    entry = [
        {"dist": "constant", "ms": 20, "tolerance_ms": 5},
        {"dist": "constant", "ms": 30, "tolerance_ms": 5, "direction": "both"},
    ]
    with pytest.raises(LabelsError, match="has two targets"):
        _labels(_packets(entry))
    with pytest.raises(LabelsError, match="must not be an empty list"):
        _labels(_packets([]))


def test_a_target_without_within_defaults_to_nine_tenths() -> None:
    labels = _labels(_packets({"dist": "constant", "ms": [30, 20, 20], "tolerance_ms": 4}))
    (target,) = labels.temporal.targets  # type: ignore[union-attr]
    dist = target.distribution
    assert (target.prop, target.application, target.direction) == ("packet_gap", "rtp", "both")
    assert dist.values == (0.02, 0.03)
    assert dist.within == 0.9
    assert dist.tolerance == pytest.approx(0.004)


def test_the_labels_schema_accepts_and_rejects_packet_targets() -> None:
    schema = json.loads(LABELS_SCHEMA_PATH.read_text())
    validator = jsonschema.Draft202012Validator(schema)

    def doc(entry: Any) -> dict[str, Any]:
        return {
            "labels": "fps-realism-labels",
            "version": 1,
            "sessions": [{"application": "rtp", "match": {"transport": "udp"}}],
            "temporal": {"packets": {"overall": entry, "applications": {"rtp": entry}}},
        }

    good = [
        PACED,
        {"ms": 20, "tolerance_ms": 5},
        {"dist": "constant", "ms": 20, "tolerance_ms": 5, "within": 0.95, "direction": "client"},
        [PACED | {"direction": "client"}, PACED | {"direction": "server"}],
        {"dist": "exponential", "mean_ms": 500, "direction": "server"},
        {"dist": "uniform", "min_ms": 10, "max_ms": 30},
        {"dist": "lognormal", "mu": 3.0, "sigma": 0.4, "unit": "ms"},
        {"dist": "pareto", "alpha": 2, "scale_ms": 5},
        {"dist": "empirical", "sample": [0.02, 0.021], "direction": "client"},
    ]
    for entry in good:
        validator.validate(doc(entry))
        parse(doc(entry))  # the schema and the parser agree
    bad = [
        {"dist": "constant", "ms": 20},
        {"dist": "constant", "ms": 0, "tolerance_ms": 5},
        {"dist": "constant", "ms": [], "tolerance_ms": 5},
        {"dist": "constant", "ms": 20, "tolerance_ms": 5, "within": 0},
        {"dist": "exponential", "mean_ms": 20, "within": 0.9},
        {"dist": "constant", "ms": 20, "tolerance_ms": 5, "direction": "up"},
    ]
    for entry in bad:
        assert not validator.is_valid(doc(entry)), entry
        with pytest.raises(LabelsError):
            parse(doc(entry))


# --- report --------------------------------------------------------------------------------------


STUB = StubClassifier.by_server_port({}, name="ndpi")


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(report)


def test_the_report_carries_the_packet_gap_comparison_and_validates(tmp_path: Path) -> None:
    good, jittered = pcapgen.Capture(), pcapgen.Capture()
    _call(good, 1, 20, 100, jitter_ms=1)
    _call(jittered, 1, 20, 100, jitter_ms=15, seed=4)
    doc = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "rtp", "match": {"transport": "udp"}}],
        "temporal": _packets({"dist": "constant", "ms": 20, "tolerance_ms": 5}),
    }
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(doc))
    entries = []
    for name, cap in (("good", good), ("jittered", jittered)):
        pcap = cap.write_pcap(tmp_path / f"{name}.pcap")
        report = score(pcap, labels, STUB).to_dict()
        _validate(report)
        assert report["version"] == REPORT_VERSION == "0.18.0"
        entries.append(report["checks"][-1])
    ok, bad = entries
    assert (ok["check"], ok["passed"]) == ("temporal", True)
    assert (bad["passed"], bad["comparisons"][0]["status"]) == (False, "fail")
    assert bad["failures"] == [bad["comparisons"][0]["reason"]]
    # A paced comparison has no KS statistic (CON-28), so it has no KS series, and the
    # failures series is the only metric.
    for entry in entries:
        assert [m["name"] for m in entry["metrics"]] == ["realism.check_failures"]
    assert ok["metrics"][0]["value"] == 0
    assert bad["metrics"][0]["value"] == 1


def test_paced_packet_gaps_add_no_ks_series_next_to_the_other_properties(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=1)
    section = {
        "packets": {
            "applications": {
                "rtp": {"dist": "constant", "ms": 20, "tolerance_ms": 5, "direction": "client"}
            }
        },
        "sizes": {
            "applications": {"rtp": {"dist": "constant", "value": 160, "direction": "client"}}
        },
    }
    result = _check(tmp_path, cap, _labels(section))
    labelled = [
        (m["labels"]["property"], m["name"]) for m in result.metrics() if "property" in m["labels"]
    ]
    # Only the size comparison has a KS series, and a paced comparison of the client direction
    # must not pass for request_bytes or request_gap.
    assert labelled == [
        ("request_bytes", "realism.ks_statistic"),
        ("request_bytes", "realism.ks_p_value"),
    ]


def _keepalive_capture(seed: int = 3) -> pcapgen.Capture:
    """Return a capture of 150 client datagrams and 150 server ones with exponential gaps."""
    rng = random.Random(seed)
    cap = pcapgen.Capture()
    for src, sport, dst, dport in (
        (CLIENT, 41000, SERVER, 5683),
        (SERVER, 5683, CLIENT, 41000),
    ):
        t = cap.clock
        for _ in range(150):
            t += rng.expovariate(1 / 0.5)
            cap.frames.append((t, pcapgen.ethernet(pcapgen.udp(src, sport, dst, dport, b"ka"))))
    cap.frames.sort(key=lambda frame: frame[0])
    return cap


@pytest.mark.parametrize(
    ("direction", "prop"),
    [("client", "request_gap"), ("server", "response_gap"), ("both", "packet_gap")],
)
def test_a_ks_packet_gap_comparison_carries_the_series_of_its_direction(
    tmp_path: Path, direction: str, prop: str
) -> None:
    labels = _labels(_packets({"dist": "exponential", "mean_ms": 500, "direction": direction}))
    result = _check(tmp_path, _keepalive_capture(), labels)
    assert result.passed, result.failures
    comparison = result.comparisons[0]
    assert comparison["direction"] == direction
    series = [m for m in result.metrics() if m["name"] != "realism.check_failures"]
    assert series == [
        {
            "name": "realism.ks_statistic",
            "labels": {"application": "rtp", "property": prop},
            "value": comparison["statistic"],
        },
        {
            "name": "realism.ks_p_value",
            "labels": {"application": "rtp", "property": prop},
            "value": comparison["p_value"],
        },
    ]
    catalog = yaml.safe_load((ROOT / "contracts" / "metrics" / "catalog.yaml").read_text())
    assert prop in catalog["labels"]["property"]["values"]


def test_the_three_directions_of_packet_gaps_are_three_series_with_unique_labels(
    tmp_path: Path,
) -> None:
    targets = [
        {"dist": "exponential", "mean_ms": 500, "direction": d}
        for d in ("client", "server", "both")
    ]
    result = _check(tmp_path, _keepalive_capture(), _labels(_packets(targets)))
    keys = [
        (m["name"], m["labels"]["property"])
        for m in result.metrics()
        if m["name"] != "realism.check_failures"
    ]
    assert len(keys) == len(set(keys)) == 6
    assert {k[1] for k in keys} == {"request_gap", "response_gap", "packet_gap"}


def test_a_failing_ks_packet_gap_comparison_reports_a_small_p_value_in_a_valid_report(
    tmp_path: Path,
) -> None:
    pcap = _keepalive_capture().write_pcap(tmp_path / "ka.pcap")
    doc = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "rtp", "match": {"transport": "udp"}}],
        "temporal": _packets({"dist": "exponential", "mean_ms": 100, "direction": "both"}),
    }
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(doc))
    report = score(pcap, labels, STUB).to_dict()
    _validate(report)
    entry = report["checks"][-1]
    assert entry["passed"] is False
    by_name = {m["name"]: m for m in entry["metrics"]}
    assert by_name["realism.ks_p_value"]["labels"] == {
        "application": "rtp",
        "property": "packet_gap",
    }
    assert by_name["realism.ks_p_value"]["value"] < 0.01
    assert by_name["realism.check_failures"]["value"] == 1


def test_a_temporal_entry_of_an_earlier_format_still_validates(tmp_path: Path) -> None:
    """The fields that 0.16.0 added are optional, so the controller still takes older reports."""
    cap = pcapgen.Capture()
    _call(cap, 1, 20, 100, jitter_ms=1)
    doc = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "rtp", "match": {"transport": "udp"}}],
        "temporal": _packets({"dist": "exponential", "mean_ms": 20, "direction": "client"}),
    }
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(doc))
    report = score(cap.write_pcap(tmp_path / "old.pcap"), labels, STUB).to_dict()
    entry = report["checks"][-1]["comparisons"][0]
    assert entry["streams"] is None
    assert entry["target"]["values"] is None
    del entry["streams"], entry["target"]["values"]
    entry["property"] = "payload_size"  # the earlier formats know no other property here
    _validate(report)
