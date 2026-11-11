# SPDX-License-Identifier: GPL-3.0-only
"""Encryption check of the realism runner.

The specification asks for real TLS handshakes with common browser fingerprints,
checked by decrypting a sample with the exported session keys and by computing
JA4 fingerprints. The check takes the sessions that the labels tie to a client
profile, samples up to ``tls.sample`` of them, and reports two things for the
sample.

* The share of sessions that tshark decrypts with the keys of the capture (the
  decryption secrets blocks of a pcapng file) or with a sidecar key log in the NSS
  format. A session counts as decrypted when tshark reads the Finished message of
  both sides, which both TLS versions encrypt, so a key log that lacks the
  handshake secrets decrypts nothing.
* The JA4 fingerprint of each sampled ClientHello, computed by :mod:`realism.ja4`
  from the ClientHello bytes, next to the fingerprint that tshark computes where
  its version has one. A session passes when its JA4 is one that the profile lists.

The check fails when too few sessions decrypt, when one sampled ClientHello has
another JA4 than its profile, or when tshark and the Python code disagree.
"""

from __future__ import annotations

import struct
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Protocol

from realism import ja4
from realism.capture import Session, SessionKey
from realism.labels import Labels, TlsLabels
from realism.parsers import (
    _AGGREGATOR,
    DEFAULT_TIMEOUT_S,
    TSHARK,
    ParserError,
    TsharkRunner,
    _first,
    _int,
    _ip,
    _port,
    _run,
)
from realism.series import failures_series

CHECK_NAME = "encryption"
DECRYPTED_METRIC = "realism.decrypted_ratio"
JA4_METRIC = "realism.ja4_match_ratio"
MAX_SESSIONS = 100

_HANDSHAKE_CLIENT_HELLO = 1
_HANDSHAKE_FINISHED = 20
_PCAPNG_DSB = 0x0000000A
_PCAPNG_SHB = 0x0A0D0D0A
_FIELDS = (
    "frame.number",
    "ip.src",
    "ip.dst",
    "ipv6.src",
    "ipv6.dst",
    "tcp.srcport",
    "tcp.dstport",
    "tls.handshake.type",
    "tls.handshake.ja4",
    "tcp.reassembled.data",
    "tcp.payload",
)
Endpoint = tuple[str, int]


@dataclass(frozen=True)
class Hello:
    """A ClientHello that a probe found in a session."""

    key: SessionKey
    frame: int
    data: bytes
    tshark_ja4: str | None = None


@dataclass(frozen=True)
class Finished:
    """A Finished message that a probe read, which only happens after decryption."""

    key: SessionKey
    sender: Endpoint


@dataclass
class TlsObservation:
    """What a probe saw in a capture, with the tool version for the report."""

    tool: str
    version: str
    hellos: list[Hello] = field(default_factory=list)
    finished: list[Finished] = field(default_factory=list)


class TlsProbe(Protocol):
    """A reader of ClientHello bytes and decrypted Finished messages in a capture."""

    def available(self) -> bool:
        """Return True when the tool of the probe is installed."""

    def observe(self, capture: Path, ports: Sequence[int], keylog: Path | None) -> TlsObservation:
        """Read the capture, decrypt with ``keylog`` and the embedded keys, and report."""


class StubTlsProbe:
    """A probe that returns fixed observations, for tests without tshark."""

    def __init__(self, observation: TlsObservation | None = None, installed: bool = True) -> None:
        """Create a stub that reports the given observation."""
        self.observation = observation or TlsObservation(tool="stub", version="stub")
        self.installed = installed
        self.calls: list[tuple[Path, tuple[int, ...], Path | None]] = []

    def available(self) -> bool:
        """Return the installed flag of the stub."""
        return self.installed

    def observe(self, capture: Path, ports: Sequence[int], keylog: Path | None) -> TlsObservation:
        """Record the call and return the fixed observation."""
        self.calls.append((capture, tuple(ports), keylog))
        return self.observation


class TsharkTlsProbe:
    """Read TLS handshakes with tshark, which decrypts with the key log and embedded keys."""

    def __init__(self, program: str | None = None, timeout_s: float = DEFAULT_TIMEOUT_S) -> None:
        """Create a probe for a tshark path, which defaults to ``FPS_TSHARK`` and the path."""
        self.runner = TsharkRunner(program, timeout_s)

    @property
    def program(self) -> str:
        """The tshark program that the probe runs."""
        return self.runner.program

    def available(self) -> bool:
        """Return True when the tshark program exists."""
        return self.runner.available()

    def observe(self, capture: Path, ports: Sequence[int], keylog: Path | None) -> TlsObservation:
        """Run tshark once and return the ClientHellos and Finished messages it prints."""
        argv = ["-r", str(capture), "-n", "-2"]
        argv += ["-Y", "tls.handshake.type == 1 || tls.handshake.type == 20"]
        for port in sorted(set(ports) - {443}):
            argv += ["-d", f"tcp.port=={port},tls"]
        if keylog is not None:
            argv += ["-o", f"tls.keylog_file:{keylog}"]
        argv += ["-T", "fields", "-E", "header=n", "-E", "separator=/t"]
        argv += ["-E", "occurrence=a", "-E", f"aggregator={_AGGREGATOR}"]
        for name in _FIELDS:
            argv += ["-e", name]
        proc = _run(self.program, argv, f"tshark tls {capture}", self.runner.timeout_s)
        hellos, finished = parse_fields(proc.stdout, source=str(capture))
        return TlsObservation(TSHARK, self.runner.version(), hellos, finished)


def _hex_bytes(value: str) -> bytes:
    text = _first(value).replace(":", "")
    try:
        return bytes.fromhex(text)
    except ValueError:
        return b""


def parse_fields(text: str, source: str = "<tshark>") -> tuple[list[Hello], list[Finished]]:
    """Parse the field output of :class:`TsharkTlsProbe` into hellos and finished messages."""
    hellos: list[Hello] = []
    finished: list[Finished] = []
    width = len(_FIELDS)
    for line_no, line in enumerate(text.splitlines(), start=1):
        if not line.strip():
            continue
        cols = line.split("\t")
        cols += [""] * (width - len(cols))
        if len(cols) > width:
            raise ParserError(f"tshark tls {source}: line {line_no} has {len(cols)} fields")
        frame = _int(cols[0], 10)
        src_ip, dst_ip = _ip(cols[1]) or _ip(cols[3]), _ip(cols[2]) or _ip(cols[4])
        src_port, dst_port = _port(cols[5]), _port(cols[6])
        if frame is None:
            raise ParserError(f"tshark tls {source}: line {line_no} has frame {cols[0]!r}")
        if src_ip is None or dst_ip is None or src_port is None or dst_port is None:
            continue
        key = SessionKey.of("tcp", src_ip, src_port, dst_ip, dst_port)
        types = [_int(v, 10) for v in cols[7].split(_AGGREGATOR)]
        if _HANDSHAKE_CLIENT_HELLO in types:
            data = _hex_bytes(cols[9]) or _hex_bytes(cols[10])
            hellos.append(Hello(key, frame, data, _first(cols[8]) or None))
        if _HANDSHAKE_FINISHED in types:
            finished.append(Finished(key, (src_ip, src_port)))
    return hellos, finished


def has_embedded_keys(capture: Path) -> bool:
    """Return True when a pcapng capture holds a decryption secrets block."""
    try:
        with capture.open("rb") as handle:
            head = handle.read(12)
            if len(head) < 12 or struct.unpack("<I", head[:4])[0] != _PCAPNG_SHB:
                return False
            endian = "<" if head[8:12] == b"\x4d\x3c\x2b\x1a" else ">"
            handle.seek(0)
            while True:
                header = handle.read(8)
                if len(header) < 8:
                    return False
                block_type, length = struct.unpack(f"{endian}II", header)
                if block_type == _PCAPNG_DSB:
                    return True
                if length < 12:
                    return False
                handle.seek(length - 8, 1)
    except OSError:
        return False


@dataclass
class SessionResult:
    """The outcome of one sampled session."""

    session: Session
    profile: str
    expected: tuple[str, ...]
    decrypted: bool = False
    finished_from: int = 0
    ja4: str | None = None
    tshark_ja4: str | None = None
    server_name: str | None = None
    reasons: list[str] = field(default_factory=list)

    @property
    def matched(self) -> bool:
        """True when the ClientHello has a JA4 that the profile lists and tshark agrees."""
        return (
            self.ja4 is not None
            and self.ja4 in self.expected
            and (self.tshark_ja4 is None or self.tshark_ja4 == self.ja4)
        )

    def to_dict(self) -> dict[str, Any]:
        """Return the session entry of the report."""
        s = self.session
        return {
            "session": s.id,
            "profile": self.profile,
            "client": {"ip": s.client_ip, "port": s.client_port},
            "server": {"ip": s.server_ip, "port": s.server_port},
            "server_name": self.server_name,
            "decrypted": self.decrypted,
            "ja4": self.ja4,
            "tshark_ja4": self.tshark_ja4,
            "expected_ja4": list(self.expected),
            "ja4_matched": self.matched,
            "reasons": self.reasons,
        }


@dataclass
class EncryptionResult:
    """The outcome of the encryption check."""

    tool: dict[str, str] | None
    keys: dict[str, Any]
    min_decrypted: float
    sample_limit: int
    labelled_sessions: int
    results: list[SessionResult]
    failures: list[str]

    @property
    def sampled(self) -> int:
        """The number of sessions in the sample."""
        return len(self.results)

    @property
    def decrypted(self) -> int:
        """The number of sampled sessions that decrypted."""
        return sum(1 for r in self.results if r.decrypted)

    @property
    def mismatched(self) -> int:
        """The number of sampled sessions whose JA4 does not match the profile."""
        return sum(1 for r in self.results if not r.matched)

    @property
    def ratio(self) -> float:
        """The share of sampled sessions that decrypted, 0 for an empty sample."""
        return self.decrypted / self.sampled if self.sampled else 0.0

    @property
    def passed(self) -> bool:
        """True when nothing in ``failures`` stands against the capture."""
        return not self.failures

    def metrics(self) -> list[dict[str, Any]]:
        """Return the decrypted and JA4 matched shares, and the failures series.

        The two ratios are not rounded, and they are absent when no session was sampled,
        because there is no share of nothing.
        """
        out: list[dict[str, Any]] = []
        if self.sampled:
            out.append(
                {"name": DECRYPTED_METRIC, "labels": {}, "value": self.decrypted / self.sampled}
            )
            matched = self.sampled - self.mismatched
            out.append({"name": JA4_METRIC, "labels": {}, "value": matched / self.sampled})
        out.append(failures_series(CHECK_NAME, len(self.failures), self.passed))
        return out

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        profiles: dict[str, dict[str, Any]] = {}
        for r in self.results:
            entry = profiles.setdefault(
                r.profile,
                {"profile": r.profile, "expected_ja4": list(r.expected), "sessions": 0},
            )
            entry["sessions"] += 1
            entry["matched"] = entry.get("matched", 0) + (1 if r.matched else 0)
            entry["decrypted"] = entry.get("decrypted", 0) + (1 if r.decrypted else 0)
        for entry in profiles.values():
            entry.setdefault("matched", 0)
            entry.setdefault("decrypted", 0)
        return {
            "check": CHECK_NAME,
            "metrics": self.metrics(),
            "passed": self.passed,
            "tool": self.tool,
            "keys": self.keys,
            "labelled_sessions": self.labelled_sessions,
            "sample_limit": self.sample_limit,
            "sampled_sessions": self.sampled,
            "decrypted_sessions": self.decrypted,
            "decrypted_ratio": round(self.ratio, 6),
            "min_decrypted": self.min_decrypted,
            "ja4_matched_sessions": self.sampled - self.mismatched,
            "ja4_mismatched_sessions": self.mismatched,
            "profiles": sorted(profiles.values(), key=lambda p: str(p["profile"])),
            "failures": self.failures,
            "sessions": [r.to_dict() for r in self.results[:MAX_SESSIONS]],
        }


def choose_sample(sessions: Sequence[Session], limit: int) -> list[Session]:
    """Pick at most ``limit`` sessions spread evenly over the order of first packet."""
    if len(sessions) <= limit:
        return list(sessions)
    return [sessions[i * len(sessions) // limit] for i in range(limit)]


def _closest(observed: str, expected: Sequence[str]) -> str:
    """Return the expected fingerprint that shares the most parts with the observed one."""
    got = ja4.split(observed)

    def shared(candidate: str) -> int:
        want = ja4.split(candidate)
        return sum(a == b for a, b in zip(got, want, strict=True)) if got and want else 0

    return max(expected, key=shared)


def _judge_hello(result: SessionResult, hello: Hello | None) -> None:
    if hello is None:
        result.reasons.append(
            "no ClientHello found, so the session has no JA4 and tshark did not "
            "decode a TLS handshake on it"
        )
        return
    result.tshark_ja4 = hello.tshark_ja4
    try:
        parsed = ja4.parse_client_hello(hello.data)
    except ja4.ClientHelloError as exc:
        result.reasons.append(f"the ClientHello in frame {hello.frame} cannot be read: {exc}")
        return
    result.ja4 = ja4.compute(parsed)
    result.server_name = parsed.server_name
    if result.ja4 not in result.expected:
        want = _closest(result.ja4, result.expected)
        result.reasons.append(
            f"JA4 {result.ja4} of the ClientHello in frame {hello.frame} is not the JA4 of "
            f"profile {result.profile} ({', '.join(result.expected)}): "
            f"{ja4.explain_difference(result.ja4, want)}"
        )
    if hello.tshark_ja4 is not None and hello.tshark_ja4 != result.ja4:
        result.reasons.append(
            f"tshark computes JA4 {hello.tshark_ja4} for frame {hello.frame} and the runner "
            f"computes {result.ja4}, so one of them misreads the ClientHello"
        )


def check(
    capture: Path,
    sessions: Sequence[Session],
    labels: Labels,
    probe: TlsProbe,
    keylog: Path | None = None,
) -> EncryptionResult:
    """Sample the TLS sessions of the labels, decrypt them, and compare their JA4 fingerprints."""
    tls = labels.tls
    if tls is None:
        raise ValueError("the labels have no tls section")
    labelled = [s for s in sessions if labels.tls_profile_of(s) is not None]
    keys = {
        "embedded": has_embedded_keys(capture),
        "sidecar": keylog.name if keylog is not None else None,
    }
    sample = choose_sample(labelled, tls.sample)
    if not labelled:
        return _result(
            None,
            keys,
            tls,
            labelled,
            [],
            ["no session of the capture matches a rule with tls_profile"],
        )
    if not probe.available():
        return _result(
            None, keys, tls, labelled, [], ["tshark is not installed, so no session was decrypted"]
        )
    observation = probe.observe(capture, sorted({s.server_port for s in sample}), keylog)
    first_hello: dict[SessionKey, Hello] = {}
    for hello in observation.hellos:
        first_hello.setdefault(hello.key, hello)
    senders: dict[SessionKey, set[Endpoint]] = {}
    for message in observation.finished:
        senders.setdefault(message.key, set()).add(message.sender)
    results = []
    for session in sample:
        profile = labels.tls_profile_of(session)
        assert profile is not None
        result = SessionResult(session, profile, tls.profiles[profile])
        result.finished_from = len(senders.get(session.key, ()))
        result.decrypted = result.finished_from >= 2
        if not result.decrypted:
            result.reasons.append(
                f"not decrypted: tshark read the Finished message of {result.finished_from} of "
                "2 sides, so the keys of the capture or the key log do not cover this session"
            )
        _judge_hello(result, first_hello.get(session.key))
        results.append(result)
    failures = _failures(results, tls, keys)
    tool = {"tool": observation.tool, "version": observation.version}
    return _result(tool, keys, tls, labelled, results, failures)


def _failures(
    results: Sequence[SessionResult], tls: TlsLabels, keys: Mapping[str, Any]
) -> list[str]:
    failures = []
    decrypted = sum(1 for r in results if r.decrypted)
    if decrypted / len(results) < tls.min_decrypted:
        hint = ""
        if not keys["embedded"] and keys["sidecar"] is None:
            hint = ": the capture holds no embedded keys and no key log was given"
        failures.append(
            f"decrypted {decrypted} of {len(results)} sampled sessions, "
            f"below the required {tls.min_decrypted:.2f}{hint}"
        )
    for r in results:
        if not r.matched:
            reason = next((x for x in r.reasons if not x.startswith("not decrypted")), None)
            failures.append(f"session {r.session.id} ({r.profile}): {reason or 'JA4 mismatch'}")
    return failures


def _result(
    tool: dict[str, str] | None,
    keys: dict[str, Any],
    tls: TlsLabels,
    labelled: Sequence[Session],
    results: list[SessionResult],
    failures: list[str],
) -> EncryptionResult:
    return EncryptionResult(
        tool=tool,
        keys=keys,
        min_decrypted=tls.min_decrypted,
        sample_limit=tls.sample,
        labelled_sessions=len(labelled),
        results=results,
        failures=failures,
    )
