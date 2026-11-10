# SPDX-License-Identifier: GPL-3.0-only
"""Scenario generation from sessions: validation, byte counts, and the command line."""

from __future__ import annotations

import json
import random
import struct
from pathlib import Path
from typing import Any

import pytest

from capture.cli import main
from capture.templates import render

from .flowsim import run_flow, run_scenario
from .scenario_helpers import (
    generate,
    scenario_docs,
    sessions_doc,
    stream_bytes,
    tcp_session,
    udp_session,
    validation_errors,
)

BODY = ("<html>" + "lorem ipsum dolor sit amet " * 40 + "</html>").encode()
REQ1 = (
    b"GET /index.html HTTP/1.1\r\nHost: www.example.test\r\nUser-Agent: test/1.0\r\n"
    b"X-Request-Id: 7f3a9c2e41b8d605\r\nAccept: */*\r\n\r\n"
)
RESP1 = (
    b"HTTP/1.1 200 OK\r\nDate: Tue, 14 Nov 2023 22:13:25 GMT\r\nServer: x\r\n"
    b"Set-Cookie: sid=9d41c7e2b0a35f68; Path=/; HttpOnly\r\nX-Request-Id: 7f3a9c2e41b8d605\r\n"
    b"Content-Type: text/html\r\nContent-Length: %d\r\n\r\n" % len(BODY)
) + BODY
REQ2 = (
    b"GET /next HTTP/1.1\r\nHost: www.example.test\r\nCookie: lang=en; sid=9d41c7e2b0a35f68\r\n"
    b"X-Time: 1700000012\r\n\r\n"
)
RESP2 = (
    b"HTTP/1.1 204 No Content\r\nDate: Tue, 14 Nov 2023 22:13:25 GMT\r\nContent-Length: 0\r\n\r\n"
)
HTTP_TURNS = [("c", REQ1), ("s", RESP1), ("c", REQ2), ("s", RESP2)]


def within_one_percent(got: int, want: int) -> bool:
    return abs(got - want) <= 0.01 * want


def check_counts(session: dict[str, Any], result: dict[str, dict[str, Any]]) -> None:
    """Each side sent as many bytes as the capture shows, within 1%."""
    up = session["client_to_server"]["payload_bytes"]
    down = session["server_to_client"]["payload_bytes"]
    assert within_one_percent(result["client"]["sent"], up)
    assert within_one_percent(result["server"]["sent"], down)
    assert result["client"]["received"] == result["server"]["sent"]
    assert result["server"]["received"] == result["client"]["sent"]


def test_http_session_validates_and_replays_with_the_captured_byte_counts(tmp_path: Path) -> None:
    doc, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS))
    module = generate(tmp_path, path)
    assert validation_errors(module) == []
    (scenario,) = scenario_docs(module).values()
    session = doc["sessions"][0]
    for seed in range(5):
        (result,) = run_scenario(scenario, seed)
        check_counts(session, result)
        # the byte counts are exact, because every replacement keeps its length
        assert result["client"]["sent"] == session["client_to_server"]["payload_bytes"]
        client_vars, server_vars = result["client"]["vars"], result["server"]["vars"]
        # the request id that the client made reached the server, and the cookie that the
        # server made reached the client, which sent it back
        assert client_vars["id"] == server_vars["id"]
        assert client_vars["cookie_sid"] == server_vars["cookie_sid"]


def test_http_fields_become_template_expressions(tmp_path: Path) -> None:
    _, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS))
    module = generate(tmp_path, path)
    text = (module / "scenarios" / "session_1.yaml").read_text()
    assert "Content-Length: {{ vars.down_len }}" in text and "fill(vars.down_len)" in text
    assert "Date: {{ vars.http_date }}" in text and "http_date:" in text
    assert "sid={{ vars.cookie_sid }}" in text and "7f3a9c2e41b8d605" not in text
    assert "9d41c7e2b0a35f68" not in text and "X-Time: {{ rand_int(" in text
    assert "lorem ipsum" not in text  # the body is filler now
    client = scenario_docs(module)["session_1"]["flows"][0]["client"]
    assert {"set": {"cookie_sid": "{{ last }}"}} in client  # the echo is read from the response


def test_literal_scenario_replays_the_exact_bytes(tmp_path: Path) -> None:
    doc, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS))
    module = generate(tmp_path, path, "--no-fields")
    assert validation_errors(module) == []
    (scenario,) = scenario_docs(module).values()
    flow = scenario["flows"][0]
    sent = {
        side: b"".join(render(a["send"]) for a in flow[side] if "send" in a)
        for side in ("client", "server")
    }
    session = doc["sessions"][0]
    assert sent["client"] == stream_bytes(session, "client_to_server")
    assert sent["server"] == stream_bytes(session, "server_to_client")
    (result,) = run_scenario(scenario)
    check_counts(session, result)


def test_expects_use_delimiters_where_they_are_safe_and_lengths_elsewhere(tmp_path: Path) -> None:
    _, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS))
    module = generate(tmp_path, path, "--no-fields")
    server = scenario_docs(module)["session_1"]["flows"][0]["server"]
    expects = [a["expect"] for a in server if "expect" in a]
    assert expects[0] == {"pattern": "\r\n\r\n"}  # a request head ends at the first blank line
    client = scenario_docs(module)["session_1"]["flows"][0]["client"]
    sizes = [a["expect"] for a in client if "expect" in a]
    assert sizes[1] == {"bytes": len(BODY)}  # a body is read by its length


def test_line_protocol_uses_the_line_delimiter_only_when_it_ends_the_turn(tmp_path: Path) -> None:
    turns = [
        ("s", b"220 mail ready\r\n"),
        ("c", b"EHLO a.test\r\n"),
        ("s", b"250-mail\r\n250 PIPELINING\r\n"),  # the first CRLF is not the end
        ("c", b"QUIT\r\n"),
    ]
    doc, path = sessions_doc(tmp_path, tcp_session(turns, dport=25))
    module = generate(tmp_path, path)
    assert validation_errors(module) == []
    flow = scenario_docs(module)["session_1"]["flows"][0]
    assert [a["expect"] for a in flow["client"] if "expect" in a] == [
        {"pattern": "\r\n"},
        {"bytes": 26},
    ]
    (result,) = run_scenario(scenario_docs(module)["session_1"])
    check_counts(doc["sessions"][0], result)


def test_binary_frames_replay_with_fill_and_exact_sizes(tmp_path: Path) -> None:
    rng = random.Random(7)
    payload = bytes(rng.randrange(256) for _ in range(900))
    frame = b"\x17\x03\x03" + struct.pack(">H", len(payload)) + payload
    hello = b"\x16\x03\x01\x00\x10" + bytes(range(16))
    doc, path = sessions_doc(
        tmp_path, tcp_session([("c", hello), ("s", hello), ("c", frame), ("s", frame)])
    )
    module = generate(tmp_path, path)
    assert validation_errors(module) == []
    scenario = scenario_docs(module)["session_1"]
    text = (module / "scenarios" / "session_1.yaml").read_text()
    assert "fill(900)" in text and "{{ hex('1703030384') }}" in text
    (result,) = run_scenario(scenario)
    check_counts(doc["sessions"][0], result)


def test_udp_flow_with_a_token_that_both_sides_use(tmp_path: Path) -> None:
    token = b"a91f03c7d2e5"
    turns = [
        ("c", b"QUERY " + token + b" host.test\r\n"),
        ("s", b"ANSWER " + token + b" 192.0.2.1\r\n"),
        ("c", b"PING\r\n"),
    ]
    doc, path = sessions_doc(tmp_path, udp_session(turns))
    module = generate(tmp_path, path)
    assert validation_errors(module) == []
    scenario = scenario_docs(module)["session_1"]
    flow = scenario["flows"][0]
    assert flow["l4"] == "udp"
    # a datagram is consumed whole, so the other side cannot read the token: it is a
    # scenario variable that both sides share
    assert scenario["vars"]["id"] == token.decode()
    assert next(a["expect"] for a in flow["server"] if "expect" in a) == {"bytes": 30}
    (result,) = run_scenario(scenario)
    check_counts(doc["sessions"][0], result)


def test_udp_token_used_by_one_side_is_random_per_session(tmp_path: Path) -> None:
    token = b"a91f03c7d2e5b6c4"
    turns = [("c", b"A " + token + b"\r\n"), ("s", b"OK\r\n"), ("c", b"B " + token + b"\r\n")]
    _, path = sessions_doc(tmp_path, udp_session(turns))
    module = generate(tmp_path, path)
    scenario = scenario_docs(module)["session_1"]
    assert "id" not in scenario.get("vars", {})
    client = scenario["flows"][0]["client"]
    assert next(a for a in client if "set" in a)["set"]["id"].startswith("{{ rand_int(")
    assert validation_errors(module) == []


def test_sessions_combine_into_one_scenario_that_carries_the_cookie(tmp_path: Path) -> None:
    first = [
        ("c", b"GET / HTTP/1.1\r\nHost: a.test\r\n\r\n"),
        ("s", b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=9d41c7e2b0a35f68\r\nContent-Length: 0\r\n\r\n"),
    ]
    second = [
        ("c", b"GET /next HTTP/1.1\r\nCookie: sid=9d41c7e2b0a35f68\r\n\r\n"),
        ("s", b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n"),
    ]
    recs = tcp_session(first, sport=40000) + tcp_session(second, sport=40001, t0=1_700_000_001.0)
    doc, path = sessions_doc(tmp_path, recs)
    module = generate(tmp_path, path, "--combine", "browse")
    assert validation_errors(module) == []
    scenario = scenario_docs(module)["browse"]
    assert [f["id"] for f in scenario["flows"]] == ["s1", "s2"]
    assert scenario["flows"][1]["spawned"] is True and "spawned" not in scenario["flows"][0]
    results = run_scenario(scenario)
    assert len(results) == 2
    # the second flow's client sends the cookie that the first flow's client read
    assert results[0]["client"]["vars"]["cookie_sid"] == results[1]["client"]["vars"]["cookie_sid"]
    for r, s in zip(results, doc["sessions"], strict=True):
        check_counts(s, r)


def test_one_scenario_per_session_by_default_and_selection(tmp_path: Path) -> None:
    recs = tcp_session(HTTP_TURNS, sport=40000) + tcp_session(
        HTTP_TURNS, sport=40001, t0=1_700_000_005.0
    )
    _, path = sessions_doc(tmp_path, recs)
    both = generate(tmp_path, path, module="both")
    assert sorted(scenario_docs(both)) == ["session_1", "session_2"]
    only = generate(tmp_path, path, "--session", "2", module="only")
    assert sorted(scenario_docs(only)) == ["session_2"]
    manifest = (only / "manifest.yaml").read_text()
    assert "scenarios/session_2.yaml" in manifest and "expected_bytes:" in manifest


def test_close_actions_follow_the_session_end(tmp_path: Path) -> None:
    from capture.tests.pcapgen import ACK, PSH, RST, Rec, tcp_frame
    from capture.tests.scenario_helpers import C, S

    recs = tcp_session([("c", b"hello\r\n"), ("s", b"world\r\n")])[:-3]
    t = recs[-1].t + 0.01
    recs.append(
        Rec(
            t,
            tcp_frame(
                S,
                C,
                8080,
                40000,
                seq=5000 + 40000 + 1 + 7,
                ack=1000 + 40000 + 1 + 7,
                flags=RST | ACK | PSH,
            ),
        )
    )
    _, path = sessions_doc(tmp_path, recs)
    module = generate(tmp_path, path)
    assert validation_errors(module) == []
    flow = scenario_docs(module)["session_1"]["flows"][0]
    assert flow["server"][-1] == {"close": "rst"} and flow["client"][-1] == {
        "expect": {"pattern": "\r\n"}
    }


def test_keep_timing_adds_delays_and_longer_timeouts(tmp_path: Path) -> None:
    _, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS, pause=0.5))
    module = generate(tmp_path, path, "--keep-timing")
    assert validation_errors(module) == []
    flow = scenario_docs(module)["session_1"]["flows"][0]
    delays = [a["delay"]["ms"] for a in flow["client"] + flow["server"] if "delay" in a]
    assert delays and all(450 <= d <= 550 for d in delays)
    assert any(a["expect"].get("timeout_ms", 0) > 10_000 for a in flow["client"] if "expect" in a)
    plain = generate(tmp_path, path, module="plain")
    assert "delay" not in (plain / "scenarios" / "session_1.yaml").read_text()


def test_a_template_that_changes_the_length_switches_detection_off(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The generator renders every template and compares sizes. A heuristic that gets it
    wrong must fall back to the literal payload."""
    import capture.fields as fields

    monkeypatch.setattr(fields, "_digit_chunks", lambda n: ["{{ rand_int(1, 9) }}"])
    _, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS))
    module = generate(tmp_path, path)
    scenario = scenario_docs(module)["session_1"]
    assert validation_errors(module) == []
    assert "rand_int" not in json.dumps(scenario)
    (result,) = run_scenario(scenario)
    assert result["client"]["sent"] == len(REQ1) + len(REQ2)


# -- provenance and the command line ------------------------------------------------


def test_manifest_without_consent_fails_the_validator_on_purpose(tmp_path: Path) -> None:
    _, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS))
    module = generate(tmp_path, path, synthetic=False)
    errors = validation_errors(module)
    assert errors and all("provenance" in e for e in errors)
    scrubbed = tmp_path / "scrubbed.json"
    assert main(["scrub", "--sessions", str(path), "--out", str(scrubbed), "--consent", "ok"]) == 0
    ok = generate(tmp_path, scrubbed, synthetic=False, module="ok")
    assert validation_errors(ok) == []


def test_report_lists_what_was_found(tmp_path: Path) -> None:
    _, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS))
    report = tmp_path / "report.json"
    generate(tmp_path, path, "--report", str(report))
    found = json.loads(report.read_text())["scenarios"][0]["detected"]
    assert found["lengths"] == 1 and found["cookies"] == 2 and found["captures"] == 2
    assert found["http_dates"] == 2 and found["epochs"] == 1 and found["identifiers"] == 2


def test_errors_exit_with_one(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    _, path = sessions_doc(tmp_path, tcp_session(HTTP_TURNS))
    out = tmp_path / "m" / "gen"
    base = ["scenario", "--sessions", str(path), "--out-dir", str(out), "--synthetic"]
    assert main([*base, "--session", "9"]) == 1
    assert "no session with id 9" in capsys.readouterr().err
    assert (
        main(
            ["scenario", "--sessions", str(path), "--all", "--out-dir", str(tmp_path / "Bad-Name")]
        )
        == 1
    )
    assert "not an identifier" in capsys.readouterr().err
    bare = tmp_path / "bare.json"
    bare.write_text(
        json.dumps(
            {
                **json.loads(path.read_text()),
                "parameters": {"udp_idle_timeout_s": 60, "payloads": False},
            }
        )
    )
    assert main(["scenario", "--sessions", str(bare), "--all", "--out-dir", str(out)]) == 1
    assert "--payloads" in capsys.readouterr().err
    assert (
        main(
            ["scenario", "--sessions", str(tmp_path / "nope.json"), "--all", "--out-dir", str(out)]
        )
        == 1
    )
    with pytest.raises(SystemExit) as exc:  # neither --session nor --all
        main(["scenario", "--sessions", str(path), "--out-dir", str(out)])
    assert exc.value.code == 2


def test_sessions_without_payload_are_skipped_with_a_warning(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    recs = tcp_session([], sport=40000) + tcp_session(HTTP_TURNS, sport=40001, t0=1_700_000_005.0)
    _, path = sessions_doc(tmp_path, recs)
    module = generate(tmp_path, path)
    assert sorted(scenario_docs(module)) == ["session_2"]
    assert "session_1" not in (module / "manifest.yaml").read_text()
    _, only_empty = sessions_doc(tmp_path, tcp_session([]))
    assert (
        main(["scenario", "--sessions", str(only_empty), "--all", "--out-dir", str(tmp_path / "e")])
        == 1
    )
    assert "carries payload" in capsys.readouterr().err


def test_run_flow_detects_a_deadlock() -> None:
    from .flowsim import Deadlock

    flow = {"client": [{"expect": {"bytes": 3}}], "server": [{"expect": {"bytes": 3}}]}
    with pytest.raises(Deadlock):
        run_flow(flow, {})
