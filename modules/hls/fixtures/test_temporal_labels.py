# SPDX-License-Identifier: GPL-3.0-only
"""Tests that the temporal labels of expected.yaml judge the HLS playbacks.

The `temporal` section of `expected.yaml` replaced hls_check.py (MOD-31), which read the
segment cadence inside each playback and the bytes of each rung. These tests read the
committed capture with the temporal check of the realism runner, which is pure Python, so
they need neither tshark nor nDPI. They show that the capture passes, that the same capture
fails when its time is retimed, that a playback with the wrong bit rate fails, and that the
labels agree with params.json, which capture.sh and the fixture peer read.

Run them with ``python3 -m unittest discover -s modules/hls/fixtures``.
"""

from __future__ import annotations

import json
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT))

from realism import temporal  # noqa: E402
from realism.capture import Frame, decode, read_frames, summarize  # noqa: E402
from realism.labels import Labels, parse  # noqa: E402

CAPTURE = HERE / "hls.pcapng"
PARAMS = json.loads((HERE / "params.json").read_text(encoding="utf-8"))
SEGMENT_MS = PARAMS["segment_ms"]
RATES = [int(item.partition("@")[0]) for item in PARAMS["ladder"].split(",")]
PORTS = PARAMS["ports"]
APPLICATIONS = ("hls_vod", "hls_live", "hls_segment")


def document() -> dict[str, Any]:
    return yaml.safe_load((HERE / "expected.yaml").read_text(encoding="utf-8"))


def labels(doc: dict[str, Any] | None = None) -> Labels:
    return parse(doc if doc is not None else document())


def only(application: str) -> Labels:
    """The labels of expected.yaml with the temporal targets of one application."""
    doc = document()
    temporal_section = doc["temporal"]
    del temporal_section["arrivals"]
    for kind in ("packets", "rungs"):
        apps = temporal_section[kind]["applications"]
        temporal_section[kind]["applications"] = {application: apps[application]}
    return labels(doc)


def check(pcap: Path, wanted: Labels | None = None) -> temporal.TemporalResult:
    return temporal.check(pcap, summarize(pcap).sessions, wanted or labels())


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

    Every TCP session of a module port keeps its first packet and runs ``factor`` times
    slower or faster after it, so the cadence of its requests changes and its bytes do not.
    The segment connections of vod_segments (the segment port) stretch from their common
    first packet, because their starts are the cadence of that playback.
    """
    _, stream = read_frames(CAPTURE)
    frames = list(stream)
    ports = set(PORTS.values())

    def client_of(frame: Frame) -> tuple[tuple[str, int], int] | None:
        packet = decode(frame)
        if (
            not packet
            or packet.transport != "tcp"
            or not ports & {packet.src_port, packet.dst_port}
        ):
            return None
        if packet.dst_port in ports:
            return (packet.src_ip, packet.src_port), packet.dst_port
        return (packet.dst_ip, packet.dst_port), packet.src_port

    first: dict[tuple[str, int], float] = {}
    for frame in frames:
        found = client_of(frame)
        if found:
            first.setdefault(found[0], frame.timestamp)
    segment_clients = _segment_clients(frames, PORTS["segments"])
    origin = min(first[client] for client in segment_clients)
    out = []
    for frame in frames:
        found = client_of(frame)
        if found:
            client, _ = found
            base = origin if client in segment_clients else first[client]
            frame = Frame(base + (frame.timestamp - base) * factor, frame.link_type, frame.data)
        out.append(frame)
    write_pcap(path, out)


def _segment_clients(frames: list[Frame], port: int) -> set[tuple[str, int]]:
    clients = set()
    for frame in frames:
        packet = decode(frame)
        if packet and packet.transport == "tcp" and port in (packet.src_port, packet.dst_port):
            clients.add(
                (packet.src_ip, packet.src_port)
                if packet.dst_port == port
                else (packet.dst_ip, packet.dst_port)
            )
    return clients


# A synthetic capture writer for playbacks that the peers did not record. It writes the
# requests and the responses of keepalive or one-shot connections as the module sends them.

CLIENT, SERVER = bytes([10, 201, 0, 2]), bytes([10, 202, 0, 2])


def tcp_frame(src: bytes, dst: bytes, sport: int, dport: int, seq: int, payload: bytes) -> bytes:
    tcp = struct.pack("!HHIIBBHHH", sport, dport, seq, 0, 5 << 4, 0x18, 65535, 0, 0) + payload
    ip = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(tcp), 0, 0, 64, 6, 0, src, dst) + tcp
    return b"\x00" * 12 + b"\x08\x00" + ip


class Synthetic:
    def __init__(self) -> None:
        self.frames: list[Frame] = []
        self.seq: dict[tuple[int, int, bool], int] = {}

    def _send(self, t: float, sport: int, dport: int, payload: bytes, *, client: bool) -> None:
        key = (sport, dport, client)
        seq = self.seq.get(key, 1000)
        self.seq[key] = seq + len(payload)
        src, dst = (CLIENT, SERVER) if client else (SERVER, CLIENT)
        a, b = (sport, dport) if client else (dport, sport)
        self.frames.append(Frame(t, 1, tcp_frame(src, dst, a, b, seq, payload)))

    def exchange(self, t: float, sport: int, port: int, target: str, body: int) -> None:
        self._send(
            t, sport, port, f"GET {target} HTTP/1.1\r\nHost: x\r\n\r\n".encode(), client=True
        )
        head = f"HTTP/1.1 200 OK\r\nContent-Length: {body}\r\n\r\n".encode()
        self._send(t + 0.001, sport, port, head + b"\x47" * body, client=False)

    def keepalive(
        self,
        sport: int,
        port: int,
        rungs: list[int],
        *,
        gaps_ms: list[float] | None = None,
        scale: dict[int, float] | None = None,
        start: float = 100.0,
    ) -> None:
        """One connection that fetches a playlist 20 ms before every segment, as a player does."""
        t = start
        for index, rung in enumerate(rungs):
            self.exchange(t - 0.02, sport, port, "/media.m3u8", 320)
            body = RATES[rung] * SEGMENT_MS // 8000 // 188 * 188
            body = round(body * (scale or {}).get(rung, 1.0))
            self.exchange(t, sport, port, f"/v{rung}/seg{index}.ts", body)
            t += (gaps_ms[index] if gaps_ms and index < len(gaps_ms) else SEGMENT_MS) / 1000

    def one_connection_per_segment(
        self, rungs: list[int], *, scale: dict[int, float] | None = None, start: float = 100.0
    ) -> None:
        for index, rung in enumerate(rungs):
            body = round(
                RATES[rung] * SEGMENT_MS // 8000 // 188 * 188 * (scale or {}).get(rung, 1.0)
            )
            t = start + index * SEGMENT_MS / 1000
            self.exchange(t, 42000 + index, PORTS["segments"], f"/v{rung}/seg{index}.ts", body)

    def write(self, directory: str) -> Path:
        path = Path(directory) / "synthetic.pcap"
        write_pcap(path, self.frames)
        return path


WALK = [0, 0, 0, 1, 1, 1, 2, 2, 2]


class CommittedCaptureTest(unittest.TestCase):
    def test_the_committed_capture_passes_with_its_labels(self) -> None:
        result = check(CAPTURE)
        self.assertTrue(result.passed, result.failures)

    def test_every_playback_is_judged(self) -> None:
        report = check(CAPTURE).to_dict()
        gaps = {c["application"]: c for c in report["comparisons"] if c["property"] == "packet_gap"}
        self.assertEqual(
            {a: (c["streams"]["judged"], c["streams"]["failed"]) for a, c in gaps.items()},
            {
                "hls_vod": (PARAMS["sessions"]["vod_ladder"], 0),
                "hls_live": (PARAMS["sessions"]["live_refresh"], 0),
                "hls_segment": (PARAMS["sessions"]["vod_segments"], 0),
            },
        )
        # One playback of 33 segments, each on its own connection, gives 32 gaps.
        self.assertEqual(gaps["hls_segment"]["n"], PARAMS["vars"]["vod_segments"]["segments"] - 1)
        rungs = {r["application"]: r for r in report["rungs"]}
        self.assertEqual(
            {a: r["streams"]["judged"] for a, r in rungs.items()},
            {
                "hls_vod": 3,
                "hls_live": 3,
                "hls_segment": 1,
            },
        )

    def test_the_bit_rate_of_every_playback_is_near_the_ladder(self) -> None:
        report = check(CAPTURE).to_dict()
        playbacks = [p for entry in report["rungs"] for p in entry["playbacks"]]
        self.assertEqual(len(playbacks), 7)
        for p in playbacks:
            ratio = p["bits_per_second"] / p["configured_bits_per_second"]
            self.assertAlmostEqual(ratio, 1.0, delta=0.01, msg=p)


class RetimedCaptureTest(unittest.TestCase):
    def retimed(self, factor: float) -> temporal.TemporalResult:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "retimed.pcap"
            retime(path, factor)
            return check(path)

    def test_a_slow_cadence_fails_in_every_kind_of_playback(self) -> None:
        result = self.retimed(1.15)  # 460 ms between segments, 60 ms off against 40 ms
        self.assertFalse(result.passed)
        named = " ".join(result.failures)
        for application in APPLICATIONS:
            self.assertIn(f"application {application}", named, result.failures)

    def test_a_fast_cadence_fails_too(self) -> None:
        self.assertFalse(self.retimed(0.8).passed)

    def test_a_drift_inside_the_tolerance_fails_on_the_mean(self) -> None:
        result = self.retimed(1.06)  # 424 ms: every gap within 40 ms, the mean 24 ms off against 20
        self.assertFalse(result.passed)
        self.assertTrue(all("the mean gap of the" in f for f in result.failures), result.failures)
        # Without mean_tolerance_ms the same capture passes, so the label is what catches it.
        doc = document()
        for target in doc["temporal"]["packets"]["applications"].values():
            del target["mean_tolerance_ms"]
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "drift.pcap"
            retime(path, 1.06)
            self.assertTrue(check(path, labels(doc)).passed)


class SyntheticPlaybackTest(unittest.TestCase):
    """Playbacks with the wrong bytes or pacing, which the committed capture does not hold."""

    def run_vod(self, build: Any, application: str = "hls_vod") -> temporal.TemporalResult:
        cap = Synthetic()
        build(cap)
        with tempfile.TemporaryDirectory() as tmp:
            return check(cap.write(tmp), only(application))

    def vod_ladder(self, cap: Synthetic, **extra: Any) -> None:
        for sport in (40000, 40001, 40002):
            cap.keepalive(sport, PORTS["vod_ladder"], WALK, **extra)

    def test_playbacks_with_the_right_bytes_and_cadence_pass(self) -> None:
        self.assertTrue(self.run_vod(self.vod_ladder).passed)

    def test_a_ladder_whose_top_rung_has_the_wrong_bit_rate_fails(self) -> None:
        cap = Synthetic()
        cap.keepalive(40000, PORTS["vod_ladder"], WALK)
        cap.keepalive(40001, PORTS["vod_ladder"], WALK)
        cap.keepalive(40002, PORTS["vod_ladder"], WALK, scale={2: 0.8})
        with tempfile.TemporaryDirectory() as tmp:
            result = check(cap.write(tmp), only("hls_vod"))
        self.assertFalse(result.passed)
        (reason,) = result.failures
        self.assertIn("bytes per rung of application hls_vod: 1 of 3 streams", reason)
        self.assertIn("octets against 32000", reason)  # a body of 25568 octets, 20% short

    def test_a_live_playback_with_the_wrong_bit_rate_fails(self) -> None:
        def build(cap: Synthetic) -> None:
            for sport, scale in ((40037, 1.0), (40038, 1.0), (40039, 1.25)):
                cap.keepalive(sport, PORTS["live_refresh"], [1] * 8, scale={1: scale})

        result = self.run_vod(build, "hls_live")
        self.assertFalse(result.passed)
        self.assertIn("bytes per rung of application hls_live: 1 of 3 streams", result.failures[0])

    def test_live_playbacks_with_the_right_bytes_pass(self) -> None:
        def build(cap: Synthetic) -> None:
            for sport in (40037, 40038, 40039):
                cap.keepalive(sport, PORTS["live_refresh"], [1] * 8)

        result = self.run_vod(build, "hls_live")
        self.assertTrue(result.passed, result.failures)

    def test_segment_connections_with_the_wrong_bit_rate_fail(self) -> None:
        good = self.run_vod(lambda c: c.one_connection_per_segment(WALK), "hls_segment")
        self.assertTrue(good.passed, good.failures)
        bad = self.run_vod(
            lambda c: c.one_connection_per_segment(WALK, scale={0: 2.0}), "hls_segment"
        )
        self.assertFalse(bad.passed)
        self.assertIn("bytes per rung of application hls_segment", bad.failures[0])

    def test_a_drifting_playback_fails(self) -> None:
        # A plain sleep per segment adds to every gap, here 8 percent.
        def build(cap: Synthetic) -> None:
            for sport in (40000, 40001, 40002):
                cap.keepalive(sport, PORTS["vod_ladder"], WALK, gaps_ms=[432.0] * 9)

        result = self.run_vod(build)
        self.assertFalse(result.passed)
        # Every gap is within the 40 ms tolerance, so only the mean can fail it.
        self.assertIn("the mean gap of the", result.failures[0])

    def test_a_burst_fails(self) -> None:
        def build(cap: Synthetic) -> None:
            for sport in (40000, 40001, 40002):
                cap.keepalive(sport, PORTS["vod_ladder"], WALK, gaps_ms=[40.0, 760.0] * 5)

        result = self.run_vod(build)
        self.assertFalse(result.passed)
        self.assertIn("further than 40 ms from the packet time", result.failures[0])

    def test_a_missing_playback_fails_the_stream_count(self) -> None:
        def build(cap: Synthetic) -> None:
            cap.keepalive(40000, PORTS["vod_ladder"], WALK)
            cap.keepalive(40001, PORTS["vod_ladder"], WALK)

        result = self.run_vod(build)
        self.assertFalse(result.passed)
        self.assertEqual(
            result.failures,
            [
                "bytes per rung of application hls_vod: 2 streams have at least 4 requests, "
                "and the labels expect 3"
            ],
        )

    def test_a_vod_playback_that_does_not_walk_up_the_ladder_fails(self) -> None:
        def build(cap: Synthetic) -> None:
            for sport in (40000, 40001, 40002):
                cap.keepalive(sport, PORTS["vod_ladder"], [0] * 9)

        result = self.run_vod(build)
        self.assertFalse(result.passed)
        self.assertIn("do not walk up the whole ladder", result.failures[0])


class LabelsMatchParamsTest(unittest.TestCase):
    """The ladder, the duration, the counts, and the ports are given twice, so they must agree."""

    def setUp(self) -> None:
        self.doc = document()
        self.packets = self.doc["temporal"]["packets"]["applications"]
        self.rungs = self.doc["temporal"]["rungs"]["applications"]

    def test_the_ladder_and_the_duration_are_those_of_params(self) -> None:
        for name, entry in self.rungs.items():
            self.assertEqual(entry["bits_per_second"], RATES, name)
            self.assertEqual(entry["duration_ms"], SEGMENT_MS, name)

    def test_the_cadence_is_the_segment_duration(self) -> None:
        for name, entry in self.packets.items():
            self.assertEqual(entry["ms"], SEGMENT_MS, name)
        arrival = self.doc["temporal"]["arrivals"]["applications"]["hls_segment"]
        self.assertEqual(arrival["ms"], SEGMENT_MS)

    def test_segment_connections_join_within_two_segment_durations(self) -> None:
        self.assertEqual(self.packets["hls_segment"]["join_ms"], 2 * SEGMENT_MS)
        self.assertEqual(self.rungs["hls_segment"]["join_ms"], 2 * SEGMENT_MS)

    def test_the_playback_counts_are_the_sessions_of_the_peers(self) -> None:
        sessions = PARAMS["sessions"]
        self.assertEqual(self.rungs["hls_vod"]["streams"], sessions["vod_ladder"])
        self.assertEqual(self.rungs["hls_segment"]["streams"], sessions["vod_segments"])
        self.assertEqual(self.rungs["hls_live"]["streams"], sessions["live_refresh"])

    def test_the_walk_is_asked_of_the_vod_playbacks_only(self) -> None:
        self.assertEqual(self.rungs["hls_vod"]["walk"], "up")
        self.assertEqual(self.rungs["hls_segment"]["walk"], "up")
        self.assertNotIn("walk", self.rungs["hls_live"])
        stages = PARAMS["vars"]["vod_ladder"]["stages"]
        self.assertEqual(stages, len(RATES))

    def test_a_playback_is_long_enough_for_its_minimum(self) -> None:
        segments = {
            "hls_vod": PARAMS["vars"]["vod_ladder"]["segments"],
            "hls_segment": PARAMS["vars"]["vod_segments"]["segments"],
            "hls_live": PARAMS["vars"]["live_refresh"]["cycles"],
        }
        for name, count in segments.items():
            self.assertGreaterEqual(count, self.rungs[name]["min_samples"], name)
            self.assertGreaterEqual(count - 1, self.packets[name]["min_samples"], name)

    def test_the_applications_map_to_the_ports_of_params(self) -> None:
        ports = {
            entry["application"]: entry["match"]["server_port"] for entry in self.doc["sessions"]
        }
        self.assertEqual(
            ports,
            {
                "hls_vod": PORTS["vod_ladder"],
                "hls_playlist": PORTS["vod_segments"],
                "hls_segment": PORTS["segments"],
                "hls_live": PORTS["live_refresh"],
            },
        )

    def test_the_rung_paths_select_the_segments_that_the_scenarios_request(self) -> None:
        # The rung path is the segment path with the rung number as its first group.
        for name, entry in self.rungs.items():
            self.assertEqual(
                entry["path"].replace(r"(\d+)", r"\d+"), self.packets[name]["path"], name
            )


if __name__ == "__main__":
    unittest.main()
