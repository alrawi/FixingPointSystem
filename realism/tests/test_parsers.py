# SPDX-License-Identifier: GPL-3.0-only
"""Unit and component tests of the parser runners with fake tshark and Zeek programs."""

from __future__ import annotations

import json
import sys
from collections.abc import Callable
from pathlib import Path

import pytest

from realism.capture import SessionKey
from realism.parsers import (
    KIND_ERROR,
    KIND_MALFORMED,
    KIND_WEIRD,
    Finding,
    ParserError,
    StubParser,
    TsharkRunner,
    ZeekRunner,
    parse_tshark_fields,
    parse_zeek_logs,
    resolve,
)

US = "\x1f"
MALFORMED_DNS_LINE = "\t".join(
    [
        "26",
        "1700000000.025998000",
        "10.0.1.53",
        "10.0.0.10",
        "",
        "",
        "",
        "",
        "53",
        "50100",
        "eth:ethertype:ip:udp:dns",
        "8388608",
        "117440512",
        "Malformed Packet (Exception occurred)",
    ]
)


def _line(**over: str) -> str:
    cols = dict(
        number="7",
        epoch="1.5",
        ip_src="",
        ip_dst="",
        ip6_src="2001:db8::10",
        ip6_dst="2001:db8::80",
        tcp_src="41000",
        tcp_dst="80",
        udp_src="",
        udp_dst="",
        protocols="eth:ethertype:ipv6:tcp:http",
        severity=f"6291456{US}8388608",
        group=f"33554432{US}83886080",
        message=f"Connection reset{US}Bad header\twith tab",
    )
    cols.update(over)
    return "\t".join(cols.values())


def test_tshark_malformed_dns_line_is_one_malformed_finding() -> None:
    (finding,) = parse_tshark_fields(MALFORMED_DNS_LINE + "\n")
    assert finding.kind == KIND_MALFORMED
    assert finding.name == "dns"
    assert finding.frame == 26
    assert finding.timestamp == pytest.approx(1700000000.025998)
    assert finding.key == SessionKey.of("udp", "10.0.0.10", 50100, "10.0.1.53", 53)


def test_tshark_error_line_takes_the_first_error_message_and_ipv6_tcp_endpoints() -> None:
    (finding,) = parse_tshark_fields(_line())
    assert finding.kind == KIND_ERROR
    assert finding.message == "Bad header\twith tab"
    assert finding.name == "http"
    assert finding.key == SessionKey.of("tcp", "2001:db8::10", 41000, "2001:db8::80", 80)


def test_tshark_line_without_error_severity_or_ports_is_handled() -> None:
    assert parse_tshark_fields(_line(severity="6291456", group="0", message="w")) == []
    (finding,) = parse_tshark_fields(_line(protocols="eth:ethertype:arp", epoch="x"))
    assert finding.key is None
    assert finding.timestamp is None
    assert finding.name == "arp"


def test_tshark_short_or_bad_lines_raise() -> None:
    with pytest.raises(ParserError, match="line 1 has 3 fields"):
        parse_tshark_fields("1\t2\t3")
    with pytest.raises(ParserError, match="frame"):
        parse_tshark_fields(_line(number="x"))


def _weird(**fields: object) -> str:
    row: dict[str, object] = {
        "ts": 10.5,
        "uid": "C1",
        "id.orig_h": "10.0.0.10",
        "id.orig_p": 50100,
        "id.resp_h": "10.0.1.53",
        "id.resp_p": 53,
        "name": "DNS_truncated_RR_rdlength_lt_len",
        "addl": "",
        "notice": False,
        "peer": "zeek",
    }
    row.update(fields)
    return json.dumps(row)


def test_zeek_weirds_of_one_packet_merge_and_state_weirds_do_not_count() -> None:
    weird = "\n".join(
        [
            _weird(),
            _weird(name="DNS_RR_length_mismatch", addl="x"),
            _weird(),
            _weird(name="dns_unmatched_msg", ts=11.0),
            _weird(uid="C2", ts=12.0, name="truncated_header"),
        ]
    )
    conn = json.dumps({"uid": "C1", "proto": "udp"}) + "\n" + json.dumps({"uid": "C9"})
    first, second = parse_zeek_logs(weird, conn)
    assert first.kind == KIND_WEIRD
    assert first.name == "DNS_truncated_RR_rdlength_lt_len,DNS_RR_length_mismatch"
    assert first.key == SessionKey.of("udp", "10.0.0.10", 50100, "10.0.1.53", 53)
    assert second.transport is None
    assert second.key is None


def test_zeek_weird_without_connection_is_unattributed() -> None:
    line = json.dumps({"ts": 1.0, "name": "bad_IP_checksum", "notice": False})
    (finding,) = parse_zeek_logs(line + "\n#close\n")
    assert finding.key is None
    assert finding.src_ip is None


def test_zeek_bad_json_raises() -> None:
    with pytest.raises(ParserError, match=r"weird\.log line 1"):
        parse_zeek_logs("{not json")
    with pytest.raises(ParserError, match="not an object"):
        parse_zeek_logs("[1]")


def test_stub_parser_returns_fixed_or_computed_findings() -> None:
    finding = Finding("stub", KIND_ERROR, "dns")
    assert StubParser([finding]).findings(Path("x")) == [finding]
    assert StubParser(lambda p: [finding] if p.name == "y" else []).findings(Path("x")) == []
    assert StubParser().version() == "stub"


@pytest.fixture
def fake_program(tmp_path: Path) -> Callable[..., Path]:
    """Return a factory for an executable that prints fixed output or writes logs."""

    def make(
        name: str,
        version: str,
        stdout: str = "",
        logs: dict[str, str] | None = None,
        exit_code: int = 0,
        sleep_s: float = 0,
    ) -> Path:
        script = tmp_path / name
        script.write_text(
            f"#!{sys.executable}\n"
            "import pathlib, sys, time\n"
            "if sys.argv[1:] == ['--version']:\n"
            f"    print({version!r})\n"
            "    sys.exit(0)\n"
            f"time.sleep({sleep_s})\n"
            'print(\'Running as user "root" and group "root".\', file=sys.stderr)\n'
            f"if {exit_code}:\n"
            "    print('cannot read capture', file=sys.stderr)\n"
            f"    sys.exit({exit_code})\n"
            f"for log, text in {logs or {}!r}.items():\n"
            "    pathlib.Path(log).write_text(text)\n"
            f"sys.stdout.write({stdout!r})\n"
        )
        script.chmod(0o755)
        return script

    return make


def test_tshark_runner_reads_version_and_findings(fake_program: Callable[..., Path]) -> None:
    program = fake_program("tshark", "TShark (Wireshark) 4.2.2 (Git v4.2.2)", MALFORMED_DNS_LINE)
    runner = TsharkRunner(str(program))
    assert runner.available()
    assert runner.version() == "4.2.2"
    assert [f.frame for f in runner.findings(Path("cap.pcap"))] == [26]


def test_zeek_runner_reads_logs_from_its_working_directory(
    fake_program: Callable[..., Path],
) -> None:
    logs = {"weird.log": _weird() + "\n", "conn.log": json.dumps({"uid": "C1", "proto": "udp"})}
    program = fake_program("zeek", "zeek version 9.0.1", logs=logs)
    runner = ZeekRunner(str(program))
    assert runner.version() == "9.0.1"
    (finding,) = runner.findings(Path("cap.pcap"))
    assert finding.transport == "udp"
    assert ZeekRunner(str(fake_program("zeek", "zeek version 9.0.1"))).findings(Path("c")) == []


def test_runner_failures_raise_parser_errors(
    tmp_path: Path, fake_program: Callable[..., Path]
) -> None:
    failing = TsharkRunner(str(fake_program("tshark", "x", exit_code=2)))
    with pytest.raises(ParserError, match="exited with 2: cannot read capture"):
        failing.findings(Path("cap.pcap"))
    assert failing.version() == "unknown"
    slow = TsharkRunner(str(fake_program("slow", "x", sleep_s=5)), timeout_s=0.2)
    with pytest.raises(ParserError, match="ran longer"):
        slow.findings(Path("cap.pcap"))
    absent = ZeekRunner(str(tmp_path / "absent"))
    assert not absent.available()
    with pytest.raises(ParserError, match="not found"):
        absent.version()


def test_resolve_requires_or_reports_missing_parsers(
    tmp_path: Path, fake_program: Callable[..., Path]
) -> None:
    tshark = str(fake_program("tshark", "TShark (Wireshark) 4.2.2"))
    absent = str(tmp_path / "absent")
    runners, missing = resolve(["tshark", "zeek"], tshark=tshark, zeek=absent, required=False)
    assert [r.name for r in runners] == ["tshark"]
    assert missing == ["zeek"]
    with pytest.raises(ParserError, match="zeek program"):
        resolve(["zeek"], zeek=absent)
    with pytest.raises(ParserError, match="unknown parser"):
        resolve(["wireshark"])
