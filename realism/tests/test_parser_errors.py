# SPDX-License-Identifier: GPL-3.0-only
"""Component tests of the parser error check and its report entry with stub parsers."""

from __future__ import annotations

import json
import sys
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from realism.capture import summarize
from realism.classifier import StubClassifier
from realism.cli import EXIT_ERROR, EXIT_FAIL, EXIT_PASS, main
from realism.labels import parse
from realism.parser_errors import MAX_ERRORS, check
from realism.parsers import KIND_ERROR, KIND_MALFORMED, KIND_WEIRD, Finding, StubParser
from realism.report import SCHEMA_PATH, score
from realism.tests import pcapgen
from realism.tests.conftest import LABELS_HTTP_DNS, NDPI_42_HEADER

DNS_CLIENT = ("10.0.0.10", 50100)
DNS_SERVER = ("10.0.1.53", 53)
CLASSIFIER = StubClassifier.by_server_port({80: "HTTP", 53: "DNS"}, name="ndpi")


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator.check_schema(schema)
    jsonschema.Draft202012Validator(schema).validate(report)


def _malformed_dns_capture(tmp_path: Path) -> Path:
    cap = pcapgen.mixed_capture(http_sessions=2, dns_lookups=2)
    cap.dns_malformed_answer(DNS_CLIENT, DNS_SERVER, 0x2222)
    return cap.write_pcap(tmp_path / "malformed-dns.pcap")


def _reply_finding(parser: str, kind: str = KIND_MALFORMED, **over: Any) -> Finding:
    fields: dict[str, Any] = dict(
        parser=parser,
        kind=kind,
        name="dns",
        message="Malformed Packet (Exception occurred)",
        frame=26,
        timestamp=1.0,
        transport="udp",
        src_ip=DNS_SERVER[0],
        src_port=DNS_SERVER[1],
        dst_ip=DNS_CLIENT[0],
        dst_port=DNS_CLIENT[1],
    )
    fields.update(over)
    return Finding(**fields)


def _parser_entry(report: dict[str, Any]) -> dict[str, Any]:
    (entry,) = [c for c in report["checks"] if c["check"] == "parser_errors"]
    assert isinstance(entry, dict)
    return entry


def test_one_malformed_dns_message_reports_exactly_one_parser_error(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    tshark = StubParser([_reply_finding("tshark")], name="tshark")
    zeek = StubParser(
        [_reply_finding("zeek", KIND_WEIRD, frame=None, name="DNS_truncated_RR_rdlength_lt_len")],
        name="zeek",
    )
    report = score(
        _malformed_dns_capture(tmp_path), labels_file(), CLASSIFIER, parsers=[tshark, zeek]
    ).to_dict()
    _validate(report)
    entry = _parser_entry(report)
    by_app = {a["application"]: a for a in entry["applications"]}
    assert by_app["dns"]["parser_errors"] == 1
    assert by_app["dns"]["malformed_packets"] == 1
    assert by_app["dns"]["sessions"] == 3
    assert by_app["dns"]["by_parser"]["zeek"] == {"parser_errors": 1, "malformed_packets": 0}
    assert by_app["http1"]["parser_errors"] == 0
    assert entry["errors"][0]["session"] == 5
    assert entry["errors"][0]["client"] == {"ip": DNS_CLIENT[0], "port": DNS_CLIENT[1]}
    assert entry["errors_total"] == 2
    assert entry["passed"] is False
    assert report["passed"] is False


def test_a_clean_capture_passes_and_validates(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = pcapgen.mixed_capture().write_pcap(tmp_path / "clean.pcap")
    report = score(pcap, labels_file(), CLASSIFIER, parsers=[StubParser(name="tshark")])
    data = report.to_dict()
    _validate(data)
    entry = _parser_entry(data)
    assert entry["passed"] is True
    assert entry["parsers"] == [{"parser": "tshark", "version": "stub"}]
    assert [a["parser_errors"] for a in entry["applications"]] == [0, 0]
    assert report.passed


def test_no_parser_fails_the_check_and_names_the_missing_ones(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = pcapgen.mixed_capture().write_pcap(tmp_path / "clean.pcap")
    data = score(
        pcap, labels_file(), CLASSIFIER, parsers=[], missing_parsers=["tshark", "zeek"]
    ).to_dict()
    _validate(data)
    entry = _parser_entry(data)
    assert entry["passed"] is False
    assert entry["missing_parsers"] == ["tshark", "zeek"]
    assert entry["skipped_parsers"] == [
        {"parser": "tshark", "reason": "not installed"},
        {"parser": "zeek", "reason": "not installed"},
    ]


def test_a_skip_reason_overrides_the_default(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = pcapgen.mixed_capture().write_pcap(tmp_path / "clean.pcap")
    data = score(
        pcap,
        labels_file(),
        CLASSIFIER,
        parsers=[StubParser(name="tshark")],
        missing_parsers=["zeek"],
        skip_reasons={"zeek": "no program at /opt/zeek/bin/zeek"},
    ).to_dict()
    _validate(data)
    assert _parser_entry(data)["skipped_parsers"] == [
        {"parser": "zeek", "reason": "no program at /opt/zeek/bin/zeek"}
    ]


def test_every_parser_that_ran_leaves_the_skipped_list_empty(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = pcapgen.mixed_capture().write_pcap(tmp_path / "clean.pcap")
    data = score(pcap, labels_file(), CLASSIFIER, parsers=[StubParser(name="tshark")]).to_dict()
    assert _parser_entry(data)["skipped_parsers"] == []


def test_without_parsers_the_report_keeps_only_conformance(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    pcap = pcapgen.mixed_capture().write_pcap(tmp_path / "clean.pcap")
    data = score(pcap, labels_file(), CLASSIFIER).to_dict()
    assert [c["check"] for c in data["checks"]] == ["conformance"]


def test_unlabelled_and_unattributed_errors_fail_the_check(tmp_path: Path) -> None:
    cap = pcapgen.mixed_capture(http_sessions=1, dns_lookups=0)
    cap.dns_lookup(("10.0.0.10", 50000), ("10.0.1.54", 5353), 1)
    pcap = cap.write_pcap(tmp_path / "m.pcap")
    sessions = summarize(pcap).sessions
    findings = [
        _reply_finding("stub", KIND_ERROR, src_ip="10.0.1.54", src_port=5353, dst_port=50000),
        _reply_finding("stub", KIND_ERROR, transport=None),
        _reply_finding("stub", KIND_ERROR, src_ip="192.0.2.1"),
    ]
    http_only = [{"application": "http1", "match": {"transport": "tcp", "server_port": 80}}]
    labels = parse({**LABELS_HTTP_DNS, "sessions": http_only})
    result = check(pcap, sessions, labels, [StubParser(findings)])
    entry = result.to_dict()
    assert entry["unlabelled_errors"] == {"stub": 1}
    assert entry["unattributed_errors"] == {"stub": 2}
    assert entry["applications"][0]["parser_errors"] == 0
    assert (entry["errors"][1]["session"], entry["errors"][1]["transport"]) == (None, None)
    assert entry["errors"][2]["server"] == {"ip": DNS_CLIENT[0], "port": DNS_CLIENT[1]}
    assert not result.passed


def test_error_list_is_capped_but_the_total_counts_all(tmp_path: Path) -> None:
    pcap = _malformed_dns_capture(tmp_path)
    many = StubParser([_reply_finding("stub", frame=i + 1) for i in range(MAX_ERRORS + 5)])
    result = check(pcap, summarize(pcap).sessions, parse(LABELS_HTTP_DNS), [many])
    entry = result.to_dict()
    assert len(entry["errors"]) == MAX_ERRORS
    assert entry["errors_total"] == MAX_ERRORS + 5
    assert {a["application"]: a["parser_errors"] for a in entry["applications"]}["dns"] == 105


def _fake_tshark(tmp_path: Path, stdout: str, decodes: str = "") -> Path:
    script = tmp_path / "fake-tshark"
    script.write_text(
        f"#!{sys.executable}\n"
        "import sys\n"
        "if sys.argv[1:] == ['--version']:\n"
        "    print('TShark (Wireshark) 4.2.2')\n"
        "    sys.exit(0)\n"
        "if sys.argv[1:] == ['-G', 'decodes']:\n"
        f"    sys.stdout.write({decodes!r})\n"
        "    sys.exit(0)\n"
        "assert '-Y' in sys.argv, 'the runner must filter on expert severity'\n"
        f"sys.stdout.write({stdout!r})\n"
    )
    script.chmod(0o755)
    return script


def _fake_ndpi(tmp_path: Path) -> Path:
    csv_file = tmp_path / "flows.csv"
    rows = [NDPI_42_HEADER]
    rows += [f"{i},6,1,1,0,10.0.0.10,{40000 + i},10.0.1.80,80,7,HTTP,,6,414" for i in range(2)]
    rows += [f"{i},17,1,1,0,10.0.0.10,{50000 + i},10.0.1.53,53,5,DNS,,1,60" for i in range(2)]
    rows += ["9,17,1,1,0,10.0.0.10,50100,10.0.1.53,53,5,DNS,,1,60"]
    csv_file.write_text("\n".join(rows) + "\n")
    script = tmp_path / "fake-ndpi"
    script.write_text(
        f"#!{sys.executable}\n"
        "import shutil, sys\n"
        "if sys.argv[1:] == ['-h']:\n"
        "    print('nDPI 4.8')\n"
        "    sys.exit(0)\n"
        f"shutil.copy({str(csv_file)!r}, sys.argv[sys.argv.index('-C') + 1])\n"
    )
    script.chmod(0o755)
    return script


def test_cli_reports_the_malformed_dns_message_through_tshark(
    tmp_path: Path, labels_file: Callable[..., Path]
) -> None:
    from realism.tests.test_parsers import MALFORMED_DNS_LINE

    out = tmp_path / "report.json"
    argv = [
        "score",
        "--pcap",
        str(_malformed_dns_capture(tmp_path)),
        "--labels",
        str(labels_file()),
    ]
    argv += ["--ndpi-reader", str(_fake_ndpi(tmp_path)), "--output", str(out)]
    argv += ["--parsers", "tshark", "--tshark", str(_fake_tshark(tmp_path, MALFORMED_DNS_LINE))]
    assert main(argv) == EXIT_FAIL
    report = json.loads(out.read_text())
    _validate(report)
    assert report["checks"][0]["passed"] is True
    entry = _parser_entry(report)
    by_app = {a["application"]: a["parser_errors"] for a in entry["applications"]}
    assert by_app == {"dns": 1, "http1": 0}

    argv[-1] = str(_fake_tshark(tmp_path, ""))
    assert main(argv) == EXIT_PASS


def test_cli_parser_selection(
    tmp_path: Path,
    labels_file: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    pcap = pcapgen.mixed_capture(2, 2).write_pcap(tmp_path / "clean.pcap")
    base = ["score", "--pcap", str(pcap), "--labels", str(labels_file())]
    base += ["--ndpi-reader", str(_fake_ndpi(tmp_path))]
    absent = str(tmp_path / "absent")
    assert main([*base, "--parsers", "zeek", "--zeek", absent]) == EXIT_ERROR
    assert "zeek program" in capsys.readouterr().err
    assert main([*base, "--tshark", absent, "--zeek", absent]) == EXIT_FAIL
    captured = capsys.readouterr()
    assert "skipped parser tshark" in captured.err
    entry = _parser_entry(json.loads(captured.out))
    assert entry["missing_parsers"] == ["tshark", "zeek"]
    assert [item["parser"] for item in entry["skipped_parsers"]] == ["tshark", "zeek"]
    assert main([*base, "--parsers", "none"]) == EXIT_PASS
    report = json.loads(capsys.readouterr().out)
    assert [c["check"] for c in report["checks"]] == ["conformance"]
    with pytest.raises(SystemExit):
        main([*base, "--parsers", "wireshark"])


def test_auto_with_only_tshark_installed_shows_zeek_as_skipped(
    tmp_path: Path,
    labels_file: Callable[..., Path],
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    """Acceptance of RLM-11: the search path holds tshark and no Zeek."""
    from realism import parsers
    from realism.ci import summary

    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "tshark").symlink_to(_fake_tshark(tmp_path, ""))
    monkeypatch.setenv("PATH", str(bin_dir))
    monkeypatch.delenv("FPS_TSHARK", raising=False)
    monkeypatch.delenv("FPS_ZEEK", raising=False)
    monkeypatch.setattr(parsers, "ZEEK_PACKAGE_PATH", tmp_path / "opt" / "zeek")
    pcap = pcapgen.mixed_capture(2, 2).write_pcap(tmp_path / "clean.pcap")
    out = tmp_path / "report.json"
    argv = ["score", "--pcap", str(pcap), "--labels", str(labels_file()), "--output", str(out)]
    argv += ["--ndpi-reader", str(_fake_ndpi(tmp_path))]
    assert main(argv) == EXIT_PASS
    assert "skipped parser zeek" in capsys.readouterr().err
    report = json.loads(out.read_text())
    _validate(report)
    entry = _parser_entry(report)
    assert [p["parser"] for p in entry["parsers"]] == ["tshark"]
    (skipped,) = entry["skipped_parsers"]
    assert skipped["parser"] == "zeek"
    assert skipped["reason"].startswith("not installed")
    (row,) = [r for r in summary.rows("dns", report) if "| all |" in r]
    assert "ran tshark 4.2.2" in row
    assert "skipped zeek (not installed" in row
