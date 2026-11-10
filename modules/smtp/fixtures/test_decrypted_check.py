# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the view building in decrypted_check.py, which need no tool.

Run them with ``python3 -m unittest discover -s modules/smtp/fixtures``.
"""

from __future__ import annotations

import struct
import tempfile
import unittest
from pathlib import Path

import decrypted_check


def session() -> decrypted_check.Session:
    s = decrypted_check.Session(3, 40, ("10.201.0.2", 40001), ("10.202.0.2", 5587))
    s.before = [
        (False, b"220 submit.example.test ESMTP ready\r\n"),
        (True, b"EHLO laptop.example.test\r\n"),
        (False, b"250-submit.example.test Hello\r\n250-STARTTLS\r\n250 SMTPUTF8\r\n"),
        (True, b"STARTTLS\r\n"),
        (False, b"220 2.0.0 Ready to start TLS\r\n"),
    ]
    s.inside = [
        (True, b"EHLO laptop.example.test\r\n"),
        (False, b"250-submit.example.test Hello\r\n250 AUTH PLAIN\r\n"),
    ]
    return s


class SessionTest(unittest.TestCase):
    def test_the_starttls_exchange_is_recognized(self) -> None:
        self.assertTrue(session().starttls_offered())
        s = session()
        s.before = s.before[:3]
        self.assertFalse(s.starttls_offered(), "the client never asked")

    def test_a_server_that_does_not_offer_starttls_is_not_recognized(self) -> None:
        s = session()
        s.before[2] = (False, b"250-submit.example.test Hello\r\n250 SMTPUTF8\r\n")
        self.assertFalse(s.starttls_offered())

    def test_the_view_leaves_out_the_command_and_its_reply_only(self) -> None:
        kept = session().upgraded_in_dialogue()
        self.assertEqual(len(kept), 3)
        joined = b"".join(data for _, data in kept)
        self.assertNotIn(b"STARTTLS\r\n", joined.replace(b"250-STARTTLS\r\n", b""))
        self.assertNotIn(b"Ready to start TLS", joined)
        self.assertIn(b"220 submit.example.test", joined)
        self.assertIn(b"250-STARTTLS", joined, "the offer in the EHLO reply stays")


class ViewTest(unittest.TestCase):
    def test_the_view_holds_one_plain_session_per_input_on_the_view_port(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "view.pcap"
            decrypted_check.write_view(path, [session(), session()], 587)
            data = path.read_bytes()
        magic, _, _, _, _, _, link = struct.unpack("<IHHiIII", data[:24])
        self.assertEqual((magic, link), (0xA1B2C3D4, 1))
        pos, ports = 24, []
        while pos < len(data):
            _, _, caplen, _ = struct.unpack("<IIII", data[pos:pos + 16])
            frame = data[pos + 16:pos + 16 + caplen]
            sport, dport = struct.unpack("!HH", frame[34:38])
            ports.append((sport, dport))
            pos += 16 + caplen
        self.assertTrue(all(587 in pair for pair in ports))
        self.assertFalse(any(5587 in pair for pair in ports))
        # Handshake of three frames, one frame per chunk, and two FINs, per session.
        self.assertEqual(len(ports), 2 * (3 + 3 + 2 + 2))

    def test_a_missing_tool_gives_exit_code_2(self) -> None:
        rc = decrypted_check.main(["none.pcapng", "--sessions", "1", "--tshark", "/nonexistent/tshark"])
        self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main()
