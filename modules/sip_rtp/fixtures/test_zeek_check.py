# SPDX-License-Identifier: GPL-3.0-only
"""Tests of zeek_check.py with synthetic Zeek logs, which need no Zeek.

Run them with ``python3 -m unittest discover -s modules/sip_rtp/fixtures``.
"""

from __future__ import annotations

import copy
import unittest

import zeek_check


def rows(call: int) -> list[dict]:
    return [
        {"call_id": f"c{call}@host", "method": "INVITE", "status_code": 200},
        {"call_id": f"c{call}@host", "method": "BYE", "status_code": 200},
    ]


def good_logs() -> dict[str, list[dict]]:
    return {
        "sip": rows(1) + rows(2) + rows(3),
        "weird": [{"name": "possible_split_routing"}],
    }


class ZeekCheckTest(unittest.TestCase):
    def test_complete_logs_pass(self) -> None:
        self.assertEqual(zeek_check.check(good_logs(), 3), [])

    def test_missing_call_fails(self) -> None:
        self.assertTrue(zeek_check.check(good_logs(), 4))

    def test_calls_sharing_a_call_id_fail(self) -> None:
        logs = copy.deepcopy(good_logs())
        logs["sip"][2]["call_id"] = "c1@host"
        logs["sip"][3]["call_id"] = "c1@host"
        self.assertTrue(zeek_check.check(logs, 3))

    def test_call_without_a_reply_fails(self) -> None:
        logs = copy.deepcopy(good_logs())
        for r in logs["sip"][:2]:
            del r["status_code"]
        errors = zeek_check.check(logs, 3)
        self.assertEqual(len(errors), 1)
        self.assertIn("c1@host", errors[0])

    def test_a_refused_call_counts(self) -> None:
        logs = copy.deepcopy(good_logs())
        logs["sip"].append(
            {"call_id": "c4@host", "method": "INVITE", "status_code": 486}
        )
        self.assertEqual(zeek_check.check(logs, 4), [])

    def test_sip_weird_fails(self) -> None:
        logs = copy.deepcopy(good_logs())
        logs["weird"].append({"name": "SIP_bad_header"})
        self.assertTrue(zeek_check.check(logs, 3))

    def test_status_that_is_not_a_number_fails(self) -> None:
        logs = copy.deepcopy(good_logs())
        for r in logs["sip"][:2]:
            r["status_code"] = "x"
        self.assertTrue(zeek_check.check(logs, 3))


if __name__ == "__main__":
    unittest.main()
