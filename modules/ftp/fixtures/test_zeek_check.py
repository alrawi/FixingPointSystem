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


def transfer(i: int, command: str, code: int = 226) -> dict:
    return {
        "uid": f"C{i}",
        "user": "kamur.teilos",
        "command": command,
        "arg": f"file{i}.pdf",
        "reply_code": code,
        "reply_msg": "Transfer complete.",
        "data_channel.passive": True,
        "fuid": f"F{i}",
    }


def good_logs() -> dict[str, list[dict]]:
    return {
        "ftp": [
            {"command": "PASV", "reply_code": 227},
            transfer(1, "RETR"),
            transfer(2, "STOR"),
            {"command": "LIST", "reply_code": 226},
        ],
        "files": [
            {"fuid": "F1", "source": "FTP_DATA", "mime_type": "application/pdf", "seen_bytes": 9000},
            {"fuid": "F2", "source": "FTP_DATA", "mime_type": "application/zip", "seen_bytes": 7000},
            {"fuid": "F9", "source": "FTP_DATA", "mime_type": "text/plain", "seen_bytes": 700},
        ],
        "weird": [{"name": "possible_split_routing"}],
    }


class ZeekCheckTest(unittest.TestCase):
    def test_complete_logs_pass(self) -> None:
        self.assertEqual(zeek_check.check(good_logs(), 2), [])

    def test_a_missing_transfer_row_fails(self) -> None:
        logs = good_logs()
        del logs["ftp"][2]
        errors = zeek_check.check(logs, 2)
        self.assertTrue(any("ftp.log: 1 RETR and STOR rows" in e for e in errors), errors)

    def test_an_error_reply_fails(self) -> None:
        logs = good_logs()
        logs["ftp"][1]["reply_code"] = 550
        errors = zeek_check.check(logs, 2)
        self.assertTrue(any("reply code 550" in e for e in errors), errors)

    def test_an_active_data_channel_fails(self) -> None:
        logs = good_logs()
        logs["ftp"][1]["data_channel.passive"] = False
        errors = zeek_check.check(logs, 2)
        self.assertTrue(any("no passive data channel" in e for e in errors), errors)

    def test_a_missing_file_fails(self) -> None:
        logs = good_logs()
        del logs["files"][1]
        errors = zeek_check.check(logs, 2)
        self.assertTrue(any("FTP_DATA files" in e for e in errors), errors)

    def test_lost_bytes_fail(self) -> None:
        logs = good_logs()
        logs["files"][0]["missing_bytes"] = 100
        errors = zeek_check.check(logs, 2)
        self.assertTrue(any("lost 100 bytes" in e for e in errors), errors)

    def test_an_ftp_weird_fails(self) -> None:
        logs = good_logs()
        logs["weird"].append({"name": "FTP_unexpected_reply", "id.orig_h": "10.0.0.1", "id.resp_h": "10.0.0.2"})
        errors = zeek_check.check(logs, 2)
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
