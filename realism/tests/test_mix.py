# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the mix accuracy check: shares, tolerance, bases, inputs, report, and command line.

Every capture is built in code with pcapgen. The classifier is a stub, a fake ndpiReader that
prints a CSV, or the real ndpiReader when it is installed.
"""

from __future__ import annotations

import json
import os
import shutil
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest
import yaml

from realism import mix
from realism.capture import Session, summarize
from realism.classifier import NdpiClassifier, StubClassifier
from realism.cli import EXIT_ERROR, EXIT_FAIL, EXIT_PASS, main
from realism.labels import LabelsError, parse
from realism.report import LABELS_SCHEMA_PATH, SCHEMA_PATH, score
from realism.tests import pcapgen
from realism.tests.conftest import NDPI_42_HEADER

CLIENT = "10.0.0.10"
WEB = ("10.0.1.80", 80)
DNS = ("10.0.1.53", 53)
STUB = StubClassifier.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi")
NDPI_READER = os.environ.get("FPS_NDPI_READER") or shutil.which("ndpiReader")

SCENARIOS = {"http1/web_get": "http1", "dns/lookup": "dns"}


def _labels(**section: Any) -> dict[str, Any]:
    return {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [
            {"application": "http1", "match": {"transport": "tcp", "server_port": 80}},
            {"application": "dns", "match": {"transport": "udp", "server_port": 53}},
        ],
        "mix": {"scenarios": dict(SCENARIOS), **section},
    }


def _capture(web: int, dns: int, body: int = 29) -> pcapgen.Capture:
    """Build ``web`` HTTP sessions with a response of ``body`` bytes and ``dns`` lookups."""
    cap = pcapgen.Capture()
    for i in range(web):
        cap.tcp_session((CLIENT, 40000 + i), WEB, b"GET / HTTP/1.1\r\nHost: a\r\n\r\n", b"x" * body)
    for i in range(dns):
        cap.dns_lookup((CLIENT, 50000 + i), DNS, 0x2000 + i)
    return cap


def _table(
    web: int | None, dns: int | None, web_bytes: int = 0, dns_bytes: int = 0
) -> dict[str, Any]:
    rows = []
    if web is not None:
        rows.append({"scenario": "http1/web_get", "sessions": web, "bytes": web_bytes})
    if dns is not None:
        rows.append({"scenario": "dns/lookup", "sessions": dns, "bytes": dns_bytes})
    return {"scenarios": rows}


def _config(path: Path, web: float, dns: float, basis: str = "sessions") -> Path:
    doc = {
        "profile": {
            "basis": basis,
            "mix": [
                {"scenario": "http1/web_get", "weight": web},
                {"scenario": "dns/lookup", "weight": dns},
            ],
        }
    }
    path.write_text(json.dumps(doc))
    return path


class Run:
    """The files of one check run in a temporary directory."""

    def __init__(self, tmp_path: Path) -> None:
        self.dir = tmp_path

    def check(
        self,
        cap: pcapgen.Capture,
        report: dict[str, Any],
        weights: tuple[float, float] = (60, 40),
        basis: str = "sessions",
        labels: dict[str, Any] | None = None,
        classifier: Any = STUB,
        **kwargs: Any,
    ) -> mix.MixResult:
        pcap = cap.write_pcap(self.dir / "c.pcap")
        run = self.dir / "run.json"
        run.write_text(json.dumps(report))
        doc = labels or _labels()
        return mix.check(
            pcap,
            summarize(pcap).sessions,
            parse(doc),
            classifier,
            config_path=_config(self.dir / "test.json", *weights, basis),
            run_report=run,
            **kwargs,
        )


@pytest.fixture
def run(tmp_path: Path) -> Run:
    return Run(tmp_path)


def _row(result: mix.MixResult, application: str, basis: int = 0) -> dict[str, Any]:
    entry = result.to_dict()["bases"][basis]
    return next(r for r in entry["applications"] if r["application"] == application)


# --- shares and tolerance -------------------------------------------------------------


def test_a_run_that_matches_the_weights_passes(run: Run) -> None:
    result = run.check(_capture(60, 40), _table(60, 40))
    assert result.passed
    row = _row(result, "http1")
    assert (row["configured_pct"], row["metrics_pct"], row["ndpi_pct"]) == (60.0, 60.0, 60.0)
    assert (row["metrics_delta_pp"], row["ndpi_delta_pp"]) == (0.0, 0.0)
    assert result.failures == []


def test_a_metrics_share_off_by_3_points_fails_with_the_reason(run: Run) -> None:
    result = run.check(_capture(60, 40), _table(57, 43))
    assert not result.passed
    row = _row(result, "http1")
    assert row["metrics_passed"] is False and row["ndpi_passed"] is True
    assert row["metrics_delta_pp"] == -3.0 and row["ndpi_delta_pp"] == 0.0
    assert row["reasons"] == [
        "http1: the sessions share from the run metrics is 57.00% (57 sessions), 3.00 points "
        "below the configured 60.00% (tolerance 2.00 points)"
    ]
    assert not _row(result, "dns")["passed"]  # the other side of the same error
    assert result.failures[0].startswith("sessions basis, http1: ")


def test_an_ndpi_share_off_by_3_points_fails_with_the_reason(run: Run) -> None:
    result = run.check(_capture(63, 37), _table(60, 40))
    assert not result.passed
    row = _row(result, "http1")
    assert row["metrics_passed"] is True and row["ndpi_passed"] is False
    assert row["ndpi_delta_pp"] == 3.0
    assert (
        "from nDPI is 63.00% (63 sessions), 3.00 points above the configured 60.00%"
        in (row["reasons"][0])
    )


@pytest.mark.parametrize("web", [58, 59, 61, 62])
def test_a_share_within_the_tolerance_passes_and_two_points_is_inclusive(
    run: Run, web: int
) -> None:
    result = run.check(_capture(web, 100 - web), _table(web, 100 - web))
    assert result.passed, result.failures


def test_a_tolerance_is_configurable(run: Run) -> None:
    cap, report = _capture(57, 43), _table(57, 43)
    assert not run.check(cap, report).passed
    assert run.check(cap, report, tolerance_points=3).passed
    assert not run.check(cap, report, tolerance_points=2.99).passed
    labels = _labels(tolerance_points=4)
    assert run.check(cap, report, labels=labels).passed
    assert not run.check(cap, report, labels=labels, tolerance_points=1).passed


def test_weights_need_no_normalisation(run: Run) -> None:
    assert run.check(_capture(60, 40), _table(60, 40), weights=(3, 2)).passed
    assert not run.check(_capture(60, 40), _table(60, 40), weights=(1, 1)).passed


def test_scenarios_of_one_application_form_one_row(run: Run, tmp_path: Path) -> None:
    labels = _labels()
    labels["mix"]["scenarios"] = {**SCENARIOS, "http1/web_browse": "http1"}
    report = {
        "scenarios": [
            {"scenario": "http1/web_get", "sessions": 30},
            {"scenario": "http1/web_browse", "sessions": 30},
            {"scenario": "dns/lookup", "sessions": 40},
        ]
    }
    pcap = _capture(60, 40).write_pcap(tmp_path / "c.pcap")
    (tmp_path / "run.json").write_text(json.dumps(report))
    config = tmp_path / "test.json"
    config.write_text(
        json.dumps(
            {
                "profile": {
                    "basis": "sessions",
                    "mix": [
                        {"scenario": "http1/web_get", "weight": 20},
                        {"scenario": "http1/web_browse", "weight": 20, "params": {"a": 1}},
                        {"scenario": "http1/web_get", "weight": 20, "params": {"b": 1}},
                        {"scenario": "dns/lookup", "weight": 40},
                    ],
                }
            }
        )
    )
    result = mix.check(
        pcap,
        summarize(pcap).sessions,
        parse(labels),
        STUB,
        config_path=config,
        run_report=tmp_path / "run.json",
    )
    assert result.passed, result.failures
    entry = result.to_dict()
    assert [r["application"] for r in entry["bases"][0]["applications"]] == ["http1", "dns"]
    assert entry["bases"][0]["applications"][0]["scenarios"] == [
        "http1/web_get",
        "http1/web_browse",
    ]


def test_bare_scenario_names_match_module_scenarios(run: Run) -> None:
    labels = _labels()
    labels["mix"]["scenarios"] = {"web_get": "http1", "lookup": "dns"}
    assert run.check(_capture(60, 40), _table(60, 40), labels=labels).passed


# --- nDPI side and unmatched traffic ---------------------------------------------------


def test_sessions_that_ndpi_names_as_no_mix_application_fail_as_unmatched(run: Run) -> None:
    names = {80: "HTTP", 53: "DNS"}
    cap = _capture(60, 40)
    # Five HTTP sessions that nDPI cannot name: 55 HTTP and 5 Unknown of 100.
    bad = StubClassifier(
        lambda s: None if s.client_port in range(40000, 40005) else names[s.server_port],
        name="ndpi",
    )
    result = run.check(cap, _table(60, 40), classifier=bad)
    assert not result.passed
    entry = result.to_dict()["bases"][0]
    assert entry["unmatched"]["ndpi_pct"] == 5.0
    assert entry["unmatched"]["observed"] == {"Unknown": 5}
    assert any("(unmatched)" in f and "nDPI named them Unknown 5" in f for f in entry["failures"])
    assert _row(result, "http1")["ndpi_delta_pp"] == -5.0
    # With 2 or fewer of 100 the same fault is within the tolerance.
    few = StubClassifier(
        lambda s: None if s.client_port in (40000, 40001) else names[s.server_port], name="ndpi"
    )
    assert run.check(cap, _table(60, 40), classifier=few).passed


def test_unlabelled_sessions_stay_out_of_the_nDPI_shares(run: Run) -> None:
    cap = _capture(60, 40)
    cap.tcp_session((CLIENT, 41000), ("10.0.9.9", 9999), b"noise", b"noise")
    result = run.check(cap, _table(60, 40))
    assert result.passed
    assert result.to_dict()["unlabelled_sessions"] == 1


def test_without_rules_every_session_counts(run: Run) -> None:
    labels = _labels()
    del labels["sessions"]
    labels["applications"] = {"http1": {"ndpi": ["HTTP"]}, "dns": {"ndpi": ["DNS"]}}
    cap = _capture(60, 40)
    assert run.check(cap, _table(60, 40), labels=labels).passed
    cap.tcp_session((CLIENT, 41000), ("10.0.9.9", 9999), b"noise", b"noise")
    result = run.check(cap, _table(60, 40), labels=labels, tolerance_points=0.5)
    assert not result.passed
    assert result.to_dict()["bases"][0]["unmatched"]["ndpi_count"] == 1


def test_the_ndpi_names_come_from_the_conformance_names_or_an_override(run: Run) -> None:
    labels = _labels()
    labels["applications"] = {"http1": {"ndpi": ["HTTP", "HTTP_Proxy"]}}
    entry = run.check(_capture(60, 40), _table(60, 40), labels=labels).to_dict()
    assert entry["mapping"][0] == {
        "application": "http1",
        "scenarios": ["http1/web_get"],
        "ndpi": ["HTTP", "HTTP_Proxy"],
    }
    labels = _labels()
    labels["mix"]["scenarios"]["dns/lookup"] = {"application": "dns", "ndpi": ["MDNS"]}
    result = run.check(_capture(60, 40), _table(60, 40), labels=labels)
    assert not result.passed  # nDPI says DNS and the labels accept MDNS only
    assert result.to_dict()["mapping"][1]["ndpi"] == ["MDNS"]


def test_an_application_without_ndpi_names_is_refused(run: Run) -> None:
    labels = _labels()
    labels["mix"]["scenarios"]["dns/lookup"] = "mystery"
    with pytest.raises(mix.MixError, match="application mystery has no ndpi protocol names"):
        run.check(_capture(60, 40), _table(60, 40), labels=labels)


def test_two_applications_that_accept_one_name_are_refused(run: Run) -> None:
    labels = _labels()
    labels["applications"] = {"dns": {"ndpi": ["http"]}}
    with pytest.raises(mix.MixError, match="both accept the ndpi name"):
        run.check(_capture(60, 40), _table(60, 40), labels=labels)


# --- run metrics inputs ----------------------------------------------------------------


def test_a_scenario_that_the_mix_does_not_list_fails(run: Run) -> None:
    report = {"scenarios": [*_table(60, 40)["scenarios"], {"scenario": "ftp/get", "sessions": 3}]}
    result = run.check(_capture(60, 40), report)
    assert not result.passed
    assert any("scenario ftp/get: the run report counts 3 sessions" in f for f in result.failures)


def test_a_configured_scenario_that_the_run_lacks_counts_zero(run: Run) -> None:
    result = run.check(_capture(60, 40), _table(100, None))
    row = _row(result, "dns")
    assert row["metrics_count"] == 0 and row["metrics_pct"] == 0.0
    assert row["metrics_delta_pp"] == -40.0 and not row["passed"]


def test_a_run_without_sessions_fails(run: Run) -> None:
    result = run.check(_capture(60, 40), _table(0, 0))
    assert not result.passed
    assert result.failures[0] == "sessions basis: the run report counts no sessions"
    assert _row(result, "http1")["metrics_pct"] is None


def test_a_metric_series_set_gives_the_last_sample_of_each_scenario(run: Run) -> None:
    def sample(ts: str, key: str, sessions: int, tx: int = 0, rx: int = 0) -> dict[str, Any]:
        values = {"sessions.attempted": sessions, "bytes.tx": tx, "bytes.rx": rx}
        return {"ts": ts, "view": "scenario", "key": key, "values": values}

    series = {
        "view": "scenario",
        "samples": [
            sample("2026-11-03T12:00:09Z", "http1/web_get", 50),
            sample("2026-11-03T12:00:10Z", "*", 100),
            sample("2026-11-03T12:00:10Z", "http1/web_get", 60),
            sample("2026-11-03T12:00:10Z", "dns/lookup", 40),
            sample("2026-11-03T12:00:09Z", "dns/lookup", 35),
        ],
    }
    for report, form in (
        (series, "metric_series"),
        ({"run": {"id": "r"}, "metrics": series}, "bundle"),
    ):
        result = run.check(_capture(60, 40), report)
        assert result.passed, result.failures
        assert result.to_dict()["run_report"] == {"file": "run.json", "form": form}
        assert _row(result, "http1")["metrics_count"] == 60


def test_the_run_report_of_the_rest_api_alone_is_explained(run: Run) -> None:
    report = {"realism": [], "run": {"id": "r", "summary": {"sessions_attempted": 100}}}
    with pytest.raises(mix.MixError, match="holds only the totals of the run"):
        run.check(_capture(60, 40), report)


def test_a_series_of_another_view_is_refused(run: Run) -> None:
    with pytest.raises(mix.MixError, match="needs the scenario view"):
        run.check(_capture(60, 40), {"view": "application", "samples": []})


@pytest.mark.parametrize(
    ("report", "message"),
    [
        ({"scenarios": [{"scenario": "dns/lookup", "sessions": -1}]}, "whole number"),
        ({"scenarios": [{"scenario": "dns/lookup"}]}, "whole number"),
        ({"scenarios": [{"sessions": 1}]}, "needs a scenario name"),
        (
            {"scenarios": [{"scenario": "a", "sessions": 1}, {"scenario": "a", "sessions": 1}]},
            "listed twice",
        ),
        ({"view": "scenario", "samples": [{"key": "a/b", "values": {}}]}, "no sessions.attempted"),
        ([], "must be an object"),
    ],
)
def test_malformed_run_reports_name_the_fault(run: Run, report: Any, message: str) -> None:
    with pytest.raises(mix.MixError, match=message):
        run.check(_capture(60, 40), report)


# --- byte basis ------------------------------------------------------------------------


def _bytes_of(cap: pcapgen.Capture, tmp_path: Path) -> dict[int, int]:
    pcap = cap.write_pcap(tmp_path / "probe.pcap")
    sessions: list[Session] = summarize(pcap).sessions
    out = {80: 0, 53: 0}
    for s in sessions:
        out[s.server_port] += s.payload_bytes
    return out


def test_the_byte_basis_compares_payload_bytes(run: Run, tmp_path: Path) -> None:
    cap = _capture(10, 90, body=4000)
    held = _bytes_of(cap, tmp_path)
    total = held[80] + held[53]
    web_share = held[80] / total * 100
    assert 80 < web_share < 99  # a few big sessions carry most of the bytes
    weights = (web_share, 100 - web_share)
    report = _table(10, 90, web_bytes=held[80], dns_bytes=held[53])
    result = run.check(cap, report, weights=weights, basis="bytes")
    assert result.passed, result.failures
    entry = result.to_dict()
    assert entry["basis_source"] == "test configuration"
    assert [b["basis"] for b in entry["bases"]] == ["bytes"]
    assert _row(result, "http1")["ndpi_count"] == held[80]
    # The same capture by sessions is 10/90 and far from the byte weights.
    assert not run.check(cap, report, weights=weights, basis="sessions").passed


def test_the_byte_basis_catches_a_3_point_error_that_sessions_do_not_show(
    run: Run, tmp_path: Path
) -> None:
    cap = _capture(50, 50, body=900)
    held = _bytes_of(cap, tmp_path)
    total = held[80] + held[53]
    shifted = int(total * 0.03)
    report = _table(50, 50, web_bytes=held[80] - shifted, dns_bytes=held[53] + shifted)
    weights = (held[80] / total * 100, held[53] / total * 100)
    byte_result = run.check(cap, report, weights=weights, basis="bytes")
    assert not byte_result.passed
    assert _row(byte_result, "http1")["metrics_delta_pp"] == pytest.approx(-3.0, abs=0.01)
    assert _row(byte_result, "http1")["ndpi_passed"] is True


def test_both_bases_report_each_and_fail_if_either_fails(run: Run, tmp_path: Path) -> None:
    cap = _capture(60, 40, body=500)
    held = _bytes_of(cap, tmp_path)
    report = _table(60, 40, web_bytes=held[80], dns_bytes=held[53])
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    run_file = tmp_path / "run.json"
    run_file.write_text(json.dumps(report))
    result = mix.check(
        pcap,
        summarize(pcap).sessions,
        parse(_labels(basis="both")),
        STUB,
        config_path=_config(tmp_path / "test.json", 60, 40, "sessions"),
        run_report=run_file,
    )
    entry = result.to_dict()
    assert entry["basis_source"] == "labels"
    assert [b["basis"] for b in entry["bases"]] == ["sessions", "bytes"]
    assert entry["bases"][0]["passed"] is True
    assert entry["bases"][1]["passed"] is False  # the weights are session shares, not byte shares
    assert not result.passed
    forced = mix.check(
        pcap,
        summarize(pcap).sessions,
        parse(_labels(basis="bytes")),
        STUB,
        config_path=_config(tmp_path / "test.json", 60, 40, "sessions"),
        run_report=run_file,
        basis="sessions",
    )
    assert forced.passed and forced.to_dict()["basis_source"] == "command line"


def test_the_byte_basis_needs_bytes_in_the_run_report(run: Run) -> None:
    report = {"scenarios": [{"scenario": "http1/web_get", "sessions": 60}]}
    with pytest.raises(mix.MixError, match=r"has no bytes\.tx and bytes\.rx"):
        run.check(_capture(60, 40), report, basis="bytes")


# --- configuration inputs --------------------------------------------------------------


def test_weights_in_the_labels_replace_a_test_configuration(tmp_path: Path) -> None:
    pcap = _capture(60, 40).write_pcap(tmp_path / "c.pcap")
    (tmp_path / "run.json").write_text(json.dumps(_table(60, 40)))
    labels = parse(
        _labels(
            weights={"http1/web_get": 3, "dns/lookup": 2}, basis="sessions", run_report="run.json"
        )
    )
    labels.base_dir = tmp_path
    result = mix.check(pcap, summarize(pcap).sessions, labels, STUB)
    assert result.passed and result.to_dict()["test_config"] is None


def test_a_yaml_test_configuration_reads_the_example_shape(tmp_path: Path) -> None:
    path = tmp_path / "t.yaml"
    path.write_text(
        yaml.safe_dump(
            {
                "profile": {
                    "basis": "bytes",
                    "mix": [
                        {"scenario": "http1/web_browse", "weight": 60},
                        {"scenario": "dns/lookup", "weight": 0.1},
                    ],
                }
            }
        )
    )
    config = mix.load_config(path)
    assert config.basis == "bytes" and config.weights == {"http1/web_browse": 60, "dns/lookup": 0.1}


@pytest.mark.parametrize(
    "doc",
    [
        {},
        {"profile": {"basis": "packets", "mix": [{"scenario": "a", "weight": 1}]}},
        {"profile": {"basis": "bytes", "mix": []}},
        {"profile": {"basis": "bytes", "mix": [{"scenario": "a", "weight": 0}]}},
        {"profile": {"basis": "bytes", "mix": [{"scenario": "a", "weight": True}]}},
    ],
)
def test_invalid_test_configurations_are_refused(tmp_path: Path, doc: dict[str, Any]) -> None:
    path = tmp_path / "t.json"
    path.write_text(json.dumps(doc))
    with pytest.raises(mix.MixError, match="test configuration"):
        mix.load_config(path)


def test_a_scenario_without_a_mapping_is_refused(run: Run) -> None:
    labels = _labels()
    del labels["mix"]["scenarios"]["dns/lookup"]
    with pytest.raises(mix.MixError, match="scenario dns/lookup of the mix has no entry"):
        run.check(_capture(60, 40), _table(60, 40), labels=labels)


def test_missing_inputs_are_named(tmp_path: Path) -> None:
    pcap = _capture(6, 4).write_pcap(tmp_path / "c.pcap")
    sessions = summarize(pcap).sessions
    labels = parse(_labels())
    with pytest.raises(mix.MixError, match="no configured weights"):
        mix.check(pcap, sessions, labels, STUB, run_report=tmp_path / "r.json")
    with pytest.raises(mix.MixError, match="no run report"):
        mix.check(pcap, sessions, labels, STUB, config_path=_config(tmp_path / "t.json", 6, 4))
    with pytest.raises(mix.MixError, match="read run report"):
        mix.check(
            pcap,
            sessions,
            labels,
            STUB,
            config_path=_config(tmp_path / "t.json", 6, 4),
            run_report=tmp_path / "absent.json",
        )
    (tmp_path / "r.json").write_text(json.dumps(_table(6, 4)))
    no_map = parse({"labels": "fps-realism-labels", "version": 1})
    with pytest.raises(mix.MixError, match=r"no mix\.scenarios"):
        mix.check(
            pcap,
            sessions,
            no_map,
            STUB,
            config_path=_config(tmp_path / "t.json", 6, 4),
            run_report=tmp_path / "r.json",
        )
    with pytest.raises(mix.MixError, match="tolerance"):
        mix.check(
            pcap,
            sessions,
            labels,
            STUB,
            config_path=_config(tmp_path / "t.json", 6, 4),
            run_report=tmp_path / "r.json",
            tolerance_points=101,
        )


# --- labels ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("section", "message"),
    [
        ({"scenarios": {}}, "non empty mapping"),
        ({"scenarios": {"a": 3}}, "scenarios.a"),
        ({"scenarios": {"a": {"application": "x", "extra": 1}}}, "scenarios.a"),
        ({"scenarios": {"a": {"application": "x", "ndpi": []}}}, "ndpi"),
        ({"scenarios": {"a": "x"}, "basis": "packets"}, "basis"),
        ({"scenarios": {"a": "x"}, "tolerance_points": -1}, "tolerance_points"),
        ({"scenarios": {"a": "x"}, "tolerance_points": True}, "tolerance_points"),
        ({"scenarios": {"a": "x"}, "weights": {"a": 0}, "basis": "bytes"}, "weights"),
        ({"scenarios": {"a": "x"}, "weights": {"a": 1}}, "basis"),
        (
            {"scenarios": {"a": "x"}, "weights": {"a": 1}, "test_config": "t", "basis": "bytes"},
            "not both",
        ),
        ({"scenarios": {"a": "x"}, "extra": 1}, "only"),
    ],
)
def test_invalid_mix_sections_are_refused(section: dict[str, Any], message: str) -> None:
    doc = {"labels": "fps-realism-labels", "version": 1, "mix": section}
    with pytest.raises(LabelsError, match=message):
        parse(doc)
    schema = json.loads(LABELS_SCHEMA_PATH.read_text())
    with pytest.raises(jsonschema.ValidationError):
        jsonschema.validate(doc, schema)


def test_the_labels_schema_accepts_a_mix_section() -> None:
    doc = _labels(basis="both", tolerance_points=1.5, run_report="r.json", test_config="t.yaml")
    doc["mix"]["scenarios"]["ftp/get"] = {"application": "ftp", "ndpi": ["FTP_CONTROL"]}
    jsonschema.validate(doc, json.loads(LABELS_SCHEMA_PATH.read_text()))
    parse(doc)


# --- report and command line -----------------------------------------------------------


def test_the_report_entry_validates_against_the_schema(tmp_path: Path) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    pcap = _capture(60, 40).write_pcap(tmp_path / "c.pcap")
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(_labels(basis="sessions")))
    config = _config(tmp_path / "t.json", 60, 40)
    for web, expected in ((60, True), (57, False)):
        (tmp_path / "run.json").write_text(json.dumps(_table(web, 100 - web, 6000, 4000)))
        report = score(
            pcap, labels, STUB, test_config=config, run_report=tmp_path / "run.json"
        ).to_dict()
        jsonschema.validate(report, schema)
        entry = next(c for c in report["checks"] if c["check"] == "mix_accuracy")
        assert report["version"] == "0.20.0"
        assert entry["passed"] is expected
        assert {m["name"] for m in entry["metrics"]} == {
            "mix.share_pct",
            "mix.abs_error_pp",
            "mix.max_abs_error_pp",
            "realism.check_failures",
        }
    assert report["passed"] is False


def _flows_csv(pcap: Path) -> str:
    """Write the flow CSV that a nDPI run would print for the sessions of a capture."""
    rows = [NDPI_42_HEADER]
    for index, s in enumerate(summarize(pcap).sessions):
        proto, name = ("6", "HTTP") if s.server_port == 80 else ("17", "DNS")
        rows.append(
            f"{index},{proto},1,1,0,{s.client_ip},{s.client_port},{s.server_ip},{s.server_port},"
            f"7,{name},,6,414"
        )
    return "\n".join(rows) + "\n"


def _score_argv(tmp_path: Path, pcap: Path, labels: Path, reader: Path, *extra: str) -> list[str]:
    return [
        "score",
        "--pcap",
        str(pcap),
        "--labels",
        str(labels),
        "--parsers",
        "none",
        "--ndpi-reader",
        str(reader),
        "--output",
        str(tmp_path / "report.json"),
        *extra,
    ]


def test_the_command_line_runs_the_check_and_sets_the_exit_code(
    tmp_path: Path, fake_reader: Callable[..., Path], capsys: pytest.CaptureFixture[str]
) -> None:
    pcap = _capture(60, 40).write_pcap(tmp_path / "c.pcap")
    reader = fake_reader(_flows_csv(pcap))
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(_labels()))
    config = tmp_path / "test.yaml"
    config.write_text(
        yaml.safe_dump(
            {
                "profile": {
                    "basis": "sessions",
                    "mix": [
                        {"scenario": "http1/web_get", "weight": 60},
                        {"scenario": "dns/lookup", "weight": 40},
                    ],
                }
            }
        )
    )
    good, bad = tmp_path / "good.json", tmp_path / "bad.json"
    good.write_text(json.dumps(_table(60, 40)))
    bad.write_text(json.dumps(_table(57, 43)))

    argv = _score_argv(tmp_path, pcap, labels, reader, "--test-config", str(config))
    assert main([*argv, "--run-report", str(good)]) == EXIT_PASS
    report = json.loads((tmp_path / "report.json").read_text())
    entry = next(c for c in report["checks"] if c["check"] == "mix_accuracy")
    assert entry["classifier_version"] == "9.8.7-fake"
    assert entry["bases"][0]["applications"][0]["ndpi_pct"] == 60.0

    assert main([*argv, "--run-report", str(bad)]) == EXIT_FAIL
    report = json.loads((tmp_path / "report.json").read_text())
    entry = next(c for c in report["checks"] if c["check"] == "mix_accuracy")
    assert entry["passed"] is False and "3.00 points below" in entry["failures"][0]

    assert main([*argv, "--run-report", str(bad), "--mix-tolerance", "3"]) == EXIT_PASS
    assert main([*argv, "--run-report", str(good), "--mix-basis", "both"]) == EXIT_FAIL
    capsys.readouterr()
    assert main([*argv, "--run-report", str(tmp_path / "absent.json")]) == EXIT_ERROR
    assert "read run report" in capsys.readouterr().err
    assert (
        main(_score_argv(tmp_path, pcap, labels, reader, "--run-report", str(good))) == EXIT_ERROR
    )
    assert "no configured weights" in capsys.readouterr().err


def test_the_check_runs_the_classifier_once(tmp_path: Path) -> None:
    calls = 0

    class Counting(StubClassifier):
        def classify(self, capture: Path, sessions: Any) -> Any:
            nonlocal calls
            calls += 1
            return super().classify(capture, sessions)

    pcap = _capture(6, 4).write_pcap(tmp_path / "c.pcap")
    (tmp_path / "run.json").write_text(json.dumps(_table(6, 4)))
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(_labels()))
    report = score(
        pcap,
        labels,
        Counting.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi"),
        test_config=_config(tmp_path / "t.json", 6, 4),
        run_report=tmp_path / "run.json",
    )
    assert report.passed and calls == 1


def test_a_labels_file_without_mix_adds_no_entry(tmp_path: Path) -> None:
    labels = tmp_path / "labels.json"
    doc = _labels()
    del doc["mix"]
    labels.write_text(json.dumps(doc))
    report = score(_capture(6, 4).write_pcap(tmp_path / "c.pcap"), labels, STUB)
    assert [c["check"] for c in report.to_dict()["checks"]] == ["conformance"]


@pytest.mark.skipif(NDPI_READER is None, reason="ndpiReader is not installed")
def test_real_ndpi_gives_the_mix_of_a_synthetic_capture(tmp_path: Path) -> None:
    cap = pcapgen.mixed_capture(http_sessions=4, dns_lookups=1)
    pcap = cap.write_pcap(tmp_path / "mix.pcap")
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(_labels(basis="sessions")))
    config = _config(tmp_path / "t.json", 80, 20)
    classifier = NdpiClassifier(reader=NDPI_READER)
    for web, expected in ((80, True), (70, False)):
        (tmp_path / "run.json").write_text(json.dumps(_table(web, 100 - web)))
        report = score(
            pcap, labels, classifier, test_config=config, run_report=tmp_path / "run.json"
        ).to_dict()
        entry = next(c for c in report["checks"] if c["check"] == "mix_accuracy")
        row = entry["bases"][0]["applications"][0]
        assert row["ndpi_pct"] == 80.0 and row["ndpi_passed"] is True
        assert entry["passed"] is expected


# --- metric series and the REST report ------------------------------------------------

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "mix"
CATALOG = Path(__file__).resolve().parents[2] / "contracts" / "metrics" / "catalog.yaml"
CLASSIFIER_LABELS = {"source": "classifier", "classifier": "ndpi"}


def _entry_of(report: dict[str, Any]) -> dict[str, Any]:
    jsonschema.validate(report, json.loads(SCHEMA_PATH.read_text()))
    return next(c for c in report["checks"] if c["check"] == "mix_accuracy")


def _fixture_entry(run_report: str, **kwargs: Any) -> dict[str, Any]:
    """Score the committed capture (8 HTTP sessions, 2 lookups) and return the mix entry."""
    config = kwargs.pop("test_config", FIXTURE / "test.yaml")
    report = score(
        FIXTURE / "mix.pcapng",
        FIXTURE / "labels.yaml",
        STUB,
        test_config=config,
        run_report=FIXTURE / run_report,
        **kwargs,
    )
    return _entry_of(report.to_dict())


def _series(name: str, value: float, **labels: str) -> dict[str, Any]:
    return {"name": name, "labels": {**labels, **CLASSIFIER_LABELS}, "value": value}


def _FAILURES(value: int) -> dict[str, Any]:
    return {"name": "realism.check_failures", "labels": {"check": "mix_accuracy"}, "value": value}


def test_the_fixture_gives_the_literal_series_by_sessions() -> None:
    # Configured 75 and 25 percent, the capture holds 8 and 2 of 10 sessions.
    entry = _fixture_entry("run-report.json")
    assert entry["metrics_basis"] == "sessions"
    assert entry["metrics"] == [
        _series("mix.share_pct", 80.0, application="http1"),
        _series("mix.share_pct", 20.0, application="dns"),
        _series("mix.abs_error_pp", 5.0, application="http1"),
        _series("mix.abs_error_pp", 5.0, application="dns"),
        _series("mix.max_abs_error_pp", 5.0),
        _FAILURES(4),
    ]
    assert entry["failures_total"] == 4
    assert entry["passed"] is False


def test_the_fixture_gives_the_literal_series_by_bytes(tmp_path: Path) -> None:
    # 448 and 164 payload bytes of 612 are 73.202614 and 26.797386 percent.
    config = tmp_path / "test.yaml"
    config.write_text(
        (FIXTURE / "test.yaml").read_text().replace("basis: sessions", "basis: bytes")
    )
    entry = _fixture_entry("run-report.json", test_config=config)
    assert entry["metrics_basis"] == "bytes"
    assert entry["metrics"] == [
        _series("mix.share_pct", 73.202614, application="http1"),
        _series("mix.share_pct", 26.797386, application="dns"),
        _series("mix.abs_error_pp", 1.797386, application="http1"),
        _series("mix.abs_error_pp", 1.797386, application="dns"),
        _series("mix.max_abs_error_pp", 1.797386),
        _FAILURES(0),
    ]
    assert entry["passed"] is True


def test_the_series_validate_against_catalog_0_4_0() -> None:
    catalog = yaml.safe_load(CATALOG.read_text())
    assert catalog["version"] == "0.8.0"
    metrics = {m["name"]: m for m in catalog["metrics"]}
    source_values = set(catalog["labels"]["source"]["values"])
    classifier_values = set(catalog["labels"]["classifier"]["values"])
    entry = _fixture_entry("run-report.json")
    assert entry["metrics"]
    for series in entry["metrics"][:-1]:
        definition = metrics[series["name"]]
        assert set(series["labels"]) == set(definition["labels"])
        assert series["labels"]["source"] in source_values
        assert series["labels"]["classifier"] in classifier_values
        assert series["labels"]["source"] == "classifier"
        assert definition["type"] == "gauge"
        if series["name"] == "mix.share_pct":
            assert 0 <= series["value"] <= 100


def test_the_schema_refuses_series_that_break_the_catalog() -> None:
    entry = _fixture_entry("run-report.json")
    schema = json.loads(SCHEMA_PATH.read_text())
    mix_schema = {"$ref": "#/$defs/mix_accuracy", "$defs": schema["$defs"]}
    jsonschema.validate(entry, mix_schema)
    first: list[Callable[[dict[str, Any]], object]] = [
        lambda m: m["labels"].pop("source"),
        lambda m: m["labels"].update(source="metrics"),
        lambda m: m.update(name="mix.other_pp"),
        lambda m: m["labels"].update(extra="x"),
    ]
    for change in first:
        broken = json.loads(json.dumps(entry))
        change(broken["metrics"][0])
        with pytest.raises(jsonschema.ValidationError):
            jsonschema.validate(broken, mix_schema)
    broken = json.loads(json.dumps(entry))
    broken["metrics"][-1]["labels"]["application"] = "http1"
    with pytest.raises(jsonschema.ValidationError):
        jsonschema.validate(broken, mix_schema)
    broken = json.loads(json.dumps(entry))
    del broken["metrics"][0]["labels"]["application"]
    with pytest.raises(jsonschema.ValidationError):
        jsonschema.validate(broken, mix_schema)


@pytest.mark.skipif(NDPI_READER is None, reason="ndpiReader is not installed")
def test_real_ndpi_gives_the_literal_series_of_the_fixture() -> None:
    report = score(
        FIXTURE / "mix.pcapng",
        FIXTURE / "labels.yaml",
        NdpiClassifier(reader=NDPI_READER),
        test_config=FIXTURE / "test.yaml",
        run_report=FIXTURE / "run-report.json",
    )
    shares = {
        m["labels"]["application"]: m["value"]
        for m in _entry_of(report.to_dict())["metrics"]
        if m["name"] == "mix.share_pct"
    }
    assert shares == {"http1": 80.0, "dns": 20.0}


def test_the_series_do_not_depend_on_the_form_of_the_run_report() -> None:
    rest = _fixture_entry("run-report.json")
    table = _fixture_entry("run-table.json")
    assert rest["metrics"] == table["metrics"]
    assert (rest["run_report"]["form"], table["run_report"]["form"]) == ("rest_report", "table")
    assert rest["bases"] == table["bases"]


def test_weights_in_the_labels_leave_the_basis_of_the_series_to_the_bases(tmp_path: Path) -> None:
    weights = {"http1/web_get": 3, "dns/lookup": 1}
    labels = tmp_path / "labels.json"
    run_report = FIXTURE / "run-report.json"
    labels.write_text(json.dumps(_labels(basis="both", weights=weights)))
    entry = _entry_of(score(FIXTURE / "mix.pcapng", labels, STUB, run_report=run_report).to_dict())
    assert entry["metrics_basis"] is None
    assert entry["metrics"] == [_FAILURES(entry["failures_total"])]
    labels.write_text(json.dumps(_labels(basis="bytes", weights=weights)))
    entry = _entry_of(score(FIXTURE / "mix.pcapng", labels, STUB, run_report=run_report).to_dict())
    assert entry["metrics_basis"] == "bytes"
    assert entry["metrics"][0]["value"] == 73.202614


def test_a_capture_without_sessions_has_no_series(run: Run) -> None:
    result = run.check(pcapgen.Capture(), _table(6, 4))
    assert result.classifier_series() == []
    assert result.metrics() == [_FAILURES(len(result.failures))]
    assert result.metrics_basis == "sessions"


def test_unrecognised_traffic_lowers_the_shares_and_is_no_application(run: Run) -> None:
    stub = StubClassifier(lambda s: "HTTP" if s.server_port == 80 else None, name="ndpi")
    result = run.check(_capture(6, 4), _table(6, 4), classifier=stub)
    series = {(m["name"], m["labels"].get("application")): m["value"] for m in result.metrics()}
    assert series[("mix.share_pct", "http1")] == 60.0
    assert series[("mix.share_pct", "dns")] == 0.0
    assert series[("mix.max_abs_error_pp", None)] == 40.0
    assert all(m["labels"].get("application") != mix.UNMATCHED for m in result.metrics())


def test_the_rest_report_gives_the_same_counts_as_the_file_table() -> None:
    rest = mix.load_run_counts(FIXTURE / "run-report.json")
    table = mix.load_run_counts(FIXTURE / "run-table.json")
    assert (rest.form, table.form) == ("rest_report", "table")
    assert (
        rest.scenarios
        == table.scenarios
        == {"http1/web_get": mix.Counts(8, 448), "dns/lookup": mix.Counts(2, 164)}
    )


def _rest(**entry: Any) -> dict[str, Any]:
    base = {"application": "http1", "scenario": "web_get", "sessions_attempted": 5}
    base |= {"sessions_succeeded": 1, "sessions_failed": 4, "bytes_tx": 10, "bytes_rx": 32}
    return {"run": {"id": "r"}, "realism": [], "scenarios": [{**base, **entry}]}


def test_the_rest_report_counts_attempted_sessions_and_both_byte_directions(
    tmp_path: Path,
) -> None:
    path = tmp_path / "report.json"
    path.write_text(json.dumps(_rest()))
    assert mix.load_run_counts(path).scenarios == {"http1/web_get": mix.Counts(5, 42)}


def test_an_empty_scenarios_list_of_the_rest_report_counts_nothing(run: Run) -> None:
    result = run.check(_capture(6, 4), {"run": {"id": "r"}, "realism": [], "scenarios": []})
    assert not result.passed
    assert "the run report counts no sessions" in result.failures[0]
    assert result.to_dict()["run_report"]["form"] == "rest_report"


@pytest.mark.parametrize(
    ("change", "message"),
    [
        ({"application": None}, "application"),
        ({"scenario": ""}, "scenario"),
        ({"sessions_attempted": None}, "sessions_attempted"),
        ({"bytes_tx": None}, "bytes_tx"),
        ({"bytes_rx": None}, "bytes_rx"),
        ({"sessions_attempted": -1}, "sessions_attempted"),
    ],
)
def test_malformed_rest_entries_name_the_fault(
    tmp_path: Path, change: dict[str, Any], message: str
) -> None:
    path = tmp_path / "report.json"
    path.write_text(json.dumps(_rest(**change)))
    with pytest.raises(mix.MixError, match=message):
        mix.load_run_counts(path)


def test_a_scenario_listed_twice_in_the_rest_report_is_refused(tmp_path: Path) -> None:
    doc = _rest()
    doc["scenarios"] *= 2
    path = tmp_path / "report.json"
    path.write_text(json.dumps(doc))
    with pytest.raises(mix.MixError, match="listed twice"):
        mix.load_run_counts(path)


def test_a_rest_report_without_scenarios_is_explained(tmp_path: Path) -> None:
    path = tmp_path / "report.json"
    path.write_text(json.dumps({"run": {"id": "r"}, "realism": []}))
    with pytest.raises(mix.MixError, match=r"before API 0\.9\.0"):
        mix.load_run_counts(path)


def test_the_example_labels_file_loads_and_validates() -> None:
    path = Path(__file__).resolve().parents[1] / "examples" / "mix.labels.yaml"
    doc = yaml.safe_load(path.read_text())
    jsonschema.validate(doc, json.loads(LABELS_SCHEMA_PATH.read_text()))
    labels = parse(doc)
    assert labels.mix is not None and labels.mix.basis == "both"
    assert labels.mix.scenarios["dns/lookup"].application == "dns"
