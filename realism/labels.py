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
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

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
_TOP_LEVEL = {"labels", "version", "threshold", "applications", "sessions", "default", "tls"}
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


@dataclass
class Labels:
    """The expected labels of one capture."""

    rules: list[Rule]
    default: str | None = None
    threshold: float | None = None
    applications: dict[str, dict[str, tuple[str, ...]]] = field(default_factory=dict)
    tls: TlsLabels | None = None

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
    return parse(doc, source=str(path))


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
