# SPDX-License-Identifier: GPL-3.0-only
"""Transport behavior check of the realism runner.

A real client or server shows its operating system in the first packets of every TCP
connection: the IP time to live (TTL), the do not fragment flag, and the maximum segment
size, window scale, SACK permitted, and timestamp options of the SYN. A generator that
sends the TTL of Windows with the options of Linux, or that changes its mind between
connections, does not look like the host that it claims to be. The check compares the SYN
and the SYN-ACK of every connection with the operating system profile of the sending host.

**Profiles.** A profile is a YAML file of the kernel backend (``engine/fps-backend-kernel/
profiles``, TRN-03): ``socket.ttl`` and ``socket.mss`` are the values that the kernel sets, and
the ``reference`` section records the SYN of the real system with its window, window scale,
option order, and do not fragment flag. The labels name one profile for every host or one per
IP address (:class:`realism.labels.TransportLabels`).

**What is compared.**

* The SYN of each connection, from the sender of the first SYN without ACK: TTL, do not
  fragment flag (IPv4), MSS, window scale, SACK permitted, and timestamps.
* The SYN-ACK of each connection, from the other host, with the same fields. A server only
  answers an option that the client offered, so a SYN-ACK is compared with the profile in
  those options, and it must carry none that the SYN lacked. A SYN-ACK without its SYN in the
  capture leaves out the three options.
* The order of the options and the window of the SYN match the real system only in the
  ``strict`` mode, because the Linux kernel decides both and the kernel backend cannot
  reproduce the order of Windows 11 and macOS 14 or the window of macOS 14 (see the limits
  table of the Transport page). The report counts them as deviations either way.

The TTL that the capture shows is the TTL of the host less the routers between the host and the
capture point, which the labels give as ``client_hops`` and ``server_hops`` (0 by default). The
comparison is exact, so a TTL that is one too high is wrong too. Only the first SYN and the
first SYN-ACK of each connection count, so retransmissions never weigh twice.

The check fails when a field of any handshake differs from its profile, when a profile cannot
be read, and when the capture holds no handshake of a host with a profile.
"""

from __future__ import annotations

import importlib
import ipaddress
import struct
from collections import Counter
from collections.abc import Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from realism.capture import (
    Frame,
    _ipv4,
    _ipv6,
    _network_layer,
    read_frames,
)
from realism.labels import Labels, LabelsError, TransportLabels
from realism.series import failures_series

CHECK_NAME = "transport_behavior"
METRIC = "realism.transport_deviations"
_SIDE = {"syn": "client", "syn-ack": "server"}
MAX_FAILURES = 50
PROFILES_DIR = Path(__file__).resolve().parents[1] / "engine" / "fps-backend-kernel" / "profiles"
OPTION_NAMES = {0: "eol", 1: "nop", 2: "mss", 3: "ws", 4: "sok", 8: "ts"}
_VALID_OPTIONS = frozenset(OPTION_NAMES.values())
_TCP_SYN = 0x02
_TCP_ACK = 0x10
_IP_TCP = 6
_MALFORMED = "malformed"
# The fields that always decide, then the two that only the strict mode enforces.
FIELDS = ("ttl", "df", "mss", "window_scale", "sack", "timestamps")
STRICT_FIELDS = ("options", "syn_window")


class TransportError(Exception):
    """A profile cannot be found or read."""


@dataclass(frozen=True)
class OsProfile:
    """The values of one operating system profile that the check compares."""

    name: str
    ttl: int
    mss: int
    window_scale: int | None
    syn_window: int
    options: tuple[str, ...]
    dont_fragment: bool
    source: str

    @property
    def sack(self) -> bool:
        """True when the SYN offers SACK permitted."""
        return "sok" in self.options

    @property
    def timestamps(self) -> bool:
        """True when the SYN offers timestamps."""
        return "ts" in self.options

    def to_dict(self) -> dict[str, Any]:
        """Return the profile as the report lists it."""
        return {
            "name": self.name,
            "file": self.source,
            "ttl": self.ttl,
            "mss": self.mss,
            "window_scale": self.window_scale,
            "syn_window": self.syn_window,
            "options": list(self.options),
            "dont_fragment": self.dont_fragment,
        }


def _integer(value: Any, low: int, high: int) -> int | None:
    if isinstance(value, bool) or not isinstance(value, int) or not low <= value <= high:
        return None
    return value


def _section(doc: Mapping[str, Any], name: str, where: str) -> Mapping[str, Any]:
    section = doc.get(name)
    if not isinstance(section, dict):
        raise TransportError(f"{where}: the section {name} is missing")
    return section


def parse_profile(doc: Any, source: str) -> OsProfile:
    """Validate a decoded profile file and return the values that the check uses."""
    if not isinstance(doc, dict) or not isinstance(doc.get("name"), str):
        raise TransportError(f"profile {source}: the document must be a mapping with a name")
    sock = _section(doc, "socket", f"profile {source}")
    ref = _section(doc, "reference", f"profile {source}")
    ttl = _integer(sock.get("ttl"), 1, 255)
    mss = _integer(sock.get("mss"), 1, 65535)
    window = _integer(ref.get("syn_window"), 0, 65535)
    scale = _integer(ref.get("window_scale"), 0, 14)
    options = ref.get("options")
    df = ref.get("dont_fragment")
    if ttl is None or mss is None:
        raise TransportError(f"profile {source}: socket.ttl and socket.mss must be integers")
    if window is None or scale is None or not isinstance(df, bool):
        raise TransportError(
            f"profile {source}: reference needs syn_window, window_scale (0 to 14), "
            "and dont_fragment"
        )
    if (
        not isinstance(options, list)
        or not options
        or not all(isinstance(o, str) and o in _VALID_OPTIONS for o in options)
    ):
        raise TransportError(
            f"profile {source}: reference.options must list option names "
            f"from {sorted(_VALID_OPTIONS)}"
        )
    return OsProfile(
        name=doc["name"],
        ttl=ttl,
        mss=mss,
        window_scale=scale if "ws" in options else None,
        syn_window=window,
        options=tuple(options),
        dont_fragment=df,
        source=source,
    )


def load_profile(name: str, directory: Path) -> OsProfile:
    """Read the profile ``name`` from ``directory/name.yaml`` (or ``.yml``)."""
    for suffix in (".yaml", ".yml"):
        path = directory / f"{name}{suffix}"
        if path.is_file():
            break
    else:
        raise TransportError(f"profile {name} is not in {directory} (looked for {name}.yaml)")
    try:
        yaml: Any = importlib.import_module("yaml")
    except ImportError as exc:
        raise TransportError(f"read profile {path}: YAML needs PyYAML") from exc
    try:
        doc = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, yaml.YAMLError) as exc:
        raise TransportError(f"read profile {path}: {exc}") from exc
    profile = parse_profile(doc, path.name)
    if profile.name != name:
        raise TransportError(f"profile {path}: its name is {profile.name!r}, not {name!r}")
    return profile


@dataclass(frozen=True)
class Handshake:
    """The fields of one SYN or SYN-ACK that a profile decides."""

    kind: str
    src: str
    sport: int
    dst: str
    dport: int
    ttl: int
    df: bool | None
    window: int
    options: tuple[str, ...]
    mss: int | None
    window_scale: int | None

    @property
    def sack(self) -> bool:
        """True when the packet carries SACK permitted."""
        return "sok" in self.options

    @property
    def timestamps(self) -> bool:
        """True when the packet carries timestamps."""
        return "ts" in self.options

    @property
    def malformed(self) -> bool:
        """True when the option list cannot be read to its end."""
        return _MALFORMED in self.options


def parse_options(data: bytes) -> tuple[tuple[str, ...], int | None, int | None]:
    """Return the option names, the MSS, and the window scale of a TCP option list.

    A list that breaks off, or an option whose length is out of range, ends the names with
    ``malformed``. The list stops at the end of options option, as a receiver reads it.
    """
    names: list[str] = []
    mss: int | None = None
    scale: int | None = None
    pos = 0
    while pos < len(data):
        kind = data[pos]
        names.append(OPTION_NAMES.get(kind, f"kind{kind}"))
        if kind == 0:
            break
        if kind == 1:
            pos += 1
            continue
        if pos + 1 >= len(data) or data[pos + 1] < 2 or pos + data[pos + 1] > len(data):
            names.append(_MALFORMED)
            break
        length = data[pos + 1]
        if kind == 2 and length == 4:
            (mss,) = struct.unpack_from("!H", data, pos + 2)
        elif kind == 3 and length == 3:
            scale = data[pos + 2]
        pos += length
    return tuple(names), mss, scale


def decode_handshake(frame: Frame) -> Handshake | None:
    """Return the SYN or SYN-ACK in a frame, or None for any other packet."""
    layer = _network_layer(frame)
    if layer is None:
        return None
    version, data = layer
    parsed = _ipv4(data) if version == 4 else _ipv6(data)
    if parsed is None:
        return None
    proto, src, dst, segment = parsed
    if proto != _IP_TCP or len(segment) < 20:
        return None
    flags = segment[13]
    if flags & _TCP_SYN == 0 or flags & 0x04:
        return None
    header_len = (segment[12] >> 4) * 4
    if header_len < 20 or header_len > len(segment):
        return None
    sport, dport = struct.unpack("!HH", segment[:4])
    (window,) = struct.unpack("!H", segment[14:16])
    if version == 4:
        ttl = data[8]
        df: bool | None = bool(struct.unpack("!H", data[6:8])[0] & 0x4000)
    else:
        ttl = data[7]
        df = None
    names, mss, scale = parse_options(segment[20:header_len])
    kind = "syn-ack" if flags & _TCP_ACK else "syn"
    return Handshake(kind, src, sport, dst, dport, ttl, df, window, names, mss, scale)


@dataclass
class _Group:
    """The comparison of one field for the handshakes of one host in one role."""

    expected: Any
    enforced: bool
    mismatched: Counter[str] = field(default_factory=Counter)
    total: int = 0


@dataclass
class _Host:
    profile: OsProfile
    syns: int = 0
    syn_acks: int = 0
    groups: dict[tuple[str, str], _Group] = field(default_factory=dict)


def _show(value: Any) -> str:
    if value is None:
        return "none"
    if isinstance(value, bool):
        return "yes" if value else "no"
    if isinstance(value, tuple | list):
        return ",".join(value)
    return str(value)


@dataclass
class TransportResult:
    """The outcome of the transport behavior check."""

    labels: TransportLabels
    strict: bool
    profiles: list[OsProfile] = field(default_factory=list)
    hosts: dict[str, _Host] = field(default_factory=dict)
    syns_total: int = 0
    syn_acks_total: int = 0
    unprofiled_handshakes: int = 0
    failures: list[str] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        """True when a handshake was compared and every field matched."""
        return not self.failures

    def deviations(self) -> list[dict[str, Any]]:
        """Return every field that differs from its profile, enforced or not."""
        out: list[dict[str, Any]] = []
        for address, host in self.hosts.items():
            for (role, name), group in host.groups.items():
                if not group.mismatched:
                    continue
                out.append(
                    {
                        "host": address,
                        "profile": host.profile.name,
                        "role": role,
                        "field": name,
                        "expected": _show(group.expected),
                        "observed": {
                            value: count
                            for value, count in sorted(
                                group.mismatched.items(), key=lambda item: (-item[1], item[0])
                            )
                        },
                        "handshakes": group.total,
                        "mismatched": sum(group.mismatched.values()),
                        "enforced": group.enforced,
                    }
                )
        return out

    def metrics(self) -> list[dict[str, Any]]:
        """Return the deviating packets of each enforced field and role, and the failures series.

        The series sums ``mismatched`` over the hosts for each field and role in which at least
        one packet of a profiled host was compared. A SYN has the side ``client`` and a SYN-ACK
        the side ``server``. A field that the check does not enforce (the option order and the SYN
        window without ``strict``) has no series.
        """
        totals: dict[tuple[str, str], int] = {}
        for host in self.hosts.values():
            for (role, name), group in host.groups.items():
                if group.enforced:
                    key = (name, _SIDE[role])
                    totals[key] = totals.get(key, 0) + sum(group.mismatched.values())
        order = {name: i for i, name in enumerate((*FIELDS, *STRICT_FIELDS))}
        series = [
            {
                "name": METRIC,
                "labels": {"property": name, "side": side},
                "value": count,
            }
            for (name, side), count in sorted(
                totals.items(), key=lambda item: (item[0][1] != "client", order[item[0][0]])
            )
        ]
        series.append(failures_series(CHECK_NAME, len(self.failures), self.passed))
        return series

    def to_dict(self) -> dict[str, Any]:
        """Return the check entry of the realism report."""
        settings = self.labels
        return {
            "check": CHECK_NAME,
            "metrics": self.metrics(),
            "passed": self.passed,
            "strict": self.strict,
            "client_hops": settings.client_hops,
            "server_hops": settings.server_hops,
            "expect": dict(settings.expect),
            "profiles": [profile.to_dict() for profile in self.profiles],
            "hosts": [
                {
                    "host": address,
                    "profile": host.profile.name,
                    "syns": host.syns,
                    "syn_acks": host.syn_acks,
                }
                for address, host in self.hosts.items()
            ],
            "syns_total": self.syns_total,
            "syn_acks_total": self.syn_acks_total,
            "unprofiled_handshakes": self.unprofiled_handshakes,
            "deviations": self.deviations(),
            "failures_total": len(self.failures),
            "failures": self.failures[:MAX_FAILURES],
        }


def check(
    capture: Path,
    labels: Labels,
    os_profile: str | None = None,
    profiles_dir: Path | None = None,
    strict: bool | None = None,
    client_hops: int | None = None,
    server_hops: int | None = None,
) -> TransportResult:
    """Compare the SYN and SYN-ACK of every connection with the profile of the sending host.

    The ``transport`` section of the labels gives the settings, and the arguments override its
    ``os_profile``, ``strict``, ``client_hops``, and ``server_hops``. Profile files come from
    ``profiles_dir`` (the argument), from the ``profiles_dir`` of the section (relative to the
    labels file), or from the kernel backend.
    """
    base = labels.transport or TransportLabels()
    resolved = TransportLabels(
        os_profile=os_profile if os_profile is not None else base.os_profile,
        hosts=base.hosts,
        profiles_dir=base.profiles_dir,
        client_hops=base.client_hops if client_hops is None else client_hops,
        server_hops=base.server_hops if server_hops is None else server_hops,
        strict=base.strict if strict is None else strict,
        expect=base.expect,
    )
    if resolved.os_profile is None and not resolved.hosts:
        raise LabelsError(
            "transport: name an os_profile or hosts, on the command line or in labels"
        )
    directory = _profiles_dir(resolved, labels, profiles_dir)
    names = sorted({n for n in (resolved.os_profile, *resolved.hosts.values()) if n is not None})
    profiles = {name: _with_expected(load_profile(name, directory), resolved) for name in names}
    result = TransportResult(
        labels=resolved, strict=resolved.strict, profiles=[profiles[n] for n in names]
    )
    first_syn: dict[tuple[str, int, str, int], Handshake] = {}
    answers: list[Handshake] = []
    seen_answer: set[tuple[str, int, str, int]] = set()
    _, frames = read_frames(capture)
    for frame in frames:
        shake = decode_handshake(frame)
        if shake is None:
            continue
        key = (shake.src, shake.sport, shake.dst, shake.dport)
        if shake.kind == "syn":
            if key not in first_syn:
                first_syn[key] = shake
        elif key not in seen_answer:
            seen_answer.add(key)
            answers.append(shake)
    default = profiles[resolved.os_profile] if resolved.os_profile is not None else None
    by_host = {_canonical(a): profiles[n] for a, n in resolved.hosts.items()}
    for syn in first_syn.values():
        result.syns_total += 1
        profile = by_host.get(syn.src, default)
        if profile is None:
            result.unprofiled_handshakes += 1
            continue
        _compare_syn(result, profile, syn, resolved)
    for answer in answers:
        result.syn_acks_total += 1
        profile = by_host.get(answer.src, default)
        if profile is None:
            result.unprofiled_handshakes += 1
            continue
        offer = first_syn.get((answer.dst, answer.dport, answer.src, answer.sport))
        _compare_syn_ack(result, profile, answer, offer, resolved)
    _decide(result)
    return result


def _canonical(address: str) -> str:
    return str(ipaddress.ip_address(address))


def _profiles_dir(settings: TransportLabels, labels: Labels, override: Path | None) -> Path:
    if override is not None:
        return override
    if settings.profiles_dir is not None:
        return (labels.base_dir or Path()) / settings.profiles_dir
    return PROFILES_DIR


def _with_expected(profile: OsProfile, settings: TransportLabels) -> OsProfile:
    """Return the profile with the values that the labels replace."""
    expect = settings.expect
    if not expect:
        return profile
    scale = profile.window_scale
    if "window_scale" in expect and scale is not None:
        scale = expect["window_scale"]
    return OsProfile(
        name=profile.name,
        ttl=expect.get("ttl", profile.ttl),
        mss=expect.get("mss", profile.mss),
        window_scale=scale,
        syn_window=expect.get("syn_window", profile.syn_window),
        options=profile.options,
        dont_fragment=profile.dont_fragment,
        source=profile.source,
    )


def _host(result: TransportResult, address: str, profile: OsProfile) -> _Host:
    host = result.hosts.get(address)
    if host is None:
        host = result.hosts[address] = _Host(profile)
    return host


def _note(
    host: _Host, role: str, name: str, expected: Any, observed: Any, enforced: bool = True
) -> None:
    group = host.groups.get((role, name))
    if group is None:
        group = host.groups[(role, name)] = _Group(expected, enforced)
    group.total += 1
    if observed != expected:
        group.mismatched[_show(observed)] += 1


def _compare_fields(
    host: _Host,
    role: str,
    shake: Handshake,
    profile: OsProfile,
    hops: int,
    sack: bool,
    timestamps: bool,
    scale: int | None,
) -> None:
    if shake.malformed:
        _note(host, role, "options", "well formed", _MALFORMED)
        return
    _note(host, role, "ttl", profile.ttl - hops, shake.ttl)
    if shake.df is not None:
        _note(host, role, "df", profile.dont_fragment, shake.df)
    _note(host, role, "mss", profile.mss, shake.mss)
    _note(host, role, "window_scale", scale, shake.window_scale)
    _note(host, role, "sack", sack, shake.sack)
    _note(host, role, "timestamps", timestamps, shake.timestamps)


def _compare_syn(
    result: TransportResult, profile: OsProfile, syn: Handshake, settings: TransportLabels
) -> None:
    host = _host(result, syn.src, profile)
    host.syns += 1
    _compare_fields(
        host,
        "syn",
        syn,
        profile,
        settings.client_hops,
        profile.sack,
        profile.timestamps,
        profile.window_scale,
    )
    if syn.malformed:
        return
    _note(host, "syn", "options", profile.options, syn.options, settings.strict)
    _note(host, "syn", "syn_window", profile.syn_window, syn.window, settings.strict)


def _compare_syn_ack(
    result: TransportResult,
    profile: OsProfile,
    answer: Handshake,
    offer: Handshake | None,
    settings: TransportLabels,
) -> None:
    host = _host(result, answer.src, profile)
    host.syn_acks += 1
    if offer is None or offer.malformed:
        # Without the SYN the capture does not say which options the client offered.
        _note_ttl_mss(host, answer, profile, settings)
        return
    _compare_fields(
        host,
        "syn-ack",
        answer,
        profile,
        settings.server_hops,
        offer.sack and profile.sack,
        offer.timestamps and profile.timestamps,
        profile.window_scale if offer.window_scale is not None else None,
    )


def _note_ttl_mss(
    host: _Host, answer: Handshake, profile: OsProfile, settings: TransportLabels
) -> None:
    if answer.malformed:
        _note(host, "syn-ack", "options", "well formed", _MALFORMED)
        return
    _note(host, "syn-ack", "ttl", profile.ttl - settings.server_hops, answer.ttl)
    if answer.df is not None:
        _note(host, "syn-ack", "df", profile.dont_fragment, answer.df)
    _note(host, "syn-ack", "mss", profile.mss, answer.mss)


def _decide(result: TransportResult) -> None:
    """Turn the deviations into failure sentences, and judge an empty comparison."""
    for item in result.deviations():
        if not item["enforced"]:
            continue
        role = "SYN" if item["role"] == "syn" else "SYN-ACK"
        seen = ", ".join(f"{value} in {count}" for value, count in item["observed"].items())
        result.failures.append(
            f"host {item['host']} (profile {item['profile']}): {item['mismatched']} of "
            f"{item['handshakes']} {role} packets differ in {_FIELD_TEXT[item['field']]}: "
            f"the profile expects {item['expected']}, the capture shows {seen} of them"
        )
    if not result.hosts:
        reason = (
            "the capture holds no TCP handshake"
            if result.syns_total == 0 and result.syn_acks_total == 0
            else "no TCP handshake of the capture comes from a host with an OS profile"
        )
        result.failures.append(f"{reason}, so the transport behavior is not judged")


_FIELD_TEXT = {
    "ttl": "the IP TTL",
    "df": "the do not fragment flag",
    "mss": "the MSS option",
    "window_scale": "the window scale option",
    "sack": "the SACK permitted option",
    "timestamps": "the timestamp option",
    "options": "the TCP option list",
    "syn_window": "the SYN window",
}
