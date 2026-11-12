# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the port collision rule of the parser error check (RLM-19, from QA-11).

tshark binds hundreds of ports to a dissector, and the client port of a session is random, so
a well formed session can get malformed packet findings from a dissector that does not fit it.
The rule leaves such a finding out of the count and still reports it.
"""

from __future__ import annotations

import json
import shutil
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from realism.capture import summarize
from realism.classifier import StubClassifier
from realism.cli import EXIT_FAIL, EXIT_PASS, main
from realism.labels import parse
from realism.parser_errors import MAX_ERRORS, ParserErrorsResult, check, port_collision
from realism.parsers import (
    KIND_WEIRD,
    Finding,
    StubParser,
    TsharkRunner,
    ZeekRunner,
    parse_decodes,
)
from realism.report import score
from realism.tests import pcapgen
from realism.tests.conftest import LABELS_HTTP_DNS
from realism.tests.test_parser_errors import (
    DNS_CLIENT,
    DNS_SERVER,
    _fake_ndpi,
    _fake_tshark,
    _malformed_dns_capture,
    _parser_entry,
    _reply_finding,
    _validate,
)

CLASSIFIER = StubClassifier.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi")
# The DNS session of the capture below has the client port 50100, and a second one has 50200.
COLLIDING = {("udp", 50100): "manolito"}
DECODES = "udp.port\t41170\tmanolito\ntcp.port\t44322\tpmproxy\nudp.port\t5060\tsip\n"


def _two_dns_sessions(tmp_path: Path) -> Path:
    cap = pcapgen.mixed_capture(http_sessions=1, dns_lookups=0)
    cap.dns_malformed_answer(DNS_CLIENT, DNS_SERVER, 0x2222)
    cap.dns_malformed_answer(("10.0.0.10", 50200), DNS_SERVER, 0x3333)
    return cap.write_pcap(tmp_path / "two.pcap")


def _tshark(findings: list[Finding], bindings: dict[tuple[str, int], str]) -> StubParser:
    return StubParser(findings, name="tshark", bindings=bindings)


def _run(pcap: Path, parser: StubParser) -> ParserErrorsResult:
    return check(pcap, summarize(pcap).sessions, parse(LABELS_HTTP_DNS), [parser])


def _series(entry: dict[str, Any]) -> dict[tuple[str, str | None], int]:
    return {(m["name"], m["labels"].get("application")): m["value"] for m in entry["metrics"]}


def test_a_finding_of_a_colliding_client_port_is_reported_and_not_counted(tmp_path: Path) -> None:
    result = _run(_two_dns_sessions(tmp_path), _tshark([_reply_finding("tshark")], COLLIDING))
    assert result.passed
    entry = result.to_dict()
    assert (entry["errors_total"], entry["errors"]) == (0, [])
    assert entry["not_counted_total"] == 1
    (skipped,) = entry["not_counted"]
    assert (skipped["rule"], skipped["dissector"]) == ("port_collision", "manolito")
    assert (skipped["parser"], skipped["frame"], skipped["application"]) == ("tshark", 26, "dns")
    assert skipped["client"] == {"ip": DNS_CLIENT[0], "port": DNS_CLIENT[1]}
    assert entry["port_collision_ports"] == 1
    by_app = {a["application"]: a for a in entry["applications"]}
    assert by_app["dns"]["parser_errors"] == by_app["dns"]["malformed_packets"] == 0
    series = _series(entry)
    assert series[("realism.parser_errors", "all")] == 0
    assert series[("realism.check_failures", None)] == 0


def test_a_real_malformed_packet_next_to_a_collision_still_fails(tmp_path: Path) -> None:
    pcap = _two_dns_sessions(tmp_path)
    collision = _reply_finding("tshark")
    real = _reply_finding("tshark", dst_port=50200, frame=28)
    result = _run(pcap, _tshark([collision, real], COLLIDING))
    assert not result.passed
    entry = result.to_dict()
    assert (entry["errors_total"], entry["not_counted_total"]) == (1, 1)
    assert [e["frame"] for e in entry["errors"]] == [28]
    assert [e["frame"] for e in entry["not_counted"]] == [26]
    assert {a["application"]: a["malformed_packets"] for a in entry["applications"]}["dns"] == 1
    assert _series(entry)[("realism.check_failures", None)] == 1


@pytest.mark.parametrize(
    ("bindings", "collides"),
    [
        ({}, False),
        ({("udp", 50100): "manolito", ("udp", 53): "manolito"}, False),
        ({("udp", 50100): "manolito", ("udp", 53): "dns"}, True),
        ({("tcp", 50100): "manolito"}, False),
        ({("udp", 53): "dns"}, False),
    ],
    ids=[
        "no table",
        "server in the same dissector",
        "server in another",
        "other transport",
        "server only",
    ],
)
def test_the_rule_needs_a_bound_client_port_that_the_server_port_does_not_share(
    tmp_path: Path, bindings: dict[tuple[str, int], str], collides: bool
) -> None:
    pcap = _two_dns_sessions(tmp_path)
    session = next(s for s in summarize(pcap).sessions if s.client_port == DNS_CLIENT[1])
    assert (port_collision(session, bindings) is not None) is collides
    result = _run(pcap, _tshark([_reply_finding("tshark")], bindings))
    assert result.passed is collides
    assert result.not_counted_total == int(collides)


def test_a_finding_without_a_session_always_counts(tmp_path: Path) -> None:
    nowhere = _reply_finding("tshark", src_ip="192.0.2.1", dst_port=50100)
    result = _run(_two_dns_sessions(tmp_path), _tshark([nowhere], COLLIDING))
    assert not result.passed
    assert (result.errors_total, result.not_counted_total) == (1, 0)
    assert result.to_dict()["unattributed_errors"] == {"tshark": 1}


def test_zeek_findings_never_collide(tmp_path: Path) -> None:
    weird = _reply_finding("zeek", KIND_WEIRD, frame=None, name="DNS_truncated_RR_rdlength_lt_len")
    assert ZeekRunner().port_bindings() == {}
    assert _run(_two_dns_sessions(tmp_path), StubParser([weird], name="zeek")).errors_total == 1


def test_the_not_counted_list_is_capped_but_the_total_counts_all(tmp_path: Path) -> None:
    many = [_reply_finding("tshark", frame=i + 1) for i in range(MAX_ERRORS + 5)]
    result = _run(_two_dns_sessions(tmp_path), _tshark(many, COLLIDING))
    entry = result.to_dict()
    assert result.passed
    assert len(entry["not_counted"]) == MAX_ERRORS
    assert entry["not_counted_total"] == MAX_ERRORS + 5


def test_the_report_validates_and_closes_the_not_counted_entries(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    real = _reply_finding("tshark", dst_port=50200, frame=28)
    parser = _tshark([_reply_finding("tshark"), real], COLLIDING)
    report = score(
        _two_dns_sessions(tmp_path), labels_file(), CLASSIFIER, parsers=[parser]
    ).to_dict()
    _validate(report)
    entry = _parser_entry(report)
    assert (entry["errors_total"], entry["not_counted_total"]) == (1, 1)
    # A not counted entry has the fields of a finding, and the rule and the dissector.
    assert set(entry["not_counted"][0]) - set(entry["errors"][0]) == {"rule", "dissector"}
    entry["not_counted"][0]["surprise"] = 1
    with pytest.raises(jsonschema.ValidationError):
        _validate(report)


def test_parse_decodes_keeps_numeric_tcp_and_udp_ports() -> None:
    text = DECODES + "ip.proto\t6\ttcp\nudp.port\tx\tbad\nudp.port\t9\n"
    assert parse_decodes(text) == {
        ("udp", 41170): "manolito",
        ("tcp", 44322): "pmproxy",
        ("udp", 5060): "sip",
    }


def test_the_tshark_runner_reads_the_port_table_once(tmp_path: Path) -> None:
    fake = _fake_tshark(tmp_path, "", decodes=DECODES)
    runner = TsharkRunner(str(fake))
    assert len(runner.port_bindings()) == 3
    fake.unlink()
    assert runner.port_bindings()[("udp", 41170)] == "manolito"


def test_a_tshark_that_cannot_print_its_port_table_gives_no_collisions(tmp_path: Path) -> None:
    assert TsharkRunner(str(tmp_path / "absent")).port_bindings() == {}


def test_cli_does_not_count_a_collision_and_counts_it_without_the_table(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    from realism.tests.test_parsers import MALFORMED_DNS_LINE

    out = tmp_path / "report.json"
    argv = ["score", "--pcap", str(_malformed_dns_capture(tmp_path)), "--labels"]
    argv += [str(labels_file()), "--ndpi-reader", str(_fake_ndpi(tmp_path))]
    argv += ["--output", str(out), "--parsers", "tshark", "--tshark"]
    table = "udp.port\t50100\tmanolito\n"
    assert main([*argv, str(_fake_tshark(tmp_path, MALFORMED_DNS_LINE, table))]) == EXIT_PASS
    entry = _parser_entry(json.loads(out.read_text()))
    assert (entry["errors_total"], entry["not_counted_total"]) == (0, 1)
    assert main([*argv, str(_fake_tshark(tmp_path, MALFORMED_DNS_LINE))]) == EXIT_FAIL
    entry = _parser_entry(json.loads(out.read_text()))
    assert (entry["errors_total"], entry["not_counted_total"]) == (1, 0)


@pytest.mark.skipif(shutil.which("tshark") is None, reason="tshark is not installed")
def test_real_tshark_a_colliding_client_port_passes_and_a_malformed_packet_fails(
    tmp_path: Path,
) -> None:
    """Acceptance of RLM-19: UDP 41170 is bound to Manolito, and UDP 41171 is free."""
    media = {"application": "media", "match": {"transport": "udp", "server_port": 40000}}
    sessions = [*LABELS_HTTP_DNS["sessions"], media]  # type: ignore[misc]
    labels = parse({**LABELS_HTTP_DNS, "sessions": sessions})

    def capture(client_port: int, name: str, malformed: bool = False) -> Path:
        cap = pcapgen.Capture()
        for i in range(3):
            cap.add(
                pcapgen.udp("10.0.0.20", client_port, "10.0.1.20", 40000, bytes([0xAB]) * (20 + i))
            )
            cap.add(pcapgen.udp("10.0.1.20", 40000, "10.0.0.20", client_port, bytes([0xCD]) * 30))
        if malformed:
            cap.dns_malformed_answer(DNS_CLIENT, DNS_SERVER, 0x2222)
        return cap.write_pcap(tmp_path / name)

    def run(pcap: Path) -> ParserErrorsResult:
        return check(pcap, summarize(pcap).sessions, labels, [TsharkRunner()])

    collision = run(capture(41170, "collision.pcap"))
    assert collision.passed
    assert collision.errors_total == 0
    assert collision.not_counted_total > 0
    assert {e["dissector"] for e in collision.not_counted} == {"manolito"}
    assert {e["application"] for e in collision.not_counted} == {"media"}
    assert collision.port_collision_ports > 100

    free = run(capture(41171, "free.pcap"))
    assert free.passed and free.not_counted_total == 0

    mixed = run(capture(41170, "mixed.pcap", malformed=True))
    assert not mixed.passed
    assert mixed.errors_total == 1
    assert mixed.not_counted_total == collision.not_counted_total
    assert mixed.to_dict()["errors"][0]["application"] == "dns"
