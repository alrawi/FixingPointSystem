# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of the capture reader and the session table."""

from __future__ import annotations

import struct
from pathlib import Path

import pytest

from realism.capture import (
    LINKTYPE_ETHERNET,
    CaptureError,
    Frame,
    SessionKey,
    decode,
    summarize,
)
from realism.tests import pcapgen


def test_pcap_and_pcapng_yield_the_same_sessions(tmp_path: Path) -> None:
    cap = pcapgen.mixed_capture(http_sessions=2, dns_lookups=2)
    a = summarize(cap.write_pcap(tmp_path / "a.pcap"))
    b = summarize(cap.write_pcapng(tmp_path / "b.pcapng"))
    assert (a.format, b.format) == ("pcap", "pcapng")
    assert a.frames == b.frames == 2 * 10 + 2 * 2
    assert [s.key for s in a.sessions] == [s.key for s in b.sessions]
    assert len(a.sessions) == 4
    assert a.skipped_frames == 0


def test_tcp_session_orients_client_by_syn(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.http_get(("10.0.0.10", 40000), ("10.0.1.80", 80))
    summary = summarize(cap.write_pcap(tmp_path / "c.pcap"))
    (session,) = summary.sessions
    assert (session.client_ip, session.client_port) == ("10.0.0.10", 40000)
    assert (session.server_ip, session.server_port) == ("10.0.1.80", 80)
    assert session.packets == 10
    assert session.payload_bytes > 0


def test_capture_that_starts_at_syn_ack_still_finds_the_client(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.add(pcapgen.tcp("10.0.1.80", 80, "10.0.0.10", 40000, pcapgen.SYN | pcapgen.ACK, 1, 1))
    cap.add(pcapgen.tcp("10.0.0.10", 40000, "10.0.1.80", 80, pcapgen.ACK, 1, 2))
    (session,) = summarize(cap.write_pcap(tmp_path / "s.pcap")).sessions
    assert session.server_port == 80
    assert session.client_port == 40000


def test_udp_client_is_the_first_sender(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.dns_lookup(("10.0.0.10", 50000), ("10.0.1.53", 53), 7)
    (session,) = summarize(cap.write_pcap(tmp_path / "d.pcap")).sessions
    assert (session.transport, session.server_port, session.packets) == ("udp", 53, 2)


def test_ipv6_sessions_are_decoded(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.http_get(("2001:db8::1", 40000), ("2001:db8::80", 80))
    (session,) = summarize(cap.write_pcapng(tmp_path / "v6.pcapng")).sessions
    assert session.server_ip == "2001:db8::80"


def test_non_ip_frames_are_skipped(tmp_path: Path) -> None:
    arp = b"\xff" * 6 + b"\x02" * 6 + b"\x08\x06" + b"\x00" * 28
    path = tmp_path / "arp.pcap"
    header = struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1)
    path.write_bytes(header + struct.pack("<IIII", 1, 0, len(arp), len(arp)) + arp)
    summary = summarize(path)
    assert (summary.frames, summary.skipped_frames, summary.sessions) == (1, 1, [])


def test_vlan_tagged_frame_is_decoded() -> None:
    ip = pcapgen.udp("10.0.0.1", 1000, "10.0.0.2", 53, b"x")
    frame = b"\x02" * 12 + b"\x81\x00\x00\x0a" + b"\x08\x00" + ip
    packet = decode(Frame(0.0, LINKTYPE_ETHERNET, frame))
    assert packet is not None
    assert (packet.transport, packet.dst_port, packet.payload_len) == ("udp", 53, 1)


def test_big_endian_nanosecond_pcap(tmp_path: Path) -> None:
    frame = pcapgen.ethernet(pcapgen.udp("10.0.0.1", 1000, "10.0.0.2", 53, b"x"))
    header = struct.pack(">IHHiIII", 0xA1B23C4D, 2, 4, 0, 0, 65535, 1)
    record = struct.pack(">IIII", 10, 500_000_000, len(frame), len(frame)) + frame
    path = tmp_path / "ns.pcap"
    path.write_bytes(header + record)
    (session,) = summarize(path).sessions
    assert session.first_seen == pytest.approx(10.5)


@pytest.mark.parametrize(
    ("content", "message"),
    [
        (b"", "shorter than a header"),
        (b"\x00\x01\x02\x03rest", "unknown magic"),
        (b"\xd4\xc3\xb2\xa1" + b"\x00" * 10, "truncated pcap file header"),
        (struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1) + b"\x00" * 8, "truncated"),
        (
            struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 65535, 1)
            + struct.pack("<IIII", 0, 0, 500, 500),
            "claims 500 bytes",
        ),
        (struct.pack("<III", 0x0A0D0D0A, 28, 0x01020304) + b"\x00" * 16, "byte order mark"),
    ],
)
def test_malformed_captures_raise_capture_error(
    tmp_path: Path, content: bytes, message: str
) -> None:
    path = tmp_path / "bad.pcap"
    path.write_bytes(content)
    with pytest.raises(CaptureError, match=message):
        summarize(path)


def test_missing_capture_raises_capture_error(tmp_path: Path) -> None:
    with pytest.raises(CaptureError, match="read capture"):
        summarize(tmp_path / "absent.pcap")


def test_session_key_ignores_direction() -> None:
    a = SessionKey.of("tcp", "10.0.0.2", 80, "10.0.0.1", 40000)
    b = SessionKey.of("tcp", "10.0.0.1", 40000, "10.0.0.2", 80)
    assert a == b
    assert a.a_ip == "10.0.0.1"
