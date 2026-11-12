# SPDX-License-Identifier: GPL-3.0-only
"""Tests that the packet gap labels of expected.yaml judge the RTP media.

The `temporal.packets` section of `expected.yaml` replaced rtp_check.py (MOD-24). These
tests read the committed capture with the temporal check of the realism runner, which
is pure Python, so they need neither tshark nor nDPI. They show that the capture passes
and that the same capture fails when the packets of one call arrive with jitter, which
is the acceptance of MOD-24.

Run them with ``python3 -m unittest discover -s modules/sip_rtp/fixtures``.
"""

from __future__ import annotations

import random
import struct
import sys
import tempfile
import unittest
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT))

from realism import temporal  # noqa: E402
from realism.capture import Frame, decode, read_frames, summarize  # noqa: E402
from realism.labels import Labels, parse  # noqa: E402

CAPTURE = HERE / "sip_rtp.pcapng"
SIP_PORT = 5060


def labels(section: dict | None = None) -> Labels:
    """The labels of expected.yaml, with the temporal section replaced when given."""
    doc = yaml.safe_load((HERE / "expected.yaml").read_text(encoding="utf-8"))
    if section is not None:
        doc["temporal"] = section
    return parse(doc)


def write_pcap(path: Path, frames: list[Frame]) -> None:
    """Write frames as a classic pcap with microsecond timestamps, in time order."""
    ordered = sorted(frames, key=lambda frame: frame.timestamp)
    link_type = ordered[0].link_type
    out = bytearray(struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, link_type))
    for frame in ordered:
        seconds = int(frame.timestamp)
        micros = round((frame.timestamp - seconds) * 1_000_000)
        out += struct.pack("<IIII", seconds, micros, len(frame.data), len(frame.data))
        out += frame.data
    path.write_bytes(bytes(out))


def jitter_one_call(path: Path, jitter_ms: float) -> None:
    """Write the capture with the packets of its busiest media flow shifted at random."""
    _, frames = read_frames(CAPTURE)
    frames = list(frames)
    count: dict[tuple, int] = {}
    for frame in frames:
        packet = decode(frame)
        if packet and packet.transport == "udp" and SIP_PORT not in (packet.src_port, packet.dst_port):
            key = tuple(sorted([(packet.src_ip, packet.src_port), (packet.dst_ip, packet.dst_port)]))
            count[key] = count.get(key, 0) + 1
    victim = max(count, key=lambda key: count[key])
    rng = random.Random(7)
    shifted = []
    for frame in frames:
        packet = decode(frame)
        if packet and packet.transport == "udp":
            key = tuple(sorted([(packet.src_ip, packet.src_port), (packet.dst_ip, packet.dst_port)]))
            if key == victim:
                frame = Frame(frame.timestamp + rng.uniform(-jitter_ms, jitter_ms) / 1000.0, frame.link_type, frame.data)
        shifted.append(frame)
    write_pcap(path, shifted)


def check(pcap: Path) -> temporal.TemporalResult:
    return temporal.check(pcap, summarize(pcap).sessions, labels())


class PacketLabelsTest(unittest.TestCase):
    def test_the_committed_capture_passes_with_its_labels(self) -> None:
        result = check(CAPTURE)
        self.assertTrue(result.passed, result.failures)
        streams = result.comparisons[0]["streams"]
        self.assertEqual(streams["failed"], 0)
        self.assertGreaterEqual(streams["judged"], 20)  # both directions of every call with media

    def test_a_call_with_jittered_packets_fails_through_the_labels(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            pcap = Path(tmp) / "jittered.pcap"
            jitter_one_call(pcap, 14.0)
            result = check(pcap)
        self.assertFalse(result.passed)
        self.assertEqual(len(result.failures), 1)
        self.assertIn("not paced at", result.failures[0])
        self.assertIn("application rtp", result.failures[0])

    def test_a_little_jitter_is_inside_the_tolerance(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            pcap = Path(tmp) / "late.pcap"
            jitter_one_call(pcap, 1.0)
            result = check(pcap)
        self.assertTrue(result.passed, result.failures)

    def test_the_labels_name_both_packet_times_of_the_scenarios(self) -> None:
        section = yaml.safe_load((HERE / "expected.yaml").read_text(encoding="utf-8"))["temporal"]
        target = section["packets"]["applications"]["rtp"]
        self.assertEqual(target["ms"], [20, 30])
        self.assertEqual(target["dist"], "constant")


if __name__ == "__main__":
    unittest.main()
