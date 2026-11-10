# SPDX-License-Identifier: GPL-3.0-only
"""Command line of the capture tools."""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any

from .decode import Packet, Skip, decode
from .fields import Config
from .pcapio import CaptureError, open_capture
from .scenario import (
    ScenarioError,
    generate,
    load_sessions,
    provenance_record,
    select_sessions,
)
from .scrub import ScrubError, check_record, scrub_document, sha256_bytes
from .sessions import DEFAULT_UDP_IDLE_S, build_document


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1 << 20):
            digest.update(chunk)
    return digest.hexdigest()


def extract(
    path: Path,
    *,
    udp_idle_s: float = DEFAULT_UDP_IDLE_S,
    payloads: bool = False,
    interface: int | None = None,
) -> dict[str, Any]:
    """Read a capture file and return its session document."""
    reader, mapping = open_capture(path)
    try:
        packets: list[Packet] = []
        skipped: Counter[str] = Counter()
        frames = 0
        unordered = 0
        last_ts = 0
        for frame in reader.frames():
            if interface is not None and frame.interface != interface:
                continue
            frames += 1
            result = decode(frame)
            if isinstance(result, Skip):
                skipped[result.reason] += 1
                continue
            unordered += result.ts_ns < last_ts
            last_ts = max(last_ts, result.ts_ns)
            packets.append(result)
        if unordered:
            packets.sort(key=lambda p: p.ts_ns)  # stable, so equal times keep file order
        warnings = list(reader.warnings)
        if unordered:
            warnings.append(f"{unordered} packets had earlier timestamps than their predecessors")
        source = {
            "file": path.name,
            "format": reader.format,
            "sha256": _file_sha256(path),
            "frames": frames,
            "packets": len(packets),
            "skipped_frames": dict(sorted(skipped.items())),
            "warnings": warnings,
        }
        return build_document(
            packets,
            source=source,
            interfaces=reader.interfaces,
            udp_idle_s=udp_idle_s,
            payloads=payloads,
        )
    finally:
        if mapping is not None:
            mapping.close()


def _add_scenario_parser(sub: Any) -> None:
    p = sub.add_parser(
        "scenario",
        help="generate a module directory of scenarios from extracted sessions",
        description="Turn sessions that `sessions --payloads` wrote into a module directory "
        "with manifest.yaml and scenarios/*.yaml. The name of the output directory is the "
        "module name.",
    )
    p.add_argument("--sessions", type=Path, required=True, help="session JSON with payloads")
    pick = p.add_mutually_exclusive_group(required=True)
    pick.add_argument(
        "--session", type=int, action="append", metavar="ID", help="session id (repeatable)"
    )
    pick.add_argument("--all", action="store_true", help="every session of the document")
    p.add_argument("--out-dir", type=Path, required=True, help="module directory to write")
    p.add_argument(
        "--combine",
        metavar="NAME",
        help="put the selected sessions in one scenario NAME, one flow each, in start order "
        "(default: one scenario per session)",
    )
    p.add_argument("--fill-min", type=int, default=256, metavar="BYTES",
                   help="bodies and frame payloads of at least this size become fill(n) "
                   "(default %(default)s)")  # fmt: skip
    p.add_argument("--opaque-min", type=int, default=2048, metavar="BYTES",
                   help="binary turns without framing of at least this size become fill(n) "
                   "(default %(default)s)")  # fmt: skip
    p.add_argument("--timestamp-window", type=int, default=86400, metavar="SECONDS",
                   help="a timestamp within this of the capture time is dynamic "
                   "(default %(default)s)")  # fmt: skip
    p.add_argument("--no-fields", action="store_true",
                   help="copy every payload literally, with no field detection")  # fmt: skip
    p.add_argument("--keep-timing", action="store_true",
                   help="add a delay before a send for each pause between turns")  # fmt: skip
    p.add_argument("--min-delay-ms", type=int, default=10, metavar="MS",
                   help="pauses shorter than this get no delay (default %(default)s)")  # fmt: skip
    p.add_argument("--license", default="GPL-3.0-only", metavar="SPDX",
                   help="license of the generated module (default %(default)s)")  # fmt: skip
    p.add_argument("--synthetic", action="store_true",
                   help="record the provenance as synthetic, for your own traffic")  # fmt: skip
    for removed in ("--consent", "--scrubbed"):
        p.add_argument(removed, nargs="?", const="", default=None, help=argparse.SUPPRESS)
    p.add_argument("--report", type=Path, help="write the generation report as JSON")


def _run_scenario(args: argparse.Namespace) -> int:
    if args.consent is not None or args.scrubbed is not None:
        print(
            "capture: --consent and --scrubbed are gone: run `python3 -m capture scrub "
            "--consent TEXT` on the sessions, and the manifest takes both from its record",
            file=sys.stderr,
        )
        return 1
    cfg = Config(
        fill_min=args.fill_min,
        opaque_min=args.opaque_min,
        timestamp_window_s=args.timestamp_window,
        keep_timing=args.keep_timing,
        min_delay_ms=args.min_delay_ms,
        detect_fields=not args.no_fields,
    )
    try:
        doc = load_sessions(args.sessions)
        sessions = select_sessions(doc, None if args.all else args.session)
        record, problems = check_record(doc)
        prov, notes = provenance_record(
            synthetic=args.synthetic,
            scrub=record,
            problems=problems,
            source_name=doc["source"]["file"],
        )
        files, report = generate(
            doc,
            sessions,
            args.out_dir.name,
            cfg,
            combine=args.combine,
            license_id=args.license,
            provenance=prov,
        )
        for rel, text in files.items():
            target = args.out_dir / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8")
        if args.report is not None:
            args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    except ScenarioError as exc:
        print(f"capture: {exc}", file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"capture: cannot write the module: {exc}", file=sys.stderr)
        return 1
    for sc in report["scenarios"]:
        found = ", ".join(f"{k} {v}" for k, v in sc["detected"].items() if v) or "none"
        print(
            f"capture: {sc['scenario']}: {sc['expected_bytes']} bytes, fields: {found}",
            file=sys.stderr,
        )
        for w in sc["warnings"]:
            print(f"capture: warning: {w}", file=sys.stderr)
    for w in notes:
        print(f"capture: warning: {w}", file=sys.stderr)
    return 0


def _add_scrub_parser(sub: Any) -> None:
    p = sub.add_parser(
        "scrub",
        help="remove personal data and credentials from extracted sessions",
        description="Rewrite a sessions document that was written with `sessions --payloads` so "
        "that it holds no real addresses, user names, passwords, tokens or email addresses, "
        "and add the scrub record that `scenario` needs for a manifest that validates.",
    )
    p.add_argument("--sessions", type=Path, required=True, help="session JSON to scrub")
    p.add_argument("--out", type=Path, help="scrubbed JSON, default standard output")
    p.add_argument("--consent", required=True, metavar="TEXT",
                   help="who allowed the use of the capture, recorded in the manifest")  # fmt: skip
    p.add_argument("--hostnames", action="store_true",
                   help="also replace host names (Host, URLs, EHLO)")  # fmt: skip
    p.add_argument("--hostname", action="append", default=[], metavar="NAME",
                   help="replace this host name everywhere (repeatable)")  # fmt: skip
    p.add_argument("--redact", action="append", default=[], metavar="TEXT",
                   help="replace this text everywhere, such as a name (repeatable)")  # fmt: skip
    p.add_argument("--redact-file", type=Path, metavar="FILE",
                   help="like --redact, one text per line, so it stays out of the shell "
                   "history")  # fmt: skip
    p.add_argument("--drop-port", type=int, action="append", default=[], metavar="PORT",
                   help="remove the sessions that use this port, for protocols that "
                   "the scrubber cannot read (repeatable)")  # fmt: skip
    p.add_argument("--seed", type=int, default=1, help="seed of the replacements (default 1)")
    p.add_argument("--date", metavar="YYYY-MM-DD", help="date in the record (default today, UTC)")
    p.add_argument("--report", type=Path, help="write the scrub record as JSON")


def _run_scrub(args: argparse.Namespace) -> int:
    try:
        raw = args.sessions.read_bytes()
        doc = json.loads(raw)
        literals = list(args.redact)
        if args.redact_file is not None:
            literals += [
                ln for ln in args.redact_file.read_text(encoding="utf-8").splitlines() if ln
            ]
        date = args.date or datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
        if not isinstance(doc, dict) or not str(doc.get("schema_version", "")).startswith("1."):
            raise ScrubError(f"{args.sessions} is not a session document of schema version 1.x")
        out = scrub_document(
            doc,
            consent=args.consent,
            input_sha256=sha256_bytes(raw),
            date=date,
            seed=args.seed,
            hostnames=args.hostnames,
            extra_hosts=args.hostname,
            literals=literals,
            drop_ports=args.drop_port,
        )
        text = json.dumps(out, indent=2) + "\n"
        if args.out is None:
            sys.stdout.write(text)
        else:
            args.out.write_text(text, encoding="utf-8")
        if args.report is not None:
            args.report.write_text(json.dumps(out["scrub"], indent=2) + "\n", encoding="utf-8")
    except (ScrubError, OSError, ValueError) as exc:
        print(f"capture: cannot scrub: {exc}", file=sys.stderr)
        return 1
    rec = out["scrub"]
    pb = rec["payload_bytes"]
    done = ", ".join(f"{r['id']} {r['applied']}" for r in rec["rules"] if r["applied"]) or "none"
    print(
        f"capture: scrubbed: {done}; payload bytes {pb['before']} to {pb['after']} "
        f"({pb['delta']:+d}); {rec['opaque_bytes']} bytes of binary payload were only swept",
        file=sys.stderr,
    )
    if rec["opaque_bytes"]:
        print(
            "capture: warning: binary payloads (DNS names, TLS handshakes, other binary "
            "protocols) are not parsed; check them or remove them with --drop-port",
            file=sys.stderr,
        )
    return 0


def main(argv: list[str] | None = None) -> int:
    """Run the command line and return the exit code."""
    parser = argparse.ArgumentParser(prog="python3 -m capture", description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    _add_scenario_parser(sub)
    _add_scrub_parser(sub)
    p = sub.add_parser("sessions", help="extract TCP sessions and UDP flows as JSON")
    p.add_argument("--pcap", type=Path, required=True, help="pcap or pcapng capture")
    p.add_argument("--out", type=Path, help="output file, default standard output")
    p.add_argument(
        "--udp-idle-timeout",
        type=float,
        default=DEFAULT_UDP_IDLE_S,
        metavar="SECONDS",
        help="a UDP flow idle for longer starts a new session (default %(default)s)",
    )
    p.add_argument("--payloads", action="store_true", help="include payload bytes as base64")
    p.add_argument("--interface", type=int, help="read only frames of this interface id")
    args = parser.parse_args(argv)
    if args.command == "scenario":
        return _run_scenario(args)
    if args.command == "scrub":
        return _run_scrub(args)
    if args.udp_idle_timeout <= 0:
        parser.error("--udp-idle-timeout must be positive")
    try:
        doc = extract(
            args.pcap,
            udp_idle_s=args.udp_idle_timeout,
            payloads=args.payloads,
            interface=args.interface,
        )
    except CaptureError as exc:
        print(f"capture: {exc}", file=sys.stderr)
        return 1
    text = json.dumps(doc, indent=2) + "\n"
    if args.out is None:
        sys.stdout.write(text)
        return 0
    try:
        args.out.write_text(text, encoding="utf-8")
    except OSError as exc:
        print(f"capture: cannot write {args.out}: {exc}", file=sys.stderr)
        return 1
    return 0
