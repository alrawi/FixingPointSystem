# SPDX-License-Identifier: GPL-3.0-only
"""Command line entry point of the realism runner.

``python3 -m realism score --pcap FILE --labels FILE`` writes a JSON realism
report and exits with 0 when every check passes, 1 when a check fails, and 2 on a
usage, input, classifier, or parser error.

``python3 -m realism modules`` prints the native modules that a change set
selects for the module realism job, and exits with 2 when it cannot read them.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from realism.capture import CaptureError
from realism.classifier import DEFAULT_TIMEOUT_S, ClassifierError, NdpiClassifier
from realism.labels import LabelsError
from realism.modules import ModuleError, discover, matrix, select
from realism.parsers import PARSER_NAMES, ParserError, not_found_reason, resolve
from realism.report import score

EXIT_PASS = 0
EXIT_FAIL = 1
EXIT_ERROR = 2


def _threshold(text: str) -> float:
    try:
        value = float(text)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"not a number: {text}") from exc
    if not 0.0 <= value <= 1.0:
        raise argparse.ArgumentTypeError("threshold must be from 0 to 1")
    return value


def _parser_list(text: str) -> tuple[str, ...] | None:
    """Read the value of ``--parsers``, where None means every installed parser."""
    if text == "auto":
        return None
    if text == "none":
        return ()
    names = tuple(name.strip() for name in text.split(",") if name.strip())
    unknown = [name for name in names if name not in PARSER_NAMES]
    if unknown or not names:
        raise argparse.ArgumentTypeError(
            f"expected auto, none, or a list of {', '.join(PARSER_NAMES)}"
        )
    return names


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="python3 -m realism", description="Realism checks for FixingPointSystem captures."
    )
    commands = parser.add_subparsers(dest="command", required=True)
    score_cmd = commands.add_parser(
        "score", help="score a capture against expected application labels"
    )
    score_cmd.add_argument("--pcap", type=Path, required=True, help="pcap or pcapng capture")
    score_cmd.add_argument("--labels", type=Path, required=True, help="labels file in JSON or YAML")
    score_cmd.add_argument(
        "--output", type=Path, help="write the report to this file instead of standard output"
    )
    score_cmd.add_argument(
        "--threshold", type=_threshold, help="conformance threshold, overrides the labels file"
    )
    score_cmd.add_argument(
        "--ndpi-reader", help="path of ndpiReader, defaults to FPS_NDPI_READER or the search path"
    )
    score_cmd.add_argument(
        "--timeout",
        type=float,
        default=DEFAULT_TIMEOUT_S,
        help="seconds that ndpiReader and each parser may run, default %(default)s",
    )
    score_cmd.add_argument(
        "--parsers",
        type=_parser_list,
        default=None,
        help="auto (default) runs every installed parser, none skips the parser error "
        "check, and a list such as tshark,zeek requires each named parser",
    )
    score_cmd.add_argument("--tshark", help="path of tshark, defaults to FPS_TSHARK or the path")
    score_cmd.add_argument("--zeek", help="path of zeek, defaults to FPS_ZEEK or the path")
    modules_cmd = commands.add_parser(
        "modules", help="list the native modules that a change set selects"
    )
    modules_cmd.add_argument(
        "--root",
        type=Path,
        default=Path("."),
        help="repository root, default the current directory",
    )
    modules_cmd.add_argument(
        "--changed",
        help="file with one changed path per line, or - for standard input; "
        "absent selects every module",
    )
    modules_cmd.add_argument(
        "--format",
        choices=("matrix", "names"),
        default="matrix",
        help="matrix (default) prints a GitHub Actions matrix, names prints one name per line",
    )
    return parser


def _modules(args: argparse.Namespace) -> int:
    try:
        changed: list[str] | None = None
        if args.changed == "-":
            changed = sys.stdin.read().splitlines()
        elif args.changed is not None:
            changed = Path(args.changed).read_text(encoding="utf-8").splitlines()
        chosen = select(discover(args.root), changed)
    except OSError as exc:
        print(f"realism modules failed: read {args.changed}: {exc.strerror}", file=sys.stderr)
        return EXIT_ERROR
    except ModuleError as exc:
        print(f"realism modules failed: {exc}", file=sys.stderr)
        return EXIT_ERROR
    if args.format == "names":
        sys.stdout.write("".join(f"{module.name}\n" for module in chosen))
    else:
        sys.stdout.write(json.dumps(matrix(chosen), separators=(",", ":")) + "\n")
    return EXIT_PASS


def main(argv: list[str] | None = None) -> int:
    """Run the command line and return its exit code."""
    args = _parser().parse_args(argv)
    if args.command == "modules":
        return _modules(args)
    classifier = NdpiClassifier(reader=args.ndpi_reader, timeout_s=args.timeout)
    try:
        parsers, missing = resolve(
            PARSER_NAMES if args.parsers is None else args.parsers,
            tshark=args.tshark,
            zeek=args.zeek,
            timeout_s=args.timeout,
            required=args.parsers is not None,
        )
        reasons = {name: not_found_reason(name, args.tshark, args.zeek) for name in missing}
        for name in missing:
            print(f"realism score: skipped parser {name}: {reasons[name]}", file=sys.stderr)
        report = score(
            args.pcap,
            args.labels,
            classifier,
            threshold=args.threshold,
            parsers=None if args.parsers == () else parsers,
            missing_parsers=missing,
            skip_reasons=reasons,
        )
    except (CaptureError, LabelsError, ClassifierError, ParserError) as exc:
        print(f"realism score failed: {exc}", file=sys.stderr)
        return EXIT_ERROR
    text = report.to_json()
    if args.output is None:
        sys.stdout.write(text)
    else:
        try:
            args.output.write_text(text, encoding="utf-8")
        except OSError as exc:
            print(f"realism score failed: write {args.output}: {exc.strerror}", file=sys.stderr)
            return EXIT_ERROR
    return EXIT_PASS if report.passed else EXIT_FAIL
