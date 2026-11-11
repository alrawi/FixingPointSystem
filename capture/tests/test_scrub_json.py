# SPDX-License-Identifier: GPL-3.0-only
#
# Issue #451: JSON credential fields are scrubbed whatever their valid JSON
# spelling, line breaks around the colon, escaped key names, nested objects,
# arrays of secrets, and numeric values. A secret echoed elsewhere in the
# document, in its JSON-escaped form too, goes with it.

from __future__ import annotations

import re

from capture.scrub import (
    Edit,
    Scrubber,
    Sweeper,
    _content_lengths,
    _Finder,
    apply_edits,
    resolve,
)

PASSWORD = "Hunter2-Seeded-Pw"


def scrub_http(body: str) -> tuple[str, str]:
    """Scrub one HTTP request with a JSON body the way scrub_document does,
    and return the scrubbed payload and its body."""
    text = (
        "POST /login HTTP/1.1\r\nHost: example.test\r\nContent-Type: application/json\r\n"
        f"Content-Length: {len(body.encode())}\r\n\r\n{body}"
    )
    s = Scrubber(seed=7)
    finder = _Finder(s, hostnames=False)
    edits: list[Edit] = []
    finder.headers(text, edits)
    finder.tokens(text, edits)
    finder.parameters(text, edits)
    sweeper = Sweeper(s)
    edits += sweeper.edits(text, binary=False, udp=False)
    edits += sweeper.base64_edits(text)
    final = resolve(edits)
    final = resolve([*final, *_content_lengths(text, final)])
    out = apply_edits(text, final)
    head, _, rest = out.partition("\r\n\r\n")
    m = re.search(r"Content-Length: (\d+)", head)
    assert m and int(m.group(1)) == len(rest.encode()), "Content-Length no longer fits the body"
    return out, rest


def test_the_inline_form_was_already_scrubbed() -> None:
    _, body = scrub_http('{"password": "' + PASSWORD + '"}')
    assert PASSWORD not in body


def test_line_feed_around_the_colon() -> None:
    _, body = scrub_http('{"password"\n:\n"' + PASSWORD + '"}')
    assert PASSWORD not in body


def test_crlf_around_the_colon() -> None:
    _, body = scrub_http('{\r\n  "password"\r\n  :\r\n  "' + PASSWORD + '"\r\n}')
    assert PASSWORD not in body


def test_a_unicode_escape_in_the_key() -> None:
    _, body = scrub_http('{"pass\\u0077ord": "' + PASSWORD + '"}')
    assert PASSWORD not in body


def test_an_escape_in_the_value_is_scrubbed_too() -> None:
    escaped = PASSWORD.replace("e", "\\u0065")
    _, body = scrub_http('{"password": "' + escaped + '"}')
    assert escaped not in body
    assert PASSWORD not in body


def test_nested_objects() -> None:
    _, body = scrub_http('{"user": {"auth": {"password":\n"' + PASSWORD + '"}}}')
    assert PASSWORD not in body


def test_an_array_of_secrets() -> None:
    other = "Second-Seeded-Secret"
    _, body = scrub_http('{"tokens": [], "secret": ["' + PASSWORD + '", "' + other + '"]}')
    assert PASSWORD not in body
    assert other not in body


def test_a_numeric_password() -> None:
    _, body = scrub_http('{"pin": 1, "password": 83749261}')
    assert "83749261" not in body
    assert re.search(r'"password": \d{8}\}', body), body


def test_an_echo_elsewhere_in_the_document() -> None:
    body_in = '{"password"\n:"' + PASSWORD + '", "note": "you typed ' + PASSWORD + '"}'
    _, body = scrub_http(body_in)
    assert PASSWORD not in body


def test_a_json_escaped_echo_of_a_secret() -> None:
    pw = 'Quote"Back\\slash-Pw'
    raw = pw.replace("\\", "\\\\").replace('"', '\\"')
    _, body = scrub_http('{"password": "' + raw + '", "debug": "seen ' + raw + '"}')
    assert raw not in body


def test_benign_values_and_other_keys_stay() -> None:
    _, body = scrub_http('{"note": "fine", "count": 3, "password_hint": "a dog"}')
    assert body == '{"note": "fine", "count": 3, "password_hint": "a dog"}'


def test_the_body_stays_json_and_keeps_its_shape() -> None:
    import json

    _, body = scrub_http('{"password"\r\n:\r\n"' + PASSWORD + '", "user": ["a", {"pwd": 42}]}')
    doc = json.loads(body)
    assert set(doc) == {"password", "user"}
    assert len(doc["password"]) == len(PASSWORD)
    assert isinstance(doc["user"][1]["pwd"], int)
