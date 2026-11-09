# SPDX-License-Identifier: GPL-3.0-only
"""Tests of structure_check.py with synthetic session documents, which need no
capture tools.

Run them with ``python3 -m unittest discover -s modules/ftp/fixtures``.
"""

from __future__ import annotations

import base64
import unittest

import structure_check

CLIENT = "10.201.0.2"
SERVER = "10.202.0.2"


def turn(direction: str, text: bytes) -> dict:
    return {"direction": direction, "data_b64": base64.b64encode(text).decode()}


def session(sid: int, port: int, start: int, end: int, turns: list[dict] | None = None,
            **overrides: object) -> dict:
    sent = sum(len(base64.b64decode(t["data_b64"])) for t in turns or [] if t["direction"] == "client_to_server")
    got = sum(len(base64.b64decode(t["data_b64"])) for t in turns or [] if t["direction"] == "server_to_client")
    s = {
        "id": sid,
        "protocol": "tcp",
        "client": {"ip": CLIENT, "port": 40000 + sid},
        "server": {"ip": SERVER, "port": port},
        "start_ns": start,
        "end_ns": end,
        "end_reason": "fin",
        "handshake": {"state": "complete"},
        "client_to_server": {"payload_bytes": sent or 1},
        "server_to_client": {"payload_bytes": got or 1},
        "turns": turns or [],
    }
    s.update(overrides)
    return s


def control(sid: int, start: int, end: int, commands: list[bytes], replies: list[bytes]) -> dict:
    return session(
        sid, 21, start, end,
        [turn("client_to_server", b"\r\n".join(commands) + b"\r\n"),
         turn("server_to_client", b"\r\n".join(replies) + b"\r\n")],
    )


PASV_50123 = b"227 Entering Passive Mode (10,202,0,2,195,203)."
EPSV_50124 = b"229 Entering Extended Passive Mode (|||50124|)."


def doc(*sessions: dict) -> dict:
    return {"sessions": list(sessions)}


class StructureCheckTest(unittest.TestCase):
    def good(self) -> list[dict]:
        ctl = control(
            1, 100, 900,
            [b"USER a", b"PASV", b"LIST", b"EPSV", b"RETR f.pdf", b"QUIT"],
            [b"220 hi", PASV_50123, b"150 ok", EPSV_50124, b"150 ok", b"221 bye"],
        )
        return [ctl, session(2, 50123, 200, 300), session(3, 50124, 400, 500)]

    def test_one_control_and_one_data_connection_per_transfer_passes(self) -> None:
        errors, counts = structure_check.check(doc(*self.good()), controls=1, transfers=2)
        self.assertEqual(errors, [])
        self.assertEqual(counts["control"], 1)
        self.assertEqual(counts["data"], 2)
        self.assertEqual(counts["LIST"], 1)
        self.assertEqual(counts["RETR"], 1)

    def test_a_missing_data_connection_fails(self) -> None:
        errors, _ = structure_check.check(doc(*self.good()[:2]))
        self.assertTrue(any("2 transfers but 1 data connections" in e for e in errors), errors)

    def test_a_second_data_connection_for_one_transfer_fails(self) -> None:
        extra = session(4, 50123, 250, 350)
        errors, _ = structure_check.check(doc(*self.good(), extra))
        self.assertTrue(any("2 transfers but 3 data connections" in e for e in errors), errors)

    def test_a_refused_connection_fails(self) -> None:
        refused = session(4, 50123, 150, 160, end_reason="rst",
                          handshake={"state": "incomplete"},
                          client_to_server={"payload_bytes": 0},
                          server_to_client={"payload_bytes": 0})
        errors, _ = structure_check.check(doc(*self.good(), refused))
        self.assertTrue(any("refused or empty" in e for e in errors), errors)

    def test_a_data_connection_to_another_port_fails(self) -> None:
        sessions = self.good()
        sessions[2] = session(3, 50999, 400, 500)
        errors, _ = structure_check.check(doc(*sessions))
        self.assertTrue(any("announced 50124" in e for e in errors), errors)

    def test_a_data_connection_outside_every_control_connection_fails(self) -> None:
        stray = session(9, 50500, 5000, 5100)
        errors, _ = structure_check.check(doc(*self.good(), stray))
        self.assertTrue(any("belongs to no control connection" in e for e in errors), errors)

    def test_a_transfer_without_a_passive_reply_fails(self) -> None:
        ctl = control(1, 100, 900, [b"LIST", b"QUIT"], [b"220 hi", b"150 ok", b"221 bye"])
        errors, _ = structure_check.check(doc(ctl, session(2, 50123, 200, 300)))
        self.assertTrue(any("passive replies" in e for e in errors), errors)

    def test_active_mode_is_reported(self) -> None:
        ctl = control(1, 100, 900, [b"PORT 1,2,3,4,5,6", b"QUIT"], [b"220 hi", b"200 ok", b"221 bye"])
        errors, _ = structure_check.check(doc(ctl))
        self.assertTrue(any("active mode" in e for e in errors), errors)

    def test_expected_totals_are_checked(self) -> None:
        errors, _ = structure_check.check(doc(*self.good()), controls=2, transfers=3)
        self.assertEqual(len(errors), 2, errors)

    def test_a_session_that_ends_with_a_reset_fails(self) -> None:
        sessions = self.good()
        sessions[1] = session(2, 50123, 200, 300, end_reason="rst")
        errors, _ = structure_check.check(doc(*sessions))
        self.assertTrue(any("not FIN" in e for e in errors), errors)

    def test_the_lines_helper_drops_empty_lines(self) -> None:
        self.assertEqual(structure_check.lines(b"A\r\nB\r\n"), [b"A", b"B"])
        self.assertEqual(structure_check.announced_ports([PASV_50123, EPSV_50124, b"200 x"]), [50123, 50124])


if __name__ == "__main__":
    unittest.main()
