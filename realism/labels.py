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
}
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
_TEMPORAL_FIELDS = {"alpha", "min_samples", "arrivals", "sizes"}
_TEMPORAL_TARGET_FIELDS = {"overall", "applications"}
DEFAULT_KS_ALPHA = 0.01
DEFAULT_KS_MIN_SAMPLES = 30
# Parameters of each parametric distribution, by quantity. Times use the names of a
# scenario ``delay`` (milliseconds), sizes use bytes.
_TIME_PARAMS: dict[str, tuple[str, ...]] = {
    "constant": ("ms", "tolerance_ms"),
    "exponential": ("mean_ms",),
    "uniform": ("min_ms", "max_ms"),
    "lognormal": ("mu", "sigma", "unit"),
    "pareto": ("alpha", "scale_ms"),
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
    """

    kind: str
    params: Mapping[str, float] = field(default_factory=dict)
    tolerance: float = 0.0
    sample: tuple[float, ...] | None = None
    sample_file: str | None = None
    reference: Reference | None = None
    declared: Mapping[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class TemporalTarget:
    """One distribution that the temporal check compares with the capture.

    ``prop`` is ``inter_arrival`` (the gaps between the first packets of consecutive
    sessions) or ``payload_size``. ``application`` is None for all sessions of the capture.
    ``direction`` selects the payloads of a size target, and it is ``both`` for arrivals.
    """

    prop: str
    application: str | None
    direction: str
    distribution: Distribution


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
    base_dir: Path | None = None

    def tls_profile_of(self, session: Session) -> str | None:
        """Return the client profile of a session from its first matching rule, or None."""
        for rule in self.rules:
            if rule.matches(session):
                return rule.tls_profile
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
    seen: set[tuple[str, str | None, str]] = set()
    for target in targets:
        key = (target.prop, target.application, target.direction)
        if key in seen:
            raise LabelsError(
                f"{where} temporal: {target.prop} of {target.application or 'overall'} "
                f"({target.direction}) has two targets"
            )
        seen.add(key)
    return TemporalLabels(alpha=float(alpha), min_samples=minimum, targets=tuple(targets))


def _temporal_entries(
    entry: Any, application: str | None, prop: str, quantity: str, where: str
) -> list[TemporalTarget]:
    """Read one target, or a list of targets for the two directions of an application."""
    items = entry if isinstance(entry, list) and quantity == "size" else [entry]
    if not items:
        raise LabelsError(f"{where}: must not be an empty list")
    out = []
    for index, item in enumerate(items):
        here = where if len(items) == 1 else f"{where}[{index}]"
        direction = "both"
        if quantity == "size":
            if not isinstance(item, dict):
                raise LabelsError(f"{here}: must be a mapping")
            item = dict(item)
            direction = item.pop("direction", "both")
            if direction not in DIRECTIONS:
                raise LabelsError(f"{here}.direction: must be one of {', '.join(DIRECTIONS)}")
        out.append(
            TemporalTarget(prop, application, direction, _distribution(item, quantity, here))
        )
    return out


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
    table = _TIME_PARAMS if quantity == "time" else _SIZE_PARAMS
    kind = spec.get("dist")
    if kind is None and quantity == "time" and "ms" in spec:
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
    scale = 0.001 if quantity == "time" else 1.0  # milliseconds to seconds
    suffix = "_ms" if quantity == "time" else ""
    params: dict[str, float] = {}
    tolerance = 0.0
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


def _rule(item: Any, where: str) -> Rule:
    if not isinstance(item, dict) or set(item) - {"application", "match", "tls_profile"}:
        raise LabelsError(f"{where}: a rule has only application, match, and tls_profile")
    tls_profile = item.get("tls_profile")
    if tls_profile is not None and (not isinstance(tls_profile, str) or not tls_profile):
        raise LabelsError(f"{where}: tls_profile must be a profile name")
    application = item.get("application")
    if not isinstance(application, str) or not application:
        raise LabelsError(f"{where}: application must be a non empty string")
    match = item.get("match", {})
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
    return Rule(application=application, match=normalized, tls_profile=tls_profile)


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
