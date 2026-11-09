# SPDX-License-Identifier: GPL-3.0-only
"""Tests of zeek_check.py with synthetic Zeek logs, which need no Zeek.

Run them with ``python3 -m unittest discover -s modules/ftp/fixtures``.
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import zeek_check


def passive(i: int, command: str, port: int) -> dict:
    return {
        "uid": f"C{i}",
        "command": command,
        "reply_code": 227 if command == "PASV" else 229,
        "data_channel.passive": True,
        "data_channel.resp_p": port,
    }


def transfer(i: int, command: str, code: int = 226) -> dict:
    return {
        "uid": f"C{i}",
        "user": "kamur.teilos",
        "command": command,
        "arg": f"ftp://10.202.0.2/srv/ftp/exports/file{i}.pdf",
        "reply_code": code,
        "reply_msg": "Transfer complete.",
        "fuid": f"F{i}",
    }


def good_logs() -> dict[str, list[dict]]:
    """One RETR over PASV, one STOR over EPSV, and one LIST over PASV."""
    return {
        "ftp": [
            passive(1, "PASV", 50001),
            transfer(1, "RETR"),
            passive(2, "EPSV", 50002),
            transfer(2, "STOR"),
            passive(3, "PASV", 50003),
        ],
        "files": [
            {"fuid": "F1", "source": "FTP_DATA", "seen_bytes": 9000, "id.resp_p": 50001},
            {"fuid": "F2", "source": "FTP_DATA", "seen_bytes": 7000, "id.resp_p": 50002},
            {"fuid": "F9", "source": "FTP_DATA", "seen_bytes": 700, "id.resp_p": 50003},
        ],
        "weird": [{"name": "possible_split_routing"}],
    }


class ZeekCheckTest(unittest.TestCase):
    def test_complete_logs_pass(self) -> None:
        self.assertEqual(zeek_check.check(good_logs(), 3, 2), [])

    def test_a_missing_transfer_row_fails(self) -> None:
        logs = good_logs()
        del logs["ftp"][3]
        errors = zeek_check.check(logs, 3, 2)
        self.assertTrue(any("ftp.log: 1 RETR and STOR rows" in e for e in errors), errors)

    def test_an_error_reply_fails(self) -> None:
        logs = good_logs()
        logs["ftp"][1]["reply_code"] = 550
        errors = zeek_check.check(logs, 3, 2)
        self.assertTrue(any("reply code 550" in e for e in errors), errors)

    def test_an_active_data_channel_fails(self) -> None:
        logs = good_logs()
        logs["ftp"][0]["data_channel.passive"] = False
        errors = zeek_check.check(logs, 3, 2)
        self.assertTrue(any("no passive data channel" in e for e in errors), errors)

    def test_a_missing_file_fails(self) -> None:
        logs = good_logs()
        del logs["files"][1]
        errors = zeek_check.check(logs, 3, 2)
        self.assertTrue(any("FTP_DATA files" in e for e in errors), errors)

    def test_a_data_connection_that_zeek_did_not_tie_to_its_port_fails(self) -> None:
        logs = good_logs()
        logs["files"][2]["id.resp_p"] = 50999
        errors = zeek_check.check(logs, 3, 2)
        self.assertTrue(any("differ from the announced ports" in e for e in errors), errors)

    def test_a_missing_passive_row_fails(self) -> None:
        logs = good_logs()
        del logs["ftp"][4]
        errors = zeek_check.check(logs, 3, 2)
        self.assertTrue(any("2 PASV and EPSV rows, expected 3" in e for e in errors), errors)

    def test_lost_bytes_fail(self) -> None:
        logs = good_logs()
        logs["files"][0]["missing_bytes"] = 100
        errors = zeek_check.check(logs, 3, 2)
        self.assertTrue(any("lost 100 bytes" in e for e in errors), errors)

    def test_an_ftp_weird_fails(self) -> None:
        logs = good_logs()
        logs["weird"].append({"name": "FTP_unexpected_reply", "id.orig_h": "10.0.0.1", "id.resp_h": "10.0.0.2"})
        errors = zeek_check.check(logs, 3, 2)
        self.assertEqual(len(errors), 1, errors)
        self.assertIn("FTP_unexpected_reply", errors[0])

    def test_read_log_parses_json_lines_and_tolerates_a_missing_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "ftp.log"
            path.write_text(json.dumps({"command": "RETR"}) + "\n\n", encoding="utf-8")
            self.assertEqual(zeek_check.read_log(path), [{"command": "RETR"}])
            self.assertEqual(zeek_check.read_log(Path(tmp) / "none.log"), [])


if __name__ == "__main__":
    unittest.main()
