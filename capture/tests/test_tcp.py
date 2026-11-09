# SPDX-License-Identifier: GPL-3.0-only
"""TCP reassembly, handshakes, and anomalies on synthetic captures."""

from __future__ import annotations

import base64
from typing import Any

from .conftest import Extract
from .pcapgen import ACK, FIN, PSH, RST, SYN, Rec, tcp_frame, write_pcap

C, S = "10.0.0.1", "10.0.0.2"
CP, SP = 40000, 80


class Conv:
    """Builds frames of one TCP connection with a running clock."""

    def __init__(
        self, c: str = C, s: str = S, cp: int = CP, sp: int = SP, cisn: int = 1000, sisn: int = 5000
    ):
        self.c, self.s, self.cp, self.sp = c, s, cp, sp
        self.cisn, self.sisn = cisn, sisn
        self.recs: list[Rec] = []
        self.t = 1.0

    def at(self, t: float) -> Conv:
        self.t = t
        return self

    def _add(self, frame: bytes) -> None:
        self.recs.append(Rec(self.t, frame))

    def c2s(self, seq: int, *, flags: int = ACK, data: bytes = b"", ack: int | None = None) -> Conv:
        a = self.sisn + 1 if ack is None else ack
        self._add(
            tcp_frame(self.c, self.s, self.cp, self.sp, seq=seq, ack=a, flags=flags, payload=data)
        )
        return self

    def s2c(self, seq: int, *, flags: int = ACK, data: bytes = b"", ack: int | None = None) -> Conv:
        a = self.cisn + 1 if ack is None else ack
        self._add(
            tcp_frame(self.s, self.c, self.sp, self.cp, seq=seq, ack=a, flags=flags, payload=data)
        )
        return self

    def handshake(self, rtt_a: float = 0.010, rtt_b: float = 0.004) -> Conv:
        self.c2s(self.cisn, flags=SYN, ack=0)
        self.t += rtt_a
        self.s2c(self.sisn, flags=SYN | ACK, ack=self.cisn + 1)
        self.t += rtt_b
        self.c2s(self.cisn + 1)
        return self

    def close(self, c_next: int, s_next: int) -> Conv:
        self.c2s(c_next, flags=FIN | ACK, ack=s_next)
        self.s2c(s_next, flags=ACK, ack=c_next + 1)
        self.s2c(s_next, flags=FIN | ACK, ack=c_next + 1)
        self.c2s(c_next + 1, flags=ACK, ack=s_next + 1)
        return self

    def pcap(self) -> bytes:
        return write_pcap(self.recs)


def only(doc: dict[str, Any]) -> dict[str, Any]:
    sessions = doc["sessions"]
    assert isinstance(sessions, list) and len(sessions) == 1
    session = sessions[0]
    assert isinstance(session, dict)
    return session


def test_clean_session_with_handshake_and_close(run: Extract) -> None:
    cv = Conv().at(10.0).handshake(0.010, 0.004)
    cv.at(10.100).c2s(1001, flags=PSH | ACK, data=b"GET / HTTP/1.1\r\n\r\n")
    cv.at(10.150).s2c(5001, flags=PSH | ACK, data=b"HTTP/1.1 200 OK\r\n\r\nhello")
    cv.at(10.200).close(1001 + 18, 5001 + 24)
    s = only(run(cv.pcap(), payloads=True))
    assert s["protocol"] == "tcp" and s["ip_version"] == 4
    assert s["client"] == {"ip": C, "port": CP} and s["server"] == {"ip": S, "port": SP}
    assert s["roles"] == {"client_known": True, "basis": "syn"}
    assert s["handshake"] == {
        "state": "complete",
        "server_rtt_ms": 10.0,
        "client_rtt_ms": 4.0,
        "rtt_ms": 14.0,
    }
    assert s["end_reason"] == "fin" and s["anomalies"] == []
    assert s["start_ns"] == 10_000_000_000 and s["end_ns"] == 10_200_000_000
    assert s["duration_ms"] == 200.0
    assert s["data_start_ns"] == 10_100_000_000 and s["data_end_ns"] == 10_150_000_000
    assert s["data_duration_ms"] == 50.0
    c2s, s2c = s["client_to_server"], s["server_to_client"]
    assert isinstance(c2s, dict) and isinstance(s2c, dict)
    assert (c2s["packets"], c2s["payload_bytes"], c2s["stream_bytes"]) == (5, 18, 18)
    assert (s2c["packets"], s2c["payload_bytes"], s2c["stream_bytes"]) == (4, 24, 24)
    assert c2s["ip_bytes"] == 40 * 5 + 18
    turns = s["turns"]
    assert isinstance(turns, list)
    assert [(t["direction"], t["bytes"]) for t in turns] == [
        ("client_to_server", 18),
        ("server_to_client", 24),
    ]
    assert base64.b64decode(turns[1]["data_b64"]) == b"HTTP/1.1 200 OK\r\n\r\nhello"


def test_retransmission_is_counted_once_in_the_stream(run: Extract) -> None:
    cv = Conv().handshake()
    cv.c2s(1001, data=b"AAAA")
    cv.at(1.5).c2s(1001, data=b"AAAA")  # exact duplicate
    cv.c2s(1005, data=b"BBBB")
    cv.c2s(1003, data=b"AABB")  # overlaps both, carries nothing new
    s = only(run(cv.pcap(), payloads=True))
    c2s = s["client_to_server"]
    assert isinstance(c2s, dict)
    assert c2s["payload_bytes"] == 16 and c2s["stream_bytes"] == 8
    assert c2s["retransmitted_packets"] == 2 and c2s["retransmitted_bytes"] == 8
    assert c2s["out_of_order_packets"] == 0 and c2s["missing_bytes"] == 0
    assert "retransmission" in s["anomalies"] and "overlap_mismatch" not in s["anomalies"]
    turns = s["turns"]
    assert isinstance(turns, list) and len(turns) == 1
    assert base64.b64decode(turns[0]["data_b64"]) == b"AAAABBBB"


def test_partial_overlap_keeps_only_new_bytes(run: Extract) -> None:
    cv = Conv().handshake()
    cv.c2s(1001, data=b"AAAA")
    cv.c2s(1003, data=b"AACC")  # two old bytes, two new
    s = only(run(cv.pcap(), payloads=True))
    c2s = s["client_to_server"]
    assert isinstance(c2s, dict)
    assert c2s["stream_bytes"] == 6 and c2s["retransmitted_bytes"] == 2
    turns = s["turns"]
    assert isinstance(turns, list)
    assert base64.b64decode(turns[0]["data_b64"]) == b"AAAAC" + b"C"


def test_retransmission_with_different_bytes_is_flagged_and_first_copy_wins(run: Extract) -> None:
    cv = Conv().handshake()
    cv.c2s(1001, data=b"AAAA")
    cv.c2s(1001, data=b"ZZZZ")
    s = only(run(cv.pcap(), payloads=True))
    assert "overlap_mismatch" in s["anomalies"]
    turns = s["turns"]
    assert isinstance(turns, list)
    assert base64.b64decode(turns[0]["data_b64"]) == b"AAAA"


def test_out_of_order_segments_are_buffered_and_delivered_in_order(run: Extract) -> None:
    cv = Conv().at(2.0).handshake()
    cv.at(2.10).c2s(1005, data=b"CCCC")  # arrives first
    cv.at(2.11).c2s(1009, data=b"DDDD")
    cv.at(2.12).c2s(1001, data=b"AAAA")  # still a hole at 1005..1008? no: 1001..1004 fills it
    cv.at(2.13).c2s(1013, data=b"EEEE")
    s = only(run(cv.pcap(), payloads=True))
    c2s = s["client_to_server"]
    assert isinstance(c2s, dict)
    assert c2s["stream_bytes"] == 16 and c2s["missing_bytes"] == 0
    assert c2s["out_of_order_packets"] == 1 and c2s["retransmitted_packets"] == 0
    assert "out_of_order" in s["anomalies"] and "retransmission" not in s["anomalies"]
    turns = s["turns"]
    assert isinstance(turns, list) and len(turns) == 1
    # the buffered bytes became available when the hole filled, at 2.12
    assert turns[0]["start_ns"] == 2_120_000_000 and turns[0]["end_ns"] == 2_130_000_000
    assert (
        base64.b64decode(turns[0]["data_b64"]) == b"AAAACCCCDDDDEEEE"
        and turns[0]["gap_before"] == 0
    )


def test_a_hole_that_never_fills_is_reported_and_skipped(run: Extract) -> None:
    cv = Conv().handshake()
    cv.c2s(1001, data=b"AAAA")
    cv.c2s(1009, data=b"CCCC")  # 1005..1008 never seen
    s = only(run(cv.pcap(), payloads=True))
    c2s = s["client_to_server"]
    assert isinstance(c2s, dict)
    assert c2s["stream_bytes"] == 8 and c2s["missing_bytes"] == 4
    assert "data_gap" in s["anomalies"]
    turns = s["turns"]
    assert isinstance(turns, list) and [t["gap_before"] for t in turns] == [0, 4]
    assert [base64.b64decode(t["data_b64"]) for t in turns] == [b"AAAA", b"CCCC"]


def test_session_without_handshake_starts_mid_stream(run: Extract) -> None:
    cv = Conv()
    cv.at(5.0).s2c(7000, data=b"late response", ack=1500)
    cv.at(5.1).c2s(1500, data=b"next request", ack=7013)
    cv.at(5.2).s2c(7013, data=b"more", ack=1512)
    s = only(run(cv.pcap(), payloads=True))
    assert s["handshake"] == {
        "state": "missing",
        "server_rtt_ms": None,
        "client_rtt_ms": None,
        "rtt_ms": None,
    }
    assert "missing_handshake" in s["anomalies"]
    # the sender of the first packet came from the well known port, so it is the server
    assert s["server"] == {"ip": S, "port": SP}
    assert s["roles"] == {"client_known": False, "basis": "port"}
    assert "role_guessed" in s["anomalies"]
    assert s["end_reason"] == "capture_end"
    s2c = s["server_to_client"]
    assert isinstance(s2c, dict) and s2c["stream_bytes"] == 17 and s2c["missing_bytes"] == 0


def test_mid_stream_first_packet_decides_when_both_ports_are_high(run: Extract) -> None:
    cv = Conv(cp=40000, sp=50000)
    cv.s2c(7000, data=b"x", ack=1500)
    s = only(run(cv.pcap()))
    assert s["roles"] == {"client_known": False, "basis": "first_packet"}
    assert s["client"] == {"ip": S, "port": 50000}


def test_mid_stream_data_arriving_before_earlier_data(run: Extract) -> None:
    cv = Conv()
    cv.at(1.0).c2s(2000, data=b"BBBB")
    cv.at(1.1).c2s(1996, data=b"AAAA")  # before the first segment that was seen
    s = only(run(cv.pcap()))
    c2s = s["client_to_server"]
    assert isinstance(c2s, dict)
    assert c2s["stream_bytes"] == 8 and c2s["missing_bytes"] == 0
    assert c2s["out_of_order_packets"] == 1


def test_handshake_without_final_ack_and_syn_retransmission(run: Extract) -> None:
    cv = Conv()
    cv.at(1.0).c2s(1000, flags=SYN, ack=0)
    cv.at(2.0).c2s(1000, flags=SYN, ack=0)  # the SYN went unanswered for a second
    cv.at(2.05).s2c(5000, flags=SYN | ACK, ack=1001)
    s = only(run(cv.pcap()))
    assert s["handshake"]["state"] == "incomplete"
    assert s["handshake"]["server_rtt_ms"] == 50.0 and s["handshake"]["rtt_ms"] is None
    assert {"handshake_incomplete", "syn_retransmitted"} <= set(s["anomalies"])


def test_syn_ack_first_mid_handshake_start(run: Extract) -> None:
    cv = Conv()
    cv.at(1.0).s2c(5000, flags=SYN | ACK, ack=1001)
    cv.at(1.01).c2s(1001)
    s = only(run(cv.pcap()))
    assert s["roles"] == {"client_known": True, "basis": "synack"}
    assert s["handshake"]["state"] == "incomplete"
    assert s["handshake"]["client_rtt_ms"] == 10.0 and s["handshake"]["server_rtt_ms"] is None


def test_reset_closes_the_session(run: Extract) -> None:
    cv = Conv().handshake()
    cv.c2s(1001, data=b"hi")
    cv.s2c(5001, flags=RST | ACK, ack=1003)
    s = only(run(cv.pcap()))
    assert s["end_reason"] == "rst" and "reset" in s["anomalies"]
    assert s["server_to_client"]["rst"] is True and s["client_to_server"]["rst"] is False


def test_one_sided_close_is_a_half_close(run: Extract) -> None:
    cv = Conv().handshake()
    cv.c2s(1001, flags=FIN | ACK)
    cv.s2c(5001, flags=ACK, ack=1002)
    s = only(run(cv.pcap()))
    assert s["end_reason"] == "half_close" and "half_close" in s["anomalies"]


def test_port_reuse_after_close_makes_a_second_session(run: Extract) -> None:
    cv = Conv().at(1.0).handshake().at(1.1).close(1001, 5001)
    cv.at(9.0)
    cv.cisn, cv.sisn = 90000, 70000
    cv.handshake()
    doc = run(cv.pcap())
    first, second = doc["sessions"]
    assert (first["id"], second["id"]) == (1, 2)
    assert first["end_reason"] == "fin" and second["end_reason"] == "capture_end"
    assert second["handshake"]["state"] == "complete"


def test_new_syn_during_an_open_connection_ends_it_as_new_connection(run: Extract) -> None:
    cv = Conv().at(1.0).handshake()
    cv.c2s(1001, data=b"x")
    cv.at(3.0)
    cv.cisn, cv.sisn = 424242, 9999
    cv.handshake()
    first, second = run(cv.pcap())["sessions"]
    assert first["end_reason"] == "new_connection"
    assert second["handshake"]["state"] == "complete"


def test_sequence_numbers_wrap_around(run: Extract) -> None:
    top = 2**32 - 6
    cv = Conv(cisn=top - 1)
    cv.handshake()
    cv.c2s(top, data=b"AAAAAAAA")  # crosses the 32 bit boundary
    cv.c2s((top + 8) % 2**32, data=b"BBBB")
    cv.c2s(top, data=b"AAAAAAAA")  # retransmission after the wrap
    s = only(run(cv.pcap(), payloads=True))
    c2s = s["client_to_server"]
    assert c2s["stream_bytes"] == 12 and c2s["retransmitted_bytes"] == 8
    assert base64.b64decode(s["turns"][0]["data_b64"]) == b"AAAAAAAABBBB"


def test_turns_follow_direction_changes(run: Extract) -> None:
    cv = Conv().handshake()
    cv.at(1.1).c2s(1001, data=b"a" * 10)
    cv.at(1.2).c2s(1011, data=b"b" * 10)
    cv.at(1.3).s2c(5001, data=b"c" * 7)
    cv.at(1.4).c2s(1021, data=b"d" * 3)
    turns = only(run(cv.pcap()))["turns"]
    assert [(t["direction"][0], t["bytes"], t["packets"]) for t in turns] == [
        ("c", 20, 2),
        ("s", 7, 1),
        ("c", 3, 1),
    ]


def test_ipv6_session(run: Extract) -> None:
    c6, s6 = "2001:db8::1", "2001:db8::2"
    cv = Conv(c6, s6).handshake()
    cv.c2s(1001, data=b"v6 data")
    cv.close(1008, 5001)
    s = only(run(cv.pcap()))
    assert s["ip_version"] == 6
    assert s["client"] == {"ip": c6, "port": CP}
    assert s["handshake"]["state"] == "complete" and s["end_reason"] == "fin"
    assert s["client_to_server"]["ip_bytes"] == 5 * 60 + 7


def test_vlan_tagged_frames_decode(run: Extract) -> None:
    cv = Conv().handshake()
    tagged = []
    for r in cv.recs:
        f = r.frame
        tagged.append(Rec(r.t, f[:12] + b"\x81\x00\x00\x07" + f[12:]))
    s = only(run(write_pcap(tagged)))
    assert s["handshake"]["state"] == "complete"


def test_snaplen_truncated_segment_is_flagged(run: Extract) -> None:
    cv = Conv().handshake()
    cv.c2s(1001, data=b"0123456789")
    short = cv.recs[-1].frame[:-4]  # the capture kept 4 bytes less than the wire carried
    cv.recs[-1] = Rec(cv.recs[-1].t, short)
    s = only(run(cv.pcap()))
    assert "truncated_payload" in s["anomalies"]
    assert s["client_to_server"]["payload_bytes"] == 10


def test_two_connections_between_the_same_hosts_stay_apart(run: Extract) -> None:
    a = Conv(cp=40001).handshake()
    b = Conv(cp=40002).at(1.001).handshake()
    recs = sorted(a.recs + b.recs, key=lambda r: r.t)
    doc = run(write_pcap(recs))
    assert [s["client"]["port"] for s in doc["sessions"]] == [40001, 40002]
