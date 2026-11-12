# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of mix_findings.py, which need no root and no tools."""

from __future__ import annotations

import json
import struct
from pathlib import Path
from typing import Any

import mix_findings as mf
import pytest

CLIENT = "10.201.0.2"
SERVER = "10.202.0.2"
SYN, ACK, FIN, RST, PSH = 0x02, 0x10, 0x01, 0x04, 0x08


def block(block_type: int, body: bytes) -> bytes:
    body += b"\x00" * (-len(body) % 4)
    total = len(body) + 12
    return struct.pack("<II", block_type, total) + body + struct.pack("<I", total)


def pcapng(frames: list[tuple[float, bytes]]) -> bytes:
    """Return a pcapng file of one Ethernet interface with nanosecond timestamps."""
    out = bytearray(block(0x0A0D0D0A, struct.pack("<IHHq", 0x1A2B3C4D, 1, 0, -1)))
    options = struct.pack("<HHBxxx", 9, 1, 9) + struct.pack("<HH", 0, 0)
    out += block(1, struct.pack("<HHI", 1, 0, 262144) + options)
    for stamp, data in frames:
        ticks = round(stamp * 1e9)
        header = struct.pack(
            "<IIIII", 0, ticks >> 32, ticks & 0xFFFFFFFF, len(data), len(data)
        )
        out += block(6, header + data)
    return bytes(out)


def tcp_frame(
    t: float,
    src: str,
    sport: int,
    dst: str,
    dport: int,
    flags: int,
    payload: bytes = b"",
    seq: int = 1,
) -> tuple[float, bytes]:
    """Return a timestamp and an Ethernet frame that carries one TCP segment."""
    tcp = (
        struct.pack("!HHIIBBHHH", sport, dport, seq, 1, 5 << 4, flags, 65535, 0, 0)
        + payload
    )
    ip = struct.pack(
        "!BBHHHBBH4s4s",
        0x45,
        0,
        20 + len(tcp),
        0,
        0,
        64,
        6,
        0,
        bytes(int(x) for x in src.split(".")),
        bytes(int(x) for x in dst.split(".")),
    )
    return t, b"\x00" * 12 + b"\x08\x00" + ip + tcp


def session(
    t: float, port: int, *, syn: bool = True, close: bool = True, isn: int = 1
) -> list[tuple[float, bytes]]:
    """Return the six frames of one HTTP session that starts at t."""
    frames = []
    if syn:
        frames += [
            tcp_frame(t, CLIENT, port, SERVER, 80, SYN, seq=isn),
            tcp_frame(t + 0.001, SERVER, 80, CLIENT, port, SYN | ACK),
        ]
    frames.append(
        tcp_frame(
            t + 0.002, CLIENT, port, SERVER, 80, PSH | ACK, b"GET / HTTP/1.1\r\n\r\n"
        )
    )
    frames.append(
        tcp_frame(
            t + 0.003, SERVER, 80, CLIENT, port, PSH | ACK, b"HTTP/1.1 200 OK\r\n\r\n"
        )
    )
    if close:
        frames.append(tcp_frame(t + 0.004, CLIENT, port, SERVER, 80, FIN | ACK))
        frames.append(tcp_frame(t + 0.005, SERVER, 80, CLIENT, port, FIN | ACK))
    return frames


def finding(
    session_id: int | None, port: int, parser: str = "tshark", frame: int | None = 9
) -> dict[str, Any]:
    return {
        "parser": parser,
        "kind": "malformed",
        "name": "http",
        "message": "Malformed packet",
        "frame": frame,
        "timestamp": 110.002,
        "session": session_id,
        "application": "http1" if session_id else None,
        "transport": "tcp",
        "client": {"ip": CLIENT, "port": port} if session_id else None,
        "server": {"ip": SERVER, "port": 80} if session_id else None,
    }


def prepare(
    tmp_path: Path,
    frames: list[tuple[float, bytes]],
    errors: list[dict[str, Any]],
    total: int | None = None,
) -> Path:
    mix = tmp_path / "mix"
    mix.mkdir()
    (mix / "mix.pcapng").write_bytes(pcapng(sorted(frames)))
    check = {
        "check": "parser_errors",
        "passed": not errors,
        "parsers": [{"parser": "tshark", "version": "4.2.2"}],
        "errors": errors,
        "errors_total": len(errors) if total is None else total,
    }
    (mix / "realism.json").write_text(json.dumps({"checks": [check]}), encoding="utf-8")
    return tmp_path


def test_clean_run_writes_an_empty_findings_file(tmp_path: Path) -> None:
    out = prepare(tmp_path, session(100.0, 40000) + session(101.0, 40001), [])
    assert mf.run(out, 2.0) == []
    doc = json.loads((out / "mix" / "parser-findings.json").read_text())
    assert doc["findings"] == [] and doc["errors_total"] == 0


def test_finding_in_the_middle_of_the_capture(tmp_path: Path) -> None:
    frames = session(100.0, 40000) + session(110.0, 40001) + session(130.0, 40002)
    out = prepare(tmp_path, frames, [finding(2, 40001)])
    [entry] = mf.run(out, 2.0)
    assert entry["cut"] == "none" and entry["reused_tuple"] is False
    assert entry["offset_s"] == pytest.approx(10.002)
    assert entry["session_facts"]["packets"] == 6
    line = mf.describe(entry, 30.005)
    assert line == (
        "tshark malformed http (Malformed packet), frame 9, at +10.002 s of 30.005 s,"
        " session 2 of http1 (tcp 10.201.0.2:40001 -> 10.202.0.2:80, 6 packets,"
        " 1 connection(s), 2 FIN, 0 RST, +10.000 s to +10.005 s), cut none,"
        " reused_tuple false"
    )


def test_session_cut_at_the_stop(tmp_path: Path) -> None:
    frames = (
        session(100.0, 40000)
        + session(110.0, 40001)
        + session(119.5, 40002, close=False)
    )
    out = prepare(tmp_path, frames, [finding(3, 40002)])
    [entry] = mf.run(out, 2.0)
    assert entry["cut"] == "stop"


def test_session_cut_at_the_start(tmp_path: Path) -> None:
    frames = (
        session(100.2, 40000, syn=False) + session(110.0, 40001) + session(120.0, 40002)
    )
    out = prepare(tmp_path, frames, [finding(1, 40000)])
    [entry] = mf.run(out, 2.0)
    assert entry["cut"] == "start"


def test_two_connections_of_one_tuple_are_a_reused_session(tmp_path: Path) -> None:
    frames = (
        session(100.0, 40000, isn=1000)
        + session(110.0, 40000, isn=5000)
        + session(120.0, 40001)
    )
    out = prepare(tmp_path, frames, [finding(1, 40000)])
    [entry] = mf.run(out, 2.0)
    assert entry["reused_tuple"] is True
    assert entry["session_facts"]["connections"] == 2
    doc = json.loads((out / "mix" / "parser-findings.json").read_text())
    assert doc["reused_sessions"] == [1]


def test_a_retransmitted_syn_is_one_connection(tmp_path: Path) -> None:
    frames = session(100.0, 40000) + [
        tcp_frame(100.0005, CLIENT, 40000, SERVER, 80, SYN)
    ]
    out = prepare(tmp_path, frames, [finding(1, 40000)])
    [entry] = mf.run(out, 2.0)
    assert entry["reused_tuple"] is False


def test_unattributed_finding_has_no_session(tmp_path: Path) -> None:
    out = prepare(
        tmp_path,
        session(100.0, 40000),
        [finding(None, 40000, parser="zeek", frame=None)],
    )
    [entry] = mf.run(out, 2.0)
    assert entry["cut"] == "unknown" and entry["session_facts"] is None
    assert mf.describe(entry, 1.0).endswith(
        ", at +10.002 s of 1.000 s, no session of the capture"
    )


def test_main_prints_every_finding(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    frames = session(100.0, 40000) + session(110.0, 40001)
    out = prepare(
        tmp_path, frames, [finding(2, 40001), finding(2, 40001, "zeek")], total=12
    )
    assert mf.main(["--out-dir", str(out)]) == 0
    err = capsys.readouterr().err.splitlines()
    assert (
        err[0]
        == "mix_findings.py: 12 parser findings in a capture of 10.005 s, 2 listed"
    )
    assert err[1].startswith("mix_findings.py: 1. tshark malformed http")
    assert err[2].startswith("mix_findings.py: 2. zeek malformed http")
    doc = json.loads((out / "mix" / "parser-findings.json").read_text())
    assert doc["by_name"] == {"tshark http": 1, "zeek http": 1}
    assert doc["by_application"] == {"http1": 2}


def test_missing_inputs_exit_with_2(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    (tmp_path / "mix").mkdir()
    assert mf.main(["--out-dir", str(tmp_path)]) == 2
    assert "realism.json is missing" in capsys.readouterr().err
