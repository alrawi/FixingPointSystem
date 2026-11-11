# SPDX-License-Identifier: GPL-3.0-only
"""Message count check of the realism runner.

A capture can classify as the right application and still lack the traffic that the
scenario describes: an HTTP session without its response, an SMTP dialogue without its
message, an IMAP session that never fetched one. The conformance check cannot see that,
because nDPI names a session from its first packets. This check counts the messages that
tshark parses in each session and compares them with the counts that the ``messages``
section of the labels expects per session.

A rule selects sessions with the match fields of a session rule and names the protocol
that tshark reads them as. What counts as a message depends on the protocol.

* ``http1``: a request is a packet with an HTTP request method, a response one with an
  HTTP status code.
* ``http2``: a request is a HEADERS frame with a ``:method`` pseudo header, a response one
  with a ``:status`` pseudo header.
* ``smtp`` and ``imap``: ``commands.NAME`` counts the commands, ``replies.CODE`` counts the
  SMTP reply codes or IMAP completion statuses, and ``messages`` counts the RFC 5322
  messages that tshark reads from the DATA phase or from a ``BODY[]`` literal.

The check fails for each session whose count lies outside its expected range, with a
reason that names the missing or surplus messages, for a rule that matches no session,
and when tshark is missing, so a skipped tool never passes.
"""

from __future__ import annotations

from collections import Counter
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Protocol

from realism.capture import Session, SessionKey
from realism.labels import Count, Labels, MessageRule
from realism.parsers import (
    _AGGREGATOR,
    DEFAULT_TIMEOUT_S,
    TSHARK,
    TsharkRunner,
    _ip,
    _port,
    _run,
)

CHECK_NAME = "messages"
MAX_FAILURES = 50
MAX_SESSIONS = 100

# Per protocol: the display filter, the dissector that a port is decoded as, and the
# tshark field behind each kind of count. A kind is ``requests``, ``responses``,
# ``messages`` (one count per occurrence of the field), ``commands``, or ``replies`` (one
# count per value of the field).
_PROTOCOLS: dict[str, tuple[str, str, tuple[tuple[str, str], ...]]] = {
    "http1": (
        "http",
        "http",
        (("requests", "http.request.method"), ("responses", "http.response.code")),
    ),
    "http2": (
        "http2.type == 1",
        "http2",
        (("requests", "http2.headers.method"), ("responses", "http2.headers.status")),
    ),
    "smtp": (
        "smtp || imf",
        "smtp",
        (
            ("commands", "smtp.req.command"),
            ("replies", "smtp.response.code"),
            ("messages", "imf.from"),
        ),
    ),
    "imap": (
        "imap || imf",
        "imap",
        (
            ("commands", "imap.request.command"),
            ("replies", "imap.response.status"),
            ("messages", "imf.from"),
        ),
    ),
}
_ENDPOINT_FIELDS = ("ip.src", "ipv6.src", "tcp.srcport", "ip.dst", "ipv6.dst", "tcp.dstport")
_SINGULAR = {
    "requests": "request",
    "responses": "response",
    "messages": "message",
    "commands": "command",
    "replies": "reply",
}


@dataclass
class MessageObservation:
    """What a probe counted in a capture for one protocol, with the tool version."""

    tool: str
    version: str
    counts: dict[SessionKey, Counter[str]] = field(default_factory=dict)


class MessageProbe(Protocol):
    """A reader of the messages that a capture holds, per session."""

    def available(self) -> bool:
        """Return True when the tool of the probe is installed."""

    def observe(self, capture: Path, protocol: str, ports: Sequence[int]) -> MessageObservation:
        """Read the capture as the protocol on the ports and count messages per session.

        A count name is ``requests``, ``responses``, ``messages``, ``commands.NAME`` with the
        command in upper case, or ``replies.CODE``.
        """


class StubMessageProbe:
    """A probe that returns fixed counts, for tests without tshark."""

    def __init__(
        self,
        counts: Mapping[SessionKey, Mapping[str, int]] | None = None,
        installed: bool = True,
    ) -> None:
        """Create a stub that reports the given counts for every protocol."""
        self.counts = {key: Counter(value) for key, value in (counts or {}).items()}
        self.installed = installed
        self.calls: list[tuple[Path, str, tuple[int, ...]]] = []

    def available(self) -> bool:
        """Return the installed flag of the stub."""
        return self.installed

    def observe(self, capture: Path, protocol: str, ports: Sequence[int]) -> MessageObservation:
        """Record the call and return the fixed counts."""
        self.calls.append((capture, protocol, tuple(ports)))
        return MessageObservation("stub", "stub", dict(self.counts))


class TsharkMessageProbe:
    """Count messages with tshark, one run for each protocol."""

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

    def observe(self, capture: Path, protocol: str, ports: Sequence[int]) -> MessageObservation:
        """Run tshark on the capture and count the messages of the protocol per session."""
        display_filter, dissector, kinds = _PROTOCOLS[protocol]
        argv = ["-r", str(capture), "-n"]
        for port in sorted(set(ports)):
            argv += ["-d", f"tcp.port=={port},{dissector}"]
        argv += ["-Y", display_filter, "-T", "fields", "-E", "header=n", "-E", "separator=/t"]
        argv += ["-E", "occurrence=a", "-E", f"aggregator={_AGGREGATOR}"]
        for name in (*_ENDPOINT_FIELDS, *(tshark_field for _, tshark_field in kinds)):
            argv += ["-e", name]
        proc = _run(self.program, argv, f"tshark {protocol} {capture}", self.runner.timeout_s)
        counts = parse_fields(proc.stdout, [kind for kind, _ in kinds], source=str(capture))
        return MessageObservation(TSHARK, self.runner.version(), counts)


def parse_fields(
    text: str, kinds: Sequence[str], source: str = "<tshark>"
) -> dict[SessionKey, Counter[str]]:
    """Parse the field output of :class:`TsharkMessageProbe` into counts per session.

    Each line holds the endpoint fields and then one field per entry of ``kinds``, separated
    by tabs, with the occurrences of a field joined by the unit separator character.
    """
    counts: dict[SessionKey, Counter[str]] = {}
    width = len(_ENDPOINT_FIELDS) + len(kinds)
    for line in text.splitlines():
        if not line.strip():
            continue
        cols = line.split("\t")
        cols += [""] * (width - len(cols))
        src_ip, dst_ip = _ip(cols[0]) or _ip(cols[1]), _ip(cols[3]) or _ip(cols[4])
        src_port, dst_port = _port(cols[2]), _port(cols[5])
        if src_ip is None or dst_ip is None or src_port is None or dst_port is None:
            continue
        counter = counts.setdefault(
            SessionKey.of("tcp", src_ip, src_port, dst_ip, dst_port), Counter()
        )
        for kind, value in zip(kinds, cols[len(_ENDPOINT_FIELDS) : width], strict=True):
            for occurrence in filter(None, (v.strip() for v in value.split(_AGGREGATOR))):
                if kind in ("commands", "replies"):
                    counter[f"{kind}.{occurrence.upper()}"] += 1
                else:
                    counter[kind] += 1
    return counts


@dataclass
class SessionResult:
    """The counts of one checked session against the expectation of its rule."""

    session: Session
    rule: int
    protocol: str
    observed: dict[str, int]
    reasons: list[str] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        """True when every expected count holds."""
        return not self.reasons

    def to_dict(self) -> dict[str, Any]:
        """Return the session entry of the report."""
        s = self.session
        return {
            "session": s.id,
            "rule": self.rule,
            "protocol": self.protocol,
            "client": {"ip": s.client_ip, "port": s.client_port},
            "server": {"ip": s.server_ip, "port": s.server_port},
            "observed": self.observed,
            "passed": self.passed,
            "reasons": self.reasons,
        }


@dataclass
class RuleResult:
    """The outcome of one rule over its sessions."""

    index: int
    rule: MessageRule
    sessions: int = 0
    failed: int = 0
    observed_total: Counter[str] = field(default_factory=Counter)

    def to_dict(self) -> dict[str, Any]:
        """Return the rule entry of the report."""
        return {
            "rule": self.index,
            "protocol": self.rule.protocol,
            "match": dict(self.rule.match),
            "expected": {name: count.to_dict() for name, count in self.rule.expect.items()},
            "sessions": self.sessions,
            "failed_sessions": self.failed,
            "observed_total": {name: self.observed_total[name] for name in self.rule.expect},
        }


@dataclass
class MessagesResult:
    """The outcome of the message count check."""

    tool: dict[str, str] | None
    rules: list[RuleResult]
    results: list[SessionResult]
    failures: list[str]

    @property
    def passed(self) -> bool:
        """True when nothing in ``failures`` stands against the capture."""
        return not self.failures

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        shown = sorted(self.results, key=lambda r: r.passed)[:MAX_SESSIONS]
        return {
            "check": CHECK_NAME,
            # The catalog label `check` has no value `messages` (catalog 0.5.0), so the check
            # carries no realism.check_failures series until a catalog change adds one.
            "metrics": [],
            "passed": self.passed,
            "tool": self.tool,
            "sessions_checked": len(self.results),
            "sessions_failed": sum(1 for r in self.results if not r.passed),
            "rules": [r.to_dict() for r in self.rules],
            "failures_total": len(self.failures),
            "failures": self.failures[:MAX_FAILURES],
            "sessions": [r.to_dict() for r in shown],
        }


def describe(rule: MessageRule, index: int) -> str:
    """Name a rule in a failure, with its protocol and the sessions it selects."""
    match = ", ".join(f"{name} {value}" for name, value in rule.match.items())
    return f"messages rule {index} ({rule.protocol}{', ' + match if match else ''})"


def _label(protocol: str, name: str) -> str:
    """Name a count for a failure, for example ``smtp command EHLO`` or ``http1 response``."""
    kind, _, key = name.partition(".")
    return f"{protocol} {_SINGULAR[kind]}{' ' + key if key else ''}"


def judge(protocol: str, observed: Mapping[str, int], expect: Mapping[str, Count]) -> list[str]:
    """Compare the observed counts with the expected ones and return one reason per mismatch."""
    reasons = []
    for name, count in expect.items():
        found = observed.get(name, 0)
        if found < count.min:
            reasons.append(
                f"missing {_label(protocol, name)}: found {found}, expected {count.text()}"
            )
        elif count.max is not None and found > count.max:
            reasons.append(
                f"surplus {_label(protocol, name)}: found {found}, expected {count.text()}"
            )
    return reasons


def check(
    capture: Path,
    sessions: Sequence[Session],
    labels: Labels,
    probe: MessageProbe,
) -> MessagesResult:
    """Count the messages of the sessions that the rules select and compare them with the labels."""
    rules = labels.messages
    if rules is None:
        raise ValueError("the labels have no messages section")
    rule_results = [RuleResult(index, rule) for index, rule in enumerate(rules, start=1)]
    selected: list[list[Session]] = [[] for _ in rules]
    for session in sessions:
        if session.transport != "tcp":
            continue
        for index, rule in enumerate(rules):
            if rule.matches(session):
                selected[index].append(session)
                break
    failures = [
        f"{describe(rule, index)} matches no TCP session of the capture"
        for index, (rule, found) in enumerate(zip(rules, selected, strict=True), start=1)
        if not found
    ]
    if not probe.available():
        failures.append("tshark is not installed, so no message was counted")
        return MessagesResult(None, rule_results, [], failures)
    results: list[SessionResult] = []
    tool: dict[str, str] | None = None
    for protocol in dict.fromkeys(rule.protocol for rule in rules):
        indexes = [i for i, rule in enumerate(rules) if rule.protocol == protocol]
        ports = sorted({s.server_port for i in indexes for s in selected[i]})
        if not ports:
            continue
        observation = probe.observe(capture, protocol, ports)
        tool = {"tool": observation.tool, "version": observation.version}
        for i in indexes:
            rule, summary = rules[i], rule_results[i]
            for session in selected[i]:
                counts = observation.counts.get(session.key, Counter())
                observed = {name: counts[name] for name in rule.expect}
                result = SessionResult(
                    session, i + 1, protocol, observed, judge(protocol, observed, rule.expect)
                )
                results.append(result)
                summary.sessions += 1
                summary.observed_total.update(observed)
                if not result.passed:
                    summary.failed += 1
                    failures.extend(
                        f"session {session.id} ({session.client_ip}:{session.client_port} to "
                        f"{session.server_ip}:{session.server_port}, {describe(rule, i + 1)}): "
                        f"{reason}"
                        for reason in result.reasons
                    )
    return MessagesResult(tool, rule_results, results, failures)
