# SPDX-License-Identifier: GPL-3.0-only
"""Unit tests of parser_findings.py, which need no root and no tools."""

from __future__ import annotations

import csv
import json
import struct
from pathlib import Path
from typing import Any

import parser_findings as pf
import pytest

CLIENT = "198.18.0.2"
SERVER = "198.19.0.2"
SYN, ACK, FIN, RST, PSH = 0x02, 0x10, 0x01, 0x04, 0x08


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
    total = 20 + len(tcp)
    ip = struct.pack(
        "!BBHHHBBH4s4s",
        0x45,
        0,
        total,
        0,
        0,
        64,
        6,
        0,
        bytes(int(x) for x in src.split(".")),
        bytes(int(x) for x in dst.split(".")),
    )
    return t, b"\x00" * 12 + b"\x08\x00" + ip + tcp


def write_pcapng(path: Path, frames: list[tuple[float, bytes]]) -> None:
    path.write_bytes(pf.pcapng_bytes(1, frames))


def session(
    t: float,
    port: int,
    *,
    syn: bool = True,
    close: bool = True,
    request: bytes = b"GET / HTTP/1.1\r\n\r\n",
    isn: int = 1,
) -> list[tuple[float, bytes]]:
    """Return the frames of one HTTP session that starts at t."""
    frames = []
    if syn:
        frames += [
            tcp_frame(t, CLIENT, port, SERVER, 80, SYN, seq=isn),
            tcp_frame(t + 0.001, SERVER, 80, CLIENT, port, SYN | ACK),
        ]
    frames.append(tcp_frame(t + 0.002, CLIENT, port, SERVER, 80, PSH | ACK, request))
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
    frame: int, session_id: int, port: int, parser: str = "tshark"
) -> dict[str, Any]:
    return {
        "parser": parser,
        "kind": "malformed",
        "name": "http",
        "message": "Malformed packet",
        "frame": frame,
        "timestamp": None,
        "session": session_id,
        "application": "http1",
        "transport": "tcp",
        "client": {"ip": CLIENT, "port": port},
        "server": {"ip": SERVER, "port": 80},
    }


def realism_report(errors: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "checks": [
            {
                "check": "parser_errors",
                "passed": not errors,
                "parsers": [{"parser": "tshark", "version": "4.2.2"}],
                "errors": errors,
                "errors_total": len(errors),
            }
        ]
    }


def prepare(
    tmp_path: Path, frames: list[tuple[float, bytes]], errors: list[dict[str, Any]]
) -> Path:
    write_pcapng(tmp_path / "gate1.pcapng", sorted(frames))
    (tmp_path / "realism.json").write_text(
        json.dumps(realism_report(errors)), encoding="utf-8"
    )
    return tmp_path


def test_clean_run_writes_empty_files(tmp_path: Path) -> None:
    out = prepare(tmp_path, session(100.0, 40000) + session(101.0, 40001), [])
    assert pf.run(out, 2.0) == []
    doc = json.loads((out / "parser-findings.json").read_text(encoding="utf-8"))
    assert doc["findings"] == [] and doc["errors_total"] == 0
    rows = list(csv.reader((out / "parser-findings.csv").open(encoding="utf-8")))
    assert rows == [list(pf.CSV_FIELDS)]
    assert not (out / "parser-findings.pcapng").exists()


def test_finding_in_the_middle_of_the_capture(tmp_path: Path) -> None:
    frames = session(100.0, 40000) + session(110.0, 40001) + session(130.0, 40002)
    # Session 2 holds frames 7 to 12, and frame 9 carries its request.
    out = prepare(tmp_path, frames, [finding(9, 2, 40001)])
    [entry] = pf.run(out, 2.0)
    assert entry["cut"] == "none"
    assert entry["offset_s"] == pytest.approx(10.002)
    assert entry["session_facts"]["packets"] == 6
    assert entry["session_facts"]["fins"] == 2
    rows = list(csv.DictReader((out / "parser-findings.csv").open(encoding="utf-8")))
    assert (
        rows[0]["parser"] == "tshark"
        and rows[0]["frame"] == "9"
        and rows[0]["cut"] == "none"
    )
    assert rows[0]["client"] == f"{CLIENT}:40001"
    assert (out / "parser-findings.pcapng").stat().st_size > 0


def test_session_cut_at_the_stop(tmp_path: Path) -> None:
    frames = (
        session(100.0, 40000)
        + session(110.0, 40001)
        + session(119.5, 40002, close=False)
    )
    out = prepare(tmp_path, frames, [finding(14, 3, 40002)])
    [entry] = pf.run(out, 2.0)
    assert entry["cut"] == "stop"


def test_session_cut_at_the_start(tmp_path: Path) -> None:
    frames = (
        session(100.2, 40000, syn=False) + session(110.0, 40001) + session(120.0, 40002)
    )
    out = prepare(tmp_path, frames, [finding(1, 1, 40000)])
    [entry] = pf.run(out, 2.0)
    assert entry["cut"] == "start"


def test_session_without_syn_far_from_the_start_is_not_cut(tmp_path: Path) -> None:
    frames = (
        session(100.0, 40000) + session(110.0, 40001, syn=False) + session(120.0, 40002)
    )
    out = prepare(tmp_path, frames, [finding(7, 2, 40001)])
    [entry] = pf.run(out, 2.0)
    assert entry["cut"] == "none"


def test_unattributed_finding_keeps_its_frame(tmp_path: Path) -> None:
    errors = [finding(3, 1, 40000)]
    errors[0]["session"] = None
    errors[0]["client"] = None
    out = prepare(tmp_path, session(100.0, 40000), errors)
    [entry] = pf.run(out, 2.0)
    assert (
        entry["cut"] == "unknown"
        and entry["frame"] == 3
        and entry["offset_s"] == pytest.approx(0.002)
    )
    assert not (out / "parser-findings.pcapng").exists()


def test_missing_inputs_are_reported(tmp_path: Path) -> None:
    with pytest.raises(pf.FindingsError, match="realism.json is missing"):
        pf.run(tmp_path, 2.0)
    (tmp_path / "realism.json").write_text("{}", encoding="utf-8")
    with pytest.raises(pf.FindingsError, match="gate1.pcapng is missing"):
        pf.run(tmp_path, 2.0)
    assert pf.main(["--out-dir", str(tmp_path)]) == 2


def test_second_connection_on_the_tuple_is_a_reused_session(tmp_path: Path) -> None:
    frames = (
        session(100.0, 40000, isn=1000)
        + session(110.0, 40000, isn=900000)
        + session(120.0, 40001)
    )
    # The two connections of port 40000 are session 1, frames 1 to 12.
    out = prepare(tmp_path, frames, [finding(9, 1, 40000)])
    [entry] = pf.run(out, 2.0)
    assert entry["reused_tuple"] is True
    assert entry["session_facts"]["connections"] == 2
    doc = json.loads((out / "parser-findings.json").read_text(encoding="utf-8"))
    assert [r["session"] for r in doc["reused_sessions"]] == [1]
    assert doc["reused_sessions"][0]["connections"] == 2
    rows = list(csv.DictReader((out / "parser-findings.csv").open(encoding="utf-8")))
    assert rows[0]["reused_tuple"] == "True"


def test_retransmitted_syn_is_not_a_reused_session(tmp_path: Path) -> None:
    retransmission = [tcp_frame(100.5, CLIENT, 40000, SERVER, 80, SYN, seq=1000)]
    frames = session(100.0, 40000, isn=1000) + retransmission
    out = prepare(tmp_path, frames, [finding(2, 1, 40000)])
    [entry] = pf.run(out, 2.0)
    assert entry["session_facts"]["syns"] == 2
    assert entry["reused_tuple"] is False
    doc = json.loads((out / "parser-findings.json").read_text(encoding="utf-8"))
    assert doc["reused_sessions"] == []
