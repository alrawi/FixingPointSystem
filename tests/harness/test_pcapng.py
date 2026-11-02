# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests for the harness pcapng tools. They never reach the network."""

from __future__ import annotations

import json
import struct
from pathlib import Path

import pcapng
import pytest


def make_pcap(
    order: str, magic: int, packets: list[tuple[int, int, bytes, int]]
) -> bytes:
    """Build a pcap file with Ethernet link type from (sec, frac, data, orig_len) records."""
    out = [struct.pack(order + "IHHiIII", magic, 2, 4, 0, 0, 65535, 1)]
    for sec, frac, data, orig in packets:
        out.append(struct.pack(order + "IIII", sec, frac, len(data), orig) + data)
    return b"".join(out)


def epb_timestamps(raw: bytes) -> list[int]:
    """Return the timestamps of every enhanced packet block in a little endian file."""
    stamps = []
    offset = 0
    while offset < len(raw):
        block_type, total = struct.unpack("<II", raw[offset : offset + 8])
        if block_type == pcapng.EPB_TYPE:
            _, high, low = struct.unpack("<III", raw[offset + 8 : offset + 20])
            stamps.append((high << 32) | low)
        offset += total
    return stamps


@pytest.mark.parametrize(
    ("order", "magic", "scale"),
    [("<", 0xA1B2C3D4, 10**6), (">", 0xA1B2C3D4, 10**6), ("<", 0xA1B23C4D, 10**9)],
)
def test_convert_keeps_packets_lengths_and_timestamps(
    order: str, magic: int, scale: int
) -> None:
    frames = [
        (1_700_000_000, 5, b"\xaa" * 60, 60),
        (1_700_000_001, 7, b"\xbb" * 61, 1514),
    ]
    out = pcapng.pcap_to_pcapng(make_pcap(order, magic, frames))
    stats = pcapng.pcapng_stats(out)
    assert stats.sections == 1
    assert stats.linktypes == [1]
    assert stats.packets == 2
    assert stats.captured_bytes == 121
    assert stats.max_orig_len == 1514
    assert epb_timestamps(out) == [sec * scale + frac for sec, frac, _, _ in frames]


def test_convert_of_empty_capture_has_header_and_interface_only() -> None:
    stats = pcapng.pcapng_stats(pcapng.pcap_to_pcapng(make_pcap("<", 0xA1B2C3D4, [])))
    assert (stats.sections, stats.linktypes, stats.packets) == (1, [1], 0)


def test_every_block_length_is_a_multiple_of_four() -> None:
    out = pcapng.pcap_to_pcapng(
        make_pcap("<", 0xA1B2C3D4, [(0, 0, b"\x01\x02\x03", 3)])
    )
    assert len(out) % 4 == 0
    assert out[:4] == struct.pack("<I", pcapng.SHB_TYPE)


def test_stats_reads_a_big_endian_section() -> None:
    body = struct.pack(">IHHq", pcapng.BYTE_ORDER_MAGIC, 1, 0, -1)
    shb = struct.pack(">II", pcapng.SHB_TYPE, 28) + body + struct.pack(">I", 28)
    idb = struct.pack(">IIHHII", pcapng.IDB_TYPE, 20, 1, 0, 0, 20)
    data = b"\x00" * 64
    epb_len = 32 + len(data)
    epb = struct.pack(">IIIIIII", pcapng.EPB_TYPE, epb_len, 0, 0, 0, 64, 64) + data
    epb += struct.pack(">I", epb_len)
    stats = pcapng.pcapng_stats(shb + idb + epb)
    assert (stats.linktypes, stats.packets, stats.max_orig_len) == ([1], 1, 64)


@pytest.mark.parametrize(
    "raw",
    [
        b"",
        b"\x00" * 24,
        make_pcap("<", 0xA1B2C3D4, [(0, 0, b"\x01" * 10, 10)])[:-1],
    ],
)
def test_convert_rejects_broken_pcap(raw: bytes) -> None:
    with pytest.raises(pcapng.FormatError):
        pcapng.pcap_to_pcapng(raw)


def test_stats_rejects_truncated_and_foreign_files() -> None:
    good = pcapng.pcap_to_pcapng(make_pcap("<", 0xA1B2C3D4, [(0, 0, b"\x01" * 10, 10)]))
    with pytest.raises(pcapng.FormatError):
        pcapng.pcapng_stats(good[:-4])
    with pytest.raises(pcapng.FormatError):
        pcapng.pcapng_stats(make_pcap("<", 0xA1B2C3D4, []))
    with pytest.raises(pcapng.FormatError):
        pcapng.pcapng_stats(b"")


def test_main_converts_and_prints_stats(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    source = tmp_path / "in.pcap"
    target = tmp_path / "out.pcapng"
    source.write_bytes(make_pcap("<", 0xA1B2C3D4, [(1, 2, b"\x00" * 42, 42)]))
    assert pcapng.main(["convert", str(source), str(target)]) == 0
    assert pcapng.main(["stats", str(target)]) == 0
    summary = json.loads(capsys.readouterr().out)
    assert summary == {
        "captured_bytes": 42,
        "linktypes": [1],
        "max_orig_len": 42,
        "packets": 1,
        "sections": 1,
        "vlan_tagged": 0,
    }


def test_main_reports_a_bad_file_with_exit_status_one(tmp_path: Path) -> None:
    bad = tmp_path / "bad.pcapng"
    bad.write_bytes(b"not a capture")
    assert pcapng.main(["stats", str(bad)]) == 1


def test_stats_counts_vlan_tagged_ethernet_frames() -> None:
    untagged = b"\x02" * 12 + b"\x08\x00" + b"\x00" * 46
    tagged = b"\x02" * 12 + b"\x81\x00\x00\x64\x08\x00" + b"\x00" * 46
    raw = make_pcap(
        "<", 0xA1B2C3D4, [(0, 0, untagged, 60), (0, 1, tagged, 64), (0, 2, tagged, 64)]
    )
    stats = pcapng.pcapng_stats(pcapng.pcap_to_pcapng(raw))
    assert (stats.packets, stats.vlan_tagged, stats.max_orig_len) == (3, 2, 64)


def test_stats_rejects_a_packet_on_an_unknown_interface() -> None:
    raw = bytearray(
        pcapng.pcap_to_pcapng(make_pcap("<", 0xA1B2C3D4, [(0, 0, b"\x01" * 8, 8)]))
    )
    shb_len = struct.unpack("<I", raw[4:8])[0]
    idb_len = struct.unpack("<I", raw[shb_len + 4 : shb_len + 8])[0]
    epb = shb_len + idb_len
    raw[epb + 8 : epb + 12] = struct.pack("<I", 3)
    with pytest.raises(pcapng.FormatError):
        pcapng.pcapng_stats(bytes(raw))
