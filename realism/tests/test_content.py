# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the content variety check: duplicates, sizes, reassembly, and the report seam.

Every capture is built in code, so no test needs a tool or a captured file.
"""

from __future__ import annotations

import json
import random
from collections.abc import Callable
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from realism import content
from realism.capture import summarize
from realism.classifier import StubClassifier
from realism.cli import EXIT_FAIL, EXIT_PASS, main
from realism.content import IGNORED_HEADERS, check, describe, quantile, reassemble, split_http
from realism.labels import LabelsError, parse
from realism.report import LABELS_SCHEMA_PATH, SCHEMA_PATH, score
from realism.tests import pcapgen
from realism.tests.conftest import NDPI_42_HEADER

SERVER = ("10.0.1.80", 80)
ACK, PSH, SYN, RST = pcapgen.ACK, pcapgen.PSH, pcapgen.SYN, 0x04


def _client(i: int) -> tuple[str, int]:
    return ("10.0.0.10", 40000 + i)


def _request(path: str, host: str = "www.example.com") -> bytes:
    return f"GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: */*\r\n\r\n".encode()


def _response(body: bytes, date: str = "Mon, 09 Nov 2026 10:00:00 GMT") -> bytes:
    head = f"HTTP/1.1 200 OK\r\nDate: {date}\r\nContent-Length: {len(body)}\r\n\r\n"
    return head.encode() + body


def _labels(content_section: dict[str, Any] | None = None, **extra: Any) -> Any:
    doc: dict[str, Any] = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [
            {"application": "http1", "match": {"transport": "tcp", "server_port": 80}},
            {"application": "dns", "match": {"transport": "udp", "server_port": 53}},
        ],
        **extra,
    }
    if content_section is not None:
        doc["content"] = content_section
    return parse(doc)


def _run(
    tmp_path: Path, cap: pcapgen.Capture, labels: Any | None = None, limit: float | None = None
) -> content.ContentResult:
    pcap = cap.write_pcap(tmp_path / "c.pcap")
    return check(pcap, summarize(pcap).sessions, labels or _labels(), limit)


def _varied(count: int, seed: int = 7) -> pcapgen.Capture:
    """HTTP sessions whose paths and bodies all differ and whose sizes spread widely."""
    rng = random.Random(seed)
    cap = pcapgen.Capture()
    for i in range(count):
        body = bytes(rng.randrange(97, 123) for _ in range(rng.randrange(200, 4000)))
        cap.tcp_session(_client(i), SERVER, _request(f"/item/{i}"), _response(body))
    return cap


def _repeated(count: int) -> pcapgen.Capture:
    cap = pcapgen.Capture()
    for i in range(count):
        cap.tcp_session(_client(i), SERVER, _request("/same"), _response(b"x" * 500))
    return cap


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(report)


# --- acceptance: repeated bodies fail, varied bodies pass --------------------------------


def test_a_capture_with_repeated_bodies_fails_at_one_percent(tmp_path: Path) -> None:
    result = _run(tmp_path, _repeated(20))
    assert not result.passed
    entry = result.to_dict()
    assert entry["max_duplicate_ratio"] == 0.01
    assert entry["units"] == 40
    assert entry["duplicate_units"] == 38
    assert entry["duplicate_ratio"] == 0.95
    assert "38 of 40 payloads (95.00%) repeat an earlier payload" in result.failures[0]
    assert "seen 20 times" in result.failures[0]
    assert entry["duplicates"][0]["copies"] == 20


def test_a_varied_capture_passes(tmp_path: Path) -> None:
    result = _run(tmp_path, _varied(30))
    assert result.passed, result.failures
    entry = result.to_dict()
    assert entry["duplicate_units"] == 0
    assert entry["duplicates"] == []
    http = entry["applications"][0]
    assert http["application"] == "http1"
    assert http["directions"]["client"]["units"] == 30
    assert http["directions"]["server"]["size"]["cv"] > 0.3


def test_the_threshold_is_configurable(tmp_path: Path) -> None:
    cap = _varied(99)
    cap.tcp_session(_client(99), SERVER, _request("/item/0"), _response(b"copy"))
    # One repeated request in 200 units is 0.5%.
    assert _run(tmp_path, cap).passed
    cap.tcp_session(_client(100), SERVER, _request("/item/0"), _response(b"copy"))
    # Two more repeated requests and one repeated response in 202 units is 1.5%.
    assert not _run(tmp_path, cap).passed
    assert _run(tmp_path, cap, limit=0.05).passed
    assert not _run(tmp_path, cap, _labels({"max_duplicate_ratio": 0.001})).passed
    assert _run(tmp_path, cap, _labels({"max_duplicate_ratio": 0.05})).passed
    assert not _run(tmp_path, cap, _labels({"max_duplicate_ratio": 0.05}), limit=0.001).passed


def test_a_failing_application_fails_even_when_the_mix_passes(tmp_path: Path) -> None:
    cap = _varied(200)
    for i in range(10):
        cap.dns_lookup(("10.0.0.10", 50000 + i), ("10.0.1.53", 53), 0x1000 + i)
    result = _run(tmp_path, cap)
    assert result.ratio < 0.1
    assert not result.passed
    assert "application dns" in result.failures[0]
    assert [a["passed"] for a in result.to_dict()["applications"]] == [False, True]


# --- normalization -----------------------------------------------------------------------


def test_headers_that_vary_by_design_do_not_make_payloads_distinct(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i in range(10):
        response = _response(b"same body", date=f"Mon, 09 Nov 2026 10:00:{i:02d} GMT")
        cap.tcp_session(_client(i), SERVER, _request(f"/p/{i}"), response)
    result = _run(tmp_path, cap)
    assert result.to_dict()["applications"][0]["directions"]["server"]["duplicate_units"] == 9
    assert not result.passed


def test_labels_can_ignore_more_headers() -> None:
    a = b"GET / HTTP/1.1\r\nHost: h\r\nX-Nonce: 1\r\n\r\n"
    b = b"GET / HTTP/1.1\r\nHost: h\r\nX-Nonce: 2\r\n\r\n"
    ignore = IGNORED_HEADERS
    assert split_http(a, ignore)[0][0].normalized != split_http(b, ignore)[0][0].normalized
    wider = ignore | {"x-nonce"}
    assert split_http(a, wider)[0][0].normalized == split_http(b, wider)[0][0].normalized


def test_header_names_ignore_case_and_padding() -> None:
    a = split_http(b"GET / HTTP/1.1\r\nHost:  h \r\nAccept: */*\r\n\r\n", IGNORED_HEADERS)
    b = split_http(b"GET / HTTP/1.1\r\nhost: h\r\nACCEPT: */*\r\n\r\n", IGNORED_HEADERS)
    assert a[0][0].normalized == b[0][0].normalized


def test_different_bodies_stay_distinct() -> None:
    a = split_http(_response(b"one"), IGNORED_HEADERS)[0][0]
    b = split_http(_response(b"two"), IGNORED_HEADERS)[0][0]
    assert a.normalized != b.normalized


def test_dns_ignores_the_transaction_identifier(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i in range(5):
        cap.dns_lookup(("10.0.0.10", 50000 + i), ("10.0.1.53", 53), 0x2000 + i)
    result = _run(tmp_path, cap)
    dns = result.to_dict()["applications"][0]
    assert dns["application"] == "dns"
    assert dns["duplicate_units"] == 8
    assert not result.passed


def test_distinct_dns_names_pass(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i in range(5):
        cap.dns_lookup(("10.0.0.10", 50000 + i), ("10.0.1.53", 53), 7, f"host{i}.example.com")
    assert _run(tmp_path, cap, _labels({"max_duplicate_ratio": 0.5})).passed


def test_a_request_is_never_a_copy_of_a_response(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.tcp_session(_client(0), SERVER, b"same bytes", b"same bytes")
    result = _run(tmp_path, cap)
    assert result.to_dict()["duplicate_units"] == 0


# --- HTTP framing ------------------------------------------------------------------------


def test_a_keep_alive_session_yields_one_unit_per_message(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    requests = b"".join(_request(f"/a/{i}") for i in range(3))
    responses = b"".join(_response(f"body {i}".encode()) for i in range(3))
    cap.tcp_session(_client(0), SERVER, requests, responses)
    entry = _run(tmp_path, cap).to_dict()
    assert entry["units"] == 6
    assert entry["duplicate_units"] == 0


def test_chunked_bodies_are_decoded_before_hashing() -> None:
    head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
    one = head + b"3\r\nabc\r\n3\r\ndef\r\n0\r\n\r\n"
    two = head + b"6\r\nabcdef\r\n0\r\n\r\n"
    got = [split_http(x + _response(b"next"), IGNORED_HEADERS) for x in (one, two)]
    assert got[0][0][0].normalized == got[1][0][0].normalized
    assert len(got[0][0]) == 2
    assert got[0][0][0].size == len(one)
    assert got[0][1] is False


def test_chunked_trailers_and_truncation() -> None:
    head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
    trailed = head + b"3\r\nabc\r\n0\r\nX-Sum: 1\r\n\r\n"
    messages, unparsed = split_http(trailed, IGNORED_HEADERS)
    assert (len(messages), messages[0].truncated, unparsed) == (1, False, False)
    messages, _ = split_http(head + b"3\r\nabc\r\n3\r\nd", IGNORED_HEADERS)
    assert messages[0].truncated
    messages, _ = split_http(head + b"zz\r\n", IGNORED_HEADERS)
    assert messages[0].truncated


def test_a_short_content_length_body_is_truncated() -> None:
    cut = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc"
    messages, _ = split_http(cut, IGNORED_HEADERS)
    assert messages[0].truncated


def test_responses_without_framing_run_to_the_end_except_204_and_304() -> None:
    until_close = split_http(b"HTTP/1.1 200 OK\r\n\r\nbody", IGNORED_HEADERS)[0]
    assert len(until_close) == 1 and until_close[0].normalized.endswith(b"body")
    empty = split_http(
        b"HTTP/1.1 204 No Content\r\n\r\nHTTP/1.1 304 Not Modified\r\n\r\n", frozenset()
    )
    assert len(empty[0]) == 2


def test_text_that_is_not_http_is_one_raw_unit_and_counted(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.tcp_session(_client(0), SERVER, b"not http at all", b"HTTP/1.1 200 OK\r\nbroken")
    result = _run(tmp_path, cap)
    entry = result.to_dict()
    assert entry["units"] == 2
    assert entry["unparsed_tails"] == 2


def test_a_good_message_followed_by_garbage_keeps_the_message(tmp_path: Path) -> None:
    messages, unparsed = split_http(_response(b"ok") + b"\x00\x01garbage", IGNORED_HEADERS)
    assert len(messages) == 1 and unparsed


def test_unlabelled_sessions_are_sniffed_for_http(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i in range(3):
        cap.tcp_session(_client(i), ("10.0.1.90", 8080), _request("/x"), _response(b"y"))
    labels = parse({"labels": "fps-realism-labels", "version": 1, "content": {}})
    entry = _run(tmp_path, cap, labels).to_dict()
    assert entry["applications"][0]["application"] == "unlabelled"
    assert entry["units"] == 6
    assert entry["duplicate_units"] == 4


# --- reassembly --------------------------------------------------------------------------


def _segments(chunks: list[tuple[int, bytes]], syn: bool = True) -> content._Half:
    half = content._Half()
    if syn:
        half.syn_seq = 1001
    half.segments = [(seq, data) for seq, data in chunks]
    return half


def test_reassembly_orders_segments_and_drops_retransmissions() -> None:
    half = _segments([(1004, b"def"), (1001, b"abc"), (1004, b"def"), (1007, b"g")])
    assert reassemble(half) == (b"abcdefg", True)


def test_the_first_copy_of_a_byte_wins() -> None:
    half = _segments([(1001, b"abc"), (1002, b"XYZ"), (1005, b"e")])
    assert reassemble(half) == (b"abcZe", True)


def test_a_hole_cuts_the_stream_and_is_reported() -> None:
    half = _segments([(1001, b"abc"), (1010, b"zzz")])
    assert reassemble(half) == (b"abc", False)


def test_reassembly_without_a_syn_starts_at_the_lowest_segment() -> None:
    half = _segments([(5000, b"cd"), (4998, b"ab"), (5002, b"ef")], syn=False)
    assert reassemble(half) == (b"abcdef", True)


def test_sequence_numbers_wrap_around() -> None:
    half = content._Half(syn_seq=(2**32 - 2) % 2**32)
    half.segments = [(2**32 - 2, b"ab"), (0, b"cd")]
    assert reassemble(half) == (b"abcd", True)


def test_segmented_and_out_of_order_packets_hash_like_one_message(tmp_path: Path) -> None:
    request, response = _request("/seg"), _response(b"z" * 3000)
    cap = pcapgen.Capture()
    (c, cp), (s, sp) = _client(0), SERVER
    cap.add(pcapgen.tcp(c, cp, s, sp, SYN, 1000, 0))
    cap.add(pcapgen.tcp(s, sp, c, cp, SYN | ACK, 5000, 1001))
    cap.add(pcapgen.tcp(c, cp, s, sp, ACK | PSH, 1001, 5001, request))
    pieces = [response[i : i + 1000] for i in range(0, len(response), 1000)]
    order = [1, 0, 2, 2, *range(3, len(pieces))]
    for n in order:
        cap.add(
            pcapgen.tcp(s, sp, c, cp, ACK | PSH, 5001 + 1000 * n, 1001 + len(request), pieces[n])
        )
    whole = pcapgen.Capture()
    whole.tcp_session(_client(1), SERVER, request, response)
    split = _run(tmp_path, cap).to_dict()
    one = _run(tmp_path, whole).to_dict()
    assert split["units"] == one["units"] == 2
    assert split["applications"][0]["size"] == one["applications"][0]["size"]
    assert split["incomplete_directions"] == 0


def test_a_missing_segment_is_counted_and_the_cut_message_dropped(tmp_path: Path) -> None:
    response = _response(b"q" * 3000)
    cap = pcapgen.Capture()
    (c, cp), (s, sp) = _client(0), SERVER
    cap.add(pcapgen.tcp(c, cp, s, sp, SYN, 1000, 0))
    cap.add(pcapgen.tcp(s, sp, c, cp, SYN | ACK, 5000, 1001))
    cap.add(pcapgen.tcp(c, cp, s, sp, ACK | PSH, 1001, 5001, _request("/cut")))
    cap.add(pcapgen.tcp(s, sp, c, cp, ACK | PSH, 5001, 1001, response[:1000]))
    cap.add(pcapgen.tcp(s, sp, c, cp, ACK | PSH, 7001, 1001, response[2000:]))
    entry = _run(tmp_path, cap).to_dict()
    assert entry["incomplete_directions"] == 1
    assert entry["truncated_units"] == 1
    assert entry["units"] == 1


# --- encrypted and empty sessions --------------------------------------------------------


def _hello(n: int) -> bytes:
    return b"\x16\x03\x01\x00\x20" + bytes([n]) * 32


def test_tls_sessions_are_skipped_and_counted(tmp_path: Path) -> None:
    cap = _varied(10)
    for i in range(4):
        cap.tcp_session(_client(50 + i), ("10.0.1.43", 443), _hello(i), b"\x16\x03\x03\x00\x04abcd")
    result = _run(tmp_path, cap)
    entry = result.to_dict()
    assert result.passed
    assert entry["skipped_encrypted_sessions"] == 4
    assert entry["sessions_analysed"] == 10
    assert entry["sessions_total"] == 14


def test_a_tls_profile_label_skips_a_session_without_a_hello(tmp_path: Path) -> None:
    labels = _labels(
        {},
        tls={"profiles": {"chrome": {"ja4": "t13d1516h2_8daaf6152771_e5627efa2ab1"}}},
    )
    labels.rules.insert(
        0, type(labels.rules[0])("https", {"server_port": 8443}, tls_profile="chrome")
    )
    cap = _varied(3)
    cap.tcp_session(_client(60), ("10.0.1.43", 8443), b"opaque", b"bytes")
    entry = _run(tmp_path, cap, labels).to_dict()
    assert entry["skipped_encrypted_sessions"] == 1
    assert [a["application"] for a in entry["applications"]] == ["http1"]


def test_a_capture_with_only_encrypted_sessions_fails(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    cap.tcp_session(_client(0), ("10.0.1.43", 443), _hello(1), b"\x16\x03\x03\x00\x01a")
    result = _run(tmp_path, cap)
    assert not result.passed
    assert "no payload could be analysed: 1 sessions are encrypted" in result.failures[0]


def test_sessions_without_payload_are_counted(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    (c, cp), (s, sp) = _client(0), SERVER
    cap.add(pcapgen.tcp(c, cp, s, sp, SYN, 1000, 0))
    cap.add(pcapgen.tcp(s, sp, c, cp, SYN | ACK, 5000, 1001))
    cap.add(pcapgen.tcp(c, cp, s, sp, RST, 1001, 0))
    entry = _run(tmp_path, cap).to_dict()
    assert entry["sessions_without_payload"] == 1
    assert entry["passed"] is False


# --- sizes -------------------------------------------------------------------------------


def test_quantiles_interpolate_linearly() -> None:
    assert quantile([10], 0.5) == 10
    assert quantile([1, 2, 3, 4], 0.5) == 2.5
    assert quantile([0, 100], 0.25) == 25
    assert quantile([1, 2, 3, 4, 5], 1.0) == 5


def test_describe_reports_dispersion() -> None:
    got = describe([2, 4, 4, 4, 5, 5, 7, 9])
    assert got["count"] == 8
    assert (got["min"], got["max"], got["mean"], got["stddev"]) == (2, 9, 5.0, 2.0)
    assert got["cv"] == 0.4
    assert got["quantiles"]["p50"] == 4.5
    assert describe([5, 5, 5])["cv"] == 0.0


def test_the_size_distribution_is_reported_per_application_and_direction(tmp_path: Path) -> None:
    cap = pcapgen.Capture()
    for i, n in enumerate([100, 200, 300, 400]):
        cap.tcp_session(_client(i), SERVER, _request(f"/s/{i}"), _response(b"b" * n))
    entry = _run(tmp_path, cap).to_dict()
    server = entry["applications"][0]["directions"]["server"]["size"]
    wire = [len(_response(b"b" * n)) for n in (100, 200, 300, 400)]
    assert (server["count"], server["min"], server["max"]) == (4, wire[0], wire[3])
    assert server["quantiles"]["p50"] == (wire[1] + wire[2]) / 2
    assert "p99" in server["quantiles"]


def _body(i: int, n: int) -> bytes:
    """A body of n bytes that no other index shares."""
    return f"{i:06d}".encode() + b"b" * (n - 6)


def _sized(tmp_path: Path, sizes: list[int], expectation: dict[str, Any]) -> content.ContentResult:
    cap = pcapgen.Capture()
    for i, n in enumerate(sizes):
        cap.tcp_session(_client(i), SERVER, _request(f"/s/{i}"), _response(_body(i, n)))
    return _run(tmp_path, cap, _labels({"sizes": {"http1": expectation}}))


def test_sizes_that_match_the_expected_distribution_pass(tmp_path: Path) -> None:
    sizes = [400, 900, 1500, 2600, 4100, 700, 1200, 2000, 3200, 5000]
    base = len(_response(b""))
    result = _sized(tmp_path, sizes, {"direction": "server", "median": 1800 + base, "cv": 0.75})
    entry = result.to_dict()["expected_sizes"][0]
    assert result.passed, result.failures
    assert entry["passed"] and entry["direction"] == "server"
    assert entry["expected_cv"] == 0.75 and 0.6 < entry["observed_cv"] < 0.9


def test_a_wrong_median_fails_with_both_numbers(tmp_path: Path) -> None:
    result = _sized(tmp_path, [1000, 1100, 1200, 1300], {"direction": "server", "median": 5000})
    assert not result.passed
    assert "not within 25% of the expected 5000 bytes" in result.failures[0]
    assert result.to_dict()["expected_sizes"][0]["passed"] is False


def test_fixed_size_traffic_fails_an_expected_dispersion(tmp_path: Path) -> None:
    base = len(_response(b""))
    sizes = [1000 + i % 2 for i in range(20)]
    result = _sized(tmp_path, sizes, {"direction": "server", "median": 1000 + base, "cv": 0.6})
    assert not result.passed
    assert "coefficient of variation" in result.failures[0]


def test_an_expectation_for_an_absent_application_fails(tmp_path: Path) -> None:
    labels = _labels({"sizes": {"http1": {"median": 100}, "smtp": {"median": 100}}})
    result = _run(tmp_path, _varied(5), labels)
    assert any("application smtp: no both payload" in f for f in result.failures)


def test_a_zero_expected_dispersion_uses_the_tolerance_as_an_absolute_bound() -> None:
    assert content._within(0.0, 0.0, 0.25)
    assert not content._within(0.5, 0.0, 0.25)


# --- labels, report, and command line ----------------------------------------------------


def test_the_labels_schema_accepts_a_content_section() -> None:
    doc = {
        "labels": "fps-realism-labels",
        "version": 1,
        "content": {
            "max_duplicate_ratio": 0.02,
            "ignore_headers": ["X-Nonce"],
            "sizes": {
                "http1": {"direction": "server", "median": 1200, "cv": 0.8, "tolerance": 0.3}
            },
        },
    }
    schema = json.loads(LABELS_SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(doc)
    parsed = parse(doc)
    assert parsed.content is not None
    assert parsed.content.ignore_headers == ("x-nonce",)
    assert parsed.content.sizes["http1"].tolerance == 0.3


@pytest.mark.parametrize(
    ("section", "message"),
    [
        ([], "mapping with only"),
        ({"other": 1}, "mapping with only"),
        ({"max_duplicate_ratio": 2}, "from 0 to 1"),
        ({"max_duplicate_ratio": True}, "from 0 to 1"),
        ({"ignore_headers": "date"}, "list of header names"),
        ({"ignore_headers": [""]}, "list of header names"),
        ({"sizes": []}, "mapping from application"),
        ({"sizes": {"a": {}}}, "median: must be a number above 0"),
        ({"sizes": {"a": {"median": 1, "x": 1}}}, "mapping with only"),
        ({"sizes": {"a": {"median": 1, "direction": "up"}}}, "direction"),
        ({"sizes": {"a": {"median": 1, "cv": -1}}}, "cv"),
        ({"sizes": {"a": {"median": 1, "tolerance": 0}}}, "tolerance"),
    ],
)
def test_bad_content_sections_are_rejected(section: Any, message: str) -> None:
    doc = {"labels": "fps-realism-labels", "version": 1, "content": section}
    with pytest.raises(LabelsError, match=message):
        parse(doc)


def _labels_path(tmp_path: Path, section: dict[str, Any] | None) -> Path:
    doc: dict[str, Any] = {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "http1", "match": {"server_port": 80}}],
    }
    if section is not None:
        doc["content"] = section
    path = tmp_path / "labels.json"
    path.write_text(json.dumps(doc))
    return path


def test_the_report_has_the_content_entry_and_validates(tmp_path: Path) -> None:
    pcap = _repeated(10).write_pcap(tmp_path / "rep.pcap")
    classifier = StubClassifier.by_server_port({80: "HTTP"}, name="ndpi")
    report = score(pcap, _labels_path(tmp_path, {}), classifier).to_dict()
    _validate(report)
    assert report["version"] == "0.9.0"
    assert [c["check"] for c in report["checks"]] == ["conformance", "content"]
    assert report["checks"][1]["passed"] is False
    assert report["passed"] is False
    varied = _varied(10).write_pcap(tmp_path / "var.pcap")
    ok = score(varied, _labels_path(tmp_path, {}), classifier).to_dict()
    _validate(ok)
    assert ok["passed"] is True


def test_the_check_is_off_without_a_section_or_an_option(tmp_path: Path) -> None:
    pcap = _repeated(3).write_pcap(tmp_path / "rep.pcap")
    classifier = StubClassifier.by_server_port({80: "HTTP"}, name="ndpi")
    off = score(pcap, _labels_path(tmp_path, None), classifier).to_dict()
    assert [c["check"] for c in off["checks"]] == ["conformance"]
    on = score(pcap, _labels_path(tmp_path, None), classifier, max_duplicate_ratio=1.0).to_dict()
    assert [c["check"] for c in on["checks"]] == ["conformance", "content"]
    assert on["checks"][1]["passed"] is True


def test_the_command_line_option_adds_and_overrides_the_check(
    tmp_path: Path,
    fake_reader: Callable[..., Path],
    capsys: pytest.CaptureFixture[str],
) -> None:
    csv = NDPI_42_HEADER + "\n"
    for i in range(10):
        csv += f"{i},6,1,1,0,10.0.0.10,{40000 + i},10.0.1.80,80,7,HTTP,,6,414\n"
    base = ["score", "--pcap", str(_repeated(10).write_pcap(tmp_path / "r.pcap"))]
    base += ["--labels", str(_labels_path(tmp_path, None)), "--parsers", "none"]
    base += ["--ndpi-reader", str(fake_reader(csv))]
    assert main(base) == EXIT_PASS
    capsys.readouterr()
    assert main([*base, "--max-duplicate-ratio", "0.01"]) == EXIT_FAIL
    report = json.loads(capsys.readouterr().out)
    assert report["checks"][1]["check"] == "content"
    assert main([*base, "--max-duplicate-ratio", "1"]) == EXIT_PASS
    capsys.readouterr()
    labels = _labels_path(tmp_path, {"max_duplicate_ratio": 1})
    base[base.index("--labels") + 1] = str(labels)
    assert main(base) == EXIT_PASS
    capsys.readouterr()
    assert main([*base, "--max-duplicate-ratio", "0.01"]) == EXIT_FAIL
