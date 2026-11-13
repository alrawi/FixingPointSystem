# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the request path packet gap targets and the rung byte targets (RLM-22).

MOD-24 could not move ``modules/hls/fixtures/hls_check.py`` into the labels. A keepalive
connection of an HLS playback alternates playlist and segment requests, the labels read no
bytes, and the script also checks the byte share and the bit rate of MOD-15. These tests show
that the labels of RLM-22 are as strong as that script: the committed hls capture passes with
them and reports the bit rates that ``hls.playbacks.json`` records, a capture with a wrong
segment cadence fails, and so does one with the wrong bytes. They read the hls fixture and
never change it. A later MOD package moves the labels into ``expected.yaml``.
"""

from __future__ import annotations

import json
import struct
from pathlib import Path
from typing import Any

import jsonschema
import pytest
import yaml

from realism import temporal
from realism.capture import Frame, decode, read_frames, summarize
from realism.classifier import StubClassifier
from realism.labels import Labels, LabelsError, parse
from realism.report import SCHEMA_PATH, score
from realism.tests import pcapgen

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "modules" / "hls" / "fixtures"
CAPTURE = FIXTURE / "hls.pcapng"
CLIENT = "10.201.0.2"
SERVER = "10.202.0.2"
RATES = [160000, 320000, 640000]
SEGMENT = r"/v\d+/seg\d+\.ts$"
RUNG = r"/v(\d+)/seg\d+\.ts$"
PACED = {
    "direction": "client",
    "dist": "constant",
    "ms": 400,
    "tolerance_ms": 40,
    "within": 0.9,
    "mean_tolerance_ms": 20,
    "min_samples": 3,
}


def _rungs(**extra: Any) -> dict[str, Any]:
    return {"path": RUNG, "duration_ms": 400, "bits_per_second": RATES, "min_samples": 4} | extra


def hls_section() -> dict[str, Any]:
    """The temporal labels that replace hls_check.py for the committed capture."""
    return {
        "alpha": 0.01,
        "min_samples": 30,
        "arrivals": {
            "applications": {"hls_segment": {"dist": "constant", "ms": 400, "tolerance_ms": 40}}
        },
        "packets": {
            "applications": {
                "hls_vod": PACED | {"path": SEGMENT},
                "hls_live": PACED | {"path": SEGMENT},
                "hls_segment": PACED | {"path": SEGMENT, "join_ms": 800},
            }
        },
        "rungs": {
            "applications": {
                "hls_vod": _rungs(walk="up", streams=3),
                "hls_segment": _rungs(walk="up", streams=1, join_ms=800),
                "hls_live": _rungs(streams=3),
            }
        },
    }


def hls_labels(section: dict[str, Any] | None = None) -> Labels:
    """The labels of expected.yaml with the temporal section of RLM-22 in place of its own."""
    doc = yaml.safe_load((FIXTURE / "expected.yaml").read_text(encoding="utf-8"))
    doc["temporal"] = section if section is not None else hls_section()
    return parse(doc)


def check(pcap: Path, labels: Labels) -> temporal.TemporalResult:
    return temporal.check(pcap, summarize(pcap).sessions, labels)


def write_pcap(path: Path, frames: list[Frame]) -> None:
    """Write frames as a classic pcap with microsecond timestamps, in time order."""
    ordered = sorted(frames, key=lambda frame: frame.timestamp)
    out = bytearray(struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, ordered[0].link_type))
    for frame in ordered:
        seconds = int(frame.timestamp)
        micros = round((frame.timestamp - seconds) * 1_000_000)
        out += struct.pack("<IIII", seconds, micros, len(frame.data), len(frame.data))
        out += frame.data
    path.write_bytes(bytes(out))


def retime(path: Path, factor: float) -> None:
    """Write the committed capture with the time inside every playback stretched by a factor.

    Every TCP session of the module ports keeps its first packet and runs ``factor`` times
    slower or faster after it, so the cadence of its requests changes and its bytes do not.
    """
    _, stream = read_frames(CAPTURE)
    frames = list(stream)
    ports = {8090, 8091, 8092, 8093}
    first: dict[tuple[str, int], float] = {}
    for frame in frames:
        packet = decode(frame)
        if packet and packet.transport == "tcp" and ports & {packet.src_port, packet.dst_port}:
            client = (
                (packet.src_ip, packet.src_port)
                if packet.dst_port in ports
                else (
                    packet.dst_ip,
                    packet.dst_port,
                )
            )
            first.setdefault(client, frame.timestamp)
    # The segments of vod_segments each have a connection, so their starts must stretch from
    # one common origin, which is the first segment connection.
    origin = min(t for (_, port), t in first.items() if 40004 <= port < 40037)
    out = []
    for frame in frames:
        packet = decode(frame)
        if packet and packet.transport == "tcp" and ports & {packet.src_port, packet.dst_port}:
            client = (
                (packet.src_ip, packet.src_port)
                if packet.dst_port in ports
                else (
                    packet.dst_ip,
                    packet.dst_port,
                )
            )
            base = origin if 40004 <= client[1] < 40037 else first[client]
            frame = Frame(base + (frame.timestamp - base) * factor, frame.link_type, frame.data)
        out.append(frame)
    write_pcap(path, out)


# --- the committed capture ----------------------------------------------------------------------


def test_the_committed_hls_capture_passes_with_labels_as_strong_as_hls_check() -> None:
    result = check(CAPTURE, hls_labels())
    assert result.passed, result.failures
    report = result.to_dict()
    gaps = {c["application"]: c for c in report["comparisons"] if c["property"] == "packet_gap"}
    assert {a: (c["streams"]["judged"], c["streams"]["failed"]) for a, c in gaps.items()} == {
        "hls_vod": (3, 0),
        "hls_live": (3, 0),
        "hls_segment": (1, 0),
    }
    assert [c["path"] for c in gaps.values()] == [SEGMENT] * 3
    assert gaps["hls_segment"]["n"] == 32  # 33 segments, each on its own connection
    by_app = {r["application"]: r for r in report["rungs"]}
    assert {a: r["streams"]["judged"] for a, r in by_app.items()} == {
        "hls_vod": 3,
        "hls_live": 3,
        "hls_segment": 1,
    }


def test_the_byte_numbers_equal_those_of_hls_check() -> None:
    """The bit rate and the rung shares are the numbers of hls.playbacks.json, per playback."""
    recorded = json.loads((FIXTURE / "hls.playbacks.json").read_text())["playbacks"]
    want = sorted(
        (
            p["segments"],
            p["bits_per_second"],
            p["configured_bits_per_second"],
            json.dumps(p["rung_share"]),
        )
        for p in recorded
    )
    report = check(CAPTURE, hls_labels()).to_dict()
    got = sorted(
        (
            p["requests"],
            p["bits_per_second"],
            p["configured_bits_per_second"],
            json.dumps({k: v for k, v in sorted(p["rung_share"].items())}),
        )
        for entry in report["rungs"]
        for p in entry["playbacks"]
    )
    assert got == want


def test_a_wrong_segment_cadence_fails_in_every_kind_of_playback(tmp_path: Path) -> None:
    slow = tmp_path / "slow.pcap"
    retime(slow, 1.15)  # 460 ms between segments, 60 ms off, outside the 40 ms tolerance
    result = check(slow, hls_labels())
    assert not result.passed
    named = " ".join(result.failures)
    for application in ("hls_vod", "hls_live", "hls_segment"):
        assert f"application {application}" in named, result.failures
    assert all("not paced at 400 ms" in f for f in result.failures if "packet gaps" in f)


def test_a_cadence_that_drifts_inside_the_tolerance_fails_on_the_mean(tmp_path: Path) -> None:
    drift = tmp_path / "drift.pcap"
    retime(drift, 1.06)  # 424 ms: every gap within 40 ms, the mean 24 ms off against 20
    result = check(drift, hls_labels())
    assert not result.passed
    assert all("the mean gap of the" in f for f in result.failures), result.failures
    section = hls_section()
    for entry in section["packets"]["applications"].values():
        del entry["mean_tolerance_ms"]
    assert check(drift, hls_labels(section)).passed  # the mean is what the new field adds


def test_a_faster_cadence_fails_too(tmp_path: Path) -> None:
    fast = tmp_path / "fast.pcap"
    retime(fast, 0.8)
    assert not check(fast, hls_labels()).passed


def test_a_wrong_bit_rate_in_the_labels_fails_on_the_committed_bytes() -> None:
    section = hls_section()
    section["rungs"]["applications"]["hls_vod"]["bits_per_second"] = [160000, 320000, 700000]
    result = check(CAPTURE, hls_labels(section))
    assert not result.passed
    (reason,) = result.failures
    assert reason.startswith("bytes per rung of application hls_vod: 3 of 3 streams")
    assert "body of" in reason


# --- synthetic playbacks ------------------------------------------------------------------------


def playback(
    cap: pcapgen.Capture,
    port: int,
    rungs: list[int],
    gap: float = 0.4,
    scale: dict[int, float] | None = None,
    playlist: bool = True,
    server_port: int = 8090,
    chunked: bool = False,
    at: float | None = None,
    pad: dict[int, int] | None = None,
) -> None:
    """Append one keepalive connection that fetches a segment of each given rung every ``gap``.

    With ``playlist`` a playlist request and its response come 20 ms before each segment
    request, as a player that refreshes its playlist does. ``scale`` multiplies the body of a
    rung, to give it the wrong bit rate, and ``pad`` adds octets of response header to a rung,
    which the bytes on the wire count and the body does not.
    """
    client, server = (CLIENT, port), (SERVER, server_port)
    cs, ss = 1000, 5000
    base = cap.clock + 1.0 if at is None else at
    cap.clock = base - 0.1
    cap.add(pcapgen.tcp(*client, *server, pcapgen.SYN, cs, 0), gap=0.0)
    cap.add(pcapgen.tcp(*server, *client, pcapgen.SYN | pcapgen.ACK, ss, cs + 1), gap=0.0001)
    cap.add(pcapgen.tcp(*client, *server, pcapgen.ACK, cs + 1, ss + 1), gap=0.0001)
    cs, ss = cs + 1, ss + 1

    def exchange(at: float, path: str, body: bytes, padding: int = 0) -> None:
        nonlocal cs, ss
        request = f"GET {path} HTTP/1.1\r\nHost: media.example\r\n\r\n".encode()
        filler = f"X-Padding: {'p' * padding}\r\n" if padding else ""
        if chunked:
            head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
        else:
            head = (f"HTTP/1.1 200 OK\r\n{filler}Content-Length: {len(body)}\r\n\r\n").encode()
        cap.clock = at
        cap.add(pcapgen.tcp(*client, *server, pcapgen.PSH | pcapgen.ACK, cs, ss, request), gap=0.0)
        cs += len(request)
        cap.clock = at + 0.001
        response = head + body
        cap.add(pcapgen.tcp(*server, *client, pcapgen.PSH | pcapgen.ACK, ss, cs, response), gap=0.0)
        ss += len(response)

    for index, rung in enumerate(rungs):
        at = base + index * gap
        if playlist:
            exchange(at - 0.02, "/live.m3u8", b"#EXTM3U\n" * 40)
        size = int(RATES[rung] * 0.4 / 8 * (scale or {}).get(rung, 1.0))
        exchange(at, f"/v{rung}/seg{index}.ts", b"\x47" * size, (pad or {}).get(rung, 0))


def hls_doc(section: dict[str, Any]) -> dict[str, Any]:
    return {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [
            {"application": "hls_vod", "match": {"transport": "tcp", "server_port": 8090}}
        ],
        "temporal": section,
    }


def vod_section(**rungs: Any) -> dict[str, Any]:
    return {
        "packets": {"applications": {"hls_vod": PACED | {"path": SEGMENT}}},
        "rungs": {"applications": {"hls_vod": _rungs(**rungs)}},
    }


def run(tmp_path: Path, cap: pcapgen.Capture, section: dict[str, Any]) -> temporal.TemporalResult:
    cap.frames.sort(key=lambda frame: frame[0])
    pcap = cap.write_pcap(tmp_path / "capture.pcap")
    return check(pcap, parse(hls_doc(section)))


LADDER_WALK = [0, 0, 0, 1, 1, 1, 2, 2, 2]


def test_a_synthetic_playback_with_the_right_bytes_and_cadence_passes(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, LADDER_WALK)
    playback(cap, 40001, LADDER_WALK)
    result = run(tmp_path, cap, vod_section(walk="up", streams=2))
    assert result.passed, result.failures


def test_a_wrong_bit_rate_in_the_capture_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, LADDER_WALK)
    playback(cap, 40001, LADDER_WALK, scale={2: 0.8})  # the top rung runs at 512 kbit/s
    result = run(tmp_path, cap, vod_section(walk="up"))
    assert not result.passed
    (reason,) = result.failures
    assert reason.startswith(
        "bytes per rung of application hls_vod: 1 of 2 streams carry the wrong bytes"
    )
    assert "session 2" in reason and "body of 25600 octets against 32000" in reason


def test_the_share_counts_the_head_of_a_response_and_fails_with_exact_bodies(
    tmp_path: Path,
) -> None:
    """hls_check counts the bytes on the wire, so a padded head moves the share."""
    cap = pcapgen.Capture()
    playback(cap, 40000, LADDER_WALK, pad={0: 700})
    result = run(tmp_path, cap, vod_section())
    assert not result.passed
    (playback_entry,) = result.rungs[0]["playbacks"]
    assert [f.split(" carries ")[0] for f in playback_entry["failures"]] == ["rung 0"]
    assert playback_entry["rung_share"]["0"] > 0.15  # against 14.3% by the bit rates


def test_a_bit_rate_that_is_off_on_the_wire_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, LADDER_WALK, pad={0: 1000, 1: 1000, 2: 1000})
    result = run(tmp_path, cap, vod_section())
    assert not result.passed
    (playback_entry,) = result.rungs[0]["playbacks"]
    assert any(f.startswith("bit rate ") for f in playback_entry["failures"])
    assert playback_entry["bits_per_second"] > 1.05 * playback_entry["configured_bits_per_second"]


def test_rungs_out_of_order_fail_the_walk(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, [2, 2, 2, 1, 1, 1, 0, 0, 0])
    result = run(tmp_path, cap, vod_section(walk="up"))
    assert not result.passed
    assert "do not walk up the whole ladder" in result.failures[0]


def test_a_playback_that_skips_a_rung_fails_the_walk(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, [0, 0, 0, 1, 1, 1, 1, 1, 1])
    result = run(tmp_path, cap, vod_section(walk="up"))
    assert not result.passed
    assert "[0, 1] do not walk up the whole ladder" in result.failures[0]


def test_a_rung_outside_the_ladder_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, [0, 1, 0, 0])
    section = vod_section()
    section["rungs"]["applications"]["hls_vod"]["bits_per_second"] = [160000]
    result = run(tmp_path, cap, section)
    assert not result.passed
    assert "names rung 1 of a ladder with 1 rungs" in result.failures[0]


def test_the_plain_packet_gap_cannot_judge_a_keepalive_but_the_path_can(tmp_path: Path) -> None:
    """Playlist requests 20 ms before each segment make half of the client gaps 20 ms."""
    cap = pcapgen.Capture()
    playback(cap, 40000, LADDER_WALK)
    plain = {"packets": {"applications": {"hls_vod": {**PACED, "path": None}}}}
    del plain["packets"]["applications"]["hls_vod"]["path"]
    plain["packets"]["applications"]["hls_vod"]["min_samples"] = 8
    assert not run(tmp_path, cap, plain).passed
    cap2 = pcapgen.Capture()
    playback(cap2, 40000, LADDER_WALK)
    assert run(tmp_path, cap2, vod_section()).passed


def test_a_stream_with_too_few_requests_is_not_judged_and_the_count_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, LADDER_WALK)
    playback(cap, 40001, [0, 1, 2])  # three segments, and the labels need four
    result = run(tmp_path, cap, vod_section(streams=2))
    assert not result.passed
    entry = result.to_dict()["rungs"][0]
    assert entry["streams"] == {"judged": 1, "failed": 0, "too_short": 1}
    assert "1 streams have at least 4 requests, and the labels expect 2" in entry["reason"]
    packets = result.to_dict()["comparisons"][0]
    assert packets["streams"] == {"judged": 1, "failed": 0, "too_short": 1}  # 3 gaps needed


def test_no_stream_long_enough_is_insufficient_data(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, [0, 1, 2])
    result = run(tmp_path, cap, vod_section())
    assert not result.passed
    statuses = {c["status"] for c in [*result.comparisons, *result.rungs]}
    assert statuses == {"insufficient_data"}
    assert result.to_dict()["insufficient_comparisons"] == 2
    assert "no stream has the 4 requests that are needed" in result.rungs[0]["reason"]


def test_the_target_minimum_replaces_the_one_of_the_section(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, LADDER_WALK)
    section = vod_section()
    section["min_samples"] = 30
    result = run(tmp_path, cap, section)
    assert result.passed, result.failures
    assert result.comparisons[0]["min_samples"] == 3
    assert result.rungs[0]["min_samples"] == 4
    section["packets"]["applications"]["hls_vod"].pop("min_samples")
    section["rungs"]["applications"]["hls_vod"].pop("min_samples")
    assert not run(tmp_path, cap, section).passed  # the section's 30 applies again


def test_a_path_comparison_has_no_ks_series(tmp_path: Path) -> None:
    """Two paths of one application and direction would give two series with equal labels."""
    cap = pcapgen.Capture()
    for index in range(3):
        playback(cap, 40000 + index, LADDER_WALK)
    section = {
        "packets": {
            "applications": {
                "hls_vod": [
                    {
                        "direction": "client",
                        "path": SEGMENT,
                        "dist": "exponential",
                        "mean_ms": 400,
                        "min_samples": 8,
                    },
                    {
                        "direction": "client",
                        "path": r"/live\.m3u8$",
                        "dist": "exponential",
                        "mean_ms": 400,
                        "min_samples": 8,
                    },
                ]
            }
        }
    }
    result = run(tmp_path, cap, section)
    assert [c["status"] for c in result.comparisons] == ["fail", "fail"]  # constant gaps
    assert [m["name"] for m in result.metrics()] == ["realism.check_failures"]


def test_a_session_that_is_not_http_is_named_and_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    playback(cap, 40000, LADDER_WALK, chunked=True)
    result = run(tmp_path, cap, vod_section())
    assert not result.passed
    assert all(
        "cannot be read as HTTP/1.1" in f and "has no Content-Length" in f for f in result.failures
    )
    assert len(result.failures) == 2


def test_joined_sessions_split_into_playbacks_at_the_gap(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    start = cap.clock + 1.0
    for index in range(6):  # one request per connection, the cadence of vod_segments
        playback(cap, 40100 + index, [index // 2], playlist=False, at=start + index * 0.4)
    section = vod_section(join_ms=800, min_samples=4)
    section["packets"]["applications"]["hls_vod"]["join_ms"] = 800
    result = run(tmp_path, cap, section)
    assert result.passed, result.failures
    assert result.rungs[0]["streams"] == {"judged": 1, "failed": 0, "too_short": 0}
    assert result.rungs[0]["playbacks"][0]["name"].startswith("run of 6 sessions")


# --- the report ---------------------------------------------------------------------------------


STUB = StubClassifier.by_server_port({}, name="ndpi")


def validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(report)


def test_the_report_with_rung_entries_validates_and_counts_the_failure(tmp_path: Path) -> None:
    labels_path = tmp_path / "labels.json"
    doc = yaml.safe_load((FIXTURE / "expected.yaml").read_text(encoding="utf-8"))
    doc["temporal"] = hls_section()
    labels_path.write_text(json.dumps(doc))
    good = score(CAPTURE, labels_path, STUB).to_dict()
    validate(good)
    entry = next(c for c in good["checks"] if c["check"] == "temporal")
    assert entry["passed"] and len(entry["rungs"]) == 3
    # The catalog has no series for the rungs, so only the arrivals have KS series and the
    # failures series closes the list.
    assert {m["labels"].get("property") for m in entry["metrics"]} == {"inter_arrival", None}
    assert entry["metrics"][-1]["name"] == "realism.check_failures"
    assert entry["metrics"][-1]["value"] == 0
    slow = tmp_path / "slow.pcap"
    retime(slow, 1.15)
    bad = score(slow, labels_path, STUB).to_dict()
    validate(bad)
    failed = next(c for c in bad["checks"] if c["check"] == "temporal")
    assert not failed["passed"]
    assert failed["metrics"][-1]["value"] == len(failed["failures"]) >= 3
    doc["temporal"]["rungs"]["applications"]["hls_vod"]["bits_per_second"] = [1, 1, 1]
    labels_path.write_text(json.dumps(doc))
    wrong = score(CAPTURE, labels_path, STUB).to_dict()
    validate(wrong)
    entry = next(c for c in wrong["checks"] if c["check"] == "temporal")
    assert [r["status"] for r in entry["rungs"]] == ["fail", "pass", "pass"]
    assert entry["metrics"][-1]["value"] == 1


def test_a_report_of_format_0_19_0_still_validates() -> None:
    """The new fields are optional, so a report of the runner before RLM-22 stays valid."""
    validate(score_report_without_rungs())


def score_report_without_rungs() -> dict[str, Any]:
    """A report of the committed capture from before RLM-22: no rungs, path, or min_samples."""
    labels = parse(yaml.safe_load((FIXTURE / "expected.yaml").read_text(encoding="utf-8")))
    result = check(CAPTURE, labels)
    entry = result.to_dict()
    del entry["rungs"]
    for comparison in entry["comparisons"]:
        del comparison["path"], comparison["min_samples"]
    summary = summarize(CAPTURE)
    return {
        "report": "fps-realism-report",
        "version": "0.19.0",
        "generator": "test",
        "passed": entry["passed"],
        "capture": {
            "file": CAPTURE.name,
            "sha256": "0" * 64,
            "format": "pcapng",
            "link_types": [1],
            "frames": summary.frames,
            "skipped_frames": summary.skipped_frames,
            "sessions": len(summary.sessions),
        },
        "labels": {"file": "expected.yaml", "sha256": "0" * 64},
        "checks": [entry],
    }


# --- labels -------------------------------------------------------------------------------------


def packets(entry: dict[str, Any], where: str = "applications") -> dict[str, Any]:
    section: dict[str, Any] = {
        "packets": {where: {"hls_vod": entry} if where == "applications" else entry}
    }
    return section


@pytest.mark.parametrize(
    ("section", "message"),
    [
        (packets(PACED | {"path": SEGMENT, "direction": "server"}), "needs direction client"),
        (packets(PACED | {"path": SEGMENT, "direction": "both"}), "needs direction client"),
        (packets(PACED | {"path": SEGMENT}, "overall"), "belongs under applications"),
        (packets(PACED | {"path": "("}), "not a regular expression"),
        (packets(PACED | {"path": ""}), "must be a regular expression"),
        (packets(PACED | {"join_ms": 800}), "needs a path"),
        (packets(PACED | {"path": SEGMENT, "join_ms": 0}), "join_ms: must be a number above 0"),
        (packets(PACED | {"min_samples": 1}), "min_samples: must be an integer of 2 or more"),
        (packets(PACED | {"min_samples": True}), "min_samples: must be an integer of 2 or more"),
        (packets(PACED | {"mean_tolerance_ms": -1}), "mean_tolerance_ms: must be a number"),
        (
            {
                "packets": {
                    "applications": {
                        "hls_vod": [PACED | {"path": SEGMENT}, PACED | {"path": SEGMENT}]
                    }
                }
            },
            "has two targets for the path",
        ),
        ({"rungs": []}, "temporal.rungs: must be a mapping with only applications"),
        ({"rungs": {"applications": {}}}, "must be a non empty mapping"),
        ({"rungs": {"applications": {"hls_vod": _rungs(path=SEGMENT)}}}, "capture group"),
        ({"rungs": {"applications": {"hls_vod": _rungs(path="(")}}}, "not a regular expression"),
        ({"rungs": {"applications": {"hls_vod": _rungs(bits_per_second=[])}}}, "bit rates above 0"),
        (
            {"rungs": {"applications": {"hls_vod": _rungs(bits_per_second=[0])}}},
            "bit rates above 0",
        ),
        ({"rungs": {"applications": {"hls_vod": _rungs(duration_ms=0)}}}, "duration_ms"),
        ({"rungs": {"applications": {"hls_vod": _rungs(walk="down")}}}, "walk: must be up"),
        ({"rungs": {"applications": {"hls_vod": _rungs(streams=-1)}}}, "streams"),
        ({"rungs": {"applications": {"hls_vod": _rungs(min_samples=1)}}}, "min_samples"),
        ({"rungs": {"applications": {"hls_vod": _rungs(share_tolerance=-0.1)}}}, "share_tolerance"),
        (
            {"rungs": {"applications": {"hls_vod": _rungs(color="red")}}},
            "must be a mapping with only",
        ),
    ],
)
def test_the_labels_reject_a_wrong_target(section: dict[str, Any], message: str) -> None:
    with pytest.raises(LabelsError, match=message):
        parse(hls_doc(section))


def test_the_labels_keep_what_they_were_given() -> None:
    labels = parse(hls_doc(hls_section()))
    assert labels.temporal is not None
    by_app = {t.application: t for t in labels.temporal.targets if t.path}
    assert by_app["hls_segment"].join == pytest.approx(0.8)
    assert by_app["hls_vod"].join is None
    assert by_app["hls_vod"].min_samples == 3
    assert by_app["hls_vod"].distribution.mean_tolerance == pytest.approx(0.02)
    rungs = {r.application: r for r in labels.temporal.rungs}
    assert rungs["hls_vod"].walk_up and not rungs["hls_live"].walk_up
    assert rungs["hls_vod"].streams is not None and rungs["hls_vod"].streams.holds(3)
    assert rungs["hls_vod"].duration == pytest.approx(0.4)
    assert rungs["hls_vod"].body_tolerance == 188
    assert rungs["hls_vod"].share_tolerance == rungs["hls_vod"].rate_tolerance == 0.05


def test_the_labels_schema_accepts_the_new_sections() -> None:
    schema = json.loads((ROOT / "realism" / "schema" / "labels.schema.json").read_text())
    doc = yaml.safe_load((FIXTURE / "expected.yaml").read_text(encoding="utf-8"))
    doc["temporal"] = hls_section()
    jsonschema.Draft202012Validator(schema).validate(doc)
    doc["temporal"]["rungs"]["applications"]["hls_vod"]["color"] = "red"
    with pytest.raises(jsonschema.ValidationError):
        jsonschema.Draft202012Validator(schema).validate(doc)
