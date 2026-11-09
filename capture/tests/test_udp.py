# SPDX-License-Identifier: GPL-3.0-only
"""UDP flows: grouping by 5-tuple, idle splits, IPv6, and roles."""

from __future__ import annotations

import base64

from .conftest import Extract
from .pcapgen import Rec, udp_frame, write_pcap

C, S = "192.0.2.10", "192.0.2.53"


def test_udp_flow_counts_both_directions(run: Extract) -> None:
    recs = [
        Rec(1.0, udp_frame(C, S, 50000, 53, b"query")),
        Rec(1.020, udp_frame(S, C, 53, 50000, b"answer!")),
        Rec(1.5, udp_frame(C, S, 50000, 53, b"q2")),
    ]
    doc = run(write_pcap(recs), payloads=True)
    assert doc["summary"] == {"sessions": 1, "tcp": 0, "udp": 1}
    s = doc["sessions"][0]
    assert s["protocol"] == "udp" and s["handshake"] is None
    assert s["client"] == {"ip": C, "port": 50000} and s["server"] == {"ip": S, "port": 53}
    assert s["client_to_server"] == {"packets": 2, "ip_bytes": 2 * 28 + 7, "payload_bytes": 7}
    assert s["server_to_client"] == {"packets": 1, "ip_bytes": 28 + 7, "payload_bytes": 7}
    assert s["duration_ms"] == 500.0 and s["end_reason"] == "capture_end"
    assert [t["datagram_sizes"] for t in s["turns"]] == [[5], [7], [2]]
    assert base64.b64decode(s["turns"][1]["datagrams_b64"][0]) == b"answer!"
    assert s["anomalies"] == ["role_guessed"]  # no handshake, so the port decided the roles


def test_idle_timeout_splits_a_flow_and_keeps_roles(run: Extract) -> None:
    recs = [
        Rec(1.0, udp_frame(C, S, 50000, 53, b"a")),
        Rec(1.1, udp_frame(S, C, 53, 50000, b"b")),
        Rec(100.0, udp_frame(S, C, 53, 50000, b"late server push")),  # 98.9 s idle
        Rec(100.5, udp_frame(C, S, 50000, 53, b"c")),
    ]
    first, second = run(write_pcap(recs))["sessions"]
    assert first["end_reason"] == "idle_timeout" and first["continues"] is None
    assert second["continues"] == first["id"] and "udp_idle_split" in second["anomalies"]
    # the second session starts with a server packet, but the roles carry over
    assert second["client"] == {"ip": C, "port": 50000}
    assert second["start_ns"] == 100_000_000_000
    assert first["client_to_server"]["packets"] == 1 and second["client_to_server"]["packets"] == 1


def test_gap_equal_to_the_timeout_does_not_split(run: Extract) -> None:
    recs = [
        Rec(1.0, udp_frame(C, S, 50000, 53, b"a")),
        Rec(11.0, udp_frame(C, S, 50000, 53, b"b")),
    ]
    assert len(run(write_pcap(recs), udp_idle_s=10.0)["sessions"]) == 1
    assert len(run(write_pcap(recs), udp_idle_s=9.9)["sessions"]) == 2


def test_different_five_tuples_are_different_flows(run: Extract) -> None:
    recs = [
        Rec(1.0, udp_frame(C, S, 50000, 53, b"a")),
        Rec(1.1, udp_frame(C, S, 50001, 53, b"b")),
        Rec(1.2, udp_frame(C, "192.0.2.54", 50000, 53, b"c")),
    ]
    doc = run(write_pcap(recs))
    assert doc["summary"]["udp"] == 3
    assert [s["id"] for s in doc["sessions"]] == [1, 2, 3]


def test_ipv6_udp_and_raw_link_type(run: Extract) -> None:
    c6, s6 = "2001:db8:1::1", "2001:db8:1::2"
    recs = [
        Rec(1.0, udp_frame(c6, s6, 40000, 123, b"x" * 48, link="raw")),
        Rec(1.1, udp_frame(s6, c6, 123, 40000, b"y" * 48, link="raw")),
    ]
    s = run(write_pcap(recs, link=101))["sessions"][0]
    assert s["ip_version"] == 6 and s["server"] == {"ip": s6, "port": 123}
    assert s["client_to_server"]["ip_bytes"] == 40 + 8 + 48
    assert s["roles"] == {"client_known": False, "basis": "port"}


def test_udp_from_a_well_known_port_first_makes_that_side_the_server(run: Extract) -> None:
    recs = [Rec(1.0, udp_frame(S, C, 53, 50000, b"response"))]
    s = run(write_pcap(recs))["sessions"][0]
    assert s["server"] == {"ip": S, "port": 53} and "role_guessed" in s["anomalies"]


def test_unsupported_frames_are_counted_not_fatal(run: Extract) -> None:
    arp = bytes(12) + b"\x08\x06" + bytes(28)
    short = bytes(8)
    recs = [
        Rec(1.0, arp),
        Rec(1.1, short),
        Rec(1.2, udp_frame(C, S, 1, 2, b"ok")),
    ]
    doc = run(write_pcap(recs))
    assert doc["source"]["frames"] == 3 and doc["source"]["packets"] == 1
    assert doc["source"]["skipped_frames"] == {"not_ip": 1, "truncated_header": 1}
