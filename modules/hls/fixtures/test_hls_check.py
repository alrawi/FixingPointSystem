# SPDX-License-Identifier: GPL-3.0-only
"""Tests of hls_check.py with synthetic captures, which need no tshark.

Run them with ``python3 -m unittest discover -s modules/hls/fixtures``.
"""

from __future__ import annotations

import json
import struct
import tempfile
import unittest
from pathlib import Path

import hls_check

LADDER = "160000@426x240,320000@640x360"
SEGMENT_MS = 400
PORTS = {"vod_ladder": 8090, "vod_segments": 8091, "segments": 8092, "live_refresh": 8093}
CLIENT, SERVER = bytes([10, 0, 0, 2]), bytes([10, 0, 0, 1])


def frame(src: bytes, dst: bytes, sport: int, dport: int, seq: int, payload: bytes) -> bytes:
    tcp = struct.pack("!HHIIBBHHH", sport, dport, seq, 0, 5 << 4, 0x18, 65535, 0, 0) + payload
    ip = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(tcp), 0, 0, 64, 6, 0, src, dst) + tcp
    return b"\x00" * 12 + b"\x08\x00" + ip


class Pcap:
    def __init__(self) -> None:
        self.records: list[tuple[float, bytes]] = []
        self.seq: dict[tuple, int] = {}

    def send(self, t: float, sport: int, dport: int, payload: bytes, *, client: bool) -> None:
        key = (sport, dport, client)
        seq = self.seq.get(key, 1000)
        self.seq[key] = seq + len(payload)
        src, dst = (CLIENT, SERVER) if client else (SERVER, CLIENT)
        a, b = (sport, dport) if client else (dport, sport)
        self.records.append((t, frame(src, dst, a, b, seq, payload)))

    def exchange(self, t: float, sport: int, port: int, target: str, body: int) -> None:
        self.send(t, sport, port, f"GET {target} HTTP/1.1\r\nHost: x\r\n\r\n".encode(), client=True)
        head = f"HTTP/1.1 200 OK\r\nContent-Length: {body}\r\n\r\n".encode()
        self.send(t + 0.001, sport, port, head + b"\x47" * body, client=False)

    def write(self, path: Path) -> None:
        out = struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1)
        for t, data in sorted(self.records, key=lambda r: r[0]):
            out += struct.pack("<IIII", int(t), int((t % 1) * 1e6), len(data), len(data)) + data
        path.write_bytes(out)


def body_of(rung: int) -> int:
    rate = [160000, 320000][rung]
    return round(rate * SEGMENT_MS / 8000 / 188) * 188


def live(gaps_ms: list[float], rung: int = 1, bodies: list[int] | None = None) -> Pcap:
    pc, t = Pcap(), 100.0
    for i in range(len(gaps_ms) + 1):
        size = bodies[i] if bodies else body_of(rung)
        pc.exchange(t, 40000, 8093, f"/v{rung}/seg{i}.ts", size)
        if i < len(gaps_ms):
            t += gaps_ms[i] / 1000
    return pc


class HlsCheckTest(unittest.TestCase):
    def run_check(self, pc: Pcap, sessions: dict | None = None) -> tuple[int, dict]:
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            pc.write(d / "c.pcap")
            params = {
                "ladder": LADDER,
                "segment_ms": SEGMENT_MS,
                "ports": PORTS,
                "sessions": sessions or {"vod_ladder": 0, "vod_segments": 0, "live_refresh": 1},
            }
            (d / "p.json").write_text(json.dumps(params))
            rc = hls_check.main(
                [str(d / "c.pcap"), "--params", str(d / "p.json"), "--output", str(d / "o.json")]
            )
            return rc, json.loads((d / "o.json").read_text()) if (d / "o.json").exists() else {}

    def test_a_steady_playback_passes(self) -> None:
        rc, out = self.run_check(live([400.0] * 8))
        self.assertEqual(rc, 0, out)
        self.assertEqual(out["playbacks"][0]["segments"], 9)

    def test_a_drifting_playback_fails(self) -> None:
        # A plain sleep per segment adds to every gap, here 8 percent.
        rc, out = self.run_check(live([432.0] * 8))
        self.assertEqual(rc, 1)
        self.assertTrue(any("mean gap" in f for f in out["playbacks"][0]["failures"]))

    def test_a_burst_fails(self) -> None:
        rc, out = self.run_check(live([40.0, 760.0] * 4))
        self.assertEqual(rc, 1)
        self.assertTrue(any("of the gaps" in f for f in out["playbacks"][0]["failures"]))

    def test_a_segment_of_the_wrong_size_fails(self) -> None:
        bodies = [body_of(1)] * 8 + [body_of(1) // 2]
        rc, out = self.run_check(live([400.0] * 8, bodies=bodies))
        self.assertEqual(rc, 1)
        self.assertTrue(any("body of" in f for f in out["playbacks"][0]["failures"]))

    def test_the_session_count_must_match(self) -> None:
        rc, _ = self.run_check(live([400.0] * 8), {"vod_ladder": 0, "vod_segments": 0, "live_refresh": 2})
        self.assertEqual(rc, 1)

    def test_segment_connections_form_one_playback(self) -> None:
        pc = Pcap()
        for i in range(8):
            pc.exchange(100.0 + i * 0.4, 41000 + i, 8092, f"/v0/seg{i}.ts", body_of(0))
        rc, out = self.run_check(pc, {"vod_ladder": 0, "vod_segments": 1, "live_refresh": 0})
        # The playback walks one rung only, so it fails the ladder walk, which vod_* playbacks need.
        self.assertEqual(rc, 1)
        self.assertEqual(out["playbacks"][0]["segments"], 8)
        self.assertEqual(len(out["playbacks"]), 1)

    def test_the_ladder_parses(self) -> None:
        self.assertEqual(hls_check.parse_ladder(LADDER), [160000, 320000])


if __name__ == "__main__":
    unittest.main()
