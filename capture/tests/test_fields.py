# SPDX-License-Identifier: GPL-3.0-only
"""Heuristics of field detection: what each one finds, and what its guard leaves alone."""

from __future__ import annotations

import random
import re
import struct

import pytest

from capture.fields import (
    Config,
    Field,
    VarAlloc,
    analyze_framing,
    cookie_like,
    detect_fields,
    entropy,
    id_template,
    random_looking,
)
from capture.templates import escape_literal, render

NOW = 1_700_000_000.0
CFG = Config()


def fields_of(*units: tuple[str, bytes], cfg: Config = CFG) -> list[Field]:
    """Run framing and field detection over ``("up"|"down", payload)`` units."""
    alloc = VarAlloc()
    analyses = [analyze_framing(d, k, cfg, alloc) for k, d in units]
    return detect_fields(list(units), analyses, [NOW] * len(units), cfg, alloc)


# -- templates ----------------------------------------------------------------------


@pytest.mark.parametrize(
    "data",
    [
        b"plain text\r\n",
        b"{{ not a placeholder }}",
        b"}}}{{{x}",
        b'json {"a": {"b": 1}}',
        b"\x00\x01binary\xff{",
        b"tail brace {",
        b"",
    ],
)
def test_escape_literal_round_trips(data: bytes) -> None:
    assert render(escape_literal(data)) == data


# -- length fields ------------------------------------------------------------------


def http_response(body: bytes, extra: bytes = b"") -> bytes:
    return b"HTTP/1.1 200 OK\r\n" + extra + b"Content-Length: %d\r\n\r\n" % len(body) + body


def test_content_length_and_body_share_a_variable() -> None:
    body = b"x" * 1000
    alloc = VarAlloc()
    a = analyze_framing(http_response(body), "down", CFG, alloc)
    assert a.protocol == "http"
    assert alloc.values == {"down_len": 1000}
    assert [s.tmpl for s in a.spans] == ["{{ vars.down_len }}", "{{ fill(vars.down_len) }}"]
    assert [(r.kind, r.length_var) for r in a.records] == [("head", None), ("body", "down_len")]


def test_equal_sizes_share_one_variable_per_direction() -> None:
    alloc = VarAlloc()
    analyze_framing(http_response(b"y" * 500), "down", CFG, alloc)
    analyze_framing(http_response(b"z" * 500), "down", CFG, alloc)
    analyze_framing(http_response(b"w" * 500), "up", CFG, alloc)
    assert alloc.values == {"down_len": 500, "up_len": 500}


def test_small_bodies_keep_their_literal_length() -> None:
    """Guard: a body below fill_min is copied, so its length stays consistent as text."""
    a = analyze_framing(http_response(b"tiny"), "down", CFG, VarAlloc())
    assert a.spans == [] and [r.kind for r in a.records] == ["head", "body"]


def test_content_length_larger_than_the_unit_is_left_alone() -> None:
    """Guard: a length that the following bytes do not cover is not a consistent field."""
    data = b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\n\r\n" + b"x" * 100
    assert analyze_framing(data, "down", CFG, VarAlloc()).spans == []


def test_duplicate_length_and_broken_chunking_are_left_alone() -> None:
    twice = b"HTTP/1.1 200 OK\r\nContent-Length: 300\r\nContent-Length: 300\r\n\r\n" + b"x" * 300
    broken = (
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n400\r\n" + b"x" * 1024
    )  # no CRLF
    for data in (twice, broken):
        assert analyze_framing(data, "down", CFG, VarAlloc()).spans == []


def test_chunked_body_keeps_its_size_lines_and_fills_the_data() -> None:
    head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
    data = head + b"400\r\n" + b"a" * 1024 + b"\r\n" + b"5\r\nhello\r\n" + b"0\r\n\r\n"
    a = analyze_framing(data, "down", CFG, VarAlloc())
    assert [(s.tmpl, data[s.start : s.start + 3]) for s in a.spans] == [
        ("{{ fill(1024) }}", b"aaa")
    ]
    assert [r.kind for r in a.records] == ["head", "head", "body", "head", "body", "head"]
    assert a.records[-1].end == len(data)


def test_pipelined_messages_each_get_a_record() -> None:
    req = b"POST /a HTTP/1.1\r\nContent-Length: 300\r\n\r\n" + b"p" * 300
    a = analyze_framing(req + req + b"GET /b HTTP/1.1\r\n\r\n", "up", CFG, VarAlloc())
    assert [r.kind for r in a.records] == ["head", "body", "head", "body", "head"]


def test_binary_length_prefix_frames_must_tile_the_unit() -> None:
    frame = struct.pack(">H", 300) + b"\x01" * 300
    a = analyze_framing(frame + frame, "up", CFG, VarAlloc())
    assert a.protocol == "frames" and [(r.start, r.end) for r in a.records] == [
        (0, 302),
        (302, 604),
    ]
    assert [s.tmpl for s in a.spans] == ["{{ fill(300) }}"] * 2
    # one byte too many, and the frames no longer tile
    assert analyze_framing(frame + b"\x00", "up", CFG, VarAlloc()).protocol == "raw"


def test_tls_record_header_is_a_length_prefix() -> None:
    record = b"\x17\x03\x03" + struct.pack(">H", 400) + bytes(range(256)) + bytes(144)
    a = analyze_framing(record, "up", CFG, VarAlloc())
    assert a.protocol == "frames" and a.records[0].end == 405


def test_text_that_adds_up_is_not_a_length_prefix() -> None:
    """Guard: length bytes in printable ASCII never match, even when the sum is right."""
    text = b"AB" + b"z" * (0x4142 - 2)  # "AB" as a big endian length equals the frame size
    assert analyze_framing(text, "up", CFG, VarAlloc()).protocol == "raw"


def test_opaque_binary_becomes_fill_but_text_stays() -> None:
    rng = random.Random(1)
    blob = bytes(rng.randrange(256) for _ in range(5000))
    assert analyze_framing(blob, "up", CFG, VarAlloc()).protocol == "opaque"
    text = b"lorem ipsum dolor sit amet\r\n" * 300
    assert analyze_framing(text, "up", CFG, VarAlloc()).spans == []


def test_detection_off_copies_everything() -> None:
    cfg = Config(detect_fields=False)
    assert analyze_framing(http_response(b"x" * 1000), "down", cfg, VarAlloc()).spans == []


# -- identifiers --------------------------------------------------------------------


@pytest.mark.parametrize(
    "token",
    ["7f3a9c2e41b8d605", "9d41c7e2b0a35f68", "Zk3pQ9x2LmA8vB4n", "a1B2c3D4e5F6g7H8"],
)
def test_random_tokens_are_identifiers(token: str) -> None:
    assert random_looking(token, strong=False)


@pytest.mark.parametrize(
    "token",
    [
        "keep-alive",  # letters only
        "20261110",  # a date: digits only and short
        "HTTP11111",  # a run of equal characters
        "abcabcab1",  # too few distinct characters
        "Mozilla5",  # low entropy
        "aaaa1111",
        "text-html",
    ],
)
def test_words_dates_and_runs_are_not_identifiers(token: str) -> None:
    assert not random_looking(token, strong=False)


def test_a_single_occurrence_needs_the_strong_form() -> None:
    assert random_looking("9d41c7e2b0a35f68", strong=True)
    assert not random_looking("9d41c7e2b0a3", strong=True)  # 12 characters
    assert entropy("aaaa") == 0.0


def test_request_id_that_repeats_across_a_pair_is_found() -> None:
    req = b"GET / HTTP/1.1\r\nX-Request-Id: 7f3a9c2e41b8\r\n\r\n"
    resp = b"HTTP/1.1 200 OK\r\nX-Request-Id: 7f3a9c2e41b8\r\nContent-Length: 0\r\n\r\n"
    (f,) = fields_of(("up", req), ("down", resp))
    assert (f.kind, f.value, [o.unit for o in f.occurrences]) == ("id", "7f3a9c2e41b8", [0, 1])


def test_uuid_is_always_an_identifier_even_alone() -> None:
    uuid = b"123e4567-e89b-12d3-a456-426614174000"
    (f,) = fields_of(("up", b"GET /?u=" + uuid + b" HTTP/1.1\r\n\r\n"))
    assert f.kind == "id" and f.value == uuid.decode()
    assert re.fullmatch(r"(\{\{ rand_int\(\d+, \d+\) \}\}|-)+", f.gen)
    assert len(render(f.gen)) == len(uuid)


def test_id_guards_leave_names_alone() -> None:
    units = [
        # a resource name in the request target that nothing else repeats
        (
            "up",
            b"GET /objects/9f8e7d6c5b4a39201f HTTP/1.1\r\n"
            b"Host: a1b2c3d4e5f6.cdn.example.test\r\n\r\n",
        ),
        # words, versions, a date, and a host name next to dots
        (
            "down",
            b"HTTP/1.1 200 OK\r\nServer: nginx/1.25.3\r\nDate-Id: 2026-11-10\r\n"
            b"Content-Length: 0\r\n\r\n",
        ),
    ]
    assert fields_of(*units) == []


def test_id_template_keeps_length_and_separators() -> None:
    for value in ("7f3a9c2e41b8d605", "ab-12_cd", "123e4567-e89b-12d3-a456-426614174000", "x" * 40):
        assert len(render(id_template(value))) == len(value)
    out = render(id_template("ab-12_cd"))
    assert re.fullmatch(rb"\d\d-\d\d_\d\d", out)


def test_numeric_looking_ids_stay_text_for_set() -> None:
    """``set`` parses numbers, so a long digit token gets a literal letter in front."""
    assert id_template("1234567890123456789").startswith("x")  # 19 digits would overflow
    assert not id_template("12345678901234567").startswith("x")  # 17 digits are exact
    assert id_template("12.5000").startswith("x")  # a float would drop the zeros


# -- timestamps ---------------------------------------------------------------------

DATE = b"Tue, 14 Nov 2023 22:13:25 GMT"


def test_http_date_near_the_capture_time_is_found() -> None:
    (f,) = fields_of(("down", b"HTTP/1.1 200 OK\r\nDate: " + DATE + b"\r\n\r\n"))
    assert (f.kind, f.value) == ("http_date", DATE.decode())


def test_http_date_guards() -> None:
    wrong_weekday = DATE.replace(b"Tue", b"Wed")
    last_year = b"Mon, 14 Nov 2022 22:13:25 GMT"
    no_such_day = b"Tue, 31 Nov 2023 22:13:25 GMT"
    for text in (wrong_weekday, last_year, no_such_day):
        assert fields_of(("down", b"HTTP/1.1 200 OK\r\nDate: " + text + b"\r\n\r\n")) == []


def test_epoch_seconds_and_milliseconds_in_the_window() -> None:
    data = b"ts=1700000012&t2=1700000012345 HTTP/1.1"
    kinds = {f.kind: f for f in fields_of(("up", data))}
    assert set(kinds) == {"epoch_s", "epoch_ms"}
    assert 1699913612 <= int(render(kinds["epoch_s"].gen)) <= 1700086412
    assert len(render(kinds["epoch_ms"].gen)) == 13


def test_epoch_guards() -> None:
    for data in (
        b"phone=5551234567",  # ten digits far from the capture time
        b"id=1700000012a",  # a letter next to it
        b"v=1700000012.5",  # a decimal part
        b"n=-1700000012",  # a sign
        b"n=11700000012",  # eleven digits
        b"old=1600000000",  # three years early
    ):
        assert fields_of(("up", data)) == [], data


# -- cookies ------------------------------------------------------------------------


def test_cookie_echo_becomes_one_field_with_all_occurrences() -> None:
    set_cookie = b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=9d41c7e2b0a35f68; Path=/\r\n\r\n"
    echo = b"GET / HTTP/1.1\r\nCookie: lang=en; sid=9d41c7e2b0a35f68\r\n\r\n"
    (f,) = fields_of(("down", set_cookie), ("up", echo))
    assert (f.kind, f.name, f.value) == ("cookie", "cookie_sid", "9d41c7e2b0a35f68")
    assert [o.unit for o in f.occurrences] == [0, 1]
    assert set_cookie[f.occurrences[0].start : f.occurrences[0].end] == f.value.encode()
    assert echo[f.occurrences[1].start : f.occurrences[1].end] == f.value.encode()


def test_cookie_that_nobody_echoes_stays() -> None:
    """Guard: a Set-Cookie without a matching later Cookie is not a variable."""
    found = fields_of(("down", b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=9d41c7e2b0a35f68\r\n\r\n"))
    assert [f.kind for f in found] == ["id"]  # random looking, but not a cookie


def test_preference_cookies_stay() -> None:
    set_cookie = b"HTTP/1.1 200 OK\r\nSet-Cookie: lang=en-US\r\n\r\n"
    echo = b"GET / HTTP/1.1\r\nCookie: lang=en-US\r\n\r\n"
    assert fields_of(("down", set_cookie), ("up", echo)) == []
    assert not cookie_like("dark") and not cookie_like("en-US") and cookie_like("a8Fk39Zq")


def test_echo_must_come_after_the_set_cookie() -> None:
    echo = b"GET / HTTP/1.1\r\nCookie: sid=9d41c7e2b0a35f68\r\n\r\n"
    set_cookie = b"HTTP/1.1 200 OK\r\nSet-Cookie: sid=9d41c7e2b0a35f68\r\n\r\n"
    assert [f for f in fields_of(("up", echo), ("down", set_cookie)) if f.kind == "cookie"] == []


def test_fill_regions_are_not_searched() -> None:
    body = b"id=7f3a9c2e41b8d605 " * 40
    assert fields_of(("down", http_response(body))) == []


@pytest.mark.parametrize(
    "data",
    [b"{{", b"}}", b"a{{b}}c", b"{" + b"\x00" + b"{", b"x{" + bytes([7]), b"{}{}", b"}{" * 3],
)
def test_braces_never_form_template_delimiters(data: bytes) -> None:
    text = escape_literal(data)
    assert render(text, {}) == data
    stripped = re.sub(r"\{\{.*?\}\}", "", text)
    assert "{{" not in stripped and "}}" not in stripped
