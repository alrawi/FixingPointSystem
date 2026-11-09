# SPDX-License-Identifier: GPL-3.0-only
"""Tests of zeek_check.py with synthetic Zeek logs, which need no Zeek.

Run them with ``python3 -m unittest discover -s modules/smtp/fixtures``.
"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import zeek_check


def message(i: int, fuids: list[str]) -> dict:
    return {
        "uid": f"C{i}",
        "helo": "client.example.test",
        "mailfrom": "a.b@example.test",
        "rcptto": ["c.d@example.test"],
        "date": "Sat, 25 Jul 2026 17:57:51 +0000",
        "from": "a.b@example.test",
        "to": ["c.d@example.test"],
        "subject": f"Statement {i}",
        "msg_id": f"<{i}@example.test>",
        "fuids": fuids,
    }


def good_logs() -> dict[str, list[dict]]:
    return {
        "smtp": [message(1, ["F1", "F2"]), message(2, ["F3"])],
        "files": [
            {"fuid": "F1", "source": "SMTP", "mime_type": "text/plain", "seen_bytes": 90},
            {"fuid": "F2", "source": "SMTP", "filename": "scan.png", "seen_bytes": 5000},
            {"fuid": "F3", "source": "SMTP", "mime_type": "text/plain", "seen_bytes": 70},
        ],
        "weird": [],
    }


class CheckTest(unittest.TestCase):
    def test_a_complete_capture_passes(self) -> None:
        self.assertEqual(zeek_check.check(good_logs(), 2, 1), [])

    def test_a_missing_message_fails(self) -> None:
        logs = good_logs()
        logs["smtp"].pop()
        errors = zeek_check.check(logs, 2, 1)
        self.assertTrue(any("1 messages, expected 2" in e for e in errors), errors)

    def test_a_message_without_header_fields_fails(self) -> None:
        logs = good_logs()
        del logs["smtp"][0]["subject"]
        logs["smtp"][1]["rcptto"] = []
        errors = zeek_check.check(logs, 2, 1)
        self.assertTrue(any("row 1 lacks subject" in e for e in errors), errors)
        self.assertTrue(any("row 2 lacks rcptto" in e for e in errors), errors)

    def test_a_missing_attachment_fails(self) -> None:
        logs = good_logs()
        del logs["files"][1]["filename"]
        errors = zeek_check.check(logs, 2, 1)
        self.assertTrue(any("0 named attachments, expected 1" in e for e in errors), errors)

    def test_a_truncated_attachment_fails(self) -> None:
        logs = good_logs()
        logs["files"][1]["missing_bytes"] = 12
        logs["files"][1]["seen_bytes"] = 0
        errors = zeek_check.check(logs, 2, 1)
        self.assertTrue(any("has no bytes" in e for e in errors), errors)
        self.assertTrue(any("lost 12 bytes" in e for e in errors), errors)

    def test_an_unlisted_attachment_fails(self) -> None:
        logs = good_logs()
        logs["smtp"][0]["fuids"] = ["F1"]
        errors = zeek_check.check(logs, 2, 1)
        self.assertTrue(any("not listed in smtp.log" in e for e in errors), errors)

    def test_only_smtp_weirds_fail(self) -> None:
        logs = good_logs()
        logs["weird"] = [{"name": "dns_unmatched_reply"}]
        self.assertEqual(zeek_check.check(logs, 2, 1), [])
        logs["weird"] = [{"name": "smtp_nested_mail_transaction", "id.orig_h": "10.0.0.1"}]
        errors = zeek_check.check(logs, 2, 1)
        self.assertTrue(any("smtp_nested_mail_transaction" in e for e in errors), errors)

    def test_files_of_other_protocols_do_not_count(self) -> None:
        logs = good_logs()
        logs["files"].append({"fuid": "F9", "source": "HTTP", "filename": "x.png", "seen_bytes": 1})
        self.assertEqual(zeek_check.check(logs, 2, 1), [])


class ReadLogTest(unittest.TestCase):
    def test_reads_json_lines_and_a_missing_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "smtp.log"
            path.write_text(json.dumps({"a": 1}) + "\n\n" + json.dumps({"a": 2}) + "\n")
            self.assertEqual(zeek_check.read_log(path), [{"a": 1}, {"a": 2}])
            self.assertEqual(zeek_check.read_log(Path(tmp) / "none.log"), [])

    def test_a_missing_program_gives_exit_code_2(self) -> None:
        rc = zeek_check.main(
            ["x.pcapng", "--messages", "1", "--attachments", "0", "--zeek", "/nonexistent/zeek"]
        )
        self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main()
