# SPDX-License-Identifier: GPL-3.0-only
"""Command line entry point of the realism runner.

``python3 -m realism score --pcap FILE --labels FILE`` writes a JSON realism
report and exits with 0 when every check passes, 1 when a check fails, and 2 on a
usage, input, or classifier error.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from realism.capture import CaptureError
from realism.classifier import DEFAULT_TIMEOUT_S, ClassifierError, NdpiClassifier
from realism.labels import LabelsError
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
        help="seconds that ndpiReader may run, default %(default)s",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    """Run the command line and return its exit code."""
    args = _parser().parse_args(argv)
    classifier = NdpiClassifier(reader=args.ndpi_reader, timeout_s=args.timeout)
    try:
        report = score(args.pcap, args.labels, classifier, threshold=args.threshold)
    except (CaptureError, LabelsError, ClassifierError) as exc:
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
