# SPDX-License-Identifier: GPL-3.0-only
"""Tests of zeek_check.py with synthetic Zeek logs, which need no Zeek.

Run them with ``python3 -m unittest discover -s modules/ssh/fixtures``.
"""

from __future__ import annotations

import copy
import unittest

import zeek_check


def row(i: int) -> dict:
    return {
        "uid": f"C{i}",
        "client": "SSH-2.0-OpenSSH_9.6",
        "server": "SSH-2.0-OpenSSH_9.6",
        "kex_alg": "curve25519-sha256",
        "cipher_alg": "chacha20-poly1305@openssh.com",
        "host_key_alg": "ssh-ed25519",
        "host_key": "aa:bb",
    }


def good_logs() -> dict[str, list[dict]]:
    return {"ssh": [row(1), row(2), row(3)], "weird": [{"name": "possible_split_routing"}]}


class ZeekCheckTest(unittest.TestCase):
    def test_complete_logs_pass(self) -> None:
        self.assertEqual(zeek_check.check(good_logs(), 3), [])

    def test_missing_row_fails(self) -> None:
        self.assertTrue(zeek_check.check(good_logs(), 4))

    def test_wrong_algorithm_fails(self) -> None:
        logs = copy.deepcopy(good_logs())
        logs["ssh"][1]["cipher_alg"] = "aes128-ctr"
        errors = zeek_check.check(logs, 3)
        self.assertEqual(len(errors), 1)
        self.assertIn("cipher_alg", errors[0])

    def test_two_host_keys_fail(self) -> None:
        logs = copy.deepcopy(good_logs())
        logs["ssh"][2]["host_key"] = "cc:dd"
        self.assertTrue(zeek_check.check(logs, 3))

    def test_ssh_weird_fails(self) -> None:
        logs = copy.deepcopy(good_logs())
        logs["weird"].append({"name": "SSH_bad_version"})
        self.assertTrue(zeek_check.check(logs, 3))

    def test_missing_banner_fails(self) -> None:
        logs = copy.deepcopy(good_logs())
        del logs["ssh"][0]["server"]
        self.assertTrue(zeek_check.check(logs, 3))


if __name__ == "__main__":
    unittest.main()
