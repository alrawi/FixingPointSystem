# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the view building in decrypted_check.py, which need no tool.

Run them with ``python3 -m unittest discover -s modules/imap/fixtures``.
"""

from __future__ import annotations

import struct
import tempfile
import unittest
from pathlib import Path

import decrypted_check

LITERAL_BODY = b"From: a@example.test\r\nSubject: Documents 1234\r\n\r\nHello\r\n"


def session() -> decrypted_check.Session:
    s = decrypted_check.Session(3, 40, ("10.201.0.2", 40001), ("10.202.0.2", 1143))
    s.before = [
        (False, b"* OK [CAPABILITY IMAP4rev1 STARTTLS LOGINDISABLED] imap.example.test ready\r\n"),
        (True, b"A001 CAPABILITY\r\n"),
        (False, b"* CAPABILITY IMAP4rev1 STARTTLS LOGINDISABLED\r\nA001 OK listed\r\n"),
        (True, b"A002 STARTTLS\r\n"),
        (False, b"A002 OK Begin TLS negotiation now.\r\n"),
    ]
    fetch = b"* 1 FETCH (UID 7 BODY[] {%d}\r\n%s)\r\nA005 OK Fetch completed\r\n" % (
        len(LITERAL_BODY), LITERAL_BODY)
    s.inside = [
        (True, b"A003 CAPABILITY\r\n"),
        (False, b"* CAPABILITY IMAP4rev1 AUTH=PLAIN\r\nA003 OK listed\r\n"),
        (True, b"A005 FETCH 1 (UID FLAGS BODY.PEEK[])\r\n"),
        (False, fetch),
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
        s.before[0] = (False, b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
        s.before[2] = (False, b"* CAPABILITY IMAP4rev1\r\nA001 OK listed\r\n")
        self.assertFalse(s.starttls_offered())

    def test_a_server_that_refuses_is_not_recognized(self) -> None:
        s = session()
        s.before[4] = (False, b"A002 NO TLS not available\r\n")
        self.assertFalse(s.starttls_offered())

    def test_a_downloaded_message_is_a_literal_with_a_subject_header(self) -> None:
        self.assertEqual(session().downloaded_messages(), 1)
        s = session()
        s.inside[3] = (False, b"* 1 FETCH (UID 7 BODY[] {5}\r\nabcde)\r\n")
        self.assertEqual(s.downloaded_messages(), 0, "no RFC 5322 header")
        s.inside[3] = (False, b"* 1 FETCH (UID 7 BODY[] {99}\r\nshort)\r\n")
        self.assertEqual(s.downloaded_messages(), 0, "the literal is cut short")


class ViewTest(unittest.TestCase):
    def test_the_view_holds_the_dialogue_inside_tls_on_the_view_port(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "view.pcap"
            decrypted_check.write_view(path, [session(), session()], 143)
            data = path.read_bytes()
        magic, _, _, _, _, _, link = struct.unpack("<IHHiIII", data[:24])
        self.assertEqual((magic, link), (0xA1B2C3D4, 1))
        pos, ports, payload = 24, [], b""
        while pos < len(data):
            _, _, caplen, _ = struct.unpack("<IIII", data[pos:pos + 16])
            frame = data[pos + 16:pos + 16 + caplen]
            sport, dport = struct.unpack("!HH", frame[34:38])
            ports.append((sport, dport))
            payload += frame[54:]
            pos += 16 + caplen
        self.assertTrue(all(143 in pair for pair in ports))
        self.assertFalse(any(1143 in pair for pair in ports))
        # Handshake of three frames, four chunks of one frame each, and two FINs, per session.
        self.assertEqual(len(ports), 2 * (3 + 4 + 2))
        self.assertIn(b"A003 CAPABILITY", payload)
        self.assertNotIn(b"STARTTLS", payload)

    def test_a_missing_tool_gives_exit_code_2(self) -> None:
        rc = decrypted_check.main(["none.pcapng", "--sessions", "1", "--tshark", "/nonexistent/tshark"])
        self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main()
