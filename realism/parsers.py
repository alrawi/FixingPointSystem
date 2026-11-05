# SPDX-License-Identifier: GPL-3.0-only
"""Reference parsers of the realism runner and their stub, tshark, and Zeek runners.

A parser runner reads a capture with a reference protocol implementation and
returns one :class:`Finding` per parser error. The tshark runner reports every
frame that carries expert information of error severity, which includes every
malformed packet. The Zeek runner reports the entries of ``weird.log``. Both run
as separate processes, so the runner never links Wireshark or Zeek code.
"""

from __future__ import annotations

import ipaddress
import json
import os
import re
import shutil
import subprocess
import tempfile
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from realism.capture import SessionKey

TSHARK = "tshark"
ZEEK = "zeek"
DEFAULT_TSHARK = "tshark"
DEFAULT_ZEEK = "zeek"
# The Zeek project packages install the program here, outside the search path.
ZEEK_PACKAGE_PATH = Path("/opt/zeek/bin/zeek")
DEFAULT_TIMEOUT_S = 300.0
PARSER_NAMES = (TSHARK, ZEEK)

KIND_MALFORMED = "malformed"
KIND_ERROR = "error"
KIND_WEIRD = "weird"

# Expert information constants of epan/proto.h. The severity and the group share
# one 32 bit value, and tshark prints each field as its number.
_PI_ERROR = 0x00800000
_PI_MALFORMED = 0x07000000
_AGGREGATOR = "\x1f"
_TSHARK_FIELDS = (
    "frame.number",
    "frame.time_epoch",
    "ip.src",
    "ip.dst",
    "ipv6.src",
    "ipv6.dst",
    "tcp.srcport",
    "tcp.dstport",
    "udp.srcport",
    "udp.dstport",
    "frame.protocols",
    "_ws.expert.severity",
    "_ws.expert.group",
    "_ws.expert.message",
)
_TSHARK_VERSION = re.compile(r"TShark \(Wireshark\)\s+v?(\d+(?:\.\d+)+)")
_ZEEK_VERSION = re.compile(r"zeek version\s+v?(\d+(?:\.\d+)+\S*)")

# Weirds that Zeek raises from connection or transaction state rather than from a
# message it failed to parse. They describe session behavior, which other realism
# checks cover, so the parser error check does not count them.
ZEEK_STATE_WEIRDS = frozenset(
    {
        "above_hole_data_without_any_acks",
        "active_connection_reuse",
        "data_before_established",
        "dns_unmatched_msg",
        "dns_unmatched_msg_quantity",
        "dns_unmatched_reply",
        "possible_split_routing",
    }
)


class ParserError(Exception):
    """A reference parser failed to run or produced output that cannot be read."""


@dataclass(frozen=True)
class Finding:
    """One parser error that a reference parser reported for one packet or event.

    The endpoint fields are None when the parser reported no transport endpoints,
    and the check then counts the finding as unattributed.
    """

    parser: str
    kind: str
    name: str
    message: str = ""
    frame: int | None = None
    timestamp: float | None = None
    transport: str | None = None
    src_ip: str | None = None
    src_port: int | None = None
    dst_ip: str | None = None
    dst_port: int | None = None

    @property
    def key(self) -> SessionKey | None:
        """The session key of the finding, or None without a complete 5-tuple."""
        if (
            self.transport is None
            or self.src_ip is None
            or self.src_port is None
            or self.dst_ip is None
            or self.dst_port is None
        ):
            return None
        return SessionKey.of(self.transport, self.src_ip, self.src_port, self.dst_ip, self.dst_port)


class ParserRunner(Protocol):
    """A reference parser that reports the parser errors of a capture.

    ``name`` is ``tshark``, ``zeek``, or ``stub``. ``findings`` returns every
    parser error of the capture in capture order.
    """

    name: str

    def version(self) -> str:
        """Return the version of the parser for the report."""
        ...

    def findings(self, capture: Path) -> list[Finding]:
        """Run the parser on the capture and return its parser errors."""
        ...


class StubParser:
    """A deterministic parser runner for tests that never reads the capture.

    The stub returns the findings of a function of the capture path, and reports
    itself under the parser ``name`` that the test chooses.
    """

    def __init__(
        self,
        findings: Iterable[Finding] | Callable[[Path], Iterable[Finding]] = (),
        name: str = "stub",
    ) -> None:
        """Create a stub that returns fixed findings or the result of a function."""
        self.name = name
        self._findings = findings

    def version(self) -> str:
        """Return the fixed stub version."""
        return "stub"

    def findings(self, capture: Path) -> list[Finding]:
        """Return the configured findings for the capture."""
        source = self._findings(capture) if callable(self._findings) else self._findings
        return list(source)


def _run(
    program: str, argv: list[str], what: str, timeout_s: float, cwd: Path | None = None
) -> subprocess.CompletedProcess[str]:
    """Run a parser program with no input and return its completed process."""
    try:
        proc = subprocess.run(
            [program, *argv],
            capture_output=True,
            text=True,
            errors="replace",
            timeout=timeout_s,
            check=False,
            stdin=subprocess.DEVNULL,
            cwd=cwd,
        )
    except FileNotFoundError as exc:
        raise ParserError(f"{what}: {program} not found, install it or pass its path") from exc
    except PermissionError as exc:
        raise ParserError(f"{what}: {program} is not executable") from exc
    except subprocess.TimeoutExpired as exc:
        raise ParserError(f"{what}: {program} ran longer than {timeout_s} s") from exc
    if proc.returncode != 0:
        lines = [
            line
            for line in (proc.stderr or proc.stdout).strip().splitlines()
            if not line.startswith("Running as user")
        ]
        raise ParserError(
            f"{what}: {program} exited with {proc.returncode}: "
            + (lines[-1] if lines else "no output")
        )
    return proc


def _available(program: str) -> bool:
    if os.sep in program:
        path = Path(program)
        return path.is_file() and os.access(path, os.X_OK)
    return shutil.which(program) is not None


class TsharkRunner:
    """Report frames with expert information of error severity by running tshark.

    Every malformed packet carries such expert information, so the runner reports
    malformed packets as findings of kind ``malformed`` and other protocol errors
    as findings of kind ``error``.
    """

    name = TSHARK

    def __init__(self, program: str | None = None, timeout_s: float = DEFAULT_TIMEOUT_S) -> None:
        """Create a runner for a tshark path, which defaults to ``FPS_TSHARK`` and the path."""
        self.program = program or os.environ.get("FPS_TSHARK") or DEFAULT_TSHARK
        self.timeout_s = timeout_s
        self._version: str | None = None

    def available(self) -> bool:
        """Return True when the tshark program exists."""
        return _available(self.program)

    def version(self) -> str:
        """Return the version that ``tshark --version`` prints."""
        if self._version is None:
            proc = _run(self.program, ["--version"], "tshark version", self.timeout_s)
            match = _TSHARK_VERSION.search(proc.stdout)
            self._version = match.group(1) if match else "unknown"
        return self._version

    def findings(self, capture: Path) -> list[Finding]:
        """Run tshark on the capture and return one finding per error frame."""
        argv = ["-r", str(capture), "-n", "-Y", f"_ws.expert.severity >= {_PI_ERROR:#010x}"]
        argv += ["-T", "fields", "-E", "header=n", "-E", "separator=/t"]
        argv += ["-E", "occurrence=a", "-E", f"aggregator={_AGGREGATOR}"]
        for name in _TSHARK_FIELDS:
            argv += ["-e", name]
        proc = _run(self.program, argv, f"tshark parse {capture}", self.timeout_s)
        return parse_tshark_fields(proc.stdout, source=str(capture))


def _first(value: str) -> str:
    return value.split(_AGGREGATOR, 1)[0].strip()


def _ip(value: str) -> str | None:
    text = _first(value)
    if not text:
        return None
    try:
        return str(ipaddress.ip_address(text))
    except ValueError:
        return None


def _port(value: str) -> int | None:
    text = _first(value)
    if not text.isdigit():
        return None
    port = int(text)
    return port if port <= 0xFFFF else None


def _int(text: str, base: int = 0) -> int | None:
    try:
        return int(text.strip(), base)
    except ValueError:
        return None


def parse_tshark_fields(text: str, source: str = "<tshark>") -> list[Finding]:
    """Parse the field output of the tshark runner into findings.

    Each line holds the fields of ``_TSHARK_FIELDS`` separated by tabs, with the
    occurrences of a repeated field joined by the unit separator character.
    """
    findings: list[Finding] = []
    width = len(_TSHARK_FIELDS)
    for line_no, line in enumerate(text.splitlines(), start=1):
        if not line.strip():
            continue
        cols = line.split("\t", width - 1)
        if len(cols) < width:
            raise ParserError(f"tshark parse {source}: line {line_no} has {len(cols)} fields")
        (number, epoch, ip_src, ip_dst, ip6_src, ip6_dst) = cols[:6]
        (tcp_src, tcp_dst, udp_src, udp_dst, protocols) = cols[6:11]
        severities = [_int(v) or 0 for v in cols[11].split(_AGGREGATOR)]
        groups = [_int(v) or 0 for v in cols[12].split(_AGGREGATOR)]
        messages = cols[13].split(_AGGREGATOR)
        frame = _int(number, 10)
        if frame is None:
            raise ParserError(f"tshark parse {source}: line {line_no} has frame {number!r}")
        errors = [i for i, sev in enumerate(severities) if sev >= _PI_ERROR]
        if not errors:
            continue
        malformed = any(i < len(groups) and groups[i] == _PI_MALFORMED for i in errors)
        layers = [p for p in protocols.split(":") if p]
        transport = next((p for p in layers if p in ("tcp", "udp")), None)
        src_port, dst_port = (
            (_port(tcp_src), _port(tcp_dst))
            if transport == "tcp"
            else (_port(udp_src), _port(udp_dst))
        )
        try:
            timestamp: float | None = float(epoch) if epoch.strip() else None
        except ValueError:
            timestamp = None
        findings.append(
            Finding(
                parser=TSHARK,
                kind=KIND_MALFORMED if malformed else KIND_ERROR,
                name=layers[-1] if layers else "frame",
                message=next((messages[i].strip() for i in errors if i < len(messages)), ""),
                frame=frame,
                timestamp=timestamp,
                transport=transport,
                src_ip=_ip(ip_src) or _ip(ip6_src),
                src_port=src_port,
                dst_ip=_ip(ip_dst) or _ip(ip6_dst),
                dst_port=dst_port,
            )
        )
    return findings


class ZeekRunner:
    """Report the entries of ``weird.log`` by running Zeek on the capture.

    The runner passes ``-C`` so that checksum offload in the capturing host does
    not count as a parser error, and it writes JSON logs into a temporary
    directory. Weirds that Zeek raises at one packet time for one connection count
    as one finding, and the state weirds of :data:`ZEEK_STATE_WEIRDS` do not count.
    """

    name = ZEEK

    def __init__(
        self,
        program: str | None = None,
        timeout_s: float = DEFAULT_TIMEOUT_S,
        ignore: Iterable[str] = ZEEK_STATE_WEIRDS,
    ) -> None:
        """Create a runner for a Zeek path, which defaults to ``FPS_ZEEK`` and the path.

        Without either, the runner uses ``/opt/zeek/bin/zeek`` when it exists.
        """
        default = DEFAULT_ZEEK
        if shutil.which(DEFAULT_ZEEK) is None and ZEEK_PACKAGE_PATH.is_file():
            default = str(ZEEK_PACKAGE_PATH)
        self.program = program or os.environ.get("FPS_ZEEK") or default
        self.timeout_s = timeout_s
        self.ignore = frozenset(ignore)
        self._version: str | None = None

    def available(self) -> bool:
        """Return True when the Zeek program exists."""
        return _available(self.program)

    def version(self) -> str:
        """Return the version that ``zeek --version`` prints."""
        if self._version is None:
            proc = _run(self.program, ["--version"], "zeek version", self.timeout_s)
            match = _ZEEK_VERSION.search(proc.stdout + proc.stderr)
            self._version = match.group(1) if match else "unknown"
        return self._version

    def findings(self, capture: Path) -> list[Finding]:
        """Run Zeek on the capture and return the findings of its weird log."""
        with tempfile.TemporaryDirectory(prefix="fps-realism-zeek-") as tmp:
            logs = Path(tmp)
            argv = ["-C", "-r", str(capture.resolve()), "LogAscii::use_json=T"]
            _run(self.program, argv, f"zeek parse {capture}", self.timeout_s, cwd=logs)
            weird = _read_log(logs / "weird.log")
            conn = _read_log(logs / "conn.log")
        return parse_zeek_logs(weird, conn, ignore=self.ignore, source=str(capture))


def _read_log(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return ""
    except OSError as exc:
        raise ParserError(f"zeek read {path.name}: {exc.strerror}") from exc


def _json_lines(text: str, log: str, source: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for line_no, line in enumerate(text.splitlines(), start=1):
        if not line.strip() or line.startswith("#"):
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ParserError(f"zeek parse {source}: {log} line {line_no}: {exc}") from exc
        if not isinstance(row, dict):
            raise ParserError(f"zeek parse {source}: {log} line {line_no} is not an object")
        rows.append(row)
    return rows


def _zeek_port(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value if 0 <= value <= 0xFFFF else None


def _zeek_ip(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    try:
        return str(ipaddress.ip_address(value))
    except ValueError:
        return None


def parse_zeek_logs(
    weird_log: str,
    conn_log: str = "",
    ignore: Iterable[str] = ZEEK_STATE_WEIRDS,
    source: str = "<zeek>",
) -> list[Finding]:
    """Parse a JSON ``weird.log`` into findings, with transports from ``conn.log``.

    Weirds that share the connection uid, or the endpoints when there is no uid,
    and the timestamp merge into one finding whose name lists their names.
    """
    skip = frozenset(ignore)
    protos: dict[str, str] = {}
    for row in _json_lines(conn_log, "conn.log", source):
        uid, proto = row.get("uid"), row.get("proto")
        if isinstance(uid, str) and proto in ("tcp", "udp"):
            protos[uid] = str(proto)
    merged: dict[tuple[Any, ...], dict[str, Any]] = {}
    for row in _json_lines(weird_log, "weird.log", source):
        name = row.get("name")
        if not isinstance(name, str) or name in skip:
            continue
        ts = row.get("ts")
        timestamp = float(ts) if isinstance(ts, int | float) and not isinstance(ts, bool) else None
        uid = row.get("uid") if isinstance(row.get("uid"), str) else None
        endpoints = (
            _zeek_ip(row.get("id.orig_h")),
            _zeek_port(row.get("id.orig_p")),
            _zeek_ip(row.get("id.resp_h")),
            _zeek_port(row.get("id.resp_p")),
        )
        group = (uid or endpoints, timestamp)
        entry = merged.get(group)
        addl = row.get("addl")
        if entry is None:
            merged[group] = {
                "names": [name],
                "addl": str(addl) if isinstance(addl, str) else "",
                "timestamp": timestamp,
                "transport": protos.get(uid) if uid else None,
                "endpoints": endpoints,
            }
        elif name not in entry["names"]:
            entry["names"].append(name)
    findings = []
    for entry in merged.values():
        src_ip, src_port, dst_ip, dst_port = entry["endpoints"]
        findings.append(
            Finding(
                parser=ZEEK,
                kind=KIND_WEIRD,
                name=",".join(entry["names"]),
                message=entry["addl"],
                timestamp=entry["timestamp"],
                transport=entry["transport"],
                src_ip=src_ip,
                src_port=src_port,
                dst_ip=dst_ip,
                dst_port=dst_port,
            )
        )
    return findings


def not_found_reason(name: str, tshark: str | None = None, zeek: str | None = None) -> str:
    """Return the reason that the report gives for skipping a parser that is not installed."""
    runner: TsharkRunner | ZeekRunner
    if name == TSHARK:
        runner = TsharkRunner(tshark)
    elif name == ZEEK:
        runner = ZeekRunner(zeek)
    else:
        raise ParserError(f"resolve parsers: unknown parser {name!r}")
    return f"not installed, no program at {runner.program}"


def resolve(
    names: Sequence[str],
    tshark: str | None = None,
    zeek: str | None = None,
    timeout_s: float = DEFAULT_TIMEOUT_S,
    required: bool = True,
) -> tuple[list[TsharkRunner | ZeekRunner], list[str]]:
    """Build the runners for parser names and return them with the missing names.

    With ``required`` a missing program raises :class:`ParserError`, and otherwise
    the name goes to the missing list and the runner is left out.
    """
    runners: list[TsharkRunner | ZeekRunner] = []
    missing: list[str] = []
    for name in names:
        runner: TsharkRunner | ZeekRunner
        if name == TSHARK:
            runner = TsharkRunner(tshark, timeout_s)
        elif name == ZEEK:
            runner = ZeekRunner(zeek, timeout_s)
        else:
            raise ParserError(f"resolve parsers: unknown parser {name!r}")
        if runner.available():
            runners.append(runner)
        elif required:
            raise ParserError(f"resolve parsers: {name} program {runner.program} not found")
        else:
            missing.append(name)
    return runners, missing
