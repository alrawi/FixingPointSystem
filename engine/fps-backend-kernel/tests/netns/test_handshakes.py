# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of handshakes.py on synthetic pcapng files."""

from __future__ import annotations

import struct
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import handshakes


def block(block_type: int, body: bytes) -> bytes:
    body += b"\0" * (-len(body) % 4)
    total = 12 + len(body)
    return struct.pack("<II", block_type, total) + body + struct.pack("<I", total)


def pcapng(frames: list[bytes]) -> bytes:
    out = block(
        handshakes.SHB_TYPE, struct.pack("<IHHq", handshakes.BYTE_ORDER_MAGIC, 1, 0, -1)
    )
    out += block(handshakes.IDB_TYPE, struct.pack("<HHI", 1, 0, 0))
    for f in frames:
        out += block(
            handshakes.EPB_TYPE, struct.pack("<IIIII", 0, 0, 0, len(f), len(f)) + f
        )
    return out


def segment(sport: int, dport: int, flags: int, ttl: int, options: bytes) -> bytes:
    options += b"\0" * (-len(options) % 4)
    tcp = struct.pack(
        "!HHIIBBHHH",
        sport,
        dport,
        1,
        0,
        (5 + len(options) // 4) << 4,
        flags,
        64240,
        0,
        0,
    )
    tcp += options
    ip = struct.pack(
        "!BBHHHBBH4s4s",
        0x45,
        0,
        20 + len(tcp),
        0,
        0x4000,
        ttl,
        6,
        0,
        bytes(4),
        bytes(4),
    )
    return b"\x02" * 12 + struct.pack("!H", 0x0800) + ip + tcp


WINDOWS = bytes([2, 4, 0x05, 0xB4, 1, 1, 4, 2, 1, 3, 3, 8])


def write(tmp_path: Path, frames: list[bytes]) -> Path:
    path = tmp_path / "c.pcapng"
    path.write_bytes(pcapng(frames))
    return path


def test_options_parse_in_wire_order() -> None:
    names, mss, ws = handshakes.tcp_options(WINDOWS)
    assert names == ["mss", "nop", "nop", "sok", "nop", "ws"]
    assert (mss, ws) == (1460, 8)


def test_malformed_option_is_an_error_not_a_crash() -> None:
    with pytest.raises(handshakes.CaptureError):
        handshakes.tcp_options(bytes([2, 9, 0]))


def test_matching_handshake_passes(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    path = write(
        tmp_path,
        [
            segment(40000, 9100, 0x02, 128, WINDOWS),
            segment(9100, 40000, 0x12, 127, WINDOWS),
        ],
    )
    args = [
        str(path),
        "--port",
        "9100",
        "--ttl",
        "128",
        "--mss",
        "1460",
        "--wscale",
        "8",
    ]
    assert handshakes.main(args + ["--sack", "1", "--ts", "0", "--df", "1"]) == 0
    assert "syn ttl=128 df=1" in capsys.readouterr().out


def test_wrong_ttl_or_missing_syn_ack_fails(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    path = write(tmp_path, [segment(40000, 9100, 0x02, 64, WINDOWS)])
    assert (
        handshakes.main([str(path), "--port", "9100", "--ttl", "128", "--mss", "1460"])
        == 1
    )
    err = capsys.readouterr().err
    assert "syn ttl is 64, want 128" in err
    assert "no syn-ack for port 9100" in err


def test_truncated_capture_reports_an_error(tmp_path: Path) -> None:
    path = tmp_path / "bad.pcapng"
    path.write_bytes(pcapng([segment(1, 9100, 0x02, 64, WINDOWS)])[:-7])
    assert (
        handshakes.main([str(path), "--port", "9100", "--ttl", "64", "--mss", "1460"])
        == 2
    )
