# SPDX-License-Identifier: GPL-3.0-only
"""Scrubbing (CAP-03): a seeded capture with known secrets must leave none of them behind."""

from __future__ import annotations

import base64
import json
import re
from pathlib import Path
from typing import Any
from urllib.parse import quote, quote_plus, unquote

import jsonschema
import pytest
import yaml

from capture.cli import main
from capture.scrub import (
    SCRUBBER_VERSION,
    Scrubber,
    apply_edits,
    canonical_digest,
    check_record,
    resolve,
    scrub_document,
)

from .conftest import SCHEMA
from .pcapgen import Rec, write_pcap
from .scenario_helpers import (
    REPO,
    T0,
    scenario_docs,
    tcp_session,
    udp_session,
    validation_errors,
)

CLIENT4, SERVER4 = "10.20.30.41", "172.16.8.9"
CLIENT6, SERVER6 = "fd12:3456:789a::7", "2a01:4f8:1c1a:1::2"
OTHER4 = "192.168.1.77"
CONSENT = "owner approved the use of the lab capture, ticket 4711"


def b64(text: str) -> str:
    return base64.b64encode(text.encode("latin-1")).decode()


JWT = (
    "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJhbGljZS5zbWl0aCIsImVtYWlsIjoiYUBhY21lLW5ldC5pbyJ9"
    ".s1gnatureAbCdEf123456"
)
SESSION_ID = "9f8e7d6c5b4a39281706f5e4d3c2b1a0"
APOP_DIGEST = "6f1d9a2b3c4e5f60718293a4b5c6d7e8"
CRAM_DIGEST = "9d5c3f1a7b2e4c6d8f0a1b3c5d7e9f02"

# Every secret of the capture, with a label. The test searches the output for each of them.
SECRETS: dict[str, str] = {
    "http basic user": "alice.smith",
    "http basic pass": "Tr0ub4dor&3xK",
    "session id": SESSION_ID,
    "api key": "k9Q2-wZ7xP4mT1vB",
    "jwt": JWT,
    "query user": "bob.jones",
    "query token": "Zx81KqPz90LmNb",
    "refresh cookie": "rT-77Hq0LpZ4mX9cV2",
    "json token": "ya29.A0ARrdaM-seeded-Token_value",
    "json email": "carol@acme-corp.net",
    "json password": "hunter2hunter2",
    "form user": "dave.r",
    "form pass": "p@ss&w0rd!",
    "form email": "dave@acme-corp.net",
    "smtp plain user": "erin.smith",
    "smtp plain pass": "S3cretP4ss!",
    "smtp login user": "frank@acme-corp.net",
    "smtp login pass": "Fr4nk-P4ssw0rd",
    "smtp cram user": "heidi",
    "smtp cram digest": CRAM_DIGEST,
    "smtp rcpt": "grace@other-org.io",
    "imap user": "ivan.petrov",
    "imap pass": "Pa$$w0rd!x",
    "imap plain user": "judy.k",
    "imap plain pass": "Ju-dy-Secret1",
    "pop3 user": "karl.m",
    "pop3 pass": "K4rl-pop-pw",
    "apop user": "leo.n",
    "apop digest": APOP_DIGEST,
    "ftp user": "mallory",
    "ftp pass": "M4ll0ry!pass",
    "syslog email": "nina@acme-corp.net",
    "syslog token": "AbCdEf123456",
    "basic anywhere": "oscar:Osc4r-in-a-body",
    "url user": "peggy",
    "url pass": "Pegg7-url-pw",
    "ipv4 client": CLIENT4,
    "ipv4 server": SERVER4,
    "ipv4 other": OTHER4,
    "ipv6 client": CLIENT6,
    "ipv6 server": SERVER6,
    "host": "files.acme-corp.net",
    "mail host": "mx.acme-corp.net",
    "display name": "Carol Jones",
}


def _http() -> list[Rec]:
    req1 = (
        "GET /api/v1/items?user=bob.jones&token=Zx81KqPz90LmNb&page=2 HTTP/1.1\r\n"
        "Host: files.acme-corp.net\r\n"
        f"Authorization: Basic {b64('alice.smith:Tr0ub4dor&3xK')}\r\n"
        f"Cookie: sessionid={SESSION_ID}; theme=dark\r\n"
        "X-API-Key: k9Q2-wZ7xP4mT1vB\r\n"
        f"X-Forwarded-For: {CLIENT4}, {OTHER4}\r\n"
        "User-Agent: curl/8.5.0\r\n\r\n"
    )
    body1 = (
        '{"access_token":"ya29.A0ARrdaM-seeded-Token_value","email":"carol@acme-corp.net",'
        '"password":"hunter2hunter2","note":"fine"}'
    )
    resp1 = (
        "HTTP/1.1 200 OK\r\n"
        f"Set-Cookie: sessionid={SESSION_ID}; Path=/; HttpOnly\r\n"
        "Set-Cookie: refresh=rT-77Hq0LpZ4mX9cV2; Path=/\r\n"
        f"Content-Type: application/json\r\nContent-Length: {len(body1)}\r\n\r\n{body1}"
    )
    body = "username=dave.r&password=p%40ss%26w0rd%21&email=dave%40acme-corp.net"
    req2 = (
        "POST /login HTTP/1.1\r\nHost: files.acme-corp.net\r\n"
        "Content-Type: application/x-www-form-urlencoded\r\n"
        f"Authorization: Bearer {JWT}\r\n"
        f"Content-Length: {len(body)}\r\n\r\n{body}"
    )
    body2 = "From curl: Basic " + b64("oscar:Osc4r-in-a-body") + " ftp://peggy:Pegg7-url-pw@h/\r\n"
    resp2 = f"HTTP/1.1 200 OK\r\nContent-Length: {len(body2)}\r\n\r\n{body2}"
    return tcp_session(
        [("c", req1.encode()), ("s", resp1.encode()), ("c", req2.encode()), ("s", resp2.encode())],
        T0,
        sport=40001,
        dport=8080,
        client=CLIENT4,
        server=SERVER4,
    )


def _smtp() -> list[Rec]:
    plain = f"AUTH PLAIN {b64(chr(0) + 'erin.smith' + chr(0) + 'S3cretP4ss!')}\r\n"
    mail = (
        "MAIL FROM:<frank@acme-corp.net>\r\nRCPT TO:<grace@other-org.io>\r\nDATA\r\n"
        'From: "Carol Jones" <carol@acme-corp.net>\r\nTo: grace@other-org.io\r\n\r\nhi\r\n.\r\n'
    )
    turns: list[tuple[str, bytes]] = [
        ("s", b"220 mx.acme-corp.net ESMTP ready\r\n"),
        ("c", b"EHLO laptop.acme-corp.net\r\n"),
        ("s", b"250-mx.acme-corp.net\r\n250 AUTH PLAIN LOGIN CRAM-MD5\r\n"),
        ("c", plain.encode()),
        ("s", b"235 2.7.0 ok\r\n"),
        ("c", mail.encode()),
        ("s", b"250 queued\r\n"),
    ]
    first = tcp_session(turns, T0 + 1, sport=40002, dport=2525, client=CLIENT4, server=SERVER4)
    login: list[tuple[str, bytes]] = [
        ("c", b"AUTH LOGIN\r\n"),
        ("s", b"334 VXNlcm5hbWU6\r\n"),
        ("c", (b64("frank@acme-corp.net") + "\r\n").encode()),
        ("s", b"334 UGFzc3dvcmQ6\r\n"),
        ("c", (b64("Fr4nk-P4ssw0rd") + "\r\n").encode()),
        ("s", b"235 ok\r\n"),
        ("c", b"AUTH CRAM-MD5\r\n"),
        ("s", ("334 " + b64("<1896.697170952@mx.acme-corp.net>") + "\r\n").encode()),
        ("c", (b64(f"heidi {CRAM_DIGEST}") + "\r\n").encode()),
        ("s", b"235 ok\r\n"),
    ]
    second = tcp_session(login, T0 + 2, sport=40003, dport=2525, client=CLIENT4, server=SERVER4)
    return first + second


def _mail_retrieval() -> list[Rec]:
    imap: list[tuple[str, bytes]] = [
        ("s", b"* OK IMAP ready\r\n"),
        ("c", b'a1 LOGIN "ivan.petrov" "Pa$$w0rd!x"\r\n'),
        ("s", b"a1 OK logged in\r\n"),
        (
            "c",
            f"a2 AUTHENTICATE PLAIN {b64(chr(0) + 'judy.k' + chr(0) + 'Ju-dy-Secret1')}\r\n".encode(),  # noqa: E501
        ),
        ("s", b"a2 OK\r\n"),
    ]
    pop: list[tuple[str, bytes]] = [
        ("s", b"+OK POP3 ready\r\n"),
        ("c", b"USER karl.m\r\n"),
        ("s", b"+OK\r\n"),
        ("c", b"PASS K4rl-pop-pw\r\n"),
        ("s", b"+OK karl.m's maildrop has 2 messages\r\n"),
    ]
    apop: list[tuple[str, bytes]] = [
        ("s", b"+OK POP3 <1896.697170952@mx.acme-corp.net>\r\n"),
        ("c", f"APOP leo.n {APOP_DIGEST}\r\n".encode()),
        ("s", b"+OK leo.n is welcome\r\n"),
    ]
    kw: dict[str, Any] = {"client": CLIENT4, "server": SERVER4}
    return (
        tcp_session(imap, T0 + 3, sport=40004, dport=1143, **kw)
        + tcp_session(pop, T0 + 4, sport=40005, dport=1110, **kw)
        + tcp_session(apop, T0 + 5, sport=40006, dport=1110, **kw)
    )


def _ftp() -> list[Rec]:
    turns: list[tuple[str, bytes]] = [
        ("s", b"220 FTP ready\r\n"),
        ("c", b"USER mallory\r\n"),
        ("s", b"331 Password required for mallory\r\n"),
        ("c", b"PASS M4ll0ry!pass\r\n"),
        ("s", b"230 User mallory logged in.\r\n"),
        ("c", b"PORT 10,20,30,41,195,80\r\n"),
        ("s", b"200 PORT ok\r\n"),
        ("c", b"PASV\r\n"),
        ("s", b"227 Entering Passive Mode (172,16,8,9,200,10).\r\n"),
    ]
    return tcp_session(turns, T0 + 6, sport=40007, dport=2121, client=CLIENT4, server=SERVER4)


def _v6_and_udp() -> list[Rec]:
    log = (
        "<14>Nov  9 10:00:00 laptop app: user nina@acme-corp.net "
        f"from {SERVER6} token=AbCdEf123456\n"
    )
    tcp = tcp_session(
        [("c", f"GET /v6?h={CLIENT6} HTTP/1.1\r\nHost: files.acme-corp.net\r\n\r\n".encode()),
         ("s", b"HTTP/1.1 204 No Content\r\n\r\n")],
        T0 + 7,
        sport=40008,
        dport=8080,
        client=CLIENT6,
        server=SERVER6,
    )  # fmt: skip
    syslog = udp_session(
        [("c", log.encode())], T0 + 8, sport=50514, dport=5140, client=CLIENT4, server=SERVER4
    )
    packed = bytes(map(int, CLIENT4.split("."))) + b"\x00\x01\xff\xfe" * 6
    binary = udp_session(
        [("c", b"\x80\x00" + packed), ("s", b"\x80\x01")],
        T0 + 9,
        sport=50515,
        dport=5141,
        client=CLIENT4,
        server=SERVER4,
    )
    return tcp + syslog + binary


def seeded_pcap() -> bytes:
    recs = _http() + _smtp() + _mail_retrieval() + _ftp() + _v6_and_udp()
    return write_pcap(sorted(recs, key=lambda r: r.t))


@pytest.fixture
def seeded(tmp_path: Path) -> Path:
    pcap = tmp_path / "alice-home-lab.pcap"
    pcap.write_bytes(seeded_pcap())
    out = tmp_path / "sessions.json"
    assert main(["sessions", "--pcap", str(pcap), "--payloads", "--out", str(out)]) == 0
    return out


def scrub(tmp_path: Path, sessions: Path, *args: str) -> Path:
    out = tmp_path / "scrubbed.json"
    argv = ["scrub", "--sessions", str(sessions), "--out", str(out), "--consent", CONSENT, *args]
    assert main(argv) == 0
    return out


# -- searching the output ---------------------------------------------------------------


def _forms(secret: str) -> list[bytes]:
    """A secret as it may appear in output: raw, percent encoded, and inside Base64 at each
    of the three alignments."""
    data = secret.encode()
    forms = [
        data,
        quote(secret, safe="").encode(),
        quote_plus(secret).encode(),
        quote(secret).encode(),
    ]
    for off in (0, 1, 2):
        enc = base64.b64encode(b"\0" * off + data).decode().rstrip("=")
        start = {0: 0, 1: 2, 2: 3}[off]
        core = enc[start:-2] if len(enc) - start > 8 else ""
        if core:
            forms.append(core.encode())
            forms.append(core.replace("+", "-").replace("/", "_").encode())
    return forms


def _views(blob: bytes) -> list[bytes]:
    """The bytes as they are, percent decoded, and with every Base64 token decoded."""
    views = [blob, _unquote(blob)]
    for tok in re.findall(rb"[A-Za-z0-9+/_-]{12,}={0,2}", blob):
        body = tok.rstrip(b"=")
        if len(body) % 4 == 1:
            continue
        alt = b"-_" if (b"-" in body or b"_" in body) else None
        try:
            views.append(base64.b64decode(body + b"=" * (-len(body) % 4), altchars=alt))
        except ValueError:
            continue
    return views


def _unquote(blob: bytes) -> bytes:
    return unquote(blob.decode("latin-1")).encode("latin-1", "replace")


def _payloads(doc: dict[str, Any]) -> list[bytes]:
    out = []
    for s in doc["sessions"]:
        for t in s["turns"]:
            for b in [t["data_b64"]] if "data_b64" in t else t["datagrams_b64"]:
                out.append(base64.b64decode(b))
    return out


def leaks(blobs: list[bytes]) -> list[str]:
    """The labels of the secrets that any view of any blob still holds."""
    found = []
    for label, secret in SECRETS.items():
        for blob in blobs:
            for view in _views(blob):
                if any(f in view for f in _forms(secret)):
                    found.append(label)
                    break
            else:
                continue
            break
    return found


def test_seeded_secrets_are_absent_from_the_sessions(tmp_path: Path, seeded: Path) -> None:
    raw = json.loads(seeded.read_text())
    assert len(leaks(_payloads(raw))) > 30  # the search finds the secrets in the original
    out = scrub(tmp_path, seeded, "--hostnames", "--redact", "Carol Jones")
    doc = json.loads(out.read_text())
    jsonschema.validate(doc, SCHEMA)
    text = out.read_bytes()
    assert leaks([*_payloads(doc), text]) == []
    # the document itself: addresses in endpoints, file name, interface names
    ips = {s[e]["ip"] for s in doc["sessions"] for e in ("client", "server")}
    assert not ips & {CLIENT4, SERVER4, CLIENT6, SERVER6}
    assert "alice" not in doc["source"]["file"]


def test_the_original_sessions_hold_every_secret(seeded: Path) -> None:
    """Guards the guard: without scrubbing, the search finds each of the secrets."""
    raw = json.loads(seeded.read_text())
    text = seeded.read_bytes()
    assert set(SECRETS) - set(leaks([*_payloads(raw), text])) == set()


def test_generated_module_has_none_of_them_and_validates(tmp_path: Path, seeded: Path) -> None:
    out = scrub(tmp_path, seeded, "--hostnames", "--redact", "Carol Jones")
    module = tmp_path / "parent" / "lab"
    assert main(["scenario", "--sessions", str(out), "--all", "--out-dir", str(module)]) == 0
    assert validation_errors(module) == []
    files = [p.read_bytes() for p in sorted(module.rglob("*.yaml"))]
    assert files and leaks(files) == []
    manifest = (module / "manifest.yaml").read_text()
    assert 'source: "capture"' in manifest or "source: capture" in manifest
    assert "scrubbed: true" in manifest
    assert CONSENT in manifest
    assert f"capture scrub {SCRUBBER_VERSION}" in manifest
    assert '{ id: "smtp_auth", version: 1 }' in manifest and "scrubbed_at:" in manifest
    assert len(scenario_docs(module)) == len(json.loads(out.read_text())["sessions"])


def test_default_run_without_hostnames_still_hides_the_secrets(
    tmp_path: Path, seeded: Path
) -> None:
    doc = json.loads(scrub(tmp_path, seeded).read_text())
    rest = {k: v for k, v in SECRETS.items() if k not in {"host", "mail host", "display name"}}
    assert [x for x in leaks(_payloads(doc)) if x in rest] == []
    # the domain of an email address is learned and replaced everywhere
    assert not any(b"acme-corp" in p for p in _payloads(doc))


def test_addresses_move_into_documentation_ranges_and_keep_equality(
    tmp_path: Path, seeded: Path
) -> None:
    doc = json.loads(scrub(tmp_path, seeded).read_text())
    clients4 = {s["client"]["ip"] for s in doc["sessions"] if s["ip_version"] == 4}
    servers4 = {s["server"]["ip"] for s in doc["sessions"] if s["ip_version"] == 4}
    assert len(clients4) == 1 and len(servers4) == 1  # one address stays one address
    (c4,) = clients4
    (s4,) = servers4
    assert c4 != s4 and c4.startswith(("192.0.2.", "198.51.100.", "203.0.113."))
    v6 = {s["client"]["ip"] for s in doc["sessions"] if s["ip_version"] == 6}
    assert len(v6) == 1 and next(iter(v6)).startswith("2001:db8::")
    text = b"\n".join(_payloads(doc)).decode("latin-1")
    # FTP PORT and PASV carry the addresses as comma lists
    assert f"PORT {c4.replace('.', ',')},195,80" in text
    assert f"({s4.replace('.', ',')},200,10)" in text
    # the IPv6 address in a URL and in a log line, and the X-Forwarded-For list
    assert f"h={next(iter(v6))} HTTP" in text
    assert f"X-Forwarded-For: {c4}, " in text
    # raw address bytes in a binary datagram follow the same mapping
    packed = bytes(map(int, c4.split(".")))
    assert packed + b"\x00\x01\xff\xfe" in b"\n".join(_payloads(doc))


def test_credentials_keep_their_length_and_shape(tmp_path: Path, seeded: Path) -> None:
    old = json.loads(seeded.read_text())
    new = json.loads(scrub(tmp_path, seeded, "--hostnames").read_text())
    http_old = _payloads(old)[0].decode()
    http_new = _payloads(new)[0].decode()
    token = re.search(r"Authorization: Basic (\S+)", http_new)
    assert token
    user, _, pw = base64.b64decode(token.group(1)).decode().partition(":")
    assert (len(user), len(pw)) == (len("alice.smith"), len("Tr0ub4dor&3xK"))
    assert "." in user  # separators stay
    sid = re.search(r"Cookie: sessionid=([0-9a-f]+);", http_new)
    assert sid and len(sid.group(1)) == len(SESSION_ID) and sid.group(1) != SESSION_ID
    assert "theme=dark" in http_new  # a harmless cookie stays
    assert "User-Agent: curl/8.5.0" in http_new
    assert http_new.count("sessionid=") == http_old.count("sessionid=")
    # the same cookie value in Set-Cookie got the same replacement
    resp = _payloads(new)[1].decode()
    assert f"sessionid={sid.group(1)};" in resp
    # Content-Length follows the body when its size changed
    for blob in (_payloads(new)[1], _payloads(new)[2]):
        head, _, body = blob.decode().partition("\r\n\r\n")
        assert int(re.search(r"Content-Length: (\d+)", head).group(1)) == len(body)  # type: ignore[union-attr]
    # JWT keeps its three dotted parts
    req2 = _payloads(new)[2].decode()
    jwt = re.search(r"Bearer (\S+)", req2)
    assert jwt and jwt.group(1).count(".") == 2 and len(jwt.group(1)) == len(JWT)
    # the byte count change is small and reported
    rec = new["scrub"]
    pb = rec["payload_bytes"]
    assert pb["delta"] == pb["after"] - pb["before"] and abs(pb["delta"]) < pb["before"] * 0.1
    # every turn size and counter follows the new bytes
    for s in new["sessions"]:
        for t in s["turns"]:
            if "data_b64" in t:
                assert t["bytes"] == len(base64.b64decode(t["data_b64"]))
            else:
                assert t["datagram_sizes"] == [len(base64.b64decode(b)) for b in t["datagrams_b64"]]


def test_sasl_dialogs_decode_to_synthetic_values_of_the_same_length(
    tmp_path: Path, seeded: Path
) -> None:
    new = json.loads(scrub(tmp_path, seeded).read_text())
    blobs = [p.decode("latin-1") for p in _payloads(new)]
    plain = next(b for b in blobs if b.startswith("AUTH PLAIN "))
    tok = re.search(r"AUTH PLAIN (\S+)", plain)
    assert tok
    parts = base64.b64decode(tok.group(1)).decode().split("\0")
    assert [len(p) for p in parts] == [0, len("erin.smith"), len("S3cretP4ss!")]
    session = next(
        t
        for t in new["sessions"]
        if any(b"AUTH LOGIN" in base64.b64decode(x.get("data_b64", "")) for x in t["turns"])
    )
    client = [
        base64.b64decode(t["data_b64"]).decode().strip()
        for t in session["turns"]
        if t["direction"] == "client_to_server"
    ]
    assert client[0] == "AUTH LOGIN"
    assert len(base64.b64decode(client[1])) == len("frank@acme-corp.net")
    assert len(base64.b64decode(client[2])) == len("Fr4nk-P4ssw0rd")
    user, digest = base64.b64decode(client[4]).decode().split(" ")
    assert len(user) == len("heidi") and re.fullmatch(r"[0-9a-f]{32}", digest)
    assert digest != CRAM_DIGEST
    # the server prompts and the CRAM challenge are not secrets, but the challenge host is
    server = [base64.b64decode(t["data_b64"]).decode() for t in session["turns"]]
    assert "334 VXNlcm5hbWU6\r\n" in server
    imap = next(b for b in blobs if "LOGIN" in b and b.startswith("a1"))
    assert re.fullmatch(r'a1 LOGIN "[a-z]{4}\.[a-z]{6}" "[^"]{10}"\r\n', imap)


def test_emails_become_example_test_addresses_consistently(tmp_path: Path, seeded: Path) -> None:
    new = json.loads(scrub(tmp_path, seeded).read_text())
    smtp = next(p.decode() for p in _payloads(new) if "MAIL FROM" in p.decode())
    sender = re.search(r"MAIL FROM:<([^>]+)>", smtp)
    assert sender and sender.group(1).endswith("@example.test")
    rcpt = re.search(r"RCPT TO:<([^>]+)>", smtp)
    assert rcpt and f"To: {rcpt.group(1)}\r\n" in smtp  # the same address, the same pseudonym
    carol = re.search(r"From: .* <([^>]+)>", smtp)
    assert carol and carol.group(1).endswith("@example.test")
    assert len({sender.group(1), rcpt.group(1), carol.group(1)}) == 3


def test_display_names_and_literals(tmp_path: Path, seeded: Path) -> None:
    new = json.loads(scrub(tmp_path, seeded).read_text())
    smtp = next(p.decode() for p in _payloads(new) if "MAIL FROM" in p.decode())
    assert "Carol Jones" not in smtp and 'From: "' in smtp


def test_hostnames_only_when_asked(tmp_path: Path, seeded: Path) -> None:
    default = json.loads(scrub(tmp_path, seeded).read_text())
    asked = json.loads(scrub(tmp_path, seeded, "--hostnames").read_text())
    names = [p.decode("latin-1") for p in _payloads(asked)]
    assert not any("acme-corp" in n for n in names)
    assert any("Host: host" in n and n.count("example.test") for n in names)
    assert asked["scrub"]["options"]["hostnames"] is True
    assert default["scrub"]["options"]["hostnames"] is False
    explicit = json.loads(scrub(tmp_path, seeded, "--hostname", "laptop").read_text())
    assert not any("laptop" in p.decode("latin-1") for p in _payloads(explicit))


def test_drop_port_removes_sessions_and_renumbers(tmp_path: Path, seeded: Path) -> None:
    old = json.loads(seeded.read_text())
    new = json.loads(scrub(tmp_path, seeded, "--drop-port", "2121").read_text())
    assert len(new["sessions"]) == len(old["sessions"]) - 1
    assert [s["id"] for s in new["sessions"]] == list(range(1, len(new["sessions"]) + 1))
    assert new["summary"]["sessions"] == len(new["sessions"])
    assert new["scrub"]["options"]["dropped_ports"] == [2121]


# -- provenance -------------------------------------------------------------------------


def test_scrub_record_and_digest(tmp_path: Path, seeded: Path) -> None:
    doc = json.loads(scrub(tmp_path, seeded).read_text())
    rec, problems = check_record(doc)
    assert rec is not None and problems == []
    assert rec["consent"] == CONSENT and rec["tool"] == "capture scrub"
    assert rec["capture_sha256"] == json.loads(seeded.read_text())["source"]["sha256"]
    assert re.fullmatch(r"\d{4}-\d{2}-\d{2}", rec["date"])
    ids = {r["id"]: r for r in rec["rules"]}
    for rule in ("ipv4", "ipv6", "email", "smtp_auth", "imap_login", "pop3", "ftp", "http_cookie"):
        assert ids[rule]["applied"] > 0 and ids[rule]["version"] >= 1
    assert ids["hostname"]["applied"] > 0  # the domain of an email address is a host name
    # the record names no secret
    text = json.dumps(rec)
    assert not any(v in text for v in SECRETS.values() if len(v) > 5)
    doc["sessions"][0]["turns"][0]["bytes"] += 1
    assert check_record(doc)[0] is None


def test_scenario_takes_provenance_only_from_the_scrubber(
    tmp_path: Path, seeded: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    def gen(sessions: Path, name: str, *extra: str) -> Path:
        out = tmp_path / f"p_{name}" / name
        assert (
            main(["scenario", "--sessions", str(sessions), "--all", "--out-dir", str(out), *extra])
            == 0
        )
        return out

    raw = gen(seeded, "raw")
    errors = validation_errors(raw)
    assert errors and all("provenance" in e for e in errors)
    assert "scrubbed: false" in (raw / "manifest.yaml").read_text()
    capsys.readouterr()
    # the old flags no longer claim anything
    for flag in (["--scrubbed"], ["--consent", "me"]):
        argv = ["scenario", "--sessions", str(seeded), "--all", "--out-dir", str(tmp_path / "x")]
        assert main([*argv, *flag]) == 1
        assert "capture scrub" in capsys.readouterr().err
    # a scrubbed document that someone edited records nothing
    out = scrub(tmp_path, seeded)
    doc = json.loads(out.read_text())
    doc["sessions"][0]["turns"][0]["data_b64"] = b64("changed")
    edited = tmp_path / "edited.json"
    edited.write_text(json.dumps(doc))
    mod = gen(edited, "edited")
    assert validation_errors(mod)
    assert "digest does not match" in capsys.readouterr().err
    # a hand written record is no better
    doc2 = json.loads(out.read_text())
    doc2["scrub"]["consent"] = "someone else"
    forged = tmp_path / "forged.json"
    forged.write_text(json.dumps(doc2))
    assert validation_errors(gen(forged, "forged"))
    # synthetic stays available for traffic that a lab generated
    assert validation_errors(gen(seeded, "synth", "--synthetic")) == []


MANIFEST_SCHEMA = json.loads(
    (REPO / "contracts" / "schemas" / "module-manifest.schema.json").read_text(encoding="utf-8")
)
HTTP1_SHA256 = "b501f54771cff9df7a8a58461f0ebaf1f8008731eba422e8344f80aa249842e6"
ALL_RULES = [
    "ipv4", "ipv6", "ip_binary", "content_length", "email", "display_name", "hostname",
    "http_authorization", "http_cookie", "http_header", "url_credentials", "parameter", "jwt",
    "api_key", "private_key", "smtp_auth", "imap_login", "pop3", "ftp", "literal", "sweep",
    "base64",
]  # fmt: skip


def _manifest(module: Path) -> dict[str, Any]:
    manifest: dict[str, Any] = yaml.safe_load(
        (module / "manifest.yaml").read_text(encoding="utf-8")
    )
    return manifest


def _http1_sessions(tmp_path: Path) -> Path:
    pcap = REPO / "modules" / "http1" / "fixtures" / "http1.pcapng"
    sessions = tmp_path / "s.json"
    assert main(["sessions", "--pcap", str(pcap), "--payloads", "--out", str(sessions)]) == 0
    return sessions


def test_manifest_provenance_fields_come_from_the_scrub_record(tmp_path: Path) -> None:
    out = scrub(tmp_path, _http1_sessions(tmp_path), "--hostnames", "--date", "2026-11-10")
    module = tmp_path / "parent" / "fx"
    assert main(["scenario", "--sessions", str(out), "--all", "--out-dir", str(module)]) == 0
    assert validation_errors(module) == []
    manifest = _manifest(module)
    jsonschema.Draft202012Validator(MANIFEST_SCHEMA).validate(manifest)  # v0.4
    prov = manifest["provenance"]
    notes = prov.pop("notes")
    assert prov == {
        "source": "capture",
        "consent": CONSENT,
        "scrubbed": True,
        "capture_sha256": HTTP1_SHA256,
        "scrub_rules": [{"id": r, "version": 1} for r in ALL_RULES],
        "scrubbed_at": "2026-11-10",
    }
    # the notes keep what the fields do not hold, and no longer repeat what they do
    assert "Sessions sha256" in notes and "Payload bytes 938681 to 938681 (+0)" in notes
    assert HTTP1_SHA256 not in notes and "Rules:" not in notes


def test_provenance_fields_are_absent_without_a_scrub_and_for_synthetic(
    tmp_path: Path,
) -> None:
    sessions = _http1_sessions(tmp_path)
    for n, extra in enumerate((["--synthetic"], [])):
        module = tmp_path / f"p{n}" / "fx"
        argv = ["scenario", "--sessions", str(sessions), "--all", "--out-dir", str(module)]
        assert main([*argv, *extra]) == 0
        if extra:  # an unscrubbed capture is invalid on purpose, a synthetic one is valid
            jsonschema.Draft202012Validator(MANIFEST_SCHEMA).validate(_manifest(module))
        prov = _manifest(module)["provenance"]
        assert not {"capture_sha256", "scrub_rules", "scrubbed_at"} & prov.keys()


def _forge(tmp_path: Path, out: Path, edit: Any) -> Path:
    """A scrubbed document that someone changed and then gave a matching digest."""
    doc = json.loads(out.read_text())
    edit(doc)
    doc["scrub"]["content_sha256"] = canonical_digest(doc)
    forged = tmp_path / "forged.json"
    forged.write_text(json.dumps(doc))
    return forged


@pytest.mark.parametrize(
    ("edit", "message"),
    [
        (lambda d: d["source"].update(sha256="a" * 64), "another capture"),
        (lambda d: d["scrub"].update(capture_sha256="a" * 64), "another capture"),
        (lambda d: d["scrub"].update(capture_sha256="B501F" + "0" * 59), "64 lowercase"),
        (lambda d: d["scrub"].update(date="10 Nov 2026"), "YYYY-MM-DD"),
        (lambda d: d["scrub"]["rules"][0].update(version=0), "version of 1 or more"),
        (lambda d: d["scrub"]["rules"].append(dict(d["scrub"]["rules"][0])), "twice"),
    ],
)
def test_a_record_that_does_not_match_the_capture_writes_no_provenance(
    tmp_path: Path,
    seeded: Path,
    capsys: pytest.CaptureFixture[str],
    edit: Any,
    message: str,
) -> None:
    out = scrub(tmp_path, seeded)
    assert check_record(json.loads(out.read_text()))[0] is not None
    forged = _forge(tmp_path, out, edit)
    rec, problems = check_record(json.loads(forged.read_text()))
    assert rec is None and any(message in p for p in problems)
    capsys.readouterr()
    module = tmp_path / "parent" / "forged"
    argv = ["scenario", "--sessions", str(forged), "--all", "--out-dir", str(module)]
    assert main(argv) == 0
    assert message in capsys.readouterr().err
    prov = _manifest(module)["provenance"]
    assert prov["scrubbed"] is False and prov["consent"] == "not recorded"
    assert not {"capture_sha256", "scrub_rules", "scrubbed_at"} & prov.keys()
    assert validation_errors(module)  # the validator rejects it


def test_scrub_refuses_without_consent_and_twice(
    tmp_path: Path, seeded: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    argv = ["scrub", "--sessions", str(seeded), "--out", str(tmp_path / "o.json")]
    with pytest.raises(SystemExit):
        main(argv)
    capsys.readouterr()
    assert main([*argv, "--consent", "  "]) == 1
    assert "consent" in capsys.readouterr().err
    out = scrub(tmp_path, seeded)
    again = ["scrub", "--sessions", str(out), "--out", str(tmp_path / "o2.json"), "--consent", "x"]
    assert main(again) == 1
    assert "already" in capsys.readouterr().err


def test_same_seed_same_output_and_other_seed_other_values(tmp_path: Path, seeded: Path) -> None:
    a = _payloads(json.loads(scrub(tmp_path, seeded, "--date", "2026-01-01").read_text()))
    b = _payloads(json.loads(scrub(tmp_path, seeded, "--date", "2026-01-01").read_text()))
    c = _payloads(json.loads(scrub(tmp_path, seeded, "--seed", "7").read_text()))
    assert a == b and a != c


# -- units ------------------------------------------------------------------------------


def test_replacements_keep_length_class_and_never_equal_the_original() -> None:
    s = Scrubber(seed=3)
    for value in ("a", "Abc-123_x", "0123456789abcdef", "ABCDEF0123456789", "p@ss w0rd", "12"):
        repl = s.secret("token", value)
        assert len(repl) == len(value) and repl != value
        assert s.secret("token", value) == repl
    assert re.fullmatch(r"[0-9a-f]{16}", s.secret("token", "0123456789abcdef"))
    assert re.fullmatch(r"[0-9A-F]{16}", s.secret("token", "ABCDEF0123456789"))
    pw = s.secret("pass", "a-b.c_d")
    assert len(pw) == 7 and re.fullmatch(r"[A-Za-z0-9!#$%*._-]+", pw)
    assert s.secret("token", "é\x01ab") != "é\x01ab" and s.secret("token", "é\x01ab").isascii()


def test_addresses_are_allocated_in_documentation_ranges() -> None:
    s = Scrubber(seed=1)
    first = [s.ip_text(f"10.0.{i // 250}.{i % 250 + 1}") for i in range(300)]
    assert first[0] == "192.0.2.1" and first[253] == "192.0.2.254" and first[254] == "198.51.100.1"
    assert len(set(first)) == 300
    assert s.ip_text("127.0.0.1") is None and s.ip_text("0.0.0.0") is None
    assert s.ip_text("999.1.1.1") is None and s.ip_text("::1") is None
    assert s.ip_text("2001:db8::5") == "2001:db8::1"
    assert s.ip_text("2001:DB8:0:0:0:0:0:5") == "2001:db8::1"  # one address, one pseudonym
    assert s.ip_text("fe80::1") == "2001:db8::2"


def test_overlapping_edits_prefer_the_structural_one() -> None:
    from capture.scrub import Edit

    edits = [
        Edit(0, 5, "AAAAA", 2, "sweep"),
        Edit(2, 4, "bb", 0, "smtp_auth"),
        Edit(9, 9, "x", 0, "y"),
    ]
    assert apply_edits("0123456789", resolve(edits)) == "01bb456789"


def test_short_values_are_replaced_where_found(tmp_path: Path) -> None:
    turns = [("c", b"USER al\r\n"), ("s", b"+OK\r\n"), ("c", b"PASS x1\r\n"), ("s", b"+OK\r\n")]
    recs = tcp_session(turns, T0, dport=1110, client=CLIENT4, server=SERVER4)
    pcap = tmp_path / "s.pcap"
    pcap.write_bytes(write_pcap(recs))
    sessions = tmp_path / "s.json"
    assert main(["sessions", "--pcap", str(pcap), "--payloads", "--out", str(sessions)]) == 0
    doc = scrub_document(
        json.loads(sessions.read_text()), consent="c", input_sha256="0" * 64, date="2026-01-01"
    )
    text = b"".join(_payloads(doc))
    assert b"USER al\r" not in text and b"PASS x1\r" not in text
    assert re.search(rb"USER [a-z]{2}\r\n", text) and re.search(rb"PASS [a-z][0-9]\r\n", text)


@pytest.mark.parametrize("name", ["http1", "dns", "smtp", "ftp", "imap", "pop3"])
def test_module_fixtures_scrub_and_still_generate_valid_scenarios(
    tmp_path: Path, name: str
) -> None:
    pcap = REPO / "modules" / name / "fixtures" / f"{name}.pcapng"
    if not pcap.exists():
        pytest.skip(f"no fixture capture of {name}")
    sessions = tmp_path / "s.json"
    assert main(["sessions", "--pcap", str(pcap), "--payloads", "--out", str(sessions)]) == 0
    out = scrub(tmp_path, sessions, "--hostnames")
    module = tmp_path / "parent" / "fx"
    assert main(["scenario", "--sessions", str(out), "--all", "--out-dir", str(module)]) == 0
    assert validation_errors(module) == []
