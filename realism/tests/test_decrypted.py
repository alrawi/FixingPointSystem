# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the decrypted view: parsing and framing without tools, real tshark for the rest.

The real tests generate TLS captures with the Python ``ssl`` module, decrypt them
with the installed tshark, and score the view with a fake ndpiReader. They skip
where tshark or openssl is missing, and the rlm workflow installs both.
"""

from __future__ import annotations

import json
import shutil
import ssl
import subprocess
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from realism import decrypted
from realism.capture import summarize
from realism.cli import EXIT_ERROR, EXIT_FAIL, EXIT_PASS, main
from realism.decrypted import (
    DecryptedError,
    TlsSession,
    Tshark,
    drop_starttls,
    parse_before,
    parse_follow,
    parse_hellos,
    write_view,
)
from realism.tests import pcapgen, tlsgen
from realism.tests.conftest import NDPI_42_HEADER, NDPI_414_HEADER

TSHARK = Tshark()
HAS_TSHARK = shutil.which(TSHARK.program) is not None
needs_tls_tools = pytest.mark.skipif(
    not HAS_TSHARK or not tlsgen.HAS_OPENSSL, reason="no tshark or no openssl"
)


def _labels(
    port: int, protocol: str = "HTTP", doc_extra: dict[str, Any] | None = None
) -> dict[str, Any]:
    return {
        "labels": "fps-realism-labels",
        "version": 1,
        "sessions": [{"application": "inner", "match": {"transport": "tcp", "server_port": port}}],
        "applications": {"inner": {"ndpi": [protocol]}},
        **(doc_extra or {}),
    }


def test_drop_starttls_removes_the_command_and_its_reply() -> None:
    chunks = [
        (False, b"220 mail ESMTP\r\n"),
        (True, b"EHLO client\r\n"),
        (False, b"250-STARTTLS\r\n250 OK\r\n"),
        (True, b"STARTTLS\r\n"),
        (False, b"220 Ready to start TLS\r\n"),
    ]
    assert drop_starttls(chunks) == chunks[:3]


@pytest.mark.parametrize("command", [b"a001 STARTTLS\r\n", b"starttls\r\n", b"STLS\r\n"])
def test_drop_starttls_knows_the_tagged_and_pop3_forms(command: bytes) -> None:
    chunks = [(False, b"* OK hi\r\n"), (True, command), (False, b"OK begin\r\n")]
    assert drop_starttls(chunks) == chunks[:1]


def test_drop_starttls_keeps_a_line_that_only_contains_the_word() -> None:
    chunks = [(True, b"NOOP STARTTLS now\r\n"), (False, b"OK\r\n")]
    assert drop_starttls(chunks) == chunks


def test_parse_hellos_names_sessions_by_stream_and_takes_the_first_hello() -> None:
    text = (
        "3\t11\t10.0.0.10\t\t40000\t10.0.1.43\t\t443\n"
        "3\t19\t10.0.0.10\t\t40000\t10.0.1.43\t\t443\n"
        "4\t30\t\t2001:db8::1\t41000\t\t2001:db8::43\t5587\n"
    )
    sessions = parse_hellos(text)
    assert sorted(sessions) == [3, 4]
    assert sessions[3].hello_frame == 11
    assert sessions[4].client == ("2001:db8::1", 41000)
    assert sessions[4].server == ("2001:db8::43", 5587)


@pytest.mark.parametrize("text", ["1\t2\t3\n", "x\t2\t10.0.0.1\t\t1\t10.0.0.2\t\t2\n"])
def test_parse_hellos_rejects_nonsense(text: str) -> None:
    with pytest.raises(DecryptedError):
        parse_hellos(text)


def test_parse_before_keeps_frames_ahead_of_the_hello_and_drops_retransmissions() -> None:
    session = TlsSession(0, 10, ("10.0.0.10", 40000), ("10.0.1.43", 5587))
    text = (
        "4\t0\t10.0.1.43\t\t5587\t5001\t3232300d0a\n"
        "5\t0\t10.0.0.10\t\t40000\t1001\t53544152540d0a\n"
        "6\t0\t10.0.0.10\t\t40000\t1001\t53544152540d0a\n"
        "10\t0\t10.0.0.10\t\t40000\t1010\t16:03:01\n"
        "7\t9\t10.0.0.10\t\t40000\t1001\t00\n"
        "8\t0\t10.0.0.10\t\t40000\t1020\t\n"
    )
    parse_before(text, {0: session})
    assert session.before == [(False, b"220\r\n"), (True, b"START\r\n")]


def test_parse_follow_reads_the_client_side_from_the_node_lines() -> None:
    sessions = {
        0: TlsSession(0, 1, ("10.0.0.10", 40000), ("10.0.1.43", 443)),
        1: TlsSession(1, 1, ("10.0.0.11", 40001), ("10.0.1.43", 443)),
    }
    text = (
        "Filter: tcp.stream eq 0\n"
        "Node 0: 10.0.1.43:443\n"
        "Node 1: 10.0.0.10:40000\n"
        "\t4745\n"
        "6869\n"
        "===================================================================\n"
        "Filter: tcp.stream eq 1\n"
        "Node 0: 10.0.0.11:40001\n"
        "Node 1: 10.0.1.43:443\n"
        "6161\n"
        "\t6262\n"
    )
    parse_follow(text, sessions)
    assert sessions[0].inside == [(True, b"GE"), (False, b"hi")]
    assert sessions[1].inside == [(True, b"aa"), (False, b"bb")]


def test_a_session_needs_plaintext_from_both_sides_to_count_as_decrypted() -> None:
    session = TlsSession(0, 1, ("10.0.0.10", 40000), ("10.0.1.43", 443))
    assert not session.decrypted
    session.inside = [(True, b"a")]
    assert not session.decrypted
    session.inside.append((False, b"b"))
    assert session.decrypted


def test_the_plaintext_choice_decides_what_the_view_carries() -> None:
    session = TlsSession(0, 1, ("10.0.0.10", 40000), ("10.0.1.43", 443))
    session.before = [(False, b"220 hi\r\n"), (True, b"STARTTLS\r\n"), (False, b"220 go\r\n")]
    session.inside = [(True, b"EHLO\r\n"), (False, b"250 OK\r\n")]
    assert session.chunks("inside") == session.inside
    assert session.chunks("upgrade") == [(False, b"220 hi\r\n"), *session.inside]


@pytest.mark.parametrize(
    ("client", "server"),
    [(("10.0.0.10", 40000), ("10.0.1.43", 443)), (("2001:db8::10", 40000), ("2001:db8::43", 443))],
)
def test_the_view_is_a_plain_tcp_capture_with_the_addresses_of_the_sessions(
    tmp_path: Path, client: tuple[str, int], server: tuple[str, int]
) -> None:
    sessions = [
        TlsSession(0, 1, client, server, inside=[(True, b"a" * 3000), (False, b"b")]),
        TlsSession(1, 1, (client[0], 40001), server, inside=[(True, b"c"), (False, b"d")]),
    ]
    view = tmp_path / "view.pcap"
    frames = write_view(view, sessions, view_port=587)
    summary = summarize(view)
    assert frames == summary.frames == (3 + 4 + 2) + (3 + 2 + 2)
    assert summary.skipped_frames == 0
    assert [(s.client_ip, s.client_port, s.server_ip, s.server_port) for s in summary.sessions] == [
        (client[0], 40000, server[0], 587),
        (client[0], 40001, server[0], 587),
    ]
    assert [s.payload_bytes for s in summary.sessions] == [3001, 2]


def test_the_view_keeps_the_server_port_by_default(tmp_path: Path) -> None:
    session = TlsSession(0, 1, ("10.0.0.10", 40000), ("10.0.1.43", 5587))
    session.inside = [(True, b"a"), (False, b"b")]
    write_view(tmp_path / "v.pcap", [session])
    assert summarize(tmp_path / "v.pcap").sessions[0].server_port == 5587


def test_mixed_ip_versions_are_refused(tmp_path: Path) -> None:
    session = TlsSession(0, 1, ("10.0.0.10", 40000), ("2001:db8::43", 443))
    session.inside = [(True, b"a")]
    with pytest.raises(DecryptedError, match="mixes IP versions"):
        write_view(tmp_path / "v.pcap", [session])


def test_a_missing_tshark_is_reported(tmp_path: Path) -> None:
    with pytest.raises(DecryptedError, match="not found"):
        Tshark(str(tmp_path / "no-such-tshark")).run(tmp_path / "x.pcap", None, [])


# The tests below run the real tshark.

CLIENT = tlsgen.ClientConfig()
TLS12 = tlsgen.ClientConfig(max_version=ssl.TLSVersion.TLSv1_2)


def _http_view_csv(count: int, port: int = 443, proto: str = "HTTP") -> str:
    rows = [
        f"{i},6,1,1,0,10.0.0.10,{40000 + i},10.0.1.43,{port},7,{proto},,6,414" for i in range(count)
    ]
    return NDPI_42_HEADER + "\n" + "\n".join(rows) + "\n"


def _args(tmp_path: Path, pcap: Path, labels: Path, reader: Path, *extra: str) -> list[str]:
    return [
        "decrypted",
        "--pcap",
        str(pcap),
        "--labels",
        str(labels),
        "--ndpi-reader",
        str(reader),
        "--output",
        str(tmp_path / "result.json"),
        *extra,
    ]


@needs_tls_tools
def test_the_view_of_a_tls_capture_scores_as_http(
    tmp_path: Path, labels_file: Callable[..., Path], fake_reader: Callable[..., Path]
) -> None:
    cap, _ = tlsgen.tls_capture(tmp_path, [CLIENT, TLS12, CLIENT])
    pcap = cap.write_pcapng(tmp_path / "tls.pcapng", tls_keys=(tmp_path / "keys.log").read_bytes())
    reader = fake_reader(_http_view_csv(3))
    code = main(
        _args(
            tmp_path,
            pcap,
            labels_file(_labels(443)),
            reader,
            "--parsers",
            "tshark",
            "--view",
            str(tmp_path / "view.pcap"),
            "--sessions",
            "3",
        )
    )
    result = json.loads((tmp_path / "result.json").read_text())
    assert code == EXIT_PASS, result["failures"]
    assert result["tls_sessions"] == result["decrypted_sessions"] == 3
    assert result["server_ports"] == {"443": 3}
    assert result["plaintext"] == "inside"
    assert result["classifier"] == "ndpi 9.8.7-fake"
    # The view holds the HTTP of the TLS sessions in the clear.
    text = subprocess.run(
        [
            TSHARK.program,
            "-d",
            "tcp.port==443,http",
            "-r",
            str(tmp_path / "view.pcap"),
            "-Y",
            "http.request",
            "-T",
            "fields",
            "-e",
            "http.request.uri",
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert text.split() == ["/fps"] * 3
    assert [c["check"] for c in result["report"]["checks"]] == ["conformance", "parser_errors"]


@needs_tls_tools
def test_a_capture_without_keys_fails_because_nothing_decrypts(
    tmp_path: Path, labels_file: Callable[..., Path], fake_reader: Callable[..., Path]
) -> None:
    cap, _ = tlsgen.tls_capture(tmp_path, [CLIENT, CLIENT])
    pcap = cap.write_pcapng(tmp_path / "nokeys.pcapng")
    reader = fake_reader(_http_view_csv(0))
    code = main(_args(tmp_path, pcap, labels_file(_labels(443)), reader, "--parsers", "none"))
    result = json.loads((tmp_path / "result.json").read_text())
    assert code == EXIT_FAIL
    assert result["tls_sessions"] == 2
    assert result["decrypted_sessions"] == 0
    assert "only 0 of 2 TLS sessions decrypt" in result["failures"]


@needs_tls_tools
def test_a_key_log_file_decrypts_a_capture_without_embedded_keys(
    tmp_path: Path, labels_file: Callable[..., Path], fake_reader: Callable[..., Path]
) -> None:
    cap, keylog = tlsgen.tls_capture(tmp_path, [CLIENT, CLIENT])
    pcap = cap.write_pcap(tmp_path / "side.pcap")
    reader = fake_reader(_http_view_csv(2))
    code = main(
        _args(
            tmp_path,
            pcap,
            labels_file(_labels(443)),
            reader,
            "--parsers",
            "none",
            "--keylog",
            str(keylog),
        )
    )
    assert code == EXIT_PASS


@needs_tls_tools
def test_the_expected_session_count_is_enforced(
    tmp_path: Path, labels_file: Callable[..., Path], fake_reader: Callable[..., Path]
) -> None:
    cap, _ = tlsgen.tls_capture(tmp_path, [CLIENT, CLIENT])
    pcap = cap.write_pcapng(tmp_path / "two.pcapng", tls_keys=(tmp_path / "keys.log").read_bytes())
    reader = fake_reader(_http_view_csv(2))
    code = main(
        _args(
            tmp_path,
            pcap,
            labels_file(_labels(443)),
            reader,
            "--parsers",
            "none",
            "--sessions",
            "8",
        )
    )
    result = json.loads((tmp_path / "result.json").read_text())
    assert code == EXIT_FAIL
    assert result["failures"] == ["the capture has 2 TLS sessions, expected 8"]


@needs_tls_tools
def test_a_nonconforming_view_fails_the_command(
    tmp_path: Path, labels_file: Callable[..., Path], fake_reader: Callable[..., Path]
) -> None:
    cap, _ = tlsgen.tls_capture(tmp_path, [CLIENT])
    pcap = cap.write_pcapng(tmp_path / "one.pcapng", tls_keys=(tmp_path / "keys.log").read_bytes())
    reader = fake_reader(_http_view_csv(1, proto="TLS"))
    code = main(_args(tmp_path, pcap, labels_file(_labels(443)), reader, "--parsers", "none"))
    result = json.loads((tmp_path / "result.json").read_text())
    assert code == EXIT_FAIL
    assert result["decrypted_sessions"] == 1
    assert result["failures"] == ["the view fails conformance"]


@needs_tls_tools
def test_the_upgrade_view_joins_the_plain_start_of_a_starttls_dialogue(
    tmp_path: Path, labels_file: Callable[..., Path], fake_reader: Callable[..., Path]
) -> None:
    cert, key = tlsgen.make_certificate(tmp_path)
    keylog = tmp_path / "keys.log"
    keylog.write_text("")
    cap = pcapgen.Capture()
    plain = [
        (False, b"220 mail.example.test ESMTP\r\n"),
        (True, b"EHLO client\r\n"),
        (False, b"250-mail.example.test\r\n250 STARTTLS\r\n"),
        (True, b"STARTTLS\r\n"),
        (False, b"220 Ready to start TLS\r\n"),
    ]
    for i in range(2):
        # Port 587 is an SMTP port of tshark. On a port that it does not know, tshark 4.2
        # misses the first TLS session of a capture whose ClientHello it must find by heuristic.
        flights = tlsgen.handshake(cert, key, keylog, TLS12)
        tlsgen.add_session(cap, ("10.0.0.10", 40000 + i), ("10.0.1.43", 587), plain + flights)
    pcap = cap.write_pcapng(tmp_path / "starttls.pcapng", tls_keys=keylog.read_bytes())
    reader = fake_reader(_http_view_csv(2, port=25, proto="SMTP"))
    view = tmp_path / "view.pcap"
    code = main(
        _args(
            tmp_path,
            pcap,
            labels_file(_labels(25, "SMTP")),
            reader,
            "--parsers",
            "none",
            "--plaintext",
            "upgrade",
            "--view-port",
            "25",
            "--view",
            str(view),
        )
    )
    result = json.loads((tmp_path / "result.json").read_text())
    assert code == EXIT_PASS, result["failures"]
    assert result["server_ports"] == {"587": 2}
    stream = subprocess.run(
        [TSHARK.program, "-r", str(view), "-q", "-z", "follow,tcp,ascii,0"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    assert "220 mail.example.test ESMTP" in stream
    assert "EHLO client" in stream
    assert "STARTTLS\r\n" not in stream.replace("250 STARTTLS", "")
    assert "Ready to start TLS" not in stream
    assert "GET /fps" in stream


def test_the_command_reports_a_missing_labels_file_and_tool(tmp_path: Path) -> None:
    pcap = pcapgen.mixed_capture(1, 1).write_pcap(tmp_path / "x.pcap")
    code = main(["decrypted", "--pcap", str(pcap), "--labels", str(tmp_path / "none.yaml")])
    assert code == EXIT_ERROR
    labels = tmp_path / "labels.json"
    labels.write_text(json.dumps(_labels(443)))
    code = main(
        [
            "decrypted",
            "--pcap",
            str(pcap),
            "--labels",
            str(labels),
            "--tshark",
            str(tmp_path / "no-such-tshark"),
        ]
    )
    assert code == EXIT_ERROR


def test_the_command_reads_the_pipe_csv_of_ndpi_414(
    tmp_path: Path,
    labels_file: Callable[..., Path],
    fake_reader: Callable[..., Path],
) -> None:
    # A fake tshark that finds no TLS session fails the command before nDPI runs, so
    # this test only checks that the dialect of the reader does not matter to it.
    fake_tshark = tmp_path / "fake-tshark"
    fake_tshark.write_text("#!/bin/sh\nexit 0\n")
    fake_tshark.chmod(0o755)
    csv = NDPI_414_HEADER + "\n"
    reader = fake_reader(csv, help_exit=1)
    pcap = pcapgen.mixed_capture(1, 1).write_pcap(tmp_path / "x.pcap")
    code = main(
        _args(
            tmp_path,
            pcap,
            labels_file(_labels(443)),
            reader,
            "--parsers",
            "none",
            "--tshark",
            str(fake_tshark),
        )
    )
    result = json.loads((tmp_path / "result.json").read_text())
    assert code == EXIT_FAIL
    assert result["failures"][0] == "the capture has no TLS session"
    assert result["classifier"] == "ndpi 9.8.7-fake"
    assert decrypted.PLAINTEXTS == ("inside", "upgrade")
