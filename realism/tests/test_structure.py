# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the session structure check: scenario graphs, instance reconstruction, and the report.

Every capture is built in code. The scenarios are the first-party files of the repository
(ftp, dns, http1) and the web_browse example of ``realism/examples``.
"""

from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from realism import scenario_graph, structure
from realism.capture import summarize
from realism.classifier import StubClassifier
from realism.cli import EXIT_FAIL, main
from realism.labels import LabelsError, load, parse
from realism.report import LABELS_SCHEMA_PATH, SCHEMA_PATH, score
from realism.scenario_graph import ScenarioError
from realism.tests import pcapgen
from realism.tests.conftest import NDPI_42_HEADER

REPO = Path(__file__).resolve().parents[2]
EXAMPLES = REPO / "realism" / "examples"
WEB_BROWSE = EXAMPLES / "web_browse.scenario.yaml"
FTP_MIXED = REPO / "modules" / "ftp" / "scenarios" / "mixed_session.yaml"
DNS_FALLBACK = REPO / "modules" / "dns" / "scenarios" / "tcp_fallback.yaml"
HTTP_KEEPALIVE = REPO / "modules" / "http1" / "scenarios" / "keepalive_browse.yaml"

DNS = ("10.0.1.53", 53)
WEB = ("10.0.1.80", 8080)
FTP = ("10.0.1.21", 21)
CLIENT = "10.0.0.10"


def _labels_doc(structure_section: dict[str, Any] | None = None) -> dict[str, Any]:
    doc: dict[str, Any] = {"labels": "fps-realism-labels", "version": 1}
    if structure_section is not None:
        doc["structure"] = structure_section
    return doc


def _check(
    tmp_path: Path,
    cap: pcapgen.Capture,
    scenario: Path = WEB_BROWSE,
    **section: Any,
) -> structure.StructureResult:
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    labels = parse(_labels_doc({"scenarios": [{"file": str(scenario)}], **section}))
    return structure.check(pcap, summarize(pcap).sessions, labels)


def _browse(
    cap: pcapgen.Capture, client: str = CLIENT, port: int = 40000, think: float = 0.02
) -> None:
    """One web_browse instance: a lookup, then the page fetch."""
    cap.dns_lookup((client, port), DNS, port)
    cap.clock += think
    cap.http_get((client, port + 1), WEB)
    cap.clock += think


def _span(
    cap: pcapgen.Capture,
    client: tuple[str, int],
    server: tuple[str, int],
    start: float,
    end: float,
    payload: bytes = b"data\r\n",
) -> None:
    """Append one TCP session that is open from ``start`` until ``end``."""
    (c, cp), (s, sp) = client, server
    cs, ss = 1000, 5000
    cap.clock = start
    cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.SYN, cs, 0), gap=0)
    cap.add(pcapgen.tcp(s, sp, c, cp, pcapgen.SYN | pcapgen.ACK, ss, cs + 1))
    cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.ACK, cs + 1, ss + 1))
    cap.add(pcapgen.tcp(s, sp, c, cp, pcapgen.PSH | pcapgen.ACK, ss + 1, cs + 1, payload))
    cap.clock = end - 0.002
    cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.FIN | pcapgen.ACK, cs + 1, ss + 1 + len(payload)))
    cap.add(pcapgen.tcp(s, sp, c, cp, pcapgen.FIN | pcapgen.ACK, ss + 1 + len(payload), cs + 2))
    cap.add(pcapgen.tcp(c, cp, s, sp, pcapgen.ACK, cs + 2, ss + 2 + len(payload)))


def _sorted(cap: pcapgen.Capture) -> pcapgen.Capture:
    cap.frames.sort(key=lambda frame: frame[0])
    return cap


def _ftp_instance(
    cap: pcapgen.Capture,
    start: float,
    data_ports: list[int],
    client: str = CLIENT,
    control_port: int = 41000,
) -> None:
    """One ftp control connection with one data connection per port, inside its lifetime."""
    end = start + 1.0
    _span(cap, (client, control_port), FTP, start, end)
    for n, port in enumerate(data_ports):
        at = start + 0.1 + 0.2 * n
        _span(cap, (client, control_port + 1 + n), (FTP[0], port), at, at + 0.1)


# --- the web_browse acceptance -------------------------------------------------------


def test_web_browse_fixture_passes(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i in range(3):
        _browse(cap, client=f"10.0.0.{10 + i}", port=40000 + 10 * i)
    result = _check(tmp_path, cap)
    assert result.failures == []
    assert result.passed
    (scenario,) = result.scenarios
    assert len(scenario.instances) == 3
    assert [i.client_ip for i in scenario.instances] == ["10.0.0.10", "10.0.0.11", "10.0.0.12"]
    assert all(i.count("lookup") == 1 and i.count("page") == 1 for i in scenario.instances)
    assert result.sessions_unassigned == 0


def test_page_fetch_before_the_lookup_fails_and_names_the_edge(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.http_get((CLIENT, 40001), WEB)
    cap.clock += 0.02
    cap.dns_lookup((CLIENT, 40000), DNS, 1)
    result = _check(tmp_path, cap)
    assert not result.passed
    (failure,) = result.failures
    assert "scenario web_browse instance 1 (client 10.0.0.10" in failure
    assert "edge after lookup -> page violated" in failure
    assert (
        "page (session 1, tcp 10.0.0.10:40001 -> 10.0.1.80:8080) starts at +0.000000 s" in failure
    )
    assert (
        "before lookup (session 2, udp 10.0.0.10:40000 -> 10.0.1.53:53) completes at +" in failure
    )


def test_the_page_fetch_may_start_after_the_lookup_with_a_tolerance(tmp_path: Path) -> None:
    # The page starts 1 ms before the answer of the lookup. A tolerance of 5 ms accepts it.
    cap = pcapgen.Capture()
    cap.add(pcapgen.udp(CLIENT, 40000, DNS[0], 53, b"\x00\x01\x01\x00" + bytes(8)))
    cap.clock += 0.004
    cap.http_get((CLIENT, 40001), WEB)
    cap.add(pcapgen.udp(DNS[0], 53, CLIENT, 40000, b"\x00\x01\x81\x80" + bytes(8)), gap=-0.0015)
    cap.frames.sort(key=lambda f: f[0])
    assert not _check(tmp_path, cap).passed
    assert _check(tmp_path, cap, tolerance_s=0.05).passed


def test_a_missing_lookup_fails_the_count_and_names_the_edge(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.http_get((CLIENT, 40001), WEB)
    result = _check(tmp_path, cap)
    assert not result.passed
    (failure,) = result.failures
    assert "flow lookup has 0 connections, the scenario allows 1" in failure
    assert "missing for edge after lookup -> page" in failure


def test_a_missing_page_fetch_fails_the_count(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.dns_lookup((CLIENT, 40000), DNS, 1)
    result = _check(tmp_path, cap)
    (failure,) = result.failures
    assert "flow page has 0 connections" in failure


def test_a_repeated_page_fetch_opens_an_instance_that_says_why(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _browse(cap)
    cap.http_get((CLIENT, 40005), WEB)
    result = _check(tmp_path, cap)
    (failure,) = result.failures
    assert "instance 2" in failure
    assert "flow lookup has 0 connections" in failure
    assert "opened because instance 1 has no room for it (page already has 1 of 1)" in failure


def test_one_client_runs_instances_one_after_another(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i in range(4):
        _browse(cap, port=40000 + 10 * i)
    result = _check(tmp_path, cap)
    assert result.passed
    assert len(result.scenarios[0].instances) == 4


def test_max_gap_splits_two_halves_that_are_far_apart(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.dns_lookup((CLIENT, 40000), DNS, 1)
    cap.clock += 120
    cap.http_get((CLIENT, 40001), WEB)
    assert _check(tmp_path, cap).passed
    result = _check(tmp_path, cap, max_gap_s=30)
    assert not result.passed
    assert len(result.scenarios[0].instances) == 2
    assert any("more than max_gap_s 30" in f for f in result.failures)


def test_unrelated_sessions_are_counted_and_not_judged(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _browse(cap)
    cap.http_get((CLIENT, 40020), ("10.0.1.99", 9999))
    result = _check(tmp_path, cap)
    assert result.passed
    assert result.sessions_unassigned == 1


def test_a_capture_without_any_matching_session_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.http_get((CLIENT, 40020), ("10.0.1.99", 9999))
    result = _check(tmp_path, cap)
    assert result.failures == [
        "scenario web_browse: no session of the capture matches a flow of it"
    ]


def test_expected_instances_must_agree(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _browse(cap)
    _browse(cap, port=40010)
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    sessions = summarize(pcap).sessions
    ok = parse(_labels_doc({"scenarios": [{"file": str(WEB_BROWSE), "expected_instances": 2}]}))
    assert structure.check(pcap, sessions, ok).passed
    bad = parse(_labels_doc({"scenarios": [{"file": str(WEB_BROWSE), "expected_instances": 3}]}))
    assert structure.check(pcap, sessions, bad).failures == [
        "scenario web_browse: 2 instances found, expected 3"
    ]


def test_match_limits_the_sessions_that_a_scenario_claims(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _browse(cap, client="10.0.0.10")
    cap.http_get(("10.0.0.77", 40100), WEB)  # a page fetch of a client that is out of scope
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    scoped = parse(
        _labels_doc({"scenarios": [{"file": str(WEB_BROWSE), "match": {"client_ip": "10.0.0.10"}}]})
    )
    result = structure.check(pcap, summarize(pcap).sessions, scoped)
    assert result.passed
    assert result.sessions_unassigned == 1


def test_a_port_override_follows_the_run(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.dns_lookup((CLIENT, 40000), DNS, 1)
    cap.clock += 0.02
    cap.http_get((CLIENT, 40001), ("10.0.1.80", 9090))
    assert not _check(tmp_path, cap).passed
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    labels = parse(
        _labels_doc({"scenarios": [{"file": str(WEB_BROWSE), "vars": {"server_port": 9090}}]})
    )
    assert structure.check(pcap, summarize(pcap).sessions, labels).passed


# --- ftp: spawn edges and data connections on announced ports -------------------------


def test_ftp_control_with_three_data_connections_passes(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _ftp_instance(cap, 10.0, [50011, 50712, 50001])
    result = _check(tmp_path, _sorted(cap), FTP_MIXED)
    assert result.failures == []
    (instance,) = result.scenarios[0].instances
    assert instance.count("control") == 1
    assert sum(instance.count(f) for f in ("data_list", "data_retr", "data_stor")) == 3


def test_ftp_data_connections_take_the_flows_in_spawn_order(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _ftp_instance(cap, 10.0, [50011, 50712, 50001])
    result = _check(tmp_path, _sorted(cap), FTP_MIXED)
    (instance,) = result.scenarios[0].instances
    by_flow = {f: c.session.server_port for f, c in instance.members}
    assert by_flow == {"control": 21, "data_list": 50011, "data_retr": 50712, "data_stor": 50001}


def test_ftp_with_a_fourth_data_connection_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _ftp_instance(cap, 10.0, [50011, 50712, 50001, 50002])
    result = _check(tmp_path, _sorted(cap), FTP_MIXED)
    assert not result.passed
    assert any("flow control has 0 connections" in f for f in result.failures)
    assert any("instance 1 has no room for it" in f for f in result.failures)


def test_ftp_with_a_missing_data_connection_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _ftp_instance(cap, 10.0, [50011, 50712])
    result = _check(tmp_path, _sorted(cap), FTP_MIXED)
    assert len(result.failures) == 1
    assert "flow data_stor has 0 connections" in result.failures[0]
    assert "missing for edge spawn control -> data_stor" in result.failures[0]


def test_ftp_data_connection_before_its_control_connection_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _span(cap, (CLIENT, 41000), FTP, 10.0, 11.0)
    for n, port in enumerate([50011, 50712, 50001]):
        _span(cap, (CLIENT, 41001 + n), (FTP[0], port), 9.5 + 0.1 * n, 9.55 + 0.1 * n)
    result = _check(tmp_path, _sorted(cap), FTP_MIXED)
    assert not result.passed
    assert any(
        "edge spawn control -> data_list violated" in f and "before control starts" in f
        for f in result.failures
    )


def test_ftp_data_connection_after_its_control_connection_ended_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _ftp_instance(cap, 10.0, [50011, 50712])
    _span(cap, (CLIENT, 41009), (FTP[0], 50001), 12.0, 12.1)
    result = _check(tmp_path, _sorted(cap), FTP_MIXED)
    assert any(
        "edge spawn control -> data_stor violated" in f and "after control ended" in f
        for f in result.failures
    )


def test_two_ftp_sessions_of_one_client_make_two_instances(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _ftp_instance(cap, 10.0, [50011, 50712, 50001], control_port=41000)
    _ftp_instance(cap, 20.0, [50021, 50022, 50023], control_port=42000)
    result = _check(tmp_path, _sorted(cap), FTP_MIXED)
    assert result.failures == []
    assert len(result.scenarios[0].instances) == 2


# --- other first-party scenarios -------------------------------------------------------


def test_dns_tcp_fallback_orders_the_tcp_retry_after_the_udp_query(tmp_path: Path) -> None:
    good = pcapgen.Capture()
    good.dns_lookup((CLIENT, 40000), DNS, 1)
    good.clock += 0.01
    good.tcp_session((CLIENT, 40001), DNS, b"\x00\x1d" + bytes(29), b"\x00\x20" + bytes(32))
    assert _check(tmp_path, good, DNS_FALLBACK).failures == []
    bad = pcapgen.Capture()
    bad.tcp_session((CLIENT, 40001), DNS, b"\x00\x1d" + bytes(29), b"\x00\x20" + bytes(32))
    bad.clock += 0.01
    bad.dns_lookup((CLIENT, 40000), DNS, 1)
    (failure,) = _check(tmp_path, bad, DNS_FALLBACK).failures
    assert "edge after udp -> tcp violated" in failure


def test_a_single_flow_scenario_makes_one_instance_per_connection(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i in range(5):
        cap.http_get((CLIENT, 40000 + i), ("10.0.1.80", 8080))
    result = _check(tmp_path, cap, HTTP_KEEPALIVE)
    assert result.passed
    assert len(result.scenarios[0].instances) == 5


# --- scenario graphs -------------------------------------------------------------------


def test_graph_of_web_browse() -> None:
    graph = scenario_graph.load(WEB_BROWSE)
    assert graph.name == "web_browse"
    assert [(e.kind, e.source, e.target) for e in graph.edges] == [("after", "lookup", "page")]
    lookup, page = graph.flows
    assert (lookup.transport, lookup.server_port) == ("udp", 53)
    assert (page.transport, page.server_port) == ("tcp", 8080)
    assert page.multiplicity.text() == "1"


def test_graph_of_ftp_mixed_session_counts_spawns_and_leaves_data_ports_open() -> None:
    graph = scenario_graph.load(FTP_MIXED)
    flows = {f.id: f for f in graph.flows}
    assert flows["control"].server_port == 21
    assert {flows[f].server_port for f in ("data_list", "data_retr", "data_stor")} == {None}
    assert {(e.source, e.target) for e in graph.edges} == {
        ("control", "data_list"),
        ("control", "data_retr"),
        ("control", "data_stor"),
    }
    assert [flows[f].spawn_order for f in ("data_list", "data_retr", "data_stor")] == [1, 2, 3]
    assert all(flows[f].multiplicity.text() == "1" for f in flows)


def _scenario(flows: str, variables: str = "vars: {n: 3}") -> dict[str, Any]:
    import yaml

    doc: dict[str, Any] = yaml.safe_load(f"scenario: s\n{variables}\nflows:\n{flows}")
    return doc


def test_a_spawn_in_a_loop_repeats_and_a_spawn_in_a_branch_may_not_happen() -> None:
    doc = _scenario(
        """
  - id: main
    module: m
    client:
      - loop: {count: "{{ vars.n }}", do: [{spawn: {flow: child}}]}
      - branch:
          - {p: 0.5, do: [{spawn: {flow: maybe}}]}
          - {p: 0.5, do: [{delay: {ms: 1}}]}
      - loop: {count: "{{ rand_int(1, 4) }}", do: [{spawn: {flow: some}}]}
  - {id: child, module: m, spawned: true, client: [{connect: {port: 1}}]}
  - {id: maybe, module: m, spawned: true, client: [{connect: {port: 2}}]}
  - {id: some, module: m, spawned: true, client: [{connect: {port: 3}}]}
"""
    )
    flows = {f.id: f.multiplicity for f in scenario_graph.parse(doc).flows}
    assert (flows["child"].low, flows["child"].high) == (3, 3)
    assert (flows["maybe"].low, flows["maybe"].high) == (0, 1)
    assert (flows["some"].low, flows["some"].high) == (0, None)
    assert flows["some"].allows(0) and flows["some"].allows(99)
    assert not flows["child"].allows(2)


def test_a_flow_that_nothing_spawns_is_not_allowed_to_run() -> None:
    doc = _scenario(
        """
  - {id: main, module: m, client: [{connect: {port: 1}}]}
  - {id: orphan, module: m, spawned: true, client: [{connect: {port: 2}}]}
"""
    )
    orphan = scenario_graph.parse(doc).flow("orphan")
    assert (orphan.multiplicity.low, orphan.multiplicity.high) == (0, 0)


def test_a_child_of_a_looped_child_multiplies() -> None:
    doc = _scenario(
        """
  - id: main
    module: m
    client: [{loop: {count: 2, do: [{spawn: {flow: mid}}]}}]
  - id: mid
    module: m
    spawned: true
    client: [{loop: {count: 3, do: [{spawn: {flow: leaf}}]}}]
  - {id: leaf, module: m, spawned: true, client: [{connect: {port: 2}}]}
"""
    )
    leaf = scenario_graph.parse(doc).flow("leaf")
    assert (leaf.multiplicity.low, leaf.multiplicity.high) == (6, 6)


@pytest.mark.parametrize(
    "text",
    [
        "scenario: s\nflows: []\n",
        "scenario: s\nflows:\n  - {id: a, module: m, after: nope, client: []}\n",
        "scenario: s\nflows:\n  - {id: a, module: m, client: [{spawn: {flow: nope}}]}\n",
        "scenario: s\nflows:\n  - {id: a, module: m, l4: icmp}\n",
        "scenario: s\nflows:\n  - {id: a, module: m}\n  - {id: a, module: m}\n",
        "- not a mapping\n",
    ],
)
def test_a_scenario_that_is_not_a_graph_is_refused(text: str, tmp_path: Path) -> None:
    path = tmp_path / "s.yaml"
    path.write_text(text)
    with pytest.raises(ScenarioError):
        scenario_graph.load(path)


def test_a_missing_or_broken_scenario_file_is_an_error(tmp_path: Path) -> None:
    with pytest.raises(ScenarioError, match="read scenario"):
        scenario_graph.load(tmp_path / "missing.yaml")
    broken = tmp_path / "broken.yaml"
    broken.write_text("scenario: [unclosed\n")
    with pytest.raises(ScenarioError, match="parse scenario"):
        scenario_graph.load(broken)


# --- labels ----------------------------------------------------------------------------


@pytest.mark.parametrize(
    "section",
    [
        {},
        {"scenarios": []},
        {"scenarios": [{}]},
        {"scenarios": [{"file": "a.yaml", "bogus": 1}]},
        {"scenarios": [{"file": "a.yaml", "expected_instances": -1}]},
        {"scenarios": [{"file": "a.yaml", "expected_instances": True}]},
        {"scenarios": [{"file": "a.yaml", "vars": []}]},
        {"scenarios": [{"file": "a.yaml", "match": {"bogus": 1}}]},
        {"scenarios": [{"file": "a.yaml"}], "max_gap_s": 0},
        {"scenarios": [{"file": "a.yaml"}], "tolerance_s": -1},
        {"scenarios": [{"file": "a.yaml"}], "extra": 1},
    ],
)
def test_a_bad_structure_section_is_refused(section: Any) -> None:
    with pytest.raises(LabelsError):
        parse(_labels_doc(section))


def test_structure_section_matches_the_labels_schema() -> None:
    schema = json.loads(LABELS_SCHEMA_PATH.read_text())
    doc = _labels_doc(
        {
            "scenarios": [
                {
                    "file": "a.yaml",
                    "vars": {"p": 1},
                    "match": {"client_ip": "10.0.0.1"},
                    "expected_instances": 2,
                }
            ],
            "max_gap_s": 5,
            "tolerance_s": 0.01,
        }
    )
    jsonschema.validate(doc, schema)
    parse(doc)
    with pytest.raises(jsonschema.ValidationError):
        jsonschema.validate(_labels_doc({"scenarios": [{"file": "a", "x": 1}]}), schema)


def test_the_example_labels_file_loads() -> None:
    labels = load(EXAMPLES / "web_browse.labels.yaml")
    assert labels.structure is not None
    assert labels.structure.scenarios[0].file == "web_browse.scenario.yaml"


# --- the report and the command line ---------------------------------------------------


_STUB = StubClassifier.by_server_port({8080: "HTTP", 53: "DNS"}, name="ndpi")


@pytest.fixture
def reader(fake_reader: Callable[..., Path]) -> Path:
    return fake_reader(NDPI_42_HEADER + "\n")


def _score(tmp_path: Path, cap: pcapgen.Capture, **kwargs: Any) -> dict[str, Any]:
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    report = score(pcap, EXAMPLES / "web_browse.labels.yaml", _STUB, **kwargs)
    return report.to_dict()


def test_report_entry_validates_against_the_schema(tmp_path: Path) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    good = pcapgen.Capture()
    _browse(good)
    report = _score(tmp_path, good)
    jsonschema.validate(report, schema)
    entry = next(c for c in report["checks"] if c["check"] == "session_structure")
    assert entry["passed"] is True
    assert entry["instances"] == 1
    assert entry["failed_instances"] == 0
    assert report["version"] == "0.15.0"

    bad = pcapgen.Capture()
    bad.http_get((CLIENT, 40001), WEB)
    bad.clock += 0.02
    bad.dns_lookup((CLIENT, 40000), DNS, 1)
    report = _score(tmp_path, bad)
    jsonschema.validate(report, schema)
    entry = next(c for c in report["checks"] if c["check"] == "session_structure")
    assert entry["passed"] is False
    assert entry["failed_instance_details"][0]["connections"][0]["flow"] == "page"
    assert "edge after lookup -> page violated" in entry["failures"][0]
    assert report["passed"] is False


def test_structure_check_is_opt_in(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    _browse(cap)
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    labels = tmp_path / "l.json"
    labels.write_text(json.dumps(_labels_doc()))
    report = score(pcap, labels, _STUB).to_dict()
    assert [c["check"] for c in report["checks"]] == ["conformance"]
    report = score(pcap, labels, _STUB, scenario_files=[WEB_BROWSE]).to_dict()
    assert [c["check"] for c in report["checks"]] == ["conformance", "session_structure"]


def test_cli_scenario_flag_runs_the_check(tmp_path: Path, reader: Path) -> None:
    labels = tmp_path / "labels.json"
    labels.write_text(
        json.dumps(
            {
                **_labels_doc(),
                "sessions": [
                    {"application": "http1", "match": {"transport": "tcp", "server_port": 8080}},
                    {"application": "dns", "match": {"transport": "udp", "server_port": 53}},
                ],
            }
        )
    )
    good = pcapgen.Capture()
    _browse(good)
    bad = pcapgen.Capture()
    bad.http_get((CLIENT, 40001), WEB)
    bad.clock += 0.02
    bad.dns_lookup((CLIENT, 40000), DNS, 1)
    out = tmp_path / "report.json"
    codes = []
    for name, cap in (("good", good), ("bad", bad)):
        pcap = cap.write_pcap(tmp_path / f"{name}.pcap")
        codes.append(
            main(
                [
                    "score",
                    "--pcap",
                    str(pcap),
                    "--labels",
                    str(labels),
                    "--ndpi-reader",
                    str(reader),
                    "--parsers",
                    "none",
                    "--scenario",
                    str(WEB_BROWSE),
                    "--threshold",
                    "0",
                    "--output",
                    str(out),
                ]
            )
        )
        checks = {c["check"]: c for c in json.loads(out.read_text())["checks"]}
        assert checks["session_structure"]["passed"] is (name == "good")
    assert codes[1] == EXIT_FAIL


def test_cli_reports_a_missing_scenario_file_as_an_input_error(
    tmp_path: Path, reader: Path
) -> None:
    cap = pcapgen.Capture()
    _browse(cap)
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(_labels_doc()))
    code = main(
        [
            "score",
            "--pcap",
            str(pcap),
            "--labels",
            str(labels),
            "--ndpi-reader",
            str(reader),
            "--parsers",
            "none",
            "--scenario",
            str(tmp_path / "nope.yaml"),
        ]
    )
    assert code == 2
