# SPDX-License-Identifier: GPL-3.0-only
"""Scrubbing of extracted sessions (CAP-03).

A capture is real traffic, so it holds addresses, user names, passwords, tokens and email
addresses. ``scrub_document`` rewrites a session document (with payloads) so that none of
them remain, and records what it did. Only this module may state that a document is
scrubbed: the scenario generator reads the record and checks its digest.

The work has two passes over every payload, so that one value gets one replacement
everywhere in the capture.

1. Discovery. Structural rules (SASL and protocol commands, HTTP headers, cookies, query
   and form parameters, JWTs, API keys, URLs with credentials) and address rules find
   values. Each value gets a synthetic replacement of the same length and character
   classes, drawn in order of discovery from a seeded generator. The same value always
   gets the same replacement. A rule also records an edit at the place where it found the
   value, so a short value is replaced even when no sweep would find it.
2. Sweep. Every learned value is replaced everywhere else, as text, percent encoded, and
   inside Base64 tokens that decode to text that holds it. Edits of both passes refer to
   the original text. Where they overlap, the structural edit wins.

The replacements keep the length of credentials, so byte counts stay close. Addresses move
into the documentation ranges and the length of an address in text changes, which the
record reports.
"""

from __future__ import annotations

import base64
import binascii
import bisect
import copy
import hashlib
import ipaddress
import json
import random
import re
import string
from collections import Counter
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import quote, quote_plus, unquote

SCRUBBER = "capture scrub"
SCRUBBER_VERSION = "1.1.0"
SCRUBBED_SCHEMA_VERSION = "1.1"

# Rule id to rule version. The version rises when a rule changes what it replaces.
RULES: dict[str, int] = {
    "ipv4": 1,
    "ipv6": 1,
    "ip_binary": 1,
    "content_length": 1,
    "email": 1,
    "display_name": 1,
    "hostname": 1,
    "http_authorization": 1,
    "http_cookie": 1,
    "http_header": 1,
    "url_credentials": 1,
    "parameter": 2,  # JSON members by token, any white space, escaped names (#451)
    "jwt": 1,
    "api_key": 1,
    "private_key": 1,
    "smtp_auth": 1,
    "imap_login": 1,
    "pop3": 1,
    "ftp": 1,
    "literal": 1,
    "sweep": 2,  # JSON-escaped forms of a value too (#451)
    "base64": 1,
}

_STRUCT, _PATTERN, _SWEEP = 0, 1, 2  # priority of an edit, lowest wins on overlap

# Longest run of addresses that fits the documentation ranges before the benchmarking range.
_V4_POOLS = ((192, 0, 2), (198, 51, 100), (203, 0, 113))
_SUFFIX = ".example.test"
_PUNCT = "!#$%*-_."
_BOUNDED = {"user", "name"}
_MIN_SWEEP = {"user": 3, "name": 3, "pass": 4, "token": 6, "literal": 1}


class ScrubError(Exception):
    """The document cannot be scrubbed."""


@dataclass(frozen=True)
class Edit:
    """Replace ``text[start:end]``. ``prio`` decides between overlapping edits."""

    start: int
    end: int
    text: str
    prio: int
    rule: str


@dataclass
class Item:
    """One payload: a TCP turn or one UDP datagram."""

    session: int
    direction: int  # 0 client to server, 1 server to client
    udp: bool
    text: str  # latin-1 view of the bytes
    edits: list[Edit] = field(default_factory=list)


# -- replacements ---------------------------------------------------------------------


def _char(c: str, rng: random.Random, keep_punct: bool) -> str:
    if "0" <= c <= "9":
        return rng.choice(string.digits)
    if "a" <= c <= "z":
        return rng.choice(string.ascii_lowercase)
    if "A" <= c <= "Z":
        return rng.choice(string.ascii_uppercase)
    if c in "\r\n":
        return c
    if ord(c) < 32 or ord(c) > 126:
        return rng.choice(string.ascii_lowercase)  # no raw non ASCII byte survives
    return c if keep_punct else rng.choice(_PUNCT)


def _synth(value: str, rng: random.Random, *, keep_punct: bool) -> str:
    """A value of the same length and character classes. Hex stays hex."""
    if len(value) >= 8 and re.fullmatch(r"[0-9a-f]+", value):
        alphabet = "0123456789abcdef"
    elif len(value) >= 8 and re.fullmatch(r"[0-9A-F]+", value):
        alphabet = "0123456789ABCDEF"
    else:
        alphabet = ""
    out = value
    for _ in range(16):
        if alphabet:
            out = "".join(rng.choice(alphabet) for _ in value)
        else:
            out = "".join(_char(c, rng, keep_punct) for c in value)
        if out != value:
            break
    return out


def _is_reserved_host(name: str) -> bool:
    n = name.lower().rstrip(".")
    return (
        n in {"localhost", "example.com", "example.net", "example.org"}
        or n.endswith(_SUFFIX)
        or n == _SUFFIX[1:]
        or n.endswith((".invalid", ".localhost", ".example"))
    )


class Scrubber:
    """The tables of one run: values learned so far and their replacements."""

    def __init__(self, *, seed: int) -> None:
        self.rng = random.Random(seed)
        self.values: dict[str, tuple[str, str]] = {}  # value -> (replacement, kind)
        self.emails: dict[str, str] = {}  # lower case address -> replacement
        self.hosts: dict[str, str] = {}  # lower case name -> replacement
        self.v4: dict[tuple[int, int, int, int], str] = {}
        self.v6: dict[bytes, str] = {}
        self.packed4: dict[bytes, bytes] = {}
        self.packed6: dict[bytes, bytes] = {}

    # values (user names, passwords, tokens, display names, literals)

    def secret(self, kind: str, value: str) -> str:
        known = self.values.get(value)
        if known is not None:
            if kind not in _BOUNDED and known[1] in _BOUNDED:
                self.values[value] = (known[0], kind)  # the wider rule needs no word edge
            return known[0]
        repl = _synth(value, self.rng, keep_punct=kind != "pass")
        self.values[value] = (repl, kind)
        return repl

    # email addresses and host names

    def email(self, address: str) -> str:
        key = address.lower()
        known = self.emails.get(key)
        if known is None:
            local = f"user{len(self.emails) + 1}"
            tail = "@example.test"
            pad = max(0, len(address) - len(local) - len(tail))
            known = local + "x" * pad + tail
            self.emails[key] = known
        domain = address.rsplit("@", 1)[-1]
        if not _is_reserved_host(domain):
            self.host(domain)
        return known

    def host(self, name: str) -> str:
        key = name.lower().rstrip(".")
        known = self.hosts.get(key)
        if known is None:
            label = f"host{len(self.hosts) + 1}"
            pad = max(0, len(key) - len(label) - len(_SUFFIX))
            known = label + "x" * pad + _SUFFIX
            self.hosts[key] = known
        return known

    # addresses

    def ipv4(self, octets: tuple[int, int, int, int]) -> str:
        known = self.v4.get(octets)
        if known is None:
            n = len(self.v4)
            if n < 254 * len(_V4_POOLS):
                a, b, c = _V4_POOLS[n // 254]
                new = (a, b, c, n % 254 + 1)
            else:
                idx = n - 254 * len(_V4_POOLS) + 1  # 198.18.0.0/15 holds the rest
                if idx >= 2**17:
                    raise ScrubError("more than 131000 distinct IPv4 addresses in one capture")
                new = (198, 18 + (idx >> 16), (idx >> 8) & 255, idx & 255)
            known = ".".join(map(str, new))
            self.v4[octets] = known
            self.packed4[bytes(octets)] = bytes(new)
        return known

    def ipv6(self, addr: ipaddress.IPv6Address) -> str:
        known = self.v6.get(addr.packed)
        if known is None:
            n = len(self.v6) + 1
            new = ipaddress.IPv6Address((0x20010DB8 << 96) | n)
            known = new.compressed
            self.v6[addr.packed] = known
            self.packed6[addr.packed] = new.packed
        return known

    def ip_text(self, text: str) -> str | None:
        """The replacement of an address in text, or None for an address that stays."""
        if ":" in text:
            try:
                v6 = ipaddress.IPv6Address(text)
            except ValueError:
                return None
            if v6 in (ipaddress.IPv6Address("::"), ipaddress.IPv6Address("::1")):
                return None
            return self.ipv6(v6)
        parts = text.split(".")
        if len(parts) != 4 or not all(p.isdigit() and int(p) < 256 for p in parts):
            return None
        o = tuple(int(p) for p in parts)
        if o[0] == 127 or o == (0, 0, 0, 0) or o == (255, 255, 255, 255):
            return None
        return self.ipv4((o[0], o[1], o[2], o[3]))


# -- finding values ---------------------------------------------------------------------

_V4 = re.compile(r"(?<![0-9A-Za-z.])(?:\d{1,3}\.){3}\d{1,3}(?![0-9A-Za-z]|\.[0-9])")
_V6 = re.compile(r"(?<![0-9A-Za-z:.])[0-9A-Fa-f:]{2,39}(?![0-9A-Za-z:]|\.[0-9])")
_EMAIL = re.compile(
    r"(?<![A-Za-z0-9._%+-])[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}"
)
_HDR = re.compile(r"(?im)^([A-Za-z][A-Za-z0-9-]*)[ \t]*:[ \t]*([^\r\n]*)")
_LINE = re.compile(r"[^\r\n]+")
_JWT = re.compile(r"\beyJ[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]*")
_API_KEY = re.compile(
    r"\b(?:AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}"
    r"|sk-[A-Za-z0-9_-]{16,}|[sp]k_(?:live|test)_[A-Za-z0-9]{16,}|xox[abprs]-[A-Za-z0-9-]{10,}"
    r"|AIza[0-9A-Za-z_-]{30,}|glpat-[A-Za-z0-9_-]{16,}"
    r"|SG\.[A-Za-z0-9_-]{16,}\.[A-Za-z0-9_-]{16,})"
)
_BEARER = re.compile(r"(?i)\bBearer[ \t]+([A-Za-z0-9._~+/=-]{4,})")
_BASIC = re.compile(r"(?i)\bBasic[ \t]+([A-Za-z0-9+/=]{8,})")
_PRIVATE_KEY = re.compile(r"-----BEGIN ([A-Z ]*PRIVATE KEY)-----([^-]+)-----END \1-----")
_USERINFO = re.compile(r"(?i)\b[a-z][a-z0-9+.-]{1,15}://([^/\s:@?#]*):([^/\s@?#]*)@")
_URL_HOST = re.compile(r"(?i)\b[a-z][a-z0-9+.-]{1,15}://(?:[^/\s@]*@)?([A-Za-z0-9.-]+)")
_DIGEST = re.compile(r'(?i)\b(username|response|cnonce|rspauth)="([^"]*)"')
_NAMES = (
    r"password|passwd|pwd|pass|passphrase|secret|client_secret|token|access_token|refresh_token"
    r"|id_token|auth_token|authtoken|api[_-]?key|apikey|sessionid|session_id|session|sessid|sid"
    r"|jsessionid|phpsessid|auth|authorization|csrf_token|csrftoken|xsrf_token|signature"
    r"|username|user_name|user|login|email|e-mail|mail|account"
)
_PARAM = re.compile(rf"(?i)(?<![A-Za-z0-9_-])(?:[A-Za-z0-9_]*\[)?({_NAMES})\]?=([^&;\s\"'<>]*)")
# JSON is scanned token by token (_json_fields), not with one regular
# expression: a key may be written with escapes, and any JSON white space,
# line breaks included, may stand around the colon (issue #451).
_JSON_STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')
_JSON_NUMBER = re.compile(r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?")
_JSON_WS = " \t\r\n"
_NAME = re.compile(rf"(?i)^(?:{_NAMES})$")
_XML_PARAM = re.compile(rf"(?i)<({_NAMES})>([^<]+)</\1>")
_PASSWORD_NAMES = re.compile(r"(?i)^(?:password|passwd|pwd|pass|passphrase)$")
_USER_NAMES = re.compile(r"(?i)^(?:username|user_name|user|login|account)$")
_EMAIL_NAMES = re.compile(r"(?i)^(?:email|e-mail|mail)$")
_SECRET_HEADER = re.compile(
    r"(?i)(token|api[-_]?key|secret|auth|session|password|credential|csrf|xsrf|signature)"
)
_SKIP_HEADER = re.compile(r"(?i)^(?:access-control-.*|www-authenticate|proxy-authenticate)$")
_SENSITIVE_COOKIE = re.compile(
    r"(?i)(sess|sid|token|auth|jwt|key|secret|pass|pwd|user|login|csrf|xsrf|^id$|_id$|uid"
    r"|account|email|mail)"
)
_BENIGN = {"true", "false", "none", "null", "omit", "include", "same-origin", "*", "0", "1"}
_ADDR_HEADERS = {
    "from", "to", "cc", "bcc", "reply-to", "sender", "delivered-to", "return-path",
}  # fmt: skip
_B64_TOKEN = re.compile(r"(?<![A-Za-z0-9+/=_-])[A-Za-z0-9+/_-]{16,}={0,2}(?![A-Za-z0-9+/=_-])")
_SASL = re.compile(
    r"(?i)^(\d{3}[ -])?(?:\S+ )?(AUTHENTICATE|AUTH)[ \t]+([A-Za-z0-9_-]+)(?:[ \t]+(\S+))?[ \t]*$"
)
# A server lists its mechanisms in a line that looks like a command, such as 250 AUTH PLAIN LOGIN.
_MECHANISMS = {
    "PLAIN", "LOGIN", "CRAM-MD5", "XOAUTH2", "NTLM", "GSSAPI", "DIGEST-MD5", "SCRAM-SHA-1",
    "SCRAM-SHA-256", "OAUTHBEARER", "ANONYMOUS", "EXTERNAL",
}  # fmt: skip
_IMAP_LOGIN = re.compile(
    r'(?i)^\S+[ \t]+LOGIN[ \t]+("(?:[^"\\]|\\.)*"|[^\s"]+)[ \t]+("(?:[^"\\]|\\.)*"|[^\s"]+)[ \t]*$'
)
_USER_CMD = re.compile(r"(?i)^USER[ \t]+(\S+)[ \t]*$")
_PASS_CMD = re.compile(r"(?i)^(?:PASS|ACCT)[ \t]+(.+?)[ \t]*$")
_APOP = re.compile(r"(?i)^APOP[ \t]+(\S+)[ \t]+([0-9a-f]{16,})[ \t]*$")
_FTP_ADDR = re.compile(r"(?i)^(?:PORT[ \t]+|227[ \t].*?\()(\d{1,3}(?:,\d{1,3}){3}),\d+,\d+")
_HELO = re.compile(r"(?i)^(?:EHLO|HELO)[ \t]+([A-Za-z0-9.-]+)[ \t]*$")
_GREETING = re.compile(r"^(?:220|250)[ -]([A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+)\b")
_HOST_HEADER = re.compile(r"(?im)^Host[ \t]*:[ \t]*([A-Za-z0-9.-]+)")


def _decode_b64(token: str) -> bytes | None:
    if not token or not re.fullmatch(r"[A-Za-z0-9+/_-]*={0,2}", token):
        return None
    body = token.rstrip("=")
    if len(body) % 4 == 1:
        return None
    padded = body + "=" * (-len(body) % 4)
    alt = b"-_" if ("-" in body or "_" in body) else None
    try:
        return base64.b64decode(padded, altchars=alt, validate=True)
    except (binascii.Error, ValueError):
        return None


def _encode_like(data: bytes, like: str) -> str:
    out = base64.b64encode(data).decode("ascii")
    if "-" in like or "_" in like:
        out = out.replace("+", "-").replace("/", "_")
    return out if like.endswith("=") else out.rstrip("=")


class _Finder:
    """Rules that read one payload and return edits. They learn values on the way."""

    def __init__(self, s: Scrubber, *, hostnames: bool) -> None:
        self.s = s
        self.hostnames = hostnames

    # helpers

    def _put(
        self, edits: list[Edit], start: int, end: int, text: str, rule: str, prio: int = _STRUCT
    ) -> None:
        edits.append(Edit(start, end, text, prio, rule))

    def _secret(
        self,
        edits: list[Edit],
        kind: str,
        value: str,
        start: int,
        rule: str,
        *,
        prio: int = _STRUCT,
    ) -> None:
        if value and not (kind == "token" and value.lower() in _BENIGN):
            self._put(edits, start, start + len(value), self.s.secret(kind, value), rule, prio)

    def _b64(
        self,
        edits: list[Edit],
        tok: str,
        start: int,
        rule: str,
        fn: Callable[[str], str],
    ) -> None:
        raw = _decode_b64(tok)
        if raw is None:
            self._secret(edits, "token", tok, start, rule)
            return
        old = raw.decode("latin-1")
        new = fn(old)
        if new == old:
            return
        self._put(edits, start, start + len(tok), _encode_like(new.encode("latin-1"), tok), rule)

    def _user_pass(self, s: str) -> str:
        user, sep, pw = s.partition(":")
        return self.s.secret("user", user) + sep + self.s.secret("pass", pw) if sep else s

    def _plain(self, s: str) -> str:
        parts = s.split("\0")
        if len(parts) == 3:
            kinds = ["user", "user", "pass"]
        elif len(parts) == 2:
            kinds = ["user", "pass"]
        else:
            return self.s.secret("token", s)
        return "\0".join(self.s.secret(k, p) if p else p for k, p in zip(kinds, parts, strict=True))

    def _cram(self, s: str) -> str:
        user, sep, digest = s.rpartition(" ")
        if not sep:
            return self.s.secret("token", s)
        return self.s.secret("user", user) + " " + self.s.secret("token", digest)

    def _xoauth(self, s: str) -> str:
        def user(m: re.Match[str]) -> str:
            return "user=" + self.s.secret("user", m.group(1))

        def tok(m: re.Match[str]) -> str:
            return "auth=Bearer " + self.s.secret("token", m.group(1))

        return re.sub(r"auth=Bearer ([^\x01]*)", tok, re.sub(r"user=([^\x01]*)", user, s))

    def _sasl(self, edits: list[Edit], mech: str, tok: str, start: int) -> None:
        mech = mech.upper()
        fns = {
            "PLAIN": self._plain,
            "LOGIN": lambda s: self.s.secret("user", s),
            "CRAM-MD5": self._cram,
            "XOAUTH2": self._xoauth,
        }
        self._b64(
            edits, tok, start, "smtp_auth", fns.get(mech, lambda s: self.s.secret("token", s))
        )

    # rules

    def addresses(self, text: str, edits: list[Edit]) -> None:
        for m in _V4.finditer(text):
            new = self.s.ip_text(m.group(0))
            if new:
                self._put(edits, m.start(), m.end(), new, "ipv4", _PATTERN)
        for m in _V6.finditer(text):
            if m.group(0).count(":") < 2:
                continue
            new = self.s.ip_text(m.group(0))
            if new:
                self._put(edits, m.start(), m.end(), new, "ipv6", _PATTERN)

    def emails(self, text: str, edits: list[Edit]) -> None:
        for m in _EMAIL.finditer(text):
            if m.group(0).lower().endswith("@example.test"):
                continue
            self._put(edits, m.start(), m.end(), self.s.email(m.group(0)), "email", _PATTERN)

    def headers(self, text: str, edits: list[Edit]) -> None:
        for m in _HDR.finditer(text):
            name = m.group(1).lower()
            value = m.group(2)
            start = m.start(2)
            if not value.strip():
                continue
            if name in {"authorization", "proxy-authorization"}:
                self._authorization(edits, value, start)
            elif name in {"cookie", "cookie2"}:
                for c in re.finditer(r"([^=;\s][^=;]*)=([^;]*)", value):
                    self._cookie(edits, c.group(1), c.group(2), start + c.start(2))
            elif name in {"set-cookie", "set-cookie2"}:
                c2 = re.match(r"\s*([^=;\s][^=;]*)=([^;]*)", value)
                if c2:
                    self._cookie(edits, c2.group(1), c2.group(2), start + c2.start(2))
            elif name in {"authentication-info", "proxy-authentication-info"}:
                self._digest(edits, value, start)
            elif name in _ADDR_HEADERS:
                self._display_names(edits, value, start)
            elif _SECRET_HEADER.search(name) and not _SKIP_HEADER.match(name):
                self._secret(edits, "token", value.rstrip(), start, "http_header")

    def _authorization(self, edits: list[Edit], value: str, start: int) -> None:
        m = re.match(r"(\S+)[ \t]+(\S.*?)[ \t]*$", value)
        if not m:
            self._secret(edits, "token", value.strip(), start + len(value) - len(value.lstrip()),
                         "http_authorization")  # fmt: skip
            return
        scheme, rest, at = m.group(1).lower(), m.group(2), start + m.start(2)
        if scheme == "basic":
            self._b64(edits, rest, at, "http_authorization", self._user_pass)
        elif scheme == "digest":
            self._digest(edits, rest, at)
        else:
            for r in re.finditer(r"[A-Za-z0-9+/=_.~:-]{4,}", rest):
                self._secret(edits, "token", r.group(0), at + r.start(), "http_authorization")

    def _digest(self, edits: list[Edit], text: str, start: int) -> None:
        for m in _DIGEST.finditer(text):
            kind = "user" if m.group(1).lower() == "username" else "token"
            self._secret(edits, kind, m.group(2), start + m.start(2), "http_authorization")

    def _cookie(self, edits: list[Edit], name: str, value: str, start: int) -> None:
        v = value.rstrip()
        if v and (len(v) >= 8 or _SENSITIVE_COOKIE.search(name.strip())):
            self._secret(edits, "token", v, start, "http_cookie")

    def _display_names(self, edits: list[Edit], value: str, start: int) -> None:
        for m in re.finditer(r'"([^"]{2,})"\s*<|(?:^|,)[ \t]*([^<>",@]{2,}?)[ \t]*<', value):
            g = 1 if m.group(1) else 2
            self._secret(edits, "name", m.group(g), start + m.start(g), "display_name")

    def tokens(self, text: str, edits: list[Edit]) -> None:
        for m in _JWT.finditer(text):
            self._secret(edits, "token", m.group(0), m.start(), "jwt", prio=_PATTERN)
        for m in _API_KEY.finditer(text):
            self._secret(edits, "token", m.group(0), m.start(), "api_key", prio=_PATTERN)
        for m in _BEARER.finditer(text):
            self._secret(
                edits, "token", m.group(1), m.start(1), "http_authorization", prio=_PATTERN
            )
        for m in _BASIC.finditer(text):
            sub: list[Edit] = []
            self._b64(sub, m.group(1), m.start(1), "http_authorization", self._user_pass)
            edits.extend(Edit(e.start, e.end, e.text, _PATTERN, e.rule) for e in sub)
        for m in _PRIVATE_KEY.finditer(text):
            self._secret(edits, "token", m.group(2), m.start(2), "private_key", prio=_PATTERN)
        for m in _USERINFO.finditer(text):
            self._secret(edits, "user", m.group(1), m.start(1), "url_credentials", prio=_PATTERN)
            self._secret(edits, "pass", m.group(2), m.start(2), "url_credentials", prio=_PATTERN)

    def parameters(self, text: str, edits: list[Edit]) -> None:
        found: list[tuple[str, str, int, int, str]] = []  # name, raw, start, end, form
        for pat, form in ((_PARAM, "form"), (_XML_PARAM, "xml")):
            for m in pat.finditer(text):
                found.append((m.group(1), m.group(2), m.start(2), m.end(2), form))
        for n, r, a, b in _json_fields(text):
            found.append((n, r, a, b, "json" if text[a - 1 : a] == '"' else "number"))
        for name, raw, start, end, form in found:
            value = unquote(raw) if form == "form" else raw
            if form == "json":
                value = _json_unescape(raw)
            if not value:
                continue
            if _PASSWORD_NAMES.match(name):
                kind = "pass"
            elif _USER_NAMES.match(name) or (_EMAIL_NAMES.match(name) and "@" not in value):
                kind = "user"
            elif _EMAIL_NAMES.match(name):
                repl = self.s.email(value)
                if form == "form":
                    repl = quote(repl, safe="@")
                elif form == "json":
                    repl = _json_escape(repl)
                self._put(edits, start, end, repl, "parameter", _PATTERN)
                continue
            else:
                kind = "token"
                if value.lower() in _BENIGN:
                    continue
            repl = self.s.secret(kind, value)
            if form == "form":
                repl = quote(repl, safe="/+=-_.~:!*")
            elif form == "json":
                repl = _json_escape(repl)
            elif form == "number":
                repl = _number_like(value, repl)
            self._put(edits, start, end, repl, "parameter", _PATTERN)

    def host_names(self, text: str) -> None:
        """Learn host names from their usual places. The sweep replaces them."""
        if not self.hostnames:
            return
        found: list[str] = [m.group(1) for m in _HOST_HEADER.finditer(text)]
        found += [m.group(1) for m in _URL_HOST.finditer(text)]
        for ln in _LINE.finditer(text):
            line = ln.group(0)
            for pat in (_HELO, _GREETING):
                m = pat.match(line)
                if m:
                    found.append(m.group(1))
        for name in found:
            name = name.strip(".")
            if len(name) >= 4 and re.search(r"[A-Za-z]", name) and not _is_reserved_host(name):
                self.s.host(name)

    def lines(self, item: Item, state: dict[int, tuple[str, int]], edits: list[Edit]) -> None:
        """Protocol commands, one line at a time, with the state of a login dialog."""
        for ln in _LINE.finditer(item.text):
            line, at = ln.group(0), ln.start()
            st = state.get(item.session)
            if st is not None and st[1] == item.direction:
                state.pop(item.session)
                if line.strip() in {"*", ""}:
                    continue
                if st[0] == "plain":
                    self._b64(edits, line.strip(), at, "smtp_auth", self._plain)
                elif st[0] == "login_user":
                    self._b64(
                        edits, line.strip(), at, "smtp_auth", lambda s: self.s.secret("user", s)
                    )
                    state[item.session] = ("login_pass", item.direction)
                elif st[0] == "login_pass":
                    self._b64(
                        edits, line.strip(), at, "smtp_auth", lambda s: self.s.secret("pass", s)
                    )
                elif st[0] == "cram":
                    self._b64(edits, line.strip(), at, "smtp_auth", self._cram)
                elif st[0] == "xoauth2":
                    self._b64(edits, line.strip(), at, "smtp_auth", self._xoauth)
                else:
                    self._secret(edits, "token", line.strip(), at, "smtp_auth")
                continue
            m = _SASL.match(line)
            if m and not (m.group(1) and m.group(2).upper() == "AUTH"):
                mech, arg = m.group(3).upper(), m.group(4)
                if arg and arg.upper() in _MECHANISMS:
                    continue  # a list of mechanisms, not a response
                if arg:
                    self._sasl(edits, mech, arg, at + m.start(4))
                    if mech == "LOGIN":
                        state[item.session] = ("login_pass", item.direction)
                else:
                    first = {"PLAIN": "plain", "LOGIN": "login_user", "CRAM-MD5": "cram",
                             "XOAUTH2": "xoauth2"}  # fmt: skip
                    state[item.session] = (first.get(mech, "opaque"), item.direction)
                continue
            m = _IMAP_LOGIN.match(line)
            if m:
                for g, kind in ((1, "user"), (2, "pass")):
                    arg = m.group(g)
                    quoted = arg.startswith('"')
                    inner = arg[1:-1] if quoted else arg
                    self._secret(edits, kind, inner, at + m.start(g) + quoted, "imap_login")
                continue
            m = _USER_CMD.match(line)
            if m:
                self._secret(edits, "user", m.group(1), at + m.start(1), "pop3")
                continue
            m = _PASS_CMD.match(line)
            if m:
                self._secret(edits, "pass", m.group(1), at + m.start(1), "ftp")
                continue
            m = _APOP.match(line)
            if m:
                self._secret(edits, "user", m.group(1), at + m.start(1), "pop3")
                self._secret(edits, "token", m.group(2), at + m.start(2), "pop3")
                continue
            m = _FTP_ADDR.match(line)
            if m:
                octets = tuple(int(p) for p in m.group(1).split(","))
                if len(octets) == 4 and all(o < 256 for o in octets):
                    new = self.s.ip_text(".".join(map(str, octets)))
                    if new:
                        self._put(edits, at + m.start(1), at + m.end(1), new.replace(".", ","),
                                  "ftp")  # fmt: skip


def _json_fields(text: str) -> list[tuple[str, str, int, int]]:
    """The values of sensitive JSON members in text, as (name, raw, start, end).

    It walks the JSON string tokens in order, decodes each one, and takes it as
    a member name when JSON white space and a colon follow. The value of a
    sensitive member is a string (raw is its escaped content), a number, or an
    array, whose strings and numbers are all taken, nested arrays included.
    Objects under a sensitive name are walked on their own, since their members
    have names of their own. Text that is not JSON yields nothing more than it
    would to a parser that stops at the first error, so the scan is bounded by
    the length of the text.
    """
    out: list[tuple[str, str, int, int]] = []
    n = len(text)

    def skip(i: int) -> int:
        while i < n and text[i] in _JSON_WS:
            i += 1
        return i

    def value_at(name: str, i: int) -> int:
        """Take the value at i. Return the index after it, or i if none."""
        if i >= n:
            return i
        if text[i] == '"':
            v = _JSON_STRING.match(text, i)
            if v:
                out.append((name, v.group(1), v.start(1), v.end(1)))
                return v.end()
            return i
        if text[i] == "[":
            depth, j = 0, i
            while j < n:
                c = text[j]
                if c == '"':
                    v = _JSON_STRING.match(text, j)
                    if not v:
                        return j
                    out.append((name, v.group(1), v.start(1), v.end(1)))
                    j = v.end()
                    continue
                if c in "[":
                    depth += 1
                elif c == "]":
                    depth -= 1
                    if depth == 0:
                        return j + 1
                elif c == "{":
                    return j  # members of an object are names of their own
                else:
                    num = _JSON_NUMBER.match(text, j)
                    if num and num.end() > j:
                        out.append((name, num.group(0), num.start(), num.end()))
                        j = num.end()
                        continue
                j += 1
            return j
        num = _JSON_NUMBER.match(text, i)
        if num and num.end() > i:
            out.append((name, num.group(0), num.start(), num.end()))
            return num.end()
        return i

    for m in _JSON_STRING.finditer(text):
        j = skip(m.end())
        if j >= n or text[j] != ":":
            continue
        name = _json_unescape(m.group(1))
        if _NAME.match(name):
            value_at(name, skip(j + 1))
    return out


def _number_like(orig: str, repl: str) -> str:
    """A JSON number of the shape of orig whose digits come from repl, so a
    numeric secret is replaced and the body stays valid JSON."""
    out, k = [], 0
    for i, c in enumerate(orig):
        if c.isdigit():
            d = ord(repl[k % len(repl)]) % 10 if repl else 0
            k += 1
            lead = i == 0 or orig[i - 1] in "-."
            if lead and d == 0 and len(orig) > i + 1 and orig[i + 1].isdigit():
                d = 1 + ord(repl[k % len(repl)]) % 9 if repl else 1
            out.append(str(d))
        else:
            out.append(c)
    new = "".join(out)
    return new if new != orig else new[:-1] + str((int(new[-1]) + 1) % 10)


def _json_escape(value: str) -> str:
    """The content of a JSON string literal that holds value."""
    return json.dumps(value, ensure_ascii=False)[1:-1]


def _json_unescape(raw: str) -> str:
    try:
        out = json.loads(f'"{raw}"')
    except ValueError:
        return raw
    return out if isinstance(out, str) else raw


# -- the sweep --------------------------------------------------------------------------


def _quote_all(value: str) -> str:
    return quote(value, safe="")


def _json_ascii(value: str) -> str:
    return json.dumps(value, ensure_ascii=True)[1:-1]


def _variants(orig: str, repl: str) -> list[tuple[str, str]]:
    out = [(orig, repl)]
    # A secret echoed inside a JSON string is escaped there (issue #451).
    fns: tuple[Callable[[str], str], ...] = (
        _quote_all,
        quote_plus,
        quote,
        _json_escape,
        _json_ascii,
    )
    for fn in fns:
        o, r = fn(orig), fn(repl)
        if o != orig and (o, r) not in out:
            out.append((o, r))
    return out


def _alternation(keys: list[str]) -> str:
    return "|".join(re.escape(k) for k in sorted(keys, key=len, reverse=True))


class Sweeper:
    """Replaces every learned value in a text. Built after discovery has finished."""

    def __init__(self, s: Scrubber) -> None:
        self.s = s
        self.exact: dict[str, str] = {}
        self.bounded: dict[str, str] = {}
        for value, (repl, kind) in s.values.items():
            if len(value) < _MIN_SWEEP[kind]:
                continue
            table = self.bounded if kind in _BOUNDED else self.exact
            for o, r in _variants(value, repl):
                table.setdefault(o, r)
        self.mail: dict[str, str] = {}
        for addr, repl in s.emails.items():
            for o, r in _variants(addr, repl):
                self.mail.setdefault(o.lower(), r)
        self.re_exact = re.compile(_alternation(list(self.exact))) if self.exact else None
        self.re_bounded = (
            re.compile(rf"(?<![A-Za-z0-9])(?:{_alternation(list(self.bounded))})(?![A-Za-z0-9])")
            if self.bounded
            else None
        )
        self.re_mail = (
            re.compile(_alternation(list(self.mail)), re.IGNORECASE) if self.mail else None
        )
        self.re_host = (
            re.compile(
                rf"(?<![A-Za-z0-9])(?!\d{{3}}-)(?:[A-Za-z0-9-]+\.)*(?:{_alternation(list(s.hosts))})"
                r"(?![A-Za-z0-9-])",
                re.IGNORECASE,
            )
            if s.hosts
            else None
        )
        packed = [re.escape(k.decode("latin-1")) for k in s.packed6]
        self.re_v6 = re.compile("|".join(packed)) if packed else None
        packed4 = [re.escape(k.decode("latin-1")) for k in s.packed4]
        self.re_v4 = re.compile("|".join(packed4)) if packed4 else None

    def edits(self, text: str, *, binary: bool, udp: bool) -> list[Edit]:
        out: list[Edit] = []
        for rx, table, rule, lower in (
            (self.re_exact, self.exact, "sweep", False),
            (self.re_bounded, self.bounded, "sweep", False),
            (self.re_mail, self.mail, "sweep", True),
        ):
            if rx is None:
                continue
            for m in rx.finditer(text):
                key = m.group(0).lower() if lower else m.group(0)
                out.append(Edit(m.start(), m.end(), table[key], _SWEEP, rule))
        if self.re_host is not None:
            for m in self.re_host.finditer(text):
                out.append(Edit(m.start(), m.end(), self.s.host(m.group(0)), _SWEEP, "hostname"))
        if binary:
            pairs = [(self.re_v6, self.s.packed6)]
            if udp:
                pairs.append((self.re_v4, self.s.packed4))
            for rx, packed in pairs:
                if rx is None:
                    continue
                for m in rx.finditer(text):
                    new = packed[m.group(0).encode("latin-1")].decode("latin-1")
                    out.append(Edit(m.start(), m.end(), new, _SWEEP, "ip_binary"))
        return out

    def string(self, text: str) -> str:
        return apply_edits(text, resolve(self.edits(text, binary=False, udp=False)))

    def base64_edits(self, text: str) -> list[Edit]:
        out: list[Edit] = []
        for m in _B64_TOKEN.finditer(text):
            raw = _decode_b64(m.group(0))
            if raw is None:
                continue
            decoded = raw.decode("latin-1")
            new = self.string(decoded)
            if new != decoded:
                out.append(
                    Edit(
                        m.start(),
                        m.end(),
                        _encode_like(new.encode("latin-1"), m.group(0)),
                        _SWEEP,
                        "base64",
                    )
                )
        return out


_CONTENT_LENGTH = re.compile(r"(?im)^Content-Length[ \t]*:[ \t]*(\d+)[ \t]*\r?\n")


def _content_lengths(text: str, edits: list[Edit]) -> list[Edit]:
    """Corrections of Content-Length for bodies whose size the edits changed.

    Only a body that lies completely in the same payload can be followed. A body that
    runs into the next turn keeps its header, and the generator then falls back."""
    out: list[Edit] = []
    for m in _CONTENT_LENGTH.finditer(text):
        end_of_head = text.find("\r\n\r\n", m.start())
        if end_of_head < 0:
            continue
        body = end_of_head + 4
        size = int(m.group(1))
        if body + size > len(text):
            continue
        delta = sum(
            len(e.text) - (e.end - e.start)
            for e in edits
            if e.start >= body and e.end <= body + size
        )
        if delta:
            out.append(
                Edit(m.start(1), m.end(1), str(max(0, size + delta)), _STRUCT, "content_length")
            )
    return out


def resolve(edits: list[Edit]) -> list[Edit]:
    """Keep the edits that do not overlap a stronger one, in text order."""
    chosen: list[Edit] = []
    starts: list[int] = []
    for e in sorted(edits, key=lambda e: (e.prio, e.start - e.end, e.start)):
        if e.start >= e.end:
            continue
        i = bisect.bisect_left(starts, e.start)
        if i > 0 and chosen[i - 1].end > e.start:
            continue
        if i < len(chosen) and chosen[i].start < e.end:
            continue
        starts.insert(i, e.start)
        chosen.insert(i, e)
    return chosen


def apply_edits(text: str, edits: list[Edit]) -> str:
    """Apply resolved edits, which are sorted and do not overlap."""
    out: list[str] = []
    pos = 0
    for e in edits:
        out.append(text[pos : e.start])
        out.append(e.text)
        pos = e.end
    out.append(text[pos:])
    return "".join(out)


# -- the document -----------------------------------------------------------------------


def _printable(data: bytes) -> bool:
    if not data:
        return True
    ok = sum(1 for b in data if 32 <= b < 127 or b in (9, 10, 13))
    return ok / len(data) >= 0.85


def canonical_digest(doc: dict[str, Any]) -> str:
    """SHA-256 of the document without its own digest, as compact sorted JSON."""
    body = copy.deepcopy(doc)
    rec = body.get("scrub")
    if isinstance(rec, dict):
        rec.pop("content_sha256", None)
    text = json.dumps(body, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha256(text.encode("ascii")).hexdigest()


def sha256_bytes(data: bytes) -> str:
    """Hex SHA-256 of ``data``."""
    return hashlib.sha256(data).hexdigest()


def scrub_document(
    doc: dict[str, Any],
    *,
    consent: str,
    input_sha256: str,
    date: str,
    seed: int = 1,
    hostnames: bool = False,
    extra_hosts: list[str] | None = None,
    literals: list[str] | None = None,
    drop_ports: list[int] | None = None,
) -> dict[str, Any]:
    """Return a scrubbed copy of a session document, with the ``scrub`` record added."""
    if not consent.strip():
        raise ScrubError("a consent note is required, for example --consent 'owner approved'")
    if "scrub" in doc:
        raise ScrubError("the document is scrubbed already; scrub the original sessions instead")
    out = copy.deepcopy(doc)
    dropped = 0
    if drop_ports:
        keep = [
            s
            for s in out["sessions"]
            if s["client"]["port"] not in drop_ports and s["server"]["port"] not in drop_ports
        ]
        dropped = len(out["sessions"]) - len(keep)
        ids = {s["id"]: n for n, s in enumerate(keep, 1)}
        for s in keep:
            s["id"] = ids[s["id"]]
            s["continues"] = ids.get(s["continues"]) if s["continues"] else None
        out["sessions"] = keep
        out["summary"] = {
            "sessions": len(keep),
            "tcp": sum(s["protocol"] == "tcp" for s in keep),
            "udp": sum(s["protocol"] == "udp" for s in keep),
        }

    s = Scrubber(seed=seed)
    finder = _Finder(s, hostnames=hostnames)
    for name in extra_hosts or []:
        s.host(name)
    for lit in literals or []:
        if lit:
            s.secret("literal", lit)

    # endpoints first, so that the first pseudonyms follow the order of the sessions
    for sess in out["sessions"]:
        for end in ("client", "server"):
            s.ip_text(sess[end]["ip"])

    items = _items(out)
    before = sum(len(i.text) for i in items)
    state: dict[int, tuple[str, int]] = {}
    for item in items:
        t = item.text
        finder.addresses(t, item.edits)
        finder.emails(t, item.edits)
        finder.headers(t, item.edits)
        finder.tokens(t, item.edits)
        finder.parameters(t, item.edits)
        finder.lines(item, state, item.edits)
        finder.host_names(t)

    sweeper = Sweeper(s)
    applied: Counter[str] = Counter()
    opaque = 0
    for item in items:
        binary = not _printable(item.text.encode("latin-1"))
        opaque += len(item.text) if binary else 0
        item.edits += sweeper.edits(item.text, binary=binary, udp=item.udp)
        item.edits += sweeper.base64_edits(item.text)
        final = resolve(item.edits)
        final = resolve([*final, *_content_lengths(item.text, final)])
        applied.update(e.rule for e in final)
        item.text = apply_edits(item.text, final)
    _write_back(out, items)
    after = sum(len(i.text) for i in items)

    # endpoints, interface names, file name, warnings
    for sess in out["sessions"]:
        for end in ("client", "server"):
            new = s.ip_text(sess[end]["ip"])
            if new:
                sess[end]["ip"] = new
                applied["ipv6" if ":" in new else "ipv4"] += 1
    src = out["source"]
    for iface in src.get("interfaces", []):
        if iface.get("name") is not None:
            iface["name"] = f"if{iface['id']}"
    capture_sha = src.get("sha256")
    ext = src.get("format", "pcap")
    src["file"] = f"capture-{capture_sha[:12]}.{ext}" if capture_sha else f"capture.{ext}"
    src["warnings"] = [sweeper.string(_free_text(finder, w)) for w in src.get("warnings", [])]

    rules = [
        {"id": r, "version": v, "applied": applied.get(r, 0)}
        for r, v in RULES.items()
        if r != "hostname" or hostnames or applied.get("hostname", 0)
    ]
    out["schema_version"] = SCRUBBED_SCHEMA_VERSION
    out["scrub"] = {
        "tool": SCRUBBER,
        "version": SCRUBBER_VERSION,
        "date": date,
        "consent": consent.strip(),
        "capture_sha256": capture_sha,
        "input_sha256": input_sha256,
        "seed": seed,
        "options": {
            "hostnames": hostnames,
            "extra_hostnames": len(extra_hosts or []),
            "literals": len([x for x in literals or [] if x]),
            "dropped_ports": sorted(drop_ports or []),
            "dropped_sessions": dropped,
        },
        "rules": rules,
        "learned": {
            "addresses": len(s.v4) + len(s.v6),
            "emails": len(s.emails),
            "hostnames": len(s.hosts),
            "values": len(s.values),
        },
        "payload_bytes": {"before": before, "after": after, "delta": after - before},
        "opaque_bytes": opaque,
    }
    out["scrub"]["content_sha256"] = canonical_digest(out)
    return out


def _free_text(finder: _Finder, text: str) -> str:
    edits: list[Edit] = []
    finder.addresses(text, edits)
    finder.emails(text, edits)
    return apply_edits(text, resolve(edits))


def _items(doc: dict[str, Any]) -> list[Item]:
    items: list[Item] = []
    for sess in doc["sessions"]:
        udp = sess["protocol"] == "udp"
        for turn in sess["turns"]:
            d = 0 if turn["direction"] == "client_to_server" else 1
            blobs = turn.get("datagrams_b64") if udp else [turn.get("data_b64")]
            for b in blobs or []:
                if b is None:
                    continue
                text = base64.b64decode(b).decode("latin-1")
                items.append(Item(sess["id"], d, udp, text))
    return items


def _write_back(doc: dict[str, Any], items: list[Item]) -> None:
    it = iter(items)
    for sess in doc["sessions"]:
        udp = sess["protocol"] == "udp"
        delta = [0, 0]
        for turn in sess["turns"]:
            d = 0 if turn["direction"] == "client_to_server" else 1
            key = "datagrams_b64" if udp else "data_b64"
            if key not in turn:
                continue
            old = turn["bytes"]
            if udp:
                new_blobs = [next(it).text.encode("latin-1") for _ in turn[key]]
                turn[key] = [base64.b64encode(b).decode("ascii") for b in new_blobs]
                turn["datagram_sizes"] = [len(b) for b in new_blobs]
                turn["bytes"] = sum(turn["datagram_sizes"])
            else:
                blob = next(it).text.encode("latin-1")
                turn[key] = base64.b64encode(blob).decode("ascii")
                turn["bytes"] = len(blob)
            delta[d] += turn["bytes"] - old
        for d, name in enumerate(("client_to_server", "server_to_client")):
            c = sess[name]
            for field_name in ("ip_bytes", "payload_bytes", "stream_bytes"):
                if field_name in c:
                    c[field_name] = max(0, c[field_name] + delta[d])


_SHA256 = re.compile(r"[0-9a-f]{64}")
_RULE_ID = re.compile(r"[a-z][a-z0-9_]*")
_DATE = re.compile(r"\d{4}-\d{2}-\d{2}")


def _record_problems(rec: dict[str, Any], doc: dict[str, Any]) -> list[str]:
    """What keeps a scrub record from becoming manifest provenance, or from matching its capture.

    The manifest schema (v0.4) takes the capture digest, the rules and the date as they stand,
    so a record that holds them in another shape is not intact."""
    problems = []
    sha = rec.get("capture_sha256")
    if sha is not None and not (isinstance(sha, str) and _SHA256.fullmatch(sha)):
        problems.append(
            "the scrub record holds a capture digest that is not 64 lowercase hex digits"
        )
    source = doc.get("source")
    source_sha = source.get("sha256") if isinstance(source, dict) else None
    if sha != source_sha:
        problems.append(
            "the scrub record names another capture than the document does "
            "(the capture digests differ)"
        )
    date = rec.get("date")
    if not (isinstance(date, str) and _DATE.fullmatch(date)):
        problems.append("the scrub record holds a date that is not YYYY-MM-DD")
    rules: Any = rec.get("rules")
    ok = isinstance(rules, list) and all(
        isinstance(r, dict)
        and isinstance(r.get("id"), str)
        and _RULE_ID.fullmatch(r["id"])
        and isinstance(r.get("version"), int)
        and not isinstance(r["version"], bool)
        and r["version"] >= 1
        for r in rules
    )
    if not ok:
        problems.append(
            "the scrub record holds rules without an identifier and a version of 1 or more"
        )
    elif len({r["id"] for r in rules}) != len(rules):
        problems.append("the scrub record lists a rule twice")
    return problems


def check_record(doc: dict[str, Any]) -> tuple[dict[str, Any] | None, list[str]]:
    """The scrub record of a document if it is intact and matches its capture, and the problems
    if it does not.

    An intact record shows that the document was not changed after `capture scrub` ran, with the
    rule versions it lists. It does not show that every credential was found: that is the work of
    the rules, and a record written by an older rule version (see RULES) may describe a document
    that a newer rule would scrub further (issue #451)."""
    rec = doc.get("scrub")
    if rec is None:
        return None, []
    if not isinstance(rec, dict) or rec.get("tool") != SCRUBBER:
        return None, ["the scrub record was not written by `capture scrub`"]
    if rec.get("content_sha256") != canonical_digest(doc):
        return None, ["the document changed after it was scrubbed (its digest does not match)"]
    if not str(rec.get("consent", "")).strip():
        return None, ["the scrub record holds no consent note"]
    problems = _record_problems(rec, doc)
    if problems:
        return None, problems
    return rec, []


def provenance_fields(rec: dict[str, Any]) -> dict[str, Any]:
    """The provenance fields of module manifest schema v0.4 from an intact scrub record.

    ``capture_sha256`` is left out when the sessions document did not record the digest.
    The fields never carry a removed value, because the record holds none."""
    fields: dict[str, Any] = {}
    if rec["capture_sha256"] is not None:
        fields["capture_sha256"] = rec["capture_sha256"]
    fields["scrub_rules"] = [{"id": r["id"], "version": r["version"]} for r in rec["rules"]]
    fields["scrubbed_at"] = rec["date"]
    return fields


def provenance_notes(rec: dict[str, Any]) -> str:
    """The text for the ``notes`` of the manifest provenance: what the three fields do not hold."""
    pb = rec["payload_bytes"]
    return (
        f"Scrubbed by {rec['tool']} {rec['version']}. "
        f"Sessions sha256 {rec['input_sha256']}. "
        f"Payload bytes {pb['before']} to {pb['after']} ({pb['delta']:+d})."
    )
