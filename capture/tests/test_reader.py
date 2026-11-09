# SPDX-License-Identifier: GPL-3.0-only
"""The pcap and pcapng readers, including multiple interfaces and damaged files."""

from __future__ import annotations

import json
import struct
import subprocess
import sys
from pathlib import Path

import pytest

from capture.cli import main
from capture.pcapio import CaptureError, Reader

from .conftest import Extract
from .pcapgen import Rec, pcapng_section, tcp_frame, udp_frame, write_pcap

C, S = "10.1.1.1", "10.1.1.2"


def frames(data: bytes) -> tuple[Reader, list[tuple[int, int, int]]]:
    reader = Reader(data)
    out = [(f.ts_ns, f.interface, f.link_type) for f in reader.frames()]
    return reader, out


def test_pcap_variants_give_the_same_timestamp() -> None:
    rec = [Rec(1700000000.25, udp_frame(C, S, 1, 2, b"x"))]
    for kwargs in ({}, {"nano": True}, {"big": True}, {"nano": True, "big": True}):
        reader, got = frames(write_pcap(rec, **kwargs))
        assert reader.format == "pcap"
        assert got == [(1_700_000_000_250_000_000, 0, 1)]


def test_pcapng_resolution_options() -> None:
    f = udp_frame(C, S, 1, 2, b"x")
    # microseconds, nanoseconds, milliseconds, and a power of two resolution
    ifaces = [(1, 6), (1, 9), (1, 3), (1, 0x80 | 10)]
    recs = [Rec(1000.5, f, i) for i in range(4)]
    reader, got = frames(pcapng_section(ifaces, recs))
    assert [g[0] for g in got] == [1_000_500_000_000] * 4
    assert [i.tsresol for i in reader.interfaces] == ["1e-6", "1e-9", "1e-3", "2^-10"]


def test_pcapng_with_multiple_interfaces_and_link_types(run: Extract) -> None:
    eth = tcp_frame(C, S, 40000, 80, flags=0x02, seq=1)
    raw = udp_frame(C, S, 5000, 5001, b"hi", link="raw")
    data = pcapng_section(
        [(1, 9), (101, 6)],
        [
            Rec(1.0, eth, 0),
            Rec(1.1, raw, 1),
            Rec(1.2, udp_frame(S, C, 5001, 5000, b"yo", "raw"), 1),
        ],
        names=["eth0", "tun0"],
    )
    doc = run(data)
    assert doc["source"]["format"] == "pcapng"
    assert [(i["id"], i["link_type"], i["name"]) for i in doc["source"]["interfaces"]] == [
        (0, 1, "eth0"),
        (1, 101, "tun0"),
    ]
    tcp, udp = doc["sessions"]
    assert tcp["interfaces"] == [0] and udp["interfaces"] == [1]
    assert udp["client_to_server"]["packets"] == 1 and udp["server_to_client"]["packets"] == 1
    only_tun = run(data, interface=1)
    assert only_tun["summary"] == {"sessions": 1, "tcp": 0, "udp": 1}


def test_concatenated_sections_number_interfaces_across_the_file(run: Extract) -> None:
    f = udp_frame(C, S, 1, 2, b"x")
    a = pcapng_section([(1, 9)], [Rec(1.0, f)], names=["agent-a"])
    b = pcapng_section([(1, 9)], [Rec(2.0, f)], names=["agent-b"], big=True)
    doc = run(a + b)
    assert [(i["id"], i["section"], i["name"]) for i in doc["source"]["interfaces"]] == [
        (0, 0, "agent-a"),
        (1, 1, "agent-b"),
    ]
    assert doc["sessions"][0]["interfaces"] == [0, 1]


def test_out_of_order_timestamps_are_sorted_with_a_warning(run: Extract) -> None:
    f = udp_frame(C, S, 1, 2, b"x")
    a = pcapng_section([(1, 9)], [Rec(5.0, f), Rec(6.0, f)])
    b = pcapng_section([(1, 9)], [Rec(1.0, f)])
    doc = run(a + b)
    assert doc["sessions"][0]["start_ns"] == 1_000_000_000
    assert any("earlier timestamps" in w for w in doc["source"]["warnings"])


def test_pcapng_without_known_interface_warns(run: Extract) -> None:
    data = bytearray(pcapng_section([(1, 6)], [Rec(1.0, udp_frame(C, S, 1, 2))]))
    data[68:72] = struct.pack("<I", 3)  # the interface id of the only packet block
    doc = run(bytes(data))
    assert doc["source"]["frames"] == 0
    assert any("unknown interface 3" in w for w in doc["source"]["warnings"])


def test_truncated_capture_keeps_frames_before_the_cut(run: Extract) -> None:
    f = udp_frame(C, S, 1, 2, b"x")
    full = write_pcap([Rec(1.0, f), Rec(2.0, f)])
    doc = run(full[:-5])
    assert doc["source"]["packets"] == 1
    assert "ends inside a record" in doc["source"]["warnings"][0]
    ng = pcapng_section([(1, 6)], [Rec(1.0, f), Rec(2.0, f)])
    doc = run(ng[:-7])
    assert doc["source"]["packets"] == 1 and doc["source"]["warnings"]


@pytest.mark.parametrize(
    "data, text",
    [
        (b"", "empty"),
        (b"\x01\x02", "too short"),
        (b"not a capture file", "unknown capture magic"),
        (write_pcap([])[:20], "pcap header"),
        (
            write_pcap([Rec(1.0, b"x" * 20)])[:24] + struct.pack("<IIII", 1, 0, 2**31, 2**31),
            "implausible",
        ),
    ],
)
def test_bad_files_raise_capture_error(tmp_path: Path, data: bytes, text: str) -> None:
    path = tmp_path / "bad.cap"
    path.write_bytes(data)
    with pytest.raises(CaptureError, match=text):
        from capture.cli import extract

        extract(path)


def test_pcapng_block_length_mismatch_is_an_error(tmp_path: Path) -> None:
    data = bytearray(pcapng_section([(1, 6)], [Rec(1.0, udp_frame(C, S, 1, 2))]))
    data[-4:] = struct.pack("<I", 99)
    path = tmp_path / "bad.pcapng"
    path.write_bytes(bytes(data))
    assert main(["sessions", "--pcap", str(path)]) == 1


def test_cli_writes_json_file_and_reports_errors(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    cap = tmp_path / "x.pcap"
    cap.write_bytes(write_pcap([Rec(1.0, udp_frame(C, S, 1, 2, b"x"))]))
    out = tmp_path / "out.json"
    assert main(["sessions", "--pcap", str(cap), "--out", str(out)]) == 0
    assert json.loads(out.read_text())["summary"]["udp"] == 1
    assert main(["sessions", "--pcap", str(cap)]) == 0
    assert json.loads(capsys.readouterr().out)["schema_version"] == "1.0"
    assert main(["sessions", "--pcap", str(tmp_path / "missing.pcap")]) == 1
    assert "cannot read capture" in capsys.readouterr().err
    assert main(["sessions", "--pcap", str(cap), "--out", str(tmp_path / "no" / "o.json")]) == 1
    with pytest.raises(SystemExit):
        main(["sessions", "--pcap", str(cap), "--udp-idle-timeout", "0"])


def test_module_entry_point_runs(tmp_path: Path) -> None:
    cap = tmp_path / "x.pcap"
    cap.write_bytes(write_pcap([Rec(1.0, udp_frame(C, S, 1, 2, b"x"))]))
    root = Path(__file__).resolve().parents[2]
    done = subprocess.run(
        [sys.executable, "-m", "capture", "sessions", "--pcap", str(cap)],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    assert json.loads(done.stdout)["summary"]["sessions"] == 1


def test_unknown_blocks_such_as_decryption_secrets_are_skipped(run: Extract) -> None:
    # fps-capture writes decryption secrets blocks (type 10) between packet blocks
    f = udp_frame(C, S, 1, 2, b"x")
    data = bytearray(pcapng_section([(1, 9)], [Rec(1.0, f)]))
    secret = b"CLIENT_RANDOM 00 00\n"
    body = struct.pack("<II", 0x544C534B, len(secret)) + secret
    body += bytes(-len(body) % 4)
    total = 12 + len(body)
    data += struct.pack("<II", 10, total) + body + struct.pack("<I", total)
    data += pcapng_section([(1, 9)], [Rec(2.0, f)])[28:]  # the packet blocks of a second capture
    doc = run(bytes(data))
    assert doc["source"]["warnings"] == []
    assert doc["sessions"][0]["client_to_server"]["packets"] == 2
