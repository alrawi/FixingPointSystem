# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the message count check: labels, counting rules, report, and the real tshark.

The first tests need no tool. They parse the messages section, judge counts, and read
canned tshark output. The stub probe tests run the whole check on views that
``realism.decrypted.write_view`` writes. The last tests run the real tshark on synthetic
dialogues and on the committed fixtures, which they break by dropping a message, and skip
where tshark is missing.
"""

from __future__ import annotations

import json
import os
import shutil
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from realism import decrypted, messages
from realism.capture import Session, SessionKey, summarize
from realism.classifier import StubClassifier
from realism.decrypted import TlsSession, Tshark, read_sessions, write_view
from realism.labels import Count, Labels, LabelsError, load, parse
from realism.messages import (
    StubMessageProbe,
    TsharkMessageProbe,
    describe,
    judge,
    parse_fields,
)
from realism.report import LABELS_SCHEMA_PATH, REPORT_VERSION, SCHEMA_PATH, score

ROOT = Path(__file__).resolve().parents[2]
EXAMPLES = ROOT / "realism" / "examples" / "decrypted"
TSHARK = os.environ.get("FPS_TSHARK") or shutil.which("tshark")
needs_tshark = pytest.mark.skipif(TSHARK is None, reason="tshark is not installed")
CLIENT = "10.0.0.10"
SERVER = "10.0.1.43"


def _doc(rules: list[dict[str, Any]] | None = None) -> dict[str, Any]:
    return {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "mail", "match": {"transport": "tcp"}}],
        "applications": {"mail": {"stub": ["SMTP"]}},
        "messages": rules
        if rules is not None
        else [{"protocol": "smtp", "match": {"server_port": 587}, "expect": {"messages": 1}}],
    }


def _validate(report: dict[str, Any]) -> None:
    schema = json.loads(SCHEMA_PATH.read_text())
    jsonschema.Draft202012Validator(schema).validate(report)


# Labels


def test_the_messages_section_reads_exact_counts_ranges_and_tables() -> None:
    labels = parse(
        _doc(
            [
                {
                    "protocol": "smtp",
                    "match": {"server_port": 587},
                    "expect": {
                        "commands": {"ehlo": 2, "RCPT": {"min": 1, "max": 3}},
                        "replies": {250: {"min": 2}},
                        "messages": 1,
                    },
                },
                {"protocol": "http1", "expect": {"requests": 4, "responses": 4}},
            ]
        )
    )
    assert labels.messages is not None
    smtp, http = labels.messages
    assert smtp.protocol == "smtp"
    assert smtp.match == {"server_port": 587}
    assert smtp.expect == {
        "commands.EHLO": Count(2, 2),
        "commands.RCPT": Count(1, 3),
        "replies.250": Count(2, None),
        "messages": Count(1, 1),
    }
    assert http.match == {}
    assert http.expect == {"requests": Count(4, 4), "responses": Count(4, 4)}


def test_a_labels_file_without_the_section_has_none() -> None:
    doc = _doc()
    del doc["messages"]
    assert parse(doc).messages is None


@pytest.mark.parametrize(
    ("rules", "message"),
    [
        ([], "at least one rule"),
        ([{"protocol": "ftp", "expect": {"messages": 1}}], "protocol must be one of"),
        ([{"expect": {"messages": 1}}], "protocol must be one of"),
        ([{"protocol": "smtp"}], "expect must be a mapping"),
        ([{"protocol": "smtp", "expect": {}}], "expect must be a mapping"),
        ([{"protocol": "smtp", "expect": {"messages": -1}}], "non negative integer"),
        ([{"protocol": "smtp", "expect": {"messages": True}}], "non negative integer"),
        ([{"protocol": "smtp", "expect": {"messages": 1.5}}], "non negative integer"),
        ([{"protocol": "smtp", "expect": {"messages": {"lo": 1}}}], "only min and max"),
        ([{"protocol": "smtp", "expect": {"messages": {}}}], "only min and max"),
        ([{"protocol": "smtp", "expect": {"messages": {"min": 3, "max": 1}}}], "max is below min"),
        ([{"protocol": "smtp", "expect": {"requests": 1}}], "smtp counts are"),
        ([{"protocol": "http1", "expect": {"messages": 1}}], "http1 counts are"),
        ([{"protocol": "http2", "expect": {"commands": {"GET": 1}}}], "http2 counts are"),
        ([{"protocol": "smtp", "expect": {"commands": {}}}], "must map names to counts"),
        ([{"protocol": "smtp", "expect": {"commands": {"E HLO": 1}}}], "not a name or a code"),
        ([{"protocol": "smtp", "expect": {"commands": 3}}], "must map names to counts"),
        ([{"protocol": "smtp", "match": {"port": 1}, "expect": {"messages": 1}}], "unknown match"),
        ([{"protocol": "smtp", "expect": {"messages": 1}, "extra": 1}], "a rule has only"),
        ([{"protocol": "smtp", "match": [], "expect": {"messages": 1}}], "match must be a mapping"),
    ],
)
def test_a_malformed_messages_section_is_refused(rules: list[dict[str, Any]], message: str) -> None:
    with pytest.raises(LabelsError, match=message):
        parse(_doc(rules))


def test_the_messages_section_must_be_a_list() -> None:
    doc = _doc()
    doc["messages"] = {"protocol": "smtp"}
    with pytest.raises(LabelsError, match="must be a list"):
        parse(doc)


@pytest.mark.parametrize("name", ["http1", "http2", "smtp", "imap"])
def test_the_example_labels_have_messages_and_follow_the_schema(name: str) -> None:
    path = EXAMPLES / f"{name}.labels.yaml"
    yaml = pytest.importorskip("yaml")
    jsonschema.Draft202012Validator(json.loads(LABELS_SCHEMA_PATH.read_text())).validate(
        yaml.safe_load(path.read_text())
    )
    labels = load(path)
    assert labels.messages
    assert {rule.protocol for rule in labels.messages} == {name}


@pytest.mark.parametrize(
    ("rule", "valid"),
    [
        ({"protocol": "smtp", "expect": {"messages": 1}}, True),
        ({"protocol": "http1", "expect": {"requests": {"min": 1}}}, True),
        ({"protocol": "imap", "expect": {"commands": {"FETCH": 3}, "replies": {"OK": 7}}}, True),
        ({"protocol": "smtp", "expect": {"requests": 1}}, False),
        ({"protocol": "http2", "expect": {"messages": 1}}, False),
        ({"protocol": "http1", "expect": {"replies": {"OK": 1}}}, False),
        ({"protocol": "smtp", "expect": {}}, False),
        ({"protocol": "pop3", "expect": {"messages": 1}}, False),
    ],
)
def test_the_labels_schema_allows_the_counts_of_each_protocol(
    rule: dict[str, Any], valid: bool
) -> None:
    validator = jsonschema.Draft202012Validator(json.loads(LABELS_SCHEMA_PATH.read_text()))
    errors = list(validator.iter_errors(_doc([rule])))
    assert (not errors) is valid


# Counting rules


def test_a_count_holds_inside_its_range() -> None:
    assert Count(2, 2).holds(2) and not Count(2, 2).holds(1) and not Count(2, 2).holds(3)
    assert Count(1, None).holds(100) and not Count(1, None).holds(0)
    assert [Count(2, 2).text(), Count(1, 3).text(), Count(4, None).text()] == [
        "2",
        "1 to 3",
        "at least 4",
    ]


def test_judge_names_missing_and_surplus_messages() -> None:
    expect = {"requests": Count(2, 2), "commands.EHLO": Count(1, 1), "messages": Count(0, 1)}
    assert judge("x", {"requests": 2, "commands.EHLO": 1, "messages": 1}, expect) == []
    assert judge("http1", {"requests": 1, "commands.EHLO": 3}, expect) == [
        "missing http1 request: found 1, expected 2",
        "surplus http1 command EHLO: found 3, expected 1",
    ]
    assert judge("smtp", {}, {"replies.250": Count(6, 6)}) == [
        "missing smtp reply 250: found 0, expected 6"
    ]


def test_parse_fields_counts_the_occurrences_of_each_field() -> None:
    unit = "\x1f"
    text = (
        f"10.0.0.2\t\t40000\t10.0.0.3\t\t587\tEHLO{unit}auth\t250{unit}250\t\n"
        f"\t2001:db8::2\t40000\t\t2001:db8::3\t587\t\t\t<1@x>\n"
        "10.0.0.3\t\t587\t10.0.0.2\t\t40000\t\t354\t\n"
        "\n"
        "no-address\t\t1\t10.0.0.3\t\t587\tQUIT\t\t\n"
    )
    counts = parse_fields(text, ["commands", "replies", "messages"])
    first = counts[SessionKey.of("tcp", "10.0.0.2", 40000, "10.0.0.3", 587)]
    assert dict(first) == {
        "commands.EHLO": 1,
        "commands.AUTH": 1,
        "replies.250": 2,
        "replies.354": 1,
    }
    assert dict(counts[SessionKey.of("tcp", "2001:db8::2", 40000, "2001:db8::3", 587)]) == {
        "messages": 1
    }
    assert len(counts) == 2


# The check with a stub probe


def _chunk_view(tmp_path: Path, ports: list[int]) -> tuple[Path, list[Session]]:
    sessions = [
        TlsSession(
            index,
            1,
            (CLIENT, 40000 + index),
            (SERVER, port),
            inside=[(True, b"hello\r\n"), (False, b"hi\r\n")],
        )
        for index, port in enumerate(ports)
    ]
    view = tmp_path / "view.pcap"
    write_view(view, sessions)
    return view, summarize(view).sessions


def _counts(
    sessions: list[Session], **by_port: dict[str, int]
) -> dict[SessionKey, Mapping[str, int]]:
    return {s.key: by_port[f"p{s.server_port}"] for s in sessions if f"p{s.server_port}" in by_port}


def test_a_stub_probe_with_the_right_counts_passes(tmp_path: Path) -> None:
    view, sessions = _chunk_view(tmp_path, [587, 587])
    probe = StubMessageProbe(_counts(sessions, p587={"messages": 1}))
    result = messages.check(view, sessions, parse(_doc()), probe)
    assert result.passed, result.failures
    entry = result.to_dict()
    assert entry["sessions_checked"] == 2
    assert entry["rules"][0]["observed_total"] == {"messages": 2}
    assert probe.calls == [(view, "smtp", (587,))]
    _validate_entry(entry)


def test_a_session_without_its_message_fails_with_a_named_reason(tmp_path: Path) -> None:
    view, sessions = _chunk_view(tmp_path, [587, 587, 587])
    counts = _counts(sessions, p587={"messages": 1})
    del counts[sessions[1].key]
    result = messages.check(view, sessions, parse(_doc()), StubMessageProbe(counts))
    assert not result.passed
    assert result.failures == [
        f"session {sessions[1].id} ({CLIENT}:40001 to {SERVER}:587, "
        "messages rule 1 (smtp, server_port 587)): missing smtp message: found 0, expected 1"
    ]
    entry = result.to_dict()
    assert entry["sessions_failed"] == 1
    assert entry["rules"][0]["failed_sessions"] == 1
    assert [s["passed"] for s in entry["sessions"]] == [False, True, True]
    _validate_entry(entry)


def test_a_surplus_message_fails_too(tmp_path: Path) -> None:
    view, sessions = _chunk_view(tmp_path, [587])
    probe = StubMessageProbe(_counts(sessions, p587={"messages": 2}))
    result = messages.check(view, sessions, parse(_doc()), probe)
    assert result.failures[0].endswith("surplus smtp message: found 2, expected 1")


def test_a_range_accepts_every_count_inside_it(tmp_path: Path) -> None:
    view, sessions = _chunk_view(tmp_path, [80, 80, 80])
    doc = _doc([{"protocol": "http1", "expect": {"requests": {"min": 1, "max": 3}}}])
    counts = {s.key: {"requests": n} for s, n in zip(sessions, (1, 3, 4), strict=True)}
    result = messages.check(view, sessions, parse(doc), StubMessageProbe(counts))
    assert len(result.failures) == 1
    assert "surplus http1 request: found 4, expected 1 to 3" in result.failures[0]


def test_a_rule_that_matches_no_session_fails(tmp_path: Path) -> None:
    view, sessions = _chunk_view(tmp_path, [587])
    doc = _doc(
        [
            {"protocol": "smtp", "match": {"server_port": 587}, "expect": {"messages": 1}},
            {"protocol": "imap", "match": {"server_port": 143}, "expect": {"messages": 2}},
        ]
    )
    probe = StubMessageProbe(_counts(sessions, p587={"messages": 1}))
    result = messages.check(view, sessions, parse(doc), probe)
    assert result.failures == [
        "messages rule 2 (imap, server_port 143) matches no TCP session of the capture"
    ]
    assert [call[1] for call in probe.calls] == ["smtp"]


def test_the_first_matching_rule_takes_the_session(tmp_path: Path) -> None:
    view, sessions = _chunk_view(tmp_path, [587, 25])
    doc = _doc(
        [
            {"protocol": "smtp", "match": {"server_port": 587}, "expect": {"messages": 1}},
            {"protocol": "smtp", "expect": {"messages": 5}},
        ]
    )
    counts = {sessions[0].key: {"messages": 1}, sessions[1].key: {"messages": 5}}
    result = messages.check(view, sessions, parse(doc), StubMessageProbe(counts))
    assert result.passed, result.failures
    assert [r.sessions for r in result.rules] == [1, 1]


def test_a_missing_tshark_fails_instead_of_passing(tmp_path: Path) -> None:
    view, sessions = _chunk_view(tmp_path, [587])
    result = messages.check(view, sessions, parse(_doc()), StubMessageProbe(installed=False))
    assert result.failures == ["tshark is not installed, so no message was counted"]
    assert result.to_dict()["tool"] is None


def test_the_report_lists_failed_sessions_first_and_caps_the_lists(tmp_path: Path) -> None:
    view, sessions = _chunk_view(tmp_path, [587] * 130)
    counts = {s.key: {"messages": 1 if s.id > 120 else 0} for s in sessions}
    result = messages.check(view, sessions, parse(_doc()), StubMessageProbe(counts))
    entry = result.to_dict()
    assert entry["failures_total"] == 120
    assert len(entry["failures"]) == messages.MAX_FAILURES
    assert len(entry["sessions"]) == messages.MAX_SESSIONS
    assert all(not s["passed"] for s in entry["sessions"])
    _validate_entry(entry)


def test_describe_names_the_rule_and_its_sessions() -> None:
    labels = parse(_doc([{"protocol": "http1", "expect": {"requests": 1}}]))
    assert labels.messages is not None
    assert describe(labels.messages[0], 3) == "messages rule 3 (http1)"


def _validate_entry(entry: dict[str, Any]) -> None:
    report = {
        "report": "fps-realism-report",
        "version": REPORT_VERSION,
        "generator": "test",
        "passed": entry["passed"],
        "capture": {
            "file": "v.pcap",
            "sha256": "0" * 64,
            "format": "pcap",
            "link_types": [1],
            "frames": 1,
            "skipped_frames": 0,
            "sessions": 1,
        },
        "labels": {"file": "l.json", "sha256": "0" * 64},
        "checks": [entry],
    }
    _validate(report)


def test_score_adds_the_check_when_the_labels_have_the_section(
    tmp_path: Path, labels_file: Any
) -> None:
    view, sessions = _chunk_view(tmp_path, [587])
    probe = StubMessageProbe(_counts(sessions, p587={"messages": 1}))
    classifier = StubClassifier(lambda _session: "SMTP")
    report = score(view, labels_file(_doc()), classifier, message_probe=probe)
    assert [c["check"] for c in report.checks] == ["conformance", "messages"]
    assert report.to_dict()["version"] == REPORT_VERSION == "0.11.0"
    _validate(report.to_dict())
    bare = _doc()
    del bare["messages"]
    assert [c["check"] for c in score(view, labels_file(bare), classifier).checks] == [
        "conformance"
    ]


# The real tshark


def _dialogue_view(tmp_path: Path, dialogues: list[tuple[int, list[bytes]]]) -> Path:
    """Write a view whose sessions alternate client and server chunks, client first."""
    sessions = [
        TlsSession(
            index,
            1,
            (CLIENT, 40000 + index),
            (SERVER, port),
            inside=[(i % 2 == 0, chunk) for i, chunk in enumerate(chunks)],
        )
        for index, (port, chunks) in enumerate(dialogues)
    ]
    view = tmp_path / "dialogues.pcap"
    write_view(view, sessions)
    return view


MAIL = (
    b"From: a@example.test\r\nTo: b@example.test\r\nSubject: hello\r\n"
    b"Date: Mon, 02 Nov 2026 10:00:00 +0000\r\nMessage-ID: <1@example.test>\r\n\r\nbody\r\n"
)
SMTP = [
    b"EHLO c\r\n",
    b"250 ok\r\n",
    b"MAIL FROM:<a@example.test>\r\n",
    b"250 ok\r\n",
    b"RCPT TO:<b@example.test>\r\n",
    b"250 ok\r\n",
    b"DATA\r\n",
    b"354 go\r\n",
    MAIL + b".\r\n",
    b"250 queued\r\n",
    b"QUIT\r\n",
    b"221 bye\r\n",
]
HTTP = [
    b"GET /a HTTP/1.1\r\nHost: h\r\n\r\n",
    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi",
    b"GET /b HTTP/1.1\r\nHost: h\r\n\r\n",
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
]
IMAP = [
    b"a1 LOGIN u p\r\n",
    b"a1 OK logged in\r\n",
    b"a2 SELECT INBOX\r\n",
    b"a2 OK selected\r\n",
    b"a3 FETCH 1 BODY[]\r\n",
    b"* 1 FETCH (BODY[] {%d}\r\n" % len(MAIL) + MAIL + b")\r\na3 OK fetched\r\n",
]


def _real_check(view: Path, rules: list[dict[str, Any]]) -> messages.MessagesResult:
    labels: Labels = parse(_doc(rules))
    sessions = summarize(view).sessions
    return messages.check(view, sessions, labels, TsharkMessageProbe())


@needs_tshark
def test_tshark_counts_http1_smtp_and_imap_messages(tmp_path: Path) -> None:
    view = _dialogue_view(tmp_path, [(80, HTTP), (587, SMTP), (143, IMAP)])
    result = _real_check(
        view,
        [
            {
                "protocol": "http1",
                "match": {"server_port": 80},
                "expect": {"requests": 2, "responses": 2},
            },
            {
                "protocol": "smtp",
                "match": {"server_port": 587},
                "expect": {
                    "commands": {"EHLO": 1, "MAIL": 1, "RCPT": 1, "DATA": 1, "QUIT": 1},
                    "replies": {250: 4, 354: 1, 221: 1},
                    "messages": 1,
                },
            },
            {
                "protocol": "imap",
                "match": {"server_port": 143},
                "expect": {
                    "commands": {"LOGIN": 1, "SELECT": 1, "FETCH": 1},
                    "replies": {"OK": 3},
                    "messages": 1,
                },
            },
        ],
    )
    assert result.passed, result.failures
    assert result.tool is not None and result.tool["tool"] == "tshark"


@needs_tshark
def test_a_missing_http_response_fails_with_a_named_reason(tmp_path: Path) -> None:
    view = _dialogue_view(tmp_path, [(80, HTTP), (80, HTTP[:3]), (80, HTTP)])
    result = _real_check(
        view,
        [{"protocol": "http1", "expect": {"requests": 2, "responses": 2}}],
    )
    assert len(result.failures) == 1
    assert "missing http1 response: found 1, expected 2" in result.failures[0]
    assert result.rules[0].failed == 1


@needs_tshark
def test_an_smtp_session_without_its_message_fails(tmp_path: Path) -> None:
    without = [*SMTP[:8], b"QUIT\r\n", b"221 bye\r\n"]
    view = _dialogue_view(tmp_path, [(587, SMTP), (587, without)])
    result = _real_check(
        view,
        [{"protocol": "smtp", "expect": {"messages": 1, "commands": {"DATA": 1}}}],
    )
    assert len(result.failures) == 1
    assert "missing smtp message: found 0, expected 1" in result.failures[0]


@needs_tshark
def test_an_imap_session_without_its_literal_fails(tmp_path: Path) -> None:
    without = [*IMAP[:5], b"a3 OK fetched\r\n"]
    view = _dialogue_view(tmp_path, [(143, IMAP), (143, without)])
    result = _real_check(view, [{"protocol": "imap", "expect": {"messages": 1}}])
    assert len(result.failures) == 1
    assert "missing imap message: found 0, expected 1" in result.failures[0]


# The committed fixtures, whose views the example labels describe.

FIXTURE_CASES = [
    ("http1", "inside", None),
    ("http2", "inside", None),
    ("smtp", "upgrade", 587),
    ("imap", "inside", 143),
]


def _fixture_view(
    tmp_path: Path, module: str, plaintext: str, view_port: int | None, truncate: bool
) -> tuple[Path, Labels, list[Session]]:
    capture = ROOT / "modules" / module / "fixtures" / f"{module}.pcapng"
    if not capture.is_file():
        pytest.skip(f"{capture} is not in this checkout")
    tls = [s for s in read_sessions(capture, None, Tshark(), plaintext) if s.decrypted]
    assert tls, f"no TLS session of {module} decrypts"
    if truncate:
        # The session in the middle stops halfway, so it loses its later messages.
        victim = tls[len(tls) // 2]
        victim.inside = victim.inside[: len(victim.inside) // 2]
    view = tmp_path / f"{module}.decrypted.pcap"
    write_view(view, tls, plaintext, view_port)
    return view, load(EXAMPLES / f"{module}.labels.yaml"), summarize(view).sessions


@needs_tshark
@pytest.mark.parametrize(("module", "plaintext", "view_port"), FIXTURE_CASES)
def test_the_committed_fixture_passes_with_the_literal_counts(
    tmp_path: Path, module: str, plaintext: str, view_port: int | None
) -> None:
    view, labels, sessions = _fixture_view(tmp_path, module, plaintext, view_port, False)
    result = messages.check(view, sessions, labels, TsharkMessageProbe())
    assert result.passed, result.failures
    assert result.to_dict()["sessions_checked"] == len(sessions)
    assert all(r.sessions > 0 for r in result.rules)


@needs_tshark
@pytest.mark.parametrize(("module", "plaintext", "view_port"), FIXTURE_CASES)
def test_a_fixture_view_that_lost_a_message_fails_with_a_named_reason(
    tmp_path: Path, module: str, plaintext: str, view_port: int | None
) -> None:
    view, labels, sessions = _fixture_view(tmp_path, module, plaintext, view_port, True)
    result = messages.check(view, sessions, labels, TsharkMessageProbe())
    assert not result.passed
    assert result.to_dict()["sessions_failed"] == 1
    assert all(" missing " in failure for failure in result.failures), result.failures
    assert all(failure.startswith("session ") for failure in result.failures)


@needs_tshark
def test_the_decrypted_command_names_the_failed_count(tmp_path: Path) -> None:
    capture = ROOT / "modules" / "imap" / "fixtures" / "imap.pcapng"
    if not capture.is_file():
        pytest.skip(f"{capture} is not in this checkout")
    text = (EXAMPLES / "imap.labels.yaml").read_text()
    classifier = StubClassifier(lambda _session: "IMAP", name="ndpi")

    def run(labels_text: str) -> decrypted.DecryptedResult:
        path = tmp_path / "imap.labels.yaml"
        path.write_text(labels_text)
        return decrypted.run(
            capture,
            path,
            classifier,
            tmp_path,
            tshark=Tshark(),
            view_port=143,
            expected_sessions=8,
        )

    assert run(text).failures == []
    broken = run(text.replace("messages: 2", "messages: 3"))
    assert broken.failures[0] == "the view fails messages"
    assert len(broken.failures) == 1 + decrypted.MAX_LISTED_FAILURES
    assert broken.failures[1].startswith("messages: session ")
    assert "missing imap message: found 2, expected 3" in broken.failures[1]
    entry = next(c for c in broken.report["checks"] if c["check"] == "messages")
    assert entry["failures_total"] == 8
