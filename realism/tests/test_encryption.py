# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the encryption check: stub probes for every branch, real tshark for the rest.

The real tests generate TLS captures with the Python ``ssl`` module and run the
installed tshark on them. They skip where tshark or openssl is missing, and the
rlm and realism workflows install tshark, so CI runs them.
"""

from __future__ import annotations

import json
import ssl
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from realism import ja4
from realism.capture import SessionKey, summarize
from realism.classifier import StubClassifier
from realism.cli import EXIT_ERROR, EXIT_FAIL, EXIT_PASS, main
from realism.encryption import (
    Finished,
    Hello,
    StubTlsProbe,
    TlsObservation,
    TsharkTlsProbe,
    check,
    choose_sample,
    has_embedded_keys,
    parse_fields,
)
from realism.labels import LabelsError, parse
from realism.parsers import ParserError
from realism.report import SCHEMA_PATH, score
from realism.tests import pcapgen, tlsgen
from realism.tests.conftest import NDPI_42_HEADER
from realism.tests.test_ja4 import CHROME, CHROME_JA4

TSHARK = TsharkTlsProbe()
needs_tls_tools = pytest.mark.skipif(
    not TSHARK.available() or not tlsgen.HAS_OPENSSL, reason="no tshark or no openssl"
)
CLIENT = ("10.0.0.10", 40000)
SERVER = ("10.0.1.43", 443)
OTHER_JA4 = "t12d1007h2_aaaaaaaaaaaa_bbbbbbbbbbbb"


def _labels(
    profiles: dict[str, Any] | None = None, tls_extra: dict[str, Any] | None = None, **rule: Any
) -> Any:
    doc = {
        "labels": "fps-realism-labels",
        "version": 1,
        "tls": {"profiles": profiles or {"chrome": {"ja4": CHROME_JA4}}, **(tls_extra or {})},
        "sessions": [
            {"application": "https", "tls_profile": "chrome", "match": {"server_port": 443}, **rule}
        ],
    }
    return parse(doc)


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator.check_schema(schema)
    jsonschema.Draft202012Validator(schema).validate(report)


def _stub_capture(tmp_path: Path, sessions: int = 3) -> Path:
    cap = pcapgen.Capture()
    for i in range(sessions):
        cap.tcp_session((CLIENT[0], CLIENT[1] + i), SERVER, b"hello", b"world")
    return cap.write_pcap(tmp_path / "stub.pcap")


def _observation(
    pcap: Path,
    sessions: int = 3,
    hello_data: bytes | None = None,
    finished: int = 2,
    tshark_ja4: str | None = None,
) -> TlsObservation:
    data = hello_data if hello_data is not None else tlsgen.build_client_hello(CHROME)
    obs = TlsObservation(tool="stub", version="stub")
    for s in summarize(pcap).sessions[:sessions]:
        obs.hellos.append(Hello(s.key, 4, data, tshark_ja4))
        for sender in [(s.client_ip, s.client_port), (s.server_ip, s.server_port)][:finished]:
            obs.finished.append(Finished(s.key, sender))
    return obs


def _run(pcap: Path, labels: Any, obs: TlsObservation, **kwargs: Any) -> dict[str, Any]:
    result = check(pcap, summarize(pcap).sessions, labels, StubTlsProbe(obs), **kwargs)
    return result.to_dict()


def test_a_matching_sample_passes(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path)
    entry = _run(pcap, _labels(), _observation(pcap, tshark_ja4=CHROME_JA4))
    assert entry["passed"] is True, entry["failures"]
    assert entry["sampled_sessions"] == 3
    assert entry["decrypted_ratio"] == 1.0
    assert entry["ja4_matched_sessions"] == 3
    assert entry["sessions"][0]["ja4"] == CHROME_JA4
    assert entry["sessions"][0]["server_name"] == "tls.example.test"
    assert entry["profiles"] == [
        {
            "profile": "chrome",
            "expected_ja4": [CHROME_JA4],
            "sessions": 3,
            "matched": 3,
            "decrypted": 3,
        }
    ]


def test_a_mismatched_profile_fails_and_says_why(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=2)
    labels = _labels(profiles={"chrome": {"ja4": "t13d1517h2_8daaf6152771_e5627efa2ab1"}})
    entry = _run(pcap, labels, _observation(pcap, sessions=2))
    assert entry["passed"] is False
    assert entry["decrypted_ratio"] == 1.0
    assert entry["ja4_mismatched_sessions"] == 2
    assert len(entry["failures"]) == 2
    reason = entry["failures"][0]
    assert f"JA4 {CHROME_JA4}" in reason
    assert "profile chrome" in reason
    assert "part a" in reason and "t13d1516h2, expected t13d1517h2" in reason


def test_the_cipher_hash_difference_is_named(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    labels = _labels(profiles={"chrome": {"ja4": "t13d1516h2_000000000000_e5627efa2ab1"}})
    entry = _run(pcap, labels, _observation(pcap, sessions=1))
    assert "cipher suite hash is 8daaf6152771, expected 000000000000" in entry["failures"][0]


def test_a_profile_may_list_several_fingerprints(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    labels = _labels(profiles={"chrome": {"ja4": [OTHER_JA4, CHROME_JA4]}})
    assert _run(pcap, labels, _observation(pcap, sessions=1))["passed"] is True


def test_sessions_that_do_not_decrypt_fail_below_the_required_share(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path)
    entry = _run(pcap, _labels(), _observation(pcap, finished=0))
    assert entry["passed"] is False
    assert entry["decrypted_sessions"] == 0
    assert entry["ja4_matched_sessions"] == 3
    assert "decrypted 0 of 3 sampled sessions, below the required 1.00" in entry["failures"][0]
    assert "no embedded keys and no key log" in entry["failures"][0]
    assert "not decrypted" in entry["sessions"][0]["reasons"][0]


def test_one_finished_message_is_not_a_decrypted_session(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    entry = _run(pcap, _labels(), _observation(pcap, sessions=1, finished=1))
    assert entry["decrypted_sessions"] == 0
    assert "1 of 2 sides" in entry["sessions"][0]["reasons"][0]


def test_min_decrypted_lets_a_share_of_the_sample_fail_to_decrypt(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=4)
    obs = _observation(pcap, sessions=4)
    obs.finished = obs.finished[:-2]  # the last session has no Finished message
    assert _run(pcap, _labels(), obs)["passed"] is False
    entry = _run(pcap, _labels(tls_extra={"min_decrypted": 0.75}), obs)
    assert entry["decrypted_ratio"] == 0.75
    assert entry["passed"] is True


def test_the_key_source_is_reported(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    entry = _run(pcap, _labels(), _observation(pcap, sessions=1), keylog=tmp_path / "k.log")
    assert entry["keys"] == {"embedded": False, "sidecar": "k.log"}
    cap = pcapgen.Capture()
    cap.tcp_session(CLIENT, SERVER, b"a", b"b")
    keyed = cap.write_pcapng(tmp_path / "keyed.pcapng", tls_keys=b"CLIENT_RANDOM aa bb\n")
    plain = cap.write_pcapng(tmp_path / "plain.pcapng")
    assert has_embedded_keys(keyed) is True
    assert has_embedded_keys(plain) is False
    assert has_embedded_keys(pcap) is False
    assert has_embedded_keys(tmp_path / "absent.pcapng") is False


def test_a_session_without_a_client_hello_fails(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=2)
    obs = _observation(pcap, sessions=1, finished=2)
    entry = _run(pcap, _labels(), obs)
    assert entry["passed"] is False
    assert entry["sessions"][1]["ja4"] is None
    assert "no ClientHello found" in entry["failures"][-1]


def test_an_unreadable_client_hello_fails(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    entry = _run(pcap, _labels(), _observation(pcap, sessions=1, hello_data=b"\x16\x03\x01"))
    assert entry["passed"] is False
    assert "cannot be read" in entry["failures"][0]


def test_a_disagreement_between_tshark_and_the_runner_fails(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    entry = _run(pcap, _labels(), _observation(pcap, sessions=1, tshark_ja4=OTHER_JA4))
    assert entry["passed"] is False
    assert f"tshark computes JA4 {OTHER_JA4}" in entry["failures"][0]
    assert entry["sessions"][0]["tshark_ja4"] == OTHER_JA4


def test_a_capture_without_labelled_tls_sessions_fails(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    labels = _labels(match={"server_port": 8443})
    probe = StubTlsProbe()
    entry = check(pcap, summarize(pcap).sessions, labels, probe).to_dict()
    assert entry["passed"] is False
    assert entry["sampled_sessions"] == 0
    assert "no session of the capture matches a rule with tls_profile" in entry["failures"]
    assert probe.calls == []


def test_a_missing_tshark_fails_visibly(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    probe = StubTlsProbe(installed=False)
    entry = check(pcap, summarize(pcap).sessions, _labels(), probe).to_dict()
    assert entry["passed"] is False
    assert entry["tool"] is None
    assert "tshark is not installed" in entry["failures"][0]
    assert probe.calls == []


def test_the_sample_is_spread_over_the_sessions_and_limited(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path, sessions=10)
    sessions = summarize(pcap).sessions
    assert [s.id for s in choose_sample(sessions, 20)] == list(range(1, 11))
    assert [s.id for s in choose_sample(sessions, 4)] == [1, 3, 6, 8]
    entry = _run(pcap, _labels(tls_extra={"sample": 4}), _observation(pcap, sessions=10))
    assert entry["sampled_sessions"] == 4
    assert entry["labelled_sessions"] == 10
    assert entry["sample_limit"] == 4


def test_the_probe_gets_the_ports_of_the_sample_and_the_key_log(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.tcp_session(CLIENT, ("10.0.1.43", 8443), b"a", b"b")
    pcap = cap.write_pcap(tmp_path / "port.pcap")
    probe = StubTlsProbe()
    labels = _labels(match={"server_port": 8443})
    keylog = tmp_path / "k.log"
    check(pcap, summarize(pcap).sessions, labels, probe, keylog)
    assert probe.calls == [(pcap, (8443,), keylog)]


def test_report_has_the_encryption_entry_and_validates(tmp_path: Path) -> None:
    pcap = _stub_capture(tmp_path)
    labels_path = tmp_path / "labels.json"
    labels_path.write_text(
        json.dumps(
            {
                "labels": "fps-realism-labels",
                "version": 1,
                "tls": {"profiles": {"chrome": {"ja4": CHROME_JA4}}},
                "sessions": [
                    {"application": "https", "tls_profile": "chrome", "match": {"server_port": 443}}
                ],
            }
        )
    )
    classifier = StubClassifier.by_server_port({443: "TLS"}, name="ndpi")
    probe = StubTlsProbe(_observation(pcap))
    report = score(pcap, labels_path, classifier, tls_probe=probe).to_dict()
    _validate(report)
    assert report["version"] == "0.20.0"
    encryption = [c for c in report["checks"] if c["check"] == "encryption"]
    assert len(encryption) == 1
    assert encryption[0]["passed"] is True


def test_labels_without_a_tls_section_add_no_encryption_check(tmp_path: Path) -> None:
    pcap = pcapgen.mixed_capture().write_pcap(tmp_path / "plain.pcap")
    labels_path = tmp_path / "labels.json"
    labels_path.write_text(
        json.dumps(
            {
                "labels": "fps-realism-labels",
                "version": 1,
                "sessions": [{"application": "http1", "match": {"server_port": 80}}],
            }
        )
    )
    classifier = StubClassifier.by_server_port({80: "HTTP"}, name="ndpi")
    report = score(pcap, labels_path, classifier).to_dict()
    assert [c["check"] for c in report["checks"]] == ["conformance"]


@pytest.mark.parametrize(
    ("change", "message"),
    [
        ({"tls": {"profiles": {}}}, "non empty mapping"),
        ({"tls": {"profiles": {"a": {"ja4": "nope"}}}}, "tls.profiles.a"),
        ({"tls": {"profiles": {"a": {"ja4": CHROME_JA4}}, "sample": 0}}, "tls.sample"),
        ({"tls": {"profiles": {"a": {"ja4": CHROME_JA4}}, "min_decrypted": 0}}, "min_decrypted"),
        ({"tls": {"profiles": {"a": {"ja4": CHROME_JA4}}, "other": 1}}, "only"),
        ({"sessions": [{"application": "x", "tls_profile": "a"}]}, "not a profile"),
    ],
)
def test_labels_reject_a_bad_tls_section(change: dict[str, Any], message: str) -> None:
    doc = {"labels": "fps-realism-labels", "version": 1, **change}
    with pytest.raises(LabelsError, match=message):
        parse(doc)


def test_parse_fields_reads_hellos_and_finished_messages() -> None:
    data = tlsgen.build_client_hello(CHROME)
    hello = "5\t10.0.0.10\t10.0.1.43\t\t\t40000\t443\t1\tt13d1516h2_x_y\t\t" + ":".join(
        f"{b:02x}" for b in data
    )
    finished = "9\t\t\t2001:db8::1\t2001:db8::2\t443\t40001\t20\t\t\t"
    hellos, done = parse_fields(hello + "\n\n" + finished + "\n")
    assert len(hellos) == 1 and hellos[0].data == data and hellos[0].tshark_ja4 == "t13d1516h2_x_y"
    assert hellos[0].key == SessionKey.of("tcp", "10.0.0.10", 40000, "10.0.1.43", 443)
    assert done == [
        Finished(
            SessionKey.of("tcp", "2001:db8::1", 443, "2001:db8::2", 40001), ("2001:db8::1", 443)
        )
    ]
    with pytest.raises(ParserError):
        parse_fields("x\ty\n")


def test_the_cli_runs_the_check_with_a_key_log(
    tmp_path: Path,
    fake_reader: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    pcap = _stub_capture(tmp_path, sessions=1)
    labels_path = tmp_path / "labels.json"
    labels_path.write_text(
        json.dumps(
            {
                "labels": "fps-realism-labels",
                "version": 1,
                "tls": {"profiles": {"chrome": {"ja4": CHROME_JA4}}},
                "sessions": [
                    {"application": "https", "tls_profile": "chrome", "match": {"server_port": 443}}
                ],
            }
        )
    )
    csv = NDPI_42_HEADER + "\n0,6,1,1,0,10.0.0.10,40000,10.0.1.43,443,91,TLS,,6,414\n"
    argv = ["score", "--pcap", str(pcap), "--labels", str(labels_path), "--parsers", "none"]
    argv += ["--ndpi-reader", str(fake_reader(csv)), "--keylog", str(tmp_path / "absent.log")]
    assert main(argv) == EXIT_ERROR
    assert "is not a file" in capsys.readouterr().err


# The tests below generate real TLS handshakes and run the installed tshark.

DEFAULT_CLIENT = tlsgen.ClientConfig()
NO_ALPN_CLIENT = tlsgen.ClientConfig(alpn=())
TLS12_CLIENT = tlsgen.ClientConfig(max_version=ssl.TLSVersion.TLSv1_2)
AES128_CLIENT = tlsgen.ClientConfig(
    ciphers="ECDHE-ECDSA-AES128-GCM-SHA256", max_version=ssl.TLSVersion.TLSv1_2
)


def _tshark_ja4(pcap: Path, keylog: Path | None = None) -> list[str]:
    """Return the JA4 values that tshark computes, an expectation that shares no code with ours."""
    obs = TSHARK.observe(pcap, [443], keylog)
    return [h.tshark_ja4 or "" for h in obs.hellos]


def _real_report(pcap: Path, labels: Any, keylog: Path | None = None) -> dict[str, Any]:
    return check(pcap, summarize(pcap).sessions, labels, TSHARK, keylog).to_dict()


@needs_tls_tools
def test_python_and_tshark_agree_on_real_client_hellos(tmp_path: Path) -> None:
    configs = [DEFAULT_CLIENT, NO_ALPN_CLIENT, TLS12_CLIENT, AES128_CLIENT]
    cap, _ = tlsgen.tls_capture(tmp_path, configs)
    pcap = cap.write_pcap(tmp_path / "hellos.pcap")
    observed = TSHARK.observe(pcap, [443], None)
    assert len(observed.hellos) == len(configs)
    seen = set()
    for hello in observed.hellos:
        assert hello.tshark_ja4, "this tshark has no JA4 field, install Wireshark 4.2 or newer"
        assert ja4.compute(ja4.parse_client_hello(hello.data)) == hello.tshark_ja4
        seen.add(hello.tshark_ja4)
    assert len(seen) == len(configs), seen
    assert observed.version[0].isdigit()


@needs_tls_tools
def test_a_matching_fixture_passes_with_embedded_keys(tmp_path: Path) -> None:
    cap, keylog = tlsgen.tls_capture(tmp_path, [DEFAULT_CLIENT] * 3)
    pcap = cap.write_pcapng(tmp_path / "match.pcapng", tls_keys=keylog.read_bytes())
    expected = set(_tshark_ja4(pcap))
    assert len(expected) == 1
    labels = _labels(profiles={"chrome": {"ja4": sorted(expected)[0]}})
    entry = _real_report(pcap, labels)
    assert entry["passed"] is True, entry["failures"]
    assert entry["keys"] == {"embedded": True, "sidecar": None}
    assert entry["decrypted_sessions"] == 3
    assert entry["ja4_matched_sessions"] == 3
    assert entry["tool"]["tool"] == "tshark"


@needs_tls_tools
def test_a_matching_fixture_passes_with_a_sidecar_key_log(tmp_path: Path) -> None:
    cap, keylog = tlsgen.tls_capture(tmp_path, [DEFAULT_CLIENT, TLS12_CLIENT])
    pcap = cap.write_pcap(tmp_path / "sidecar.pcap")
    first, second = _tshark_ja4(pcap)
    labels = _labels(profiles={"chrome": {"ja4": [first, second]}})
    entry = _real_report(pcap, labels, keylog)
    assert entry["passed"] is True, entry["failures"]
    assert entry["keys"] == {"embedded": False, "sidecar": "keys.log"}
    assert entry["decrypted_ratio"] == 1.0


@needs_tls_tools
def test_a_mismatched_profile_fails(tmp_path: Path) -> None:
    cap, keylog = tlsgen.tls_capture(tmp_path, [AES128_CLIENT, AES128_CLIENT])
    pcap = cap.write_pcapng(tmp_path / "mismatch.pcapng", tls_keys=keylog.read_bytes())
    # The profile says Chrome, and the client that produced the capture is not Chrome.
    entry = _real_report(pcap, _labels())
    assert entry["passed"] is False
    assert entry["decrypted_ratio"] == 1.0
    assert entry["ja4_mismatched_sessions"] == 2
    assert entry["failures"][0].startswith("session 1 (chrome): JA4 t12d")
    assert "is not the JA4 of profile chrome" in entry["failures"][0]
    assert "cipher suite hash" in entry["failures"][0]


@needs_tls_tools
def test_a_client_that_drops_alpn_fails_on_part_a_only(tmp_path: Path) -> None:
    cap, keylog = tlsgen.tls_capture(tmp_path, [DEFAULT_CLIENT, NO_ALPN_CLIENT])
    pcap = cap.write_pcap(tmp_path / "alpn.pcap")
    with_alpn = _tshark_ja4(pcap)[0]
    labels = _labels(profiles={"chrome": {"ja4": with_alpn}})
    entry = _real_report(pcap, labels, keylog)
    assert entry["passed"] is False
    assert entry["ja4_matched_sessions"] == 1
    assert entry["sessions"][0]["ja4_matched"] is True
    reason = entry["failures"][0]
    assert reason.startswith("session 2 (chrome)")
    assert "part a" in reason and "cipher suite hash" not in reason


@needs_tls_tools
def test_a_capture_without_keys_fails_to_decrypt(tmp_path: Path) -> None:
    cap, _ = tlsgen.tls_capture(tmp_path, [DEFAULT_CLIENT, TLS12_CLIENT])
    pcap = cap.write_pcapng(tmp_path / "blind.pcapng")
    expected = _tshark_ja4(pcap)
    labels = _labels(profiles={"chrome": {"ja4": expected}})
    entry = _real_report(pcap, labels)
    assert entry["passed"] is False
    assert entry["decrypted_sessions"] == 0
    assert entry["ja4_matched_sessions"] == 2
    assert "no embedded keys and no key log was given" in entry["failures"][0]


@needs_tls_tools
def test_a_key_log_with_the_wrong_secrets_decrypts_nothing(tmp_path: Path) -> None:
    cap, keylog = tlsgen.tls_capture(tmp_path, [DEFAULT_CLIENT, TLS12_CLIENT])
    pcap = cap.write_pcap(tmp_path / "wrong.pcap")
    wrong = tmp_path / "wrong.log"
    wrong.write_text(
        "".join(
            line[:-1] + ("0" if line[-1] != "0" else "1") + "\n"
            for line in keylog.read_text().splitlines()
            if not line.startswith("#")
        )
    )
    labels = _labels(profiles={"chrome": {"ja4": _tshark_ja4(pcap)}})
    entry = _real_report(pcap, labels, wrong)
    assert entry["decrypted_sessions"] == 0
    assert entry["passed"] is False


@needs_tls_tools
def test_tls_on_another_port_is_decoded_and_checked(tmp_path: Path) -> None:
    cap, keylog = tlsgen.tls_capture(tmp_path, [DEFAULT_CLIENT, TLS12_CLIENT], server_port=8443)
    pcap = cap.write_pcap(tmp_path / "port8443.pcap")
    expected = [h.tshark_ja4 or "" for h in TSHARK.observe(pcap, [8443], None).hellos]
    labels = _labels(profiles={"chrome": {"ja4": expected}}, match={"server_port": 8443})
    entry = _real_report(pcap, labels, keylog)
    assert entry["passed"] is True, entry["failures"]
    assert entry["decrypted_sessions"] == 2


@needs_tls_tools
def test_the_cli_exits_1_for_a_mismatch_and_0_for_a_match(
    tmp_path: Path, fake_reader: Callable[..., Path], capsys: pytest.CaptureFixture[str]
) -> None:
    cap, keylog = tlsgen.tls_capture(tmp_path, [DEFAULT_CLIENT])
    pcap = cap.write_pcap(tmp_path / "cli.pcap")
    (real,) = _tshark_ja4(pcap)
    csv = NDPI_42_HEADER + "\n0,6,1,1,0,10.0.0.10,40000,10.0.1.43,443,91,TLS,,6,414\n"
    reader = str(fake_reader(csv))

    def run(ja4_value: str) -> tuple[int, dict[str, Any]]:
        labels_path = tmp_path / "labels.json"
        labels_path.write_text(
            json.dumps(
                {
                    "labels": "fps-realism-labels",
                    "version": 1,
                    "applications": {"https": {"ndpi": ["TLS"]}},
                    "tls": {"profiles": {"chrome": {"ja4": ja4_value}}},
                    "sessions": [
                        {
                            "application": "https",
                            "tls_profile": "chrome",
                            "match": {"server_port": 443},
                        }
                    ],
                }
            )
        )
        argv = ["score", "--pcap", str(pcap), "--labels", str(labels_path), "--parsers", "none"]
        argv += ["--ndpi-reader", reader, "--keylog", str(keylog)]
        code = main(argv)
        return code, json.loads(capsys.readouterr().out)

    code, report = run(real)
    assert code == EXIT_PASS, report["checks"][-1]["failures"]
    _validate(report)
    code, report = run(OTHER_JA4)
    assert code == EXIT_FAIL
    assert report["checks"][0]["passed"] is True
    assert report["checks"][-1]["passed"] is False
    _validate(report)


def test_the_job_summary_has_an_encryption_row(tmp_path: Path) -> None:
    from realism.ci import summary

    pcap = _stub_capture(tmp_path, sessions=2)
    entry = _run(pcap, _labels(profiles={"chrome": {"ja4": OTHER_JA4}}), _observation(pcap, 2))
    (row,) = summary.rows("tls", {"checks": [entry]})
    assert row.startswith("| tls | encryption | all | fail |")
    assert "2 of 2 sampled sessions decrypted, 0 with the JA4 of their profile" in row
    assert "profile chrome" in row
