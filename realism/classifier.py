# SPDX-License-Identifier: GPL-3.0-only
"""Classifier interface of the realism runner and its stub and nDPI implementations.

A classifier names the application protocol of every session in a capture. The
nDPI implementation runs ``ndpiReader`` as a separate process and reads its CSV
output, because nDPI is LGPL-3.0 and the project never links it.
"""

from __future__ import annotations

import csv
import ipaddress
import os
import re
import subprocess
import tempfile
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Protocol

from realism.capture import Session, SessionKey

UNKNOWN = "Unknown"
DEFAULT_NDPI_READER = "ndpiReader"
DEFAULT_TIMEOUT_S = 300.0
_IP_PROTO = {"6": "tcp", "17": "udp", "tcp": "tcp", "udp": "udp"}
_VERSION = re.compile(r"nDPI\s+v?(\d+(?:\.\d+)+\S*)")


class ClassifierError(Exception):
    """A classifier failed to run or produced output that cannot be read."""


class Classifier(Protocol):
    """An independent oracle that names the application protocol of sessions.

    ``name`` is a value of the ``classifier`` label in the metric catalog, for
    example ``ndpi``. ``classify`` returns a protocol name per session key, and a
    session that the result omits counts as :data:`UNKNOWN`.
    """

    name: str

    def version(self) -> str:
        """Return the version of the classifier for the report."""
        ...

    def classify(self, capture: Path, sessions: Sequence[Session]) -> Mapping[SessionKey, str]:
        """Return the protocol name of each session that the classifier recognized."""
        ...


class StubClassifier:
    """A deterministic classifier for tests that labels sessions with a function.

    The stub never reads the capture file. It reports itself as the classifier
    ``name`` that the test chooses, so a test can stand in for nDPI.
    """

    def __init__(self, rule: Callable[[Session], str | None], name: str = "stub") -> None:
        """Create a stub that labels each session with ``rule``, None for unknown."""
        self.name = name
        self._rule = rule

    @classmethod
    def by_server_port(cls, ports: Mapping[int, str], name: str = "stub") -> StubClassifier:
        """Create a stub that labels sessions by their server port."""
        return cls(lambda session: ports.get(session.server_port), name=name)

    def version(self) -> str:
        """Return the fixed stub version."""
        return "stub"

    def classify(self, capture: Path, sessions: Sequence[Session]) -> dict[SessionKey, str]:
        """Apply the rule to every session and drop sessions it leaves unknown."""
        result: dict[SessionKey, str] = {}
        for session in sessions:
            label = self._rule(session)
            if label is not None:
                result[session.key] = label
        return result


class NdpiClassifier:
    """Classify sessions with nDPI by running ``ndpiReader`` out of process.

    The runner passes ``-d`` so that nDPI reports only payload based detections and
    never guesses a protocol from the port number.
    """

    name = "ndpi"

    def __init__(
        self,
        reader: str | None = None,
        timeout_s: float = DEFAULT_TIMEOUT_S,
    ) -> None:
        """Create a classifier for the given ndpiReader path or command name.

        The reader defaults to the ``FPS_NDPI_READER`` environment variable and then
        to ``ndpiReader`` on the search path.
        """
        self.reader = reader or os.environ.get("FPS_NDPI_READER") or DEFAULT_NDPI_READER
        self.timeout_s = timeout_s
        self._version: str | None = None

    def version(self) -> str:
        """Return the nDPI version that ``ndpiReader -h`` prints."""
        if self._version is None:
            # nDPI 4.14 prints its usage text for -h and then exits with 1, so the
            # exit code does not say whether the banner came out.
            proc = self._run([self.reader, "-h"], "read the nDPI version", ok_codes=(0, 1))
            match = _VERSION.search(proc.stdout + proc.stderr)
            self._version = match.group(1) if match else UNKNOWN
        return self._version

    def classify(self, capture: Path, sessions: Sequence[Session]) -> dict[SessionKey, str]:
        """Run ndpiReader on the capture and return its protocol per session key."""
        with tempfile.TemporaryDirectory(prefix="fps-realism-") as tmp:
            out = Path(tmp) / "flows.csv"
            self._run(
                [self.reader, "-q", "-d", "-i", str(capture), "-C", str(out)],
                f"classify {capture}",
            )
            try:
                text = out.read_text(encoding="utf-8", errors="replace")
            except OSError as exc:
                raise ClassifierError(
                    f"ndpi classify {capture}: ndpiReader wrote no CSV output"
                ) from exc
        return parse_ndpi_csv(text, source=str(capture))

    def _run(
        self, argv: list[str], what: str, ok_codes: Sequence[int] = (0,)
    ) -> subprocess.CompletedProcess[str]:
        try:
            proc = subprocess.run(
                argv,
                capture_output=True,
                text=True,
                timeout=self.timeout_s,
                check=False,
                stdin=subprocess.DEVNULL,
            )
        except FileNotFoundError as exc:
            raise ClassifierError(
                f"ndpi {what}: {self.reader} not found, install nDPI or set FPS_NDPI_READER"
            ) from exc
        except subprocess.TimeoutExpired as exc:
            raise ClassifierError(
                f"ndpi {what}: {self.reader} ran longer than {self.timeout_s} s"
            ) from exc
        if proc.returncode not in ok_codes:
            tail = (proc.stderr or proc.stdout).strip().splitlines()[-1:]
            raise ClassifierError(
                f"ndpi {what}: {self.reader} exited with {proc.returncode}: "
                + (tail[0] if tail else "no output")
            )
        return proc


def _delimiter(header: str) -> str:
    """Return the field separator of a flow CSV from its header line."""
    return "|" if header.count("|") > header.count(",") else ","


def parse_ndpi_csv(text: str, source: str = "<csv>") -> dict[SessionKey, str]:
    """Parse the flow CSV of ``ndpiReader -C`` into a protocol name per session key.

    Columns are found by header name, so the parser accepts the column order of
    nDPI 4.2 and of later releases that add columns. nDPI 4.2 separates the fields
    with commas and nDPI 4.14 with ``|``, and the header line says which: the
    dialect is the separator that the header uses most often.
    """
    lines = text.splitlines()
    delimiter = _delimiter(lines[0]) if lines else ","
    rows = csv.reader(
        lines,
        delimiter=delimiter,
        quoting=csv.QUOTE_NONE if delimiter == "|" else csv.QUOTE_MINIMAL,
    )
    header = next(rows, None)
    if header is None:
        return {}
    names = [h.strip().lstrip("#") for h in header]
    required = ("protocol", "src_ip", "src_port", "dst_ip", "dst_port", "ndpi_proto")
    missing = [name for name in required if name not in names]
    if missing:
        raise ClassifierError(f"ndpi parse {source}: CSV header lacks {missing}")
    index = {name: names.index(name) for name in required}
    width = max(index.values()) + 1
    result: dict[SessionKey, str] = {}
    for line, row in enumerate(rows, start=2):
        if not row:
            continue
        if len(row) < width:
            raise ClassifierError(f"ndpi parse {source}: line {line} has {len(row)} fields")
        transport = _IP_PROTO.get(row[index["protocol"]].strip().lower())
        if transport is None:
            continue
        try:
            src_ip = str(ipaddress.ip_address(row[index["src_ip"]].strip()))
            dst_ip = str(ipaddress.ip_address(row[index["dst_ip"]].strip()))
            src_port = int(row[index["src_port"]])
            dst_port = int(row[index["dst_port"]])
        except ValueError as exc:
            raise ClassifierError(f"ndpi parse {source}: line {line}: {exc}") from exc
        proto = row[index["ndpi_proto"]].strip() or UNKNOWN
        result[SessionKey.of(transport, src_ip, src_port, dst_ip, dst_port)] = proto
    return result


def matches(observed: str, accepted: Sequence[str]) -> bool:
    """Return True when a classifier protocol name counts as an accepted name.

    nDPI joins the master and the application protocol with a dot, as in
    ``DNS.Google``. Each part counts, and the comparison ignores case.
    """
    parts = {part.casefold() for part in observed.split(".") if part}
    return any(name.casefold() in parts for name in accepted)
