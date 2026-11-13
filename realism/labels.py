# SPDX-License-Identifier: GPL-3.0-only
"""Load the expected application labels of a capture and match sessions to them.

A labels file is JSON or YAML and follows ``realism/schema/labels.schema.json``.
Rules match sessions by transport and endpoint fields, and the first matching rule
names the intended application of the session.
"""

from __future__ import annotations

import importlib
import ipaddress
import json
import math
import re
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, TypeGuard

from realism.capture import Session
from realism.ja4 import split as ja4_split

LABELS_FORMAT = "fps-realism-labels"
LABELS_VERSION = 1

# Classifier protocol names that count as the intended application. A labels file
# overrides or extends these names in its applications section.
DEFAULT_CLASSIFIER_NAMES: dict[str, dict[str, tuple[str, ...]]] = {
    "http1": {"ndpi": ("HTTP",), "stub": ("HTTP",)},
    "dns": {"ndpi": ("DNS",), "stub": ("DNS",)},
}

_MATCH_FIELDS = {
    "transport": str,
    "client_ip": str,
    "client_port": int,
    "server_ip": str,
    "server_port": int,
}
_TOP_LEVEL = {
    "labels",
    "version",
    "threshold",
    "applications",
    "sessions",
    "default",
    "tls",
    "content",
    "structure",
    "temporal",
    "mix",
    "transport",
    "messages",
    "hosts",
}
# The parsers whose findings a rule may expect. A Zeek item is the name of a weird. A tshark
# item is the protocol of the frame that carries the expert item (the last name of
# frame.protocols), optionally followed by a colon, a space, and a part of its message.
EXPECT_PARSERS = ("tshark", "zeek")
_ZEEK_WEIRD = re.compile(r"[A-Za-z0-9_]+")
_TSHARK_ITEM = re.compile(r"[A-Za-z0-9_.-]+(: \S.*)?")
_HOSTS_FIELDS = {"count", "gateway_macs"}
_MAC = re.compile(r"[0-9a-f]{2}([:-][0-9a-f]{2}){5}", re.IGNORECASE)
MESSAGE_PROTOCOLS = ("http1", "http2", "smtp", "imap")
_MESSAGE_RULE_FIELDS = {"protocol", "match", "expect"}
# The counts that a messages rule may expect, by protocol. A scalar is one number, and a
# table maps a command name or a reply code (or status) to a number.
MESSAGE_SCALARS: dict[str, tuple[str, ...]] = {
    "http1": ("requests", "responses"),
    "http2": ("requests", "responses"),
    "smtp": ("messages",),
    "imap": ("messages",),
}
MESSAGE_TABLES: dict[str, tuple[str, ...]] = {
    "http1": (),
    "http2": (),
    "smtp": ("commands", "replies"),
    "imap": ("commands", "replies"),
}
_COUNT_FIELDS = {"min", "max"}
_MESSAGE_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*")
_TRANSPORT_FIELDS = {
    "os_profile",
    "hosts",
    "profiles_dir",
    "client_hops",
    "server_hops",
    "strict",
    "expect",
}
TRANSPORT_EXPECT_FIELDS = ("ttl", "mss", "window_scale", "syn_window")
_MIX_FIELDS = {"test_config", "run_report", "basis", "tolerance_points", "weights", "scenarios"}
_MIX_SCENARIO_FIELDS = {"application", "ndpi"}
MIX_BASES = ("sessions", "bytes", "both")
_STRUCTURE_FIELDS = {"scenarios", "max_gap_s", "tolerance_s"}
_STRUCTURE_SCENARIO_FIELDS = {"file", "vars", "match", "expected_instances"}
DEFAULT_STRUCTURE_TOLERANCE_S = 0.0
_CONTENT_FIELDS = {"max_duplicate_ratio", "ignore_headers", "sizes"}
_SIZE_FIELDS = {"direction", "median", "cv", "tolerance"}
DIRECTIONS = ("client", "server", "both")
DEFAULT_MAX_DUPLICATE_RATIO = 0.01
DEFAULT_SIZE_TOLERANCE = 0.25
_TEMPORAL_FIELDS = {"alpha", "min_samples", "arrivals", "sizes", "packets", "rungs"}
_TEMPORAL_TARGET_FIELDS = {"overall", "applications"}
_RUNG_FIELDS = {
    "path",
    "duration_ms",
    "bits_per_second",
    "body_tolerance_bytes",
    "share_tolerance",
    "rate_tolerance",
    "walk",
    "streams",
    "join_ms",
    "min_samples",
}
DEFAULT_RUNG_BODY_TOLERANCE = 188.0  # one MPEG transport stream packet
DEFAULT_RUNG_SHARE_TOLERANCE = 0.05
DEFAULT_RUNG_RATE_TOLERANCE = 0.05
DEFAULT_KS_ALPHA = 0.01
DEFAULT_KS_MIN_SAMPLES = 30
DEFAULT_PACED_WITHIN = 0.9
# Parameters of each parametric distribution, by quantity. Times use the names of a
# scenario ``delay`` (milliseconds), sizes use bytes.
_TIME_PARAMS: dict[str, tuple[str, ...]] = {
    "constant": ("ms", "tolerance_ms"),
    "exponential": ("mean_ms",),
    "uniform": ("min_ms", "max_ms"),
    "lognormal": ("mu", "sigma", "unit"),
    "pareto": ("alpha", "scale_ms"),
}
# A paced target (the constant of a packet gap) lists its packet times in ``ms`` and says
# how late a packet may be and how many of them may be late.
_GAP_PARAMS: dict[str, tuple[str, ...]] = {
    **_TIME_PARAMS,
    "constant": ("ms", "tolerance_ms", "within", "mean_tolerance_ms"),
}
_SIZE_PARAMS: dict[str, tuple[str, ...]] = {
    "constant": ("value", "tolerance"),
    "exponential": ("mean",),
    "uniform": ("min", "max"),
    "lognormal": ("mu", "sigma"),
    "pareto": ("alpha", "scale"),
}
_EMPIRICAL_SOURCES = ("sample", "sample_file", "reference")
_TLS_FIELDS = {"profiles", "sample", "min_decrypted"}
DEFAULT_TLS_SAMPLE = 100
DEFAULT_MIN_DECRYPTED = 1.0


class LabelsError(Exception):
    """A labels file cannot be read or does not follow the labels schema."""


@dataclass(frozen=True)
class Rule:
    """One session rule, which names an application for matching sessions."""

    application: str
    match: Mapping[str, str | int]
    tls_profile: str | None = None
    expect_findings: Mapping[str, tuple[str, ...]] = field(default_factory=dict)

    def matches(self, session: Session) -> bool:
        """Return True when every field of the rule equals the session field."""
        return all(getattr(session, name) == value for name, value in self.match.items())


@dataclass(frozen=True)
class TlsLabels:
    """The expected TLS clients of one capture.

    ``profiles`` maps the name of a configured client profile to the JA4
    fingerprints that its ClientHello may have. A rule with ``tls_profile`` ties
    the sessions it matches to a profile. The encryption check samples at most
    ``sample`` of those sessions and requires that ``min_decrypted`` of them decrypt.
    """

    profiles: Mapping[str, tuple[str, ...]]
    sample: int = DEFAULT_TLS_SAMPLE
    min_decrypted: float = DEFAULT_MIN_DECRYPTED


@dataclass(frozen=True)
class SizeExpectation:
    """The expected payload size distribution of one application.

    ``median`` is the expected median payload size in bytes and ``cv`` the expected
    coefficient of variation, which is absent when the labels do not give one. The
    observed values must lie within ``tolerance`` of them, as a share of the expected
    value. ``direction`` selects the payloads that count: ``client``, ``server``, or
    ``both``.
    """

    direction: str
    median: float
    cv: float | None = None
    tolerance: float = DEFAULT_SIZE_TOLERANCE


@dataclass(frozen=True)
class ContentLabels:
    """Settings of the content variety check.

    ``max_duplicate_ratio`` is the largest share of duplicate payloads that an
    application may have, ``ignore_headers`` lists HTTP header names that the check
    drops on top of its built in list, and ``sizes`` maps an application to its
    expected size distribution.
    """

    max_duplicate_ratio: float = DEFAULT_MAX_DUPLICATE_RATIO
    ignore_headers: tuple[str, ...] = ()
    sizes: Mapping[str, SizeExpectation] = field(default_factory=dict)


@dataclass(frozen=True)
class StructureScenario:
    """One scenario file whose flow graph the session structure check applies.

    ``file`` is a path relative to the labels file. ``vars`` replaces scenario variables,
    for example a port that the run changed. ``match`` limits the sessions that the
    scenario may claim, with the fields of a session rule. ``expected_instances`` is the
    number of instances that the capture must hold, absent when the labels leave it open.
    """

    file: str
    vars: Mapping[str, Any] = field(default_factory=dict)
    match: Rule | None = None
    expected_instances: int | None = None


@dataclass(frozen=True)
class StructureLabels:
    """Settings of the session structure check.

    ``max_gap_s`` starts a new instance when a client is silent for longer than this many
    seconds before its next connection, and it is absent when no gap splits instances.
    ``tolerance_s`` is the slack in seconds for the ordering comparisons.
    """

    scenarios: tuple[StructureScenario, ...] = ()
    max_gap_s: float | None = None
    tolerance_s: float = DEFAULT_STRUCTURE_TOLERANCE_S


@dataclass(frozen=True)
class Reference:
    """A reference capture whose observed distribution is the target of an empirical test.

    ``pcap`` and ``labels`` are paths relative to the labels file. The sessions of the
    reference capture get their application from ``labels``, or from the rules of the
    labels file that holds the reference when ``labels`` is absent.
    """

    pcap: str
    labels: str | None = None


@dataclass(frozen=True)
class Distribution:
    """A target distribution of the temporal check, in seconds or in bytes.

    ``kind`` is ``constant``, ``exponential``, ``uniform``, ``lognormal``, ``pareto``, or
    ``empirical``. ``params`` holds the parameters in seconds for inter arrival times and in
    bytes for sizes, under the names ``value`` (constant), ``mean`` (exponential), ``min``
    and ``max`` (uniform), ``mu`` and ``sigma`` (lognormal, of the natural logarithm of the
    value), and ``alpha`` and ``scale`` (pareto, the shape and the smallest value).
    ``tolerance`` is the distance that still counts as equal to a constant. An empirical
    distribution has exactly one of ``sample`` (the values), ``sample_file`` (a path
    relative to the labels file), or ``reference`` (a capture). ``declared`` is the
    distribution as the labels file wrote it.

    The constant of a packet gap target is paced: ``values`` lists its packet times in
    seconds (one or more, because calls of one application can use different packet
    times), and ``within`` is the share of the gaps of each stream that must lie within
    ``tolerance`` of the packet time of that stream. ``mean_tolerance`` (RLM-22) also bounds
    the distance of the mean gap of each stream from that packet time, in seconds, and is None
    when the labels do not ask for it. ``values`` is empty for the other targets, and
    ``within`` stays 1.
    """

    kind: str
    params: Mapping[str, float] = field(default_factory=dict)
    tolerance: float = 0.0
    values: tuple[float, ...] = ()
    within: float = 1.0
    mean_tolerance: float | None = None
    sample: tuple[float, ...] | None = None
    sample_file: str | None = None
    reference: Reference | None = None
    declared: Mapping[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class TemporalTarget:
    """One distribution that the temporal check compares with the capture.

    ``prop`` is ``inter_arrival`` (the gaps between the first packets of consecutive
    sessions), ``payload_size``, or ``packet_gap`` (the gaps between the packets with
    payload of one direction of a session). ``application`` is None for all sessions of the
    capture. ``direction`` selects the payloads of a size target and the streams of a packet
    gap target, and it is ``both`` for arrivals.

    ``min_samples`` is the fewest observations of this target, and replaces the value of the
    section and of the command line when set (RLM-22). ``path`` (a packet gap target of the
    client direction) is a regular expression that selects the HTTP requests of one kind by
    their request target: the stream of a session then holds only the matching requests, and
    a gap is the time between two of them. ``join`` (seconds, with ``path``) makes the matching
    requests of all sessions of the application one stream while the gap between neighbouring
    requests stays below it, which is a playback whose segments each have a connection.
    """

    prop: str
    application: str | None
    direction: str
    distribution: Distribution
    min_samples: int | None = None
    path: str | None = None
    join: float | None = None


@dataclass(frozen=True)
class RungTarget:
    """The bytes per rung of the requests of one kind of an application (RLM-22).

    The requests whose target matches ``path`` (a regular expression whose first group is the
    index of the rung, a whole number) each carry ``duration`` seconds of media. ``rates`` is
    the bit rate of each rung. A stream is the matching requests of one session, or with
    ``join`` (seconds) of the sessions of the application that follow each other closer than
    that. Each stream needs ``min_samples`` requests (the section value when None) to be
    judged. The body of every response must lie within ``body_tolerance`` bytes of
    ``rate * duration / 8``, the share of each rung in the bytes of the stream (response head
    and body) within ``share_tolerance`` of the share that the rates of the requested rungs
    give, and the bit rate of the stream within ``rate_tolerance`` of their mean. ``walk_up``
    asks that the rungs rise through the whole ladder. ``streams`` is the number of judged
    streams that the capture must hold, when set.
    """

    application: str
    path: str
    duration: float
    rates: tuple[float, ...]
    body_tolerance: float = 188.0
    share_tolerance: float = 0.05
    rate_tolerance: float = 0.05
    walk_up: bool = False
    streams: Count | None = None
    join: float | None = None
    min_samples: int | None = None


@dataclass(frozen=True)
class TemporalLabels:
    """Settings of the temporal distribution check.

    ``alpha`` is the significance level of every KS test, which rejects a target when the p
    value falls below it. ``min_samples`` is the fewest observations (in each sample of a
    two sample test) that a comparison needs, and a smaller sample reports insufficient data.
    """

    alpha: float = DEFAULT_KS_ALPHA
    min_samples: int = DEFAULT_KS_MIN_SAMPLES
    targets: tuple[TemporalTarget, ...] = ()
    rungs: tuple[RungTarget, ...] = ()


@dataclass(frozen=True)
class MixScenario:
    """The application that one scenario of the mix stands for.

    ``application`` names the application as the session rules and the ``applications``
    section do, so the nDPI names of the conformance check apply. ``ndpi`` lists protocol
    names for this application instead, and it is absent when the conformance names apply.
    """

    application: str
    ndpi: tuple[str, ...] | None = None


@dataclass(frozen=True)
class MixLabels:
    """Settings of the mix accuracy check.

    ``scenarios`` maps each scenario of the mix to its application. The configured weights come
    from ``test_config`` (a test configuration file, relative to the labels file) or from
    ``weights`` (scenario to weight). ``run_report`` is the file with the achieved counts per
    scenario, relative to the labels file. ``basis`` is ``sessions``, ``bytes``, or ``both`` and
    defaults to ``profile.basis`` of the test configuration. ``tolerance_points`` is the largest
    difference in percentage points, 2 when absent.
    """

    scenarios: Mapping[str, MixScenario] = field(default_factory=dict)
    test_config: str | None = None
    run_report: str | None = None
    basis: str | None = None
    tolerance_points: float | None = None
    weights: Mapping[str, float] = field(default_factory=dict)


@dataclass(frozen=True)
class TransportLabels:
    """Settings of the transport behavior check.

    ``os_profile`` names the operating system profile of every host that ``hosts`` does not
    list, and ``hosts`` maps an IP address to its own profile. ``profiles_dir`` is the directory
    of the profile files, relative to the labels file (the kernel backend profiles when absent).
    ``client_hops`` and ``server_hops`` count the routers between a client or a server and the
    capture point, which lower the TTL that the capture shows. ``strict`` makes the option order
    and the SYN window decide too. ``expect`` replaces a value of the profile for every host.
    """

    os_profile: str | None = None
    hosts: Mapping[str, str] = field(default_factory=dict)
    profiles_dir: str | None = None
    client_hops: int = 0
    server_hops: int = 0
    strict: bool = False
    expect: Mapping[str, int] = field(default_factory=dict)


@dataclass(frozen=True)
class Count:
    """An expected number, exact or a range, of messages in a session or of hosts in a capture.

    ``max`` is None for a range without an upper limit.
    """

    min: int
    max: int | None

    def holds(self, found: int) -> bool:
        """Return True when the number lies inside the range."""
        return found >= self.min and (self.max is None or found <= self.max)

    def text(self) -> str:
        """Return the expectation in words, for example ``1``, ``2 to 4``, or ``at least 1``."""
        if self.max == self.min:
            return str(self.min)
        if self.max is None:
            return f"at least {self.min}"
        return f"{self.min} to {self.max}"

    def to_dict(self) -> dict[str, int | None]:
        """Return the range as the report shows it."""
        return {"min": self.min, "max": self.max}


@dataclass(frozen=True)
class MessageRule:
    """Expected message counts per session for the sessions that ``match`` selects.

    ``protocol`` names how tshark reads the sessions. ``expect`` maps a count name to the
    number that every matching session must show: ``requests``, ``responses`` (HTTP),
    ``messages`` (an RFC 5322 message, SMTP and IMAP), and ``commands.NAME`` and
    ``replies.CODE`` (SMTP and IMAP).
    """

    protocol: str
    match: Mapping[str, str | int]
    expect: Mapping[str, Count]

    def matches(self, session: Session) -> bool:
        """Return True when every match field equals the session field."""
        return all(getattr(session, name) == value for name, value in self.match.items())


@dataclass(frozen=True)
class HostsLabels:
    """Settings of the host identity check.

    ``count`` is the number of hosts that the capture must hold, an exact number or a range
    (open when absent). ``gateway_macs`` lists the MAC addresses of routers, in lower case with
    colons, whose frames the search for address conflicts leaves out.
    """

    count: Count | None = None
    gateway_macs: tuple[str, ...] = ()


@dataclass
class Labels:
    """The expected labels of one capture."""

    rules: list[Rule]
    default: str | None = None
    threshold: float | None = None
    applications: dict[str, dict[str, tuple[str, ...]]] = field(default_factory=dict)
    tls: TlsLabels | None = None
    content: ContentLabels | None = None
    structure: StructureLabels | None = None
    temporal: TemporalLabels | None = None
    mix: MixLabels | None = None
    transport: TransportLabels | None = None
    messages: list[MessageRule] | None = None
    hosts: HostsLabels | None = None
    base_dir: Path | None = None

    def tls_profile_of(self, session: Session) -> str | None:
        """Return the client profile of a session from its first matching rule, or None."""
        for rule in self.rules:
            if rule.matches(session):
                return rule.tls_profile
        return None

    def rule_index_of(self, session: Session) -> int | None:
        """Return the index of the first rule that matches a session, or None."""
        for index, rule in enumerate(self.rules):
            if rule.matches(session):
                return index
        return None

    def application_of(self, session: Session) -> str | None:
        """Return the intended application of a session, or None when unlabelled."""
        for rule in self.rules:
            if rule.matches(session):
                return rule.application
        return self.default

    def accepted_names(self, application: str, classifier: str) -> tuple[str, ...]:
        """Return the classifier protocol names that count as the application."""
        names = self.applications.get(application, {}).get(classifier)
        if names is not None:
            return names
        return DEFAULT_CLASSIFIER_NAMES.get(application, {}).get(classifier, ())


def load(path: Path) -> Labels:
    """Read and validate a labels file in JSON or YAML."""
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise LabelsError(f"read labels {path}: {exc}") from exc
    doc: Any
    if path.suffix.lower() in (".yaml", ".yml"):
        doc = _load_yaml(path, text)
    else:
        try:
            doc = json.loads(text)
        except json.JSONDecodeError as exc:
            raise LabelsError(f"parse labels {path}: {exc}") from exc
    labels = parse(doc, source=str(path))
    labels.base_dir = path.resolve().parent
    return labels


def _load_yaml(path: Path, text: str) -> Any:
    try:
        yaml: Any = importlib.import_module("yaml")
    except ImportError as exc:
        raise LabelsError(
            f"parse labels {path}: YAML needs PyYAML, or write the file as JSON"
        ) from exc
    try:
        return yaml.safe_load(text)
    except yaml.YAMLError as exc:
        raise LabelsError(f"parse labels {path}: {exc}") from exc


def parse(doc: Any, source: str = "<labels>") -> Labels:
    """Validate a decoded labels document and build :class:`Labels` from it."""
    where = f"labels {source}"
    if not isinstance(doc, dict):
        raise LabelsError(f"{where}: the document must be a mapping")
    unknown = set(doc) - _TOP_LEVEL
    if unknown:
        raise LabelsError(f"{where}: unknown fields {sorted(unknown)}")
    if doc.get("labels") != LABELS_FORMAT:
        raise LabelsError(f"{where}: field labels must be {LABELS_FORMAT!r}")
    if doc.get("version") != LABELS_VERSION:
        raise LabelsError(f"{where}: field version must be {LABELS_VERSION}")
    threshold = doc.get("threshold")
    if threshold is not None and (
        isinstance(threshold, bool)
        or not isinstance(threshold, int | float)
        or not 0.0 <= threshold <= 1.0
    ):
        raise LabelsError(f"{where}: threshold must be a number from 0 to 1")
    default = doc.get("default")
    if default is not None and (not isinstance(default, str) or not default):
        raise LabelsError(f"{where}: default must be an application name")
    rules = [
        _rule(item, f"{where} sessions[{index}]")
        for index, item in enumerate(_list(doc.get("sessions", []), f"{where} sessions"))
    ]
    applications = _applications(doc.get("applications", {}), where)
    tls = _tls(doc["tls"], where) if "tls" in doc else None
    for index, rule in enumerate(rules):
        if rule.tls_profile is not None and (tls is None or rule.tls_profile not in tls.profiles):
            raise LabelsError(
                f"{where} sessions[{index}]: tls_profile {rule.tls_profile!r} "
                "is not a profile of the tls section"
            )
    return Labels(
        rules=rules,
        default=default,
        threshold=float(threshold) if threshold is not None else None,
        applications=applications,
        tls=tls,
        content=_content(doc["content"], where) if "content" in doc else None,
        structure=_structure(doc["structure"], where) if "structure" in doc else None,
        temporal=_temporal(doc["temporal"], where) if "temporal" in doc else None,
        mix=_mix(doc["mix"], where) if "mix" in doc else None,
        transport=_transport(doc["transport"], where) if "transport" in doc else None,
        messages=_messages(doc["messages"], where) if "messages" in doc else None,
        hosts=_hosts(doc["hosts"], where) if "hosts" in doc else None,
    )


def _number(value: Any) -> TypeGuard[int | float]:
    return not isinstance(value, bool) and isinstance(value, int | float)


def _content(value: Any, where: str) -> ContentLabels:
    """Validate the content section of the content variety check."""
    if not isinstance(value, dict) or set(value) - _CONTENT_FIELDS:
        raise LabelsError(f"{where} content: must be a mapping with only {sorted(_CONTENT_FIELDS)}")
    ratio = value.get("max_duplicate_ratio", DEFAULT_MAX_DUPLICATE_RATIO)
    if not _number(ratio) or not 0 <= ratio <= 1:
        raise LabelsError(f"{where} content.max_duplicate_ratio: must be a number from 0 to 1")
    headers = value.get("ignore_headers", [])
    if not isinstance(headers, list) or not all(isinstance(h, str) and h.strip() for h in headers):
        raise LabelsError(f"{where} content.ignore_headers: must be a list of header names")
    raw = value.get("sizes", {})
    if not isinstance(raw, dict):
        raise LabelsError(f"{where} content.sizes: must be a mapping from application to sizes")
    sizes: dict[str, SizeExpectation] = {}
    for name, entry in raw.items():
        here = f"{where} content.sizes.{name}"
        if (
            not isinstance(name, str)
            or not name
            or not isinstance(entry, dict)
            or set(entry) - _SIZE_FIELDS
        ):
            raise LabelsError(f"{here}: must be a mapping with only {sorted(_SIZE_FIELDS)}")
        direction = entry.get("direction", "both")
        median, cv = entry.get("median"), entry.get("cv")
        tolerance = entry.get("tolerance", DEFAULT_SIZE_TOLERANCE)
        if direction not in DIRECTIONS:
            raise LabelsError(f"{here}.direction: must be one of {', '.join(DIRECTIONS)}")
        if not _number(median) or median <= 0:
            raise LabelsError(f"{here}.median: must be a number above 0")
        if cv is not None and (not _number(cv) or cv < 0):
            raise LabelsError(f"{here}.cv: must be a number of 0 or more")
        if not _number(tolerance) or not 0 < tolerance <= 10:
            raise LabelsError(f"{here}.tolerance: must be a number above 0 and up to 10")
        sizes[name] = SizeExpectation(
            direction, float(median), None if cv is None else float(cv), float(tolerance)
        )
    return ContentLabels(
        max_duplicate_ratio=float(ratio),
        ignore_headers=tuple(h.strip().lower() for h in headers),
        sizes=sizes,
    )


def _structure(value: Any, where: str) -> StructureLabels:
    """Validate the structure section of the session structure check."""
    if not isinstance(value, dict) or set(value) - _STRUCTURE_FIELDS:
        raise LabelsError(
            f"{where} structure: must be a mapping with only {sorted(_STRUCTURE_FIELDS)}"
        )
    raw = value.get("scenarios")
    if not isinstance(raw, list) or not raw:
        raise LabelsError(f"{where} structure.scenarios: must be a non empty list")
    scenarios = []
    for index, entry in enumerate(raw):
        here = f"{where} structure.scenarios[{index}]"
        if (
            not isinstance(entry, dict)
            or set(entry) - _STRUCTURE_SCENARIO_FIELDS
            or not isinstance(entry.get("file"), str)
            or not entry["file"]
        ):
            raise LabelsError(
                f"{here}: must be a mapping with a file and only "
                f"{sorted(_STRUCTURE_SCENARIO_FIELDS)}"
            )
        overrides = entry.get("vars", {})
        if not isinstance(overrides, dict) or not all(isinstance(k, str) for k in overrides):
            raise LabelsError(f"{here}.vars: must be a mapping of variable names")
        expected = entry.get("expected_instances")
        if expected is not None and (isinstance(expected, bool) or not isinstance(expected, int)):
            raise LabelsError(f"{here}.expected_instances: must be an integer of 0 or more")
        if expected is not None and expected < 0:
            raise LabelsError(f"{here}.expected_instances: must be an integer of 0 or more")
        match = entry.get("match")
        scenarios.append(
            StructureScenario(
                file=entry["file"],
                vars=dict(overrides),
                match=_rule({"application": "scenario", "match": match}, here)
                if match is not None
                else None,
                expected_instances=expected,
            )
        )
    gap = value.get("max_gap_s")
    if gap is not None and (not _number(gap) or gap <= 0):
        raise LabelsError(f"{where} structure.max_gap_s: must be a number above 0")
    tolerance = value.get("tolerance_s", DEFAULT_STRUCTURE_TOLERANCE_S)
    if not _number(tolerance) or tolerance < 0:
        raise LabelsError(f"{where} structure.tolerance_s: must be a number of 0 or more")
    return StructureLabels(
        scenarios=tuple(scenarios),
        max_gap_s=float(gap) if gap is not None else None,
        tolerance_s=float(tolerance),
    )


def _mix(value: Any, where: str) -> MixLabels:
    """Validate the mix section of the mix accuracy check."""
    here = f"{where} mix"
    if not isinstance(value, dict) or set(value) - _MIX_FIELDS:
        raise LabelsError(f"{here}: must be a mapping with only {sorted(_MIX_FIELDS)}")
    raw = value.get("scenarios")
    if not isinstance(raw, dict) or not raw:
        raise LabelsError(f"{here}.scenarios: must be a non empty mapping from scenario to entry")
    scenarios: dict[str, MixScenario] = {}
    for name, entry in raw.items():
        if isinstance(entry, str):
            entry = {"application": entry}
        if (
            not isinstance(name, str)
            or not name
            or not isinstance(entry, dict)
            or set(entry) - _MIX_SCENARIO_FIELDS
            or not _is_name(entry.get("application"))
        ):
            raise LabelsError(
                f"{here}.scenarios.{name}: must be an application name or a mapping with an "
                f"application and only {sorted(_MIX_SCENARIO_FIELDS)}"
            )
        ndpi = entry.get("ndpi")
        if ndpi is not None and (
            not isinstance(ndpi, list) or not ndpi or not all(_is_name(n) for n in ndpi)
        ):
            raise LabelsError(f"{here}.scenarios.{name}.ndpi: must be a list of protocol names")
        scenarios[name] = MixScenario(entry["application"], tuple(ndpi) if ndpi else None)
    for key in ("test_config", "run_report"):
        if key in value and not _is_name(value[key]):
            raise LabelsError(f"{here}.{key}: must be a file path")
    basis = value.get("basis")
    if basis is not None and basis not in MIX_BASES:
        raise LabelsError(f"{here}.basis: must be one of {', '.join(MIX_BASES)}")
    tolerance = value.get("tolerance_points")
    if tolerance is not None and (not _finite(tolerance) or not 0 <= tolerance <= 100):
        raise LabelsError(f"{here}.tolerance_points: must be a number from 0 to 100")
    weights = value.get("weights", {})
    if not isinstance(weights, dict) or not all(
        isinstance(k, str) and k and _finite(w) and w > 0 for k, w in weights.items()
    ):
        raise LabelsError(f"{here}.weights: must map scenario names to numbers above 0")
    if "test_config" in value and weights:
        raise LabelsError(f"{here}: give test_config or weights, not both")
    if weights and basis is None:
        raise LabelsError(f"{here}.basis: is required with weights, because no test says it")
    return MixLabels(
        scenarios=scenarios,
        test_config=value.get("test_config"),
        run_report=value.get("run_report"),
        basis=basis,
        tolerance_points=float(tolerance) if tolerance is not None else None,
        weights={k: float(w) for k, w in weights.items()},
    )


_PROFILE_NAME = re.compile(r"[a-z][a-z0-9_-]*")


def _transport(value: Any, where: str) -> TransportLabels:
    """Validate the transport section of the transport behavior check."""
    here = f"{where} transport"
    if not isinstance(value, dict) or set(value) - _TRANSPORT_FIELDS:
        raise LabelsError(f"{here}: must be a mapping with only {sorted(_TRANSPORT_FIELDS)}")
    profile = value.get("os_profile")
    if profile is not None and not _is_profile_name(profile):
        raise LabelsError(f"{here}.os_profile: must be a profile name such as linux6")
    raw_hosts = value.get("hosts", {})
    hosts: dict[str, str] = {}
    if not isinstance(raw_hosts, dict):
        raise LabelsError(f"{here}.hosts: must map IP addresses to profile names")
    for address, name in raw_hosts.items():
        try:
            canonical = str(ipaddress.ip_address(address))
        except ValueError as exc:
            raise LabelsError(f"{here}.hosts: {address!r} is not an IP address") from exc
        if not _is_profile_name(name):
            raise LabelsError(f"{here}.hosts.{address}: must be a profile name such as linux6")
        hosts[canonical] = name
    if profile is None and not hosts:
        raise LabelsError(f"{here}: give os_profile, hosts, or both")
    directory = value.get("profiles_dir")
    if directory is not None and not _is_name(directory):
        raise LabelsError(f"{here}.profiles_dir: must be a directory path")
    hops = {}
    for key in ("client_hops", "server_hops"):
        count = value.get(key, 0)
        if isinstance(count, bool) or not isinstance(count, int) or not 0 <= count <= 254:
            raise LabelsError(f"{here}.{key}: must be an integer from 0 to 254")
        hops[key] = count
    strict = value.get("strict", False)
    if not isinstance(strict, bool):
        raise LabelsError(f"{here}.strict: must be true or false")
    raw_expect = value.get("expect", {})
    if not isinstance(raw_expect, dict) or set(raw_expect) - set(TRANSPORT_EXPECT_FIELDS):
        raise LabelsError(f"{here}.expect: must be a mapping with only {TRANSPORT_EXPECT_FIELDS}")
    for key, number in raw_expect.items():
        if isinstance(number, bool) or not isinstance(number, int) or number < 0:
            raise LabelsError(f"{here}.expect.{key}: must be a non negative integer")
    return TransportLabels(
        os_profile=profile,
        hosts=hosts,
        profiles_dir=directory,
        client_hops=hops["client_hops"],
        server_hops=hops["server_hops"],
        strict=strict,
        expect=dict(raw_expect),
    )


def _hosts(value: Any, where: str) -> HostsLabels:
    """Validate the hosts section of the host identity check."""
    here = f"{where} hosts"
    if not isinstance(value, dict) or set(value) - _HOSTS_FIELDS:
        raise LabelsError(f"{here}: must be a mapping with only {sorted(_HOSTS_FIELDS)}")
    count = _count(value["count"], f"{here}.count") if "count" in value else None
    raw = value.get("gateway_macs", [])
    if not isinstance(raw, list) or not all(
        isinstance(mac, str) and _MAC.fullmatch(mac) for mac in raw
    ):
        raise LabelsError(f"{here}.gateway_macs: must be a list of MAC addresses")
    macs = tuple(dict.fromkeys(mac.replace("-", ":").lower() for mac in raw))
    return HostsLabels(count=count, gateway_macs=macs)


def _is_profile_name(value: Any) -> bool:
    return isinstance(value, str) and _PROFILE_NAME.fullmatch(value) is not None


def _finite(value: Any) -> TypeGuard[int | float]:
    return _number(value) and math.isfinite(value)


def _temporal(value: Any, where: str) -> TemporalLabels:
    """Validate the temporal section, whose targets name the expected distributions."""
    if not isinstance(value, dict) or set(value) - _TEMPORAL_FIELDS:
        raise LabelsError(
            f"{where} temporal: must be a mapping with only {sorted(_TEMPORAL_FIELDS)}"
        )
    alpha = value.get("alpha", DEFAULT_KS_ALPHA)
    if not _finite(alpha) or not 0 < alpha < 1:
        raise LabelsError(f"{where} temporal.alpha: must be a number above 0 and below 1")
    minimum = value.get("min_samples", DEFAULT_KS_MIN_SAMPLES)
    if isinstance(minimum, bool) or not isinstance(minimum, int) or minimum < 2:
        raise LabelsError(f"{where} temporal.min_samples: must be an integer of 2 or more")
    targets: list[TemporalTarget] = []
    for section, prop, quantity in (
        ("arrivals", "inter_arrival", "time"),
        ("sizes", "payload_size", "size"),
        ("packets", "packet_gap", "gap"),
    ):
        if section not in value:
            continue
        raw = value[section]
        here = f"{where} temporal.{section}"
        if not isinstance(raw, dict) or set(raw) - _TEMPORAL_TARGET_FIELDS:
            raise LabelsError(
                f"{here}: must be a mapping with only {sorted(_TEMPORAL_TARGET_FIELDS)}"
            )
        if "overall" in raw:
            targets += _temporal_entries(raw["overall"], None, prop, quantity, f"{here}.overall")
        apps = raw.get("applications", {})
        if not isinstance(apps, dict):
            raise LabelsError(f"{here}.applications: must be a mapping from application name")
        for name, entry in apps.items():
            if not isinstance(name, str) or not name:
                raise LabelsError(f"{here}.applications: names must be non empty strings")
            targets += _temporal_entries(entry, name, prop, quantity, f"{here}.applications.{name}")
    seen: set[tuple[str, str | None, str, str | None]] = set()
    for target in targets:
        key = (target.prop, target.application, target.direction, target.path)
        if key in seen:
            raise LabelsError(
                f"{where} temporal: {target.prop} of {target.application or 'overall'} "
                f"({target.direction}) has two targets"
                + ("" if target.path is None else f" for the path {target.path!r}")
            )
        seen.add(key)
    rungs = _rungs(value["rungs"], f"{where} temporal.rungs") if "rungs" in value else ()
    return TemporalLabels(
        alpha=float(alpha), min_samples=minimum, targets=tuple(targets), rungs=rungs
    )


def _temporal_entries(
    entry: Any, application: str | None, prop: str, quantity: str, where: str
) -> list[TemporalTarget]:
    """Read one target, or a list of targets for the two directions of an application."""
    items = entry if isinstance(entry, list) and quantity != "time" else [entry]
    if not items:
        raise LabelsError(f"{where}: must not be an empty list")
    out = []
    for index, item in enumerate(items):
        here = where if len(items) == 1 else f"{where}[{index}]"
        direction = "both"
        if isinstance(item, dict):
            item = dict(item)
        elif quantity != "time":
            raise LabelsError(f"{here}: must be a mapping")
        if quantity != "time":
            direction = item.pop("direction", "both")
            if direction not in DIRECTIONS:
                raise LabelsError(f"{here}.direction: must be one of {', '.join(DIRECTIONS)}")
        minimum = _target_min_samples(item, here) if isinstance(item, dict) else None
        path = join = None
        if quantity == "gap" and isinstance(item, dict):
            path, join = _request_selector(item, application, direction, here)
        out.append(
            TemporalTarget(
                prop,
                application,
                direction,
                _distribution(item, quantity, here),
                minimum,
                path,
                join,
            )
        )
    return out


def _target_min_samples(item: dict[str, Any], where: str) -> int | None:
    """Remove and validate the ``min_samples`` of one target, which replaces the section's."""
    if "min_samples" not in item:
        return None
    minimum = item.pop("min_samples")
    if isinstance(minimum, bool) or not isinstance(minimum, int) or minimum < 2:
        raise LabelsError(f"{where}.min_samples: must be an integer of 2 or more")
    return minimum


def _request_selector(
    item: dict[str, Any], application: str | None, direction: str, where: str
) -> tuple[str | None, float | None]:
    """Remove and validate ``path`` and ``join_ms``, which select the requests of one kind."""
    path = item.pop("path", None)
    join = item.pop("join_ms", None)
    if path is None:
        if join is not None:
            raise LabelsError(f"{where}.join_ms: needs a path, which selects the requests to join")
        return None, None
    _pattern(path, where + ".path", groups=0)
    if application is None:
        raise LabelsError(
            f"{where}.path: a path target belongs under applications, because the sessions of "
            "an application are the ones that speak HTTP"
        )
    if direction != "client":
        raise LabelsError(
            f"{where}.path: needs direction client, because the server sends no request"
        )
    if join is None:
        return path, None
    if not _finite(join) or join <= 0:
        raise LabelsError(f"{where}.join_ms: must be a number above 0")
    return path, float(join) * 0.001


def _pattern(value: Any, where: str, groups: int) -> re.Pattern[str]:
    """Compile a request path pattern, which must have at least ``groups`` capture groups."""
    if not isinstance(value, str) or not value:
        raise LabelsError(f"{where}: must be a regular expression on the request target")
    try:
        compiled = re.compile(value)
    except re.error as exc:
        raise LabelsError(f"{where}: not a regular expression: {exc}") from exc
    if compiled.groups < groups:
        raise LabelsError(f"{where}: must have a capture group for the rung, as in (\\d+)")
    return compiled


def _rungs(value: Any, where: str) -> tuple[RungTarget, ...]:
    """Validate the rungs section, which names the bytes per rung of each application."""
    if not isinstance(value, dict) or set(value) - {"applications"}:
        raise LabelsError(f"{where}: must be a mapping with only applications")
    apps = value.get("applications")
    if not isinstance(apps, dict) or not apps:
        raise LabelsError(f"{where}.applications: must be a non empty mapping from application")
    out = []
    for name, spec in apps.items():
        here = f"{where}.applications.{name}"
        if not isinstance(name, str) or not name:
            raise LabelsError(f"{where}.applications: names must be non empty strings")
        if not isinstance(spec, dict) or set(spec) - _RUNG_FIELDS:
            raise LabelsError(f"{here}: must be a mapping with only {sorted(_RUNG_FIELDS)}")
        spec = dict(spec)
        _pattern(spec.get("path"), here + ".path", groups=1)
        rates = spec.get("bits_per_second")
        if not isinstance(rates, list) or not rates or not all(_finite(r) and r > 0 for r in rates):
            raise LabelsError(f"{here}.bits_per_second: must be a list of bit rates above 0")
        duration = spec.get("duration_ms")
        if not _finite(duration) or duration <= 0:
            raise LabelsError(f"{here}.duration_ms: must be the media time of one request, above 0")
        join = spec.get("join_ms")
        if join is not None and (not _finite(join) or join <= 0):
            raise LabelsError(f"{here}.join_ms: must be a number above 0")
        walk = spec.get("walk")
        if walk not in (None, "up"):
            raise LabelsError(f"{here}.walk: must be up")
        tolerances = {}
        for key, default in (
            ("body_tolerance_bytes", DEFAULT_RUNG_BODY_TOLERANCE),
            ("share_tolerance", DEFAULT_RUNG_SHARE_TOLERANCE),
            ("rate_tolerance", DEFAULT_RUNG_RATE_TOLERANCE),
        ):
            number = spec.get(key, default)
            if not _finite(number) or number < 0:
                raise LabelsError(f"{here}.{key}: must be a number of 0 or more")
            tolerances[key] = float(number)
        out.append(
            RungTarget(
                application=name,
                path=spec["path"],
                duration=float(duration) * 0.001,
                rates=tuple(float(r) for r in rates),
                body_tolerance=tolerances["body_tolerance_bytes"],
                share_tolerance=tolerances["share_tolerance"],
                rate_tolerance=tolerances["rate_tolerance"],
                walk_up=walk == "up",
                streams=_count(spec["streams"], here + ".streams") if "streams" in spec else None,
                join=None if join is None else float(join) * 0.001,
                min_samples=_target_min_samples(spec, here),
            )
        )
    return tuple(out)


def _param(spec: Mapping[str, Any], name: str, here: str, strict: bool) -> float:
    """Read a required finite parameter that is above 0, or 0 or more when not strict."""
    value = spec.get(name)
    if _finite(value) and (value > 0 if strict else value >= 0):
        return float(value)
    raise LabelsError(f"{here}.{name}: must be a number {'above 0' if strict else '0 or more'}")


def _distribution(spec: Any, quantity: str, where: str) -> Distribution:
    """Validate a target distribution and convert its parameters to seconds or bytes."""
    if not isinstance(spec, dict):
        raise LabelsError(f"{where}: must be a mapping with dist")
    table = {"time": _TIME_PARAMS, "gap": _GAP_PARAMS, "size": _SIZE_PARAMS}[quantity]
    kind = spec.get("dist")
    if kind is None and quantity != "size" and "ms" in spec:
        kind = "constant"  # the constant delay of a scenario, written without dist
    if kind == "empirical":
        return _empirical(spec, where)
    if kind not in table:
        raise LabelsError(f"{where}.dist: must be one of {', '.join([*table, 'empirical'])}")
    unknown = set(spec) - {"dist", *table[kind]}
    if unknown:
        raise LabelsError(
            f"{where}: {kind} takes only dist and {', '.join(table[kind])}, not {sorted(unknown)}"
        )
    scale = 1.0 if quantity == "size" else 0.001  # milliseconds to seconds
    suffix = "" if quantity == "size" else "_ms"
    params: dict[str, float] = {}
    tolerance = 0.0
    if kind == "constant" and quantity == "gap":
        return _paced(spec, where)
    if kind == "constant":
        params["value"] = (
            _param(spec, "ms" if quantity == "time" else "value", where, False) * scale
        )
        if "tolerance" + suffix in spec:
            tolerance = _param(spec, "tolerance" + suffix, where, False) * scale
    elif kind == "exponential":
        params["mean"] = _param(spec, "mean" + suffix, where, True) * scale
    elif kind == "uniform":
        low = _param(spec, "min" + suffix, where, False) * scale
        high = _param(spec, "max" + suffix, where, False) * scale
        if high <= low:
            raise LabelsError(f"{where}: max{suffix} must be above min{suffix}")
        params.update(min=low, max=high)
    elif kind == "lognormal":
        mu = spec.get("mu")
        if not _finite(mu):
            raise LabelsError(f"{where}.mu: must be a number")
        params["sigma"] = _param(spec, "sigma", where, True)
        unit = spec.get("unit", "s")
        if unit not in ("s", "ms"):
            raise LabelsError(f"{where}.unit: must be s or ms")
        # mu is the logarithm of the value in the unit, so ms values shift it by ln 1000.
        params["mu"] = float(mu) - (math.log(1000.0) if unit == "ms" else 0.0)
    else:  # pareto
        params["alpha"] = _param(spec, "alpha", where, True)
        params["scale"] = _param(spec, "scale" + suffix, where, True) * scale
    return Distribution(kind, params, tolerance, declared=dict(spec) | {"dist": kind})


def _paced(spec: Mapping[str, Any], where: str) -> Distribution:
    """Validate the constant of a packet gap target: packet times, tolerance, and share."""
    raw = spec.get("ms")
    times: list[Any] = raw if isinstance(raw, list) else [raw]
    if not times or not all(_finite(t) and t > 0 for t in times):
        raise LabelsError(f"{where}.ms: must be a packet time above 0, or a list of them")
    if "tolerance_ms" not in spec:
        raise LabelsError(
            f"{where}.tolerance_ms: a paced target needs the lateness that still counts as on "
            "time, because a real sender wakes up a few milliseconds late"
        )
    tolerance = _param(spec, "tolerance_ms", where, False) * 0.001
    within = spec.get("within", DEFAULT_PACED_WITHIN)
    if not _finite(within) or not 0 < within <= 1:
        raise LabelsError(f"{where}.within: must be a number above 0 and up to 1")
    values = tuple(sorted({float(t) * 0.001 for t in times}))
    params = {"tolerance": tolerance, "within": float(within)}
    mean_tolerance = None
    if "mean_tolerance_ms" in spec:
        mean_tolerance = _param(spec, "mean_tolerance_ms", where, False) * 0.001
        params["mean_tolerance"] = mean_tolerance
    if len(values) == 1:
        params = {"value": values[0], **params}
    return Distribution(
        "constant",
        params,
        tolerance,
        values=values,
        within=float(within),
        mean_tolerance=mean_tolerance,
        declared=dict(spec) | {"dist": "constant"},
    )


def _empirical(spec: Mapping[str, Any], where: str) -> Distribution:
    """Validate an empirical target, given as values, a file of values, or a capture."""
    unknown = set(spec) - {"dist", *_EMPIRICAL_SOURCES}
    given = [name for name in _EMPIRICAL_SOURCES if name in spec]
    if unknown or len(given) != 1:
        raise LabelsError(
            f"{where}: an empirical target has dist and exactly one of "
            f"{', '.join(_EMPIRICAL_SOURCES)}"
        )
    source = given[0]
    value = spec[source]
    if source == "sample":
        if not isinstance(value, list) or not value or not all(_finite(v) for v in value):
            raise LabelsError(f"{where}.sample: must be a non empty list of numbers")
        return Distribution(
            "empirical",
            sample=tuple(float(v) for v in value),
            declared={"dist": "empirical", "sample_size": len(value)},
        )
    if source == "sample_file":
        if not isinstance(value, str) or not value:
            raise LabelsError(f"{where}.sample_file: must be a file name")
        return Distribution(
            "empirical", sample_file=value, declared={"dist": "empirical", "sample_file": value}
        )
    if (
        not isinstance(value, dict)
        or set(value) - {"pcap", "labels"}
        or not isinstance(value.get("pcap"), str)
        or not value["pcap"]
        or not isinstance(value.get("labels", ""), str)
    ):
        raise LabelsError(f"{where}.reference: must be a mapping with pcap and optional labels")
    return Distribution(
        "empirical",
        reference=Reference(value["pcap"], value.get("labels") or None),
        declared={"dist": "empirical", "reference": dict(value)},
    )


def _tls(value: Any, where: str) -> TlsLabels:
    """Validate the tls section, whose profiles hold the expected JA4 of each client."""
    if not isinstance(value, dict) or set(value) - _TLS_FIELDS:
        raise LabelsError(f"{where} tls: must be a mapping with only {sorted(_TLS_FIELDS)}")
    raw = value.get("profiles")
    if not isinstance(raw, dict) or not raw:
        raise LabelsError(f"{where} tls.profiles: must be a non empty mapping")
    profiles: dict[str, tuple[str, ...]] = {}
    for name, entry in raw.items():
        ja4 = entry.get("ja4") if isinstance(entry, dict) and set(entry) == {"ja4"} else None
        listed = [ja4] if isinstance(ja4, str) else ja4
        if (
            not isinstance(name, str)
            or not name
            or not isinstance(listed, list)
            or not listed
            or not all(isinstance(item, str) and ja4_split(item) is not None for item in listed)
        ):
            raise LabelsError(
                f"{where} tls.profiles.{name}: must be a mapping with ja4, one JA4 string "
                "such as t13d1516h2_8daaf6152771_e5627efa2ab1 or a list of them"
            )
        profiles[name] = tuple(listed)
    sample = value.get("sample", DEFAULT_TLS_SAMPLE)
    if isinstance(sample, bool) or not isinstance(sample, int) or sample < 1:
        raise LabelsError(f"{where} tls.sample: must be a positive integer")
    minimum = value.get("min_decrypted", DEFAULT_MIN_DECRYPTED)
    if isinstance(minimum, bool) or not isinstance(minimum, int | float) or not 0 < minimum <= 1:
        raise LabelsError(f"{where} tls.min_decrypted: must be a number above 0 and up to 1")
    return TlsLabels(profiles=profiles, sample=sample, min_decrypted=float(minimum))


def _list(value: Any, where: str) -> Sequence[Any]:
    if not isinstance(value, list):
        raise LabelsError(f"{where}: must be a list")
    return value


def _is_name(value: Any) -> bool:
    return isinstance(value, str) and bool(value)


def _expect_findings(value: Any, where: str) -> dict[str, tuple[str, ...]]:
    """Validate the expected parser findings of a rule: a list of items per parser."""
    if not isinstance(value, dict):
        raise LabelsError(f"{where}: expect_findings must map a parser to a list of items")
    out: dict[str, tuple[str, ...]] = {}
    for parser, items in value.items():
        if parser not in EXPECT_PARSERS:
            raise LabelsError(
                f"{where}: expect_findings names the parser {parser!r}, "
                f"which must be one of {list(EXPECT_PARSERS)}"
            )
        names = _list(items, f"{where} expect_findings.{parser}")
        seen: list[str] = []
        for item in names:
            if not _is_name(item) or item != item.strip():
                raise LabelsError(f"{where}: expect_findings.{parser} items must be non empty text")
            if parser == "zeek" and not _ZEEK_WEIRD.fullmatch(item):
                raise LabelsError(
                    f"{where}: expect_findings.zeek item {item!r} is no weird name, "
                    "which has letters, digits, and underscores"
                )
            if parser == "tshark" and not _TSHARK_ITEM.fullmatch(item):
                raise LabelsError(
                    f"{where}: expect_findings.tshark item {item!r} must be a protocol name, "
                    "or a protocol name, a colon, a space, and a part of the message"
                )
            if item in seen:
                raise LabelsError(f"{where}: expect_findings.{parser} lists {item!r} twice")
            seen.append(item)
        if not seen:
            raise LabelsError(f"{where}: expect_findings.{parser} must list an item")
        out[parser] = tuple(seen)
    return out


def _rule(item: Any, where: str) -> Rule:
    if not isinstance(item, dict) or set(item) - {
        "application",
        "match",
        "tls_profile",
        "expect_findings",
    }:
        raise LabelsError(
            f"{where}: a rule has only application, match, tls_profile, and expect_findings"
        )
    tls_profile = item.get("tls_profile")
    if tls_profile is not None and (not isinstance(tls_profile, str) or not tls_profile):
        raise LabelsError(f"{where}: tls_profile must be a profile name")
    application = item.get("application")
    if not isinstance(application, str) or not application:
        raise LabelsError(f"{where}: application must be a non empty string")
    return Rule(
        application=application,
        match=_match_fields(item.get("match", {}), where),
        tls_profile=tls_profile,
        expect_findings=_expect_findings(item["expect_findings"], where)
        if "expect_findings" in item
        else {},
    )


def _match_fields(match: Any, where: str) -> dict[str, str | int]:
    """Validate the match fields of a rule and return them with canonical addresses."""
    if not isinstance(match, dict):
        raise LabelsError(f"{where}: match must be a mapping")
    for name, value in match.items():
        kind = _MATCH_FIELDS.get(name)
        if kind is None:
            raise LabelsError(f"{where}: unknown match field {name!r}")
        if isinstance(value, bool) or not isinstance(value, kind):
            raise LabelsError(f"{where}: match field {name} must be {kind.__name__}")
        if name == "transport" and value not in ("tcp", "udp"):
            raise LabelsError(f"{where}: transport must be tcp or udp")
        if isinstance(value, int) and not 0 <= value <= 65535:
            raise LabelsError(f"{where}: match field {name} must be a port number")
    normalized: dict[str, str | int] = dict(match)
    for name in ("client_ip", "server_ip"):
        if name in normalized:
            try:
                normalized[name] = str(ipaddress.ip_address(normalized[name]))
            except ValueError as exc:
                raise LabelsError(f"{where}: match field {name}: {exc}") from exc
    return normalized


def _messages(value: Any, where: str) -> list[MessageRule]:
    """Validate the messages section of the message count check."""
    here = f"{where} messages"
    rules = _list(value, here)
    if not rules:
        raise LabelsError(f"{here}: give at least one rule")
    return [_message_rule(item, f"{here}[{index}]") for index, item in enumerate(rules)]


def _message_rule(item: Any, where: str) -> MessageRule:
    if not isinstance(item, dict) or set(item) - _MESSAGE_RULE_FIELDS:
        raise LabelsError(f"{where}: a rule has only {sorted(_MESSAGE_RULE_FIELDS)}")
    protocol = item.get("protocol")
    if protocol not in MESSAGE_PROTOCOLS:
        raise LabelsError(f"{where}: protocol must be one of {', '.join(MESSAGE_PROTOCOLS)}")
    raw = item.get("expect")
    if not isinstance(raw, dict) or not raw:
        raise LabelsError(f"{where}: expect must be a mapping with at least one count")
    scalars, tables = MESSAGE_SCALARS[protocol], MESSAGE_TABLES[protocol]
    expect: dict[str, Count] = {}
    for name, spec in raw.items():
        if name in scalars:
            expect[name] = _count(spec, f"{where} expect.{name}")
        elif name in tables:
            if not isinstance(spec, dict) or not spec:
                raise LabelsError(f"{where} expect.{name}: must map names to counts")
            for key, count in spec.items():
                text = str(key)
                if isinstance(key, bool) or not _MESSAGE_NAME.fullmatch(text):
                    raise LabelsError(f"{where} expect.{name}: {key!r} is not a name or a code")
                expect[f"{name}.{text.upper()}"] = _count(count, f"{where} expect.{name}.{text}")
        else:
            allowed = sorted(scalars + tables)
            raise LabelsError(f"{where} expect.{name}: {protocol} counts are {allowed}")
    return MessageRule(protocol, _match_fields(item.get("match", {}), where), expect)


def _count(spec: Any, where: str) -> Count:
    """Read an exact count, or a mapping with ``min`` and ``max``."""
    if isinstance(spec, dict):
        if not spec or set(spec) - _COUNT_FIELDS:
            raise LabelsError(f"{where}: a range has only min and max")
        low, high = spec.get("min", 0), spec.get("max")
        for key, number in (("min", low), ("max", high)):
            if number is not None and (
                isinstance(number, bool) or not isinstance(number, int) or number < 0
            ):
                raise LabelsError(f"{where}.{key}: must be a non negative integer")
        if high is not None and high < low:
            raise LabelsError(f"{where}: max is below min")
        return Count(low, high)
    if isinstance(spec, bool) or not isinstance(spec, int) or spec < 0:
        raise LabelsError(f"{where}: must be a non negative integer or a range with min and max")
    return Count(spec, spec)


def _applications(value: Any, where: str) -> dict[str, dict[str, tuple[str, ...]]]:
    if not isinstance(value, dict):
        raise LabelsError(f"{where} applications: must be a mapping")
    result: dict[str, dict[str, tuple[str, ...]]] = {}
    for application, classifiers in value.items():
        if not isinstance(classifiers, dict):
            raise LabelsError(f"{where} applications.{application}: must be a mapping")
        names: dict[str, tuple[str, ...]] = {}
        for classifier, accepted in classifiers.items():
            if not isinstance(accepted, list) or not all(_is_name(n) for n in accepted):
                raise LabelsError(
                    f"{where} applications.{application}.{classifier}: "
                    "must be a list of protocol names"
                )
            names[classifier] = tuple(accepted)
        result[application] = names
    return result
