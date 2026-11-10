# SPDX-License-Identifier: GPL-3.0-only
"""Tests of rtp_check.py with synthetic packets, which need no tshark.

Run them with ``python3 -m unittest discover -s modules/sip_rtp/fixtures``.
"""

from __future__ import annotations

import unittest

import rtp_check
from rtp_check import Packet


def stream(
    gaps_ms: list[float], pt: int = 0, step: int = 160, key: str = "a"
) -> list[Packet]:
    """Packets whose timestamps step by `step` and whose arrivals follow `gaps_ms`."""
    out, t = [], 100.0
    for i in range(len(gaps_ms) + 1):
        out.append(
            Packet(
                t, ("10.0.0.1", "4000", "10.0.0.2", "5000", key), pt, 1000 + i * step
            )
        )
        if i < len(gaps_ms):
            t += gaps_ms[i] / 1000
    return out


def errors(packets: list[Packet]) -> list[str]:
    return [e for r in rtp_check.analyze(packets) for e in r.errors]


class RtpCheckTest(unittest.TestCase):
    def test_a_steady_stream_passes(self) -> None:
        self.assertEqual(errors(stream([20.0] * 50)), [])

    def test_jitter_within_the_tolerance_passes(self) -> None:
        self.assertEqual(errors(stream([19.0, 21.0] * 25)), [])

    def test_a_drifting_stream_fails(self) -> None:
        # A plain sleep per packet adds a little to every gap, here 1.5 ms.
        found = errors(stream([21.5] * 50))
        self.assertEqual(len(found), 1)
        self.assertIn("mean inter arrival", found[0])

    def test_a_burst_fails(self) -> None:
        # The mean is right, but half of the packets arrive back to back.
        found = errors(stream([1.0, 39.0] * 25))
        self.assertTrue(any("within" in e for e in found))

    def test_the_interval_comes_from_the_timestamps(self) -> None:
        # Opus counts at 48000 hertz, so 960 ticks are 20 ms and 30 ms gaps fail.
        self.assertEqual(errors(stream([20.0] * 50, pt=111, step=960)), [])
        self.assertTrue(errors(stream([30.0] * 50, pt=111, step=960)))

    def test_a_short_stream_fails(self) -> None:
        self.assertTrue(errors(stream([20.0] * 3)))

    def test_an_unknown_payload_type_fails(self) -> None:
        self.assertTrue(errors(stream([20.0] * 50, pt=96)))

    def test_timestamps_wrap_around(self) -> None:
        packets = [
            Packet(
                100 + i * 0.02,
                ("a", "1", "b", "2", "s"),
                0,
                (2**32 - 320 + i * 160) % 2**32,
            )
            for i in range(40)
        ]
        self.assertEqual(errors(packets), [])

    def test_each_direction_is_its_own_stream(self) -> None:
        both = stream([20.0] * 50, key="a") + stream([21.5] * 50, key="b")
        results = rtp_check.analyze(both)
        self.assertEqual(sorted(bool(r.errors) for r in results), [False, True])

    def test_tshark_output_parses(self) -> None:
        line = "1794332812.5\t10.0.0.1\t4000\t10.0.0.2\t5000\t0x1234\t0\t99"
        packets = rtp_check.parse(line + "\nnot\ta\trow\n")
        self.assertEqual(len(packets), 1)
        self.assertEqual(packets[0].timestamp, 99)


if __name__ == "__main__":
    unittest.main()
