# SPDX-License-Identifier: GPL-3.0-only
"""Command line entry point of the realism runner.

``python3 -m realism score --pcap FILE --labels FILE`` writes a JSON realism
report and exits with 0 when every check passes, 1 when a check fails, and 2 on a
usage, input, classifier, or parser error. With ``--attach-run ID`` it also delivers the
report to the controller of that run, and exits with 2 when the controller does not accept it.

``python3 -m realism decrypted --pcap FILE --labels FILE`` decrypts the TLS sessions of a
capture with tshark, writes their plaintext as a capture of plain TCP sessions, and scores
that decrypted view with the labels file. It exits with 0 when every TLS session decrypts and
the view passes, 1 when not, and 2 on a usage, input, tool, or classifier error.

``python3 -m realism modules`` prints the native modules that a change set
selects for the module realism job, and exits with 2 when it cannot read them.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from pathlib import Path

from realism import decrypted, deliver
from realism.capture import CaptureError
from realism.classifier import DEFAULT_TIMEOUT_S, ClassifierError, NdpiClassifier
from realism.encryption import TsharkTlsProbe
from realism.labels import LabelsError
from realism.messages import TsharkMessageProbe
from realism.mix import BASES, MixError
from realism.modules import ModuleError, discover, matrix, select
from realism.parsers import PARSER_NAMES, ParserError, not_found_reason, resolve
from realism.report import score
from realism.scenario_graph import ScenarioError
from realism.transport import TransportError

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


def _starttls_port(text: str) -> tuple[int, str]:
    try:
        return decrypted.parse_starttls_port(text)
    except decrypted.DecryptedError as exc:
        raise argparse.ArgumentTypeError(str(exc)) from exc


def _alpha(text: str) -> float:
    try:
        value = float(text)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"not a number: {text}") from exc
    if not 0.0 < value < 1.0:
        raise argparse.ArgumentTypeError("alpha must be above 0 and below 1")
    return value


def _min_samples(text: str) -> int:
    try:
        value = int(text)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"not an integer: {text}") from exc
    if value < 2:
        raise argparse.ArgumentTypeError("the minimum must be 2 or more")
    return value


def _points(text: str) -> float:
    try:
        value = float(text)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"not a number: {text}") from exc
    if not 0.0 <= value <= 100.0:
        raise argparse.ArgumentTypeError("tolerance must be from 0 to 100 percentage points")
    return value


def _hops(text: str) -> int:
    try:
        value = int(text)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"not an integer: {text}") from exc
    if not 0 <= value <= 254:
        raise argparse.ArgumentTypeError("hops must be from 0 to 254")
    return value


def _host_count(text: str) -> int:
    try:
        value = int(text)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(f"not an integer: {text}") from exc
    if value < 0:
        raise argparse.ArgumentTypeError("the host count must not be negative")
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


def _delivery_target(args: argparse.Namespace) -> deliver.Target:
    """Read the settings and the token of ``--attach-run`` before the capture is scored."""
    env = os.environ
    ca_file = args.ca_file if args.ca_file is not None else _path(env.get("FPS_CA_FILE"))
    if ca_file is not None and not ca_file.is_file():
        raise deliver.DeliveryError(f"CA file {ca_file} is not a file")
    insecure = args.insecure_http
    if insecure is None:
        insecure = env.get("FPS_INSECURE_HTTP", "").lower() in ("1", "true", "yes")
    base = args.url or env.get("FPS_URL") or deliver.DEFAULT_URL
    url = deliver.endpoint(base, args.attach_run, insecure_http=insecure)
    token, warning = deliver.read_token(args.token_file, env)
    if warning is not None:
        print(f"realism score: warning: {warning}", file=sys.stderr)
    return deliver.Target(url=url, token=token, ca_file=ca_file)


def _path(text: str | None) -> Path | None:
    return Path(text) if text else None


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="python3 -m realism", description="Realism checks for FixingPointSystem captures."
    )
    commands = parser.add_subparsers(dest="command", required=True)
    # No abbreviations: "--token SECRET" must not mean "--token-file SECRET".
    score_cmd = commands.add_parser(
        "score", help="score a capture against expected application labels", allow_abbrev=False
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
    score_cmd.add_argument(
        "--keylog",
        type=Path,
        help="TLS key log in the NSS format for the encryption check, in addition to the "
        "keys that a pcapng capture embeds",
    )
    score_cmd.add_argument(
        "--max-duplicate-ratio",
        type=_threshold,
        help="largest share of duplicate payloads per application, from 0 to 1; adds the "
        "content variety check and overrides the labels file, whose default is 0.01",
    )
    score_cmd.add_argument(
        "--scenario",
        type=Path,
        action="append",
        default=[],
        help="scenario file whose flow graph the session structure check compares with the "
        "capture; repeat it for several scenarios; adds the check next to the structure "
        "section of the labels file",
    )
    score_cmd.add_argument(
        "--ks-alpha",
        type=_alpha,
        help="significance level of the KS tests, above 0 and below 1; adds the temporal "
        "distribution check and overrides the labels file, whose default is 0.01",
    )
    score_cmd.add_argument(
        "--ks-min-samples",
        type=_min_samples,
        help="fewest observations that a KS comparison needs, 2 or more; adds the temporal "
        "distribution check and overrides the labels file, whose default is 30",
    )
    score_cmd.add_argument(
        "--test-config",
        type=Path,
        help="test configuration whose profile.mix weights the mix accuracy check compares with "
        "the achieved mix; adds the check next to the mix section of the labels file",
    )
    score_cmd.add_argument(
        "--run-report",
        type=Path,
        help="run report file with the sessions and bytes per scenario, a metric series of "
        "GET /v1/runs/ID/metrics?view=scenario or a bundle with it under metrics; adds the "
        "mix accuracy check",
    )
    score_cmd.add_argument(
        "--mix-tolerance",
        type=_points,
        help="largest difference between an achieved and a configured share in percentage "
        "points, default 2; adds the mix accuracy check and overrides the labels file",
    )
    score_cmd.add_argument(
        "--mix-basis",
        choices=(*BASES, "both"),
        help="basis of the mix accuracy check, default profile.basis of the test configuration; "
        "adds the check and overrides the labels file",
    )
    score_cmd.add_argument(
        "--os-profile",
        help="operating system profile of every host, such as windows11, linux6, or macos14; "
        "adds the transport behavior check next to the transport section of the labels file "
        "and overrides its os_profile",
    )
    score_cmd.add_argument(
        "--os-profiles-dir",
        type=Path,
        help="directory of the profile files, default the profiles of the kernel backend; "
        "adds the transport behavior check",
    )
    score_cmd.add_argument(
        "--transport-strict",
        action="store_true",
        default=None,
        help="also fail on the TCP option order and the SYN window; adds the transport "
        "behavior check",
    )
    score_cmd.add_argument(
        "--client-hops",
        type=_hops,
        help="routers between the clients and the capture point, which lower the TTL of a SYN; "
        "adds the transport behavior check",
    )
    score_cmd.add_argument(
        "--server-hops",
        type=_hops,
        help="routers between the servers and the capture point, which lower the TTL of a "
        "SYN-ACK; adds the transport behavior check",
    )
    score_cmd.add_argument(
        "--host-identity",
        action="store_true",
        help="count the hosts of the capture and fail for an IP address with more than one MAC "
        "address; adds the host identity check next to the hosts section of the labels file",
    )
    score_cmd.add_argument(
        "--host-count",
        type=_host_count,
        help="number of hosts that the capture must hold; adds the host identity check and "
        "overrides the count of the labels file",
    )
    score_cmd.add_argument("--tshark", help="path of tshark, defaults to FPS_TSHARK or the path")
    score_cmd.add_argument("--zeek", help="path of zeek, defaults to FPS_ZEEK or the path")
    score_cmd.add_argument(
        "--attach-run",
        metavar="ID",
        help="deliver the report to the controller with PUT /v1/runs/ID/realism after scoring; "
        "the run must have ended; the run token comes from --token-file, FPS_TOKEN_FILE, or "
        "FPS_TOKEN, and never from the command line",
    )
    score_cmd.add_argument(
        "--url",
        help=f"controller base URL for --attach-run, default FPS_URL or {deliver.DEFAULT_URL}",
    )
    score_cmd.add_argument(
        "--token-file",
        type=Path,
        help="file that holds the run token for --attach-run, default FPS_TOKEN_FILE",
    )
    score_cmd.add_argument(
        "--ca-file",
        type=Path,
        help="CA certificate file that verifies the controller for --attach-run, "
        "default FPS_CA_FILE",
    )
    score_cmd.add_argument(
        "--insecure-http",
        action="store_true",
        default=None,
        help="allow the token over plain http to a host that is not loopback, default "
        "FPS_INSECURE_HTTP=1",
    )
    dec_cmd = commands.add_parser(
        "decrypted",
        help="decrypt the TLS sessions of a capture and score the decrypted view",
        description="Decrypt the TLS sessions of a capture with tshark and the keys that the "
        "capture embeds (or --keylog), write their plaintext as plain TCP sessions with the "
        "same addresses, and score that view with nDPI and the parsers. The labels file "
        "describes the view: its session rules match the server ports of the TLS sessions "
        "(or --view-port) and its applications name the protocol inside TLS.",
    )
    dec_cmd.add_argument("--pcap", type=Path, required=True, help="pcap or pcapng capture")
    dec_cmd.add_argument("--labels", type=Path, required=True, help="labels of the decrypted view")
    dec_cmd.add_argument("--keylog", type=Path, help="TLS key log in the NSS format")
    dec_cmd.add_argument(
        "--plaintext",
        choices=decrypted.PLAINTEXTS,
        default="inside",
        help="inside (default) keeps what was sent inside TLS, upgrade puts the plain start of "
        "a STARTTLS dialogue in front of it, without the STARTTLS command and its reply",
    )
    dec_cmd.add_argument(
        "--view-port",
        type=int,
        help="server port of every session of the view, default the port of the TLS session",
    )
    dec_cmd.add_argument(
        "--starttls-port",
        type=_starttls_port,
        action="append",
        default=[],
        metavar="PORT:PROTOCOL",
        help="capture port of STARTTLS sessions with their protocol (smtp, imap, or pop), for "
        "example 5587:smtp. tshark decodes the port as that protocol, so TLS starts after the "
        "upgrade. Repeat for several ports",
    )
    dec_cmd.add_argument(
        "--sessions", type=int, help="number of TLS sessions that the capture must hold"
    )
    dec_cmd.add_argument("--view", type=Path, help="keep the decrypted view in this pcap file")
    dec_cmd.add_argument("--output", type=Path, help="write the result JSON to this file")
    dec_cmd.add_argument(
        "--threshold", type=_threshold, help="conformance threshold, overrides the labels file"
    )
    dec_cmd.add_argument("--ndpi-reader", help="path of ndpiReader, default FPS_NDPI_READER")
    dec_cmd.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT_S)
    dec_cmd.add_argument(
        "--parsers",
        type=_parser_list,
        default=None,
        help="auto (default) runs every installed parser on the view, none skips the parser "
        "error check, and a list such as tshark,zeek requires each named parser",
    )
    dec_cmd.add_argument("--tshark", help="path of tshark, defaults to FPS_TSHARK or the path")
    dec_cmd.add_argument("--zeek", help="path of zeek, defaults to FPS_ZEEK or the path")
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


def _decrypted(args: argparse.Namespace) -> int:
    for name, path in (("key log", args.keylog), ("labels file", args.labels)):
        if path is not None and not path.is_file():
            print(f"realism decrypted failed: {name} {path} is not a file", file=sys.stderr)
            return EXIT_ERROR
    if args.view_port is not None and not 0 < args.view_port < 65536:
        print("realism decrypted failed: --view-port must be from 1 to 65535", file=sys.stderr)
        return EXIT_ERROR
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
            print(f"realism decrypted: skipped parser {name}: {reasons[name]}", file=sys.stderr)
        with tempfile.TemporaryDirectory(prefix="fps-decrypted-") as tmp:
            result = decrypted.run(
                args.pcap,
                args.labels,
                classifier,
                Path(tmp),
                tshark=decrypted.Tshark(args.tshark, args.timeout),
                keylog=args.keylog,
                plaintext=args.plaintext,
                view_port=args.view_port,
                starttls_ports=dict(args.starttls_port),
                expected_sessions=args.sessions,
                keep_view=args.view,
                threshold=args.threshold,
                parsers=None if args.parsers == () else parsers,
                missing_parsers=missing,
                skip_reasons=reasons,
            )
    except (
        decrypted.DecryptedError,
        CaptureError,
        LabelsError,
        ClassifierError,
        ParserError,
    ) as exc:
        print(f"realism decrypted failed: {exc}", file=sys.stderr)
        return EXIT_ERROR
    text = json.dumps(result.to_dict(), indent=2) + "\n"
    if args.output is None:
        sys.stdout.write(text)
    else:
        try:
            args.output.write_text(text, encoding="utf-8")
        except OSError as exc:
            print(f"realism decrypted failed: write {args.output}: {exc.strerror}", file=sys.stderr)
            return EXIT_ERROR
    print(result.summary(), file=sys.stderr)
    for reason in result.failures:
        print(f"realism decrypted: FAIL {reason}", file=sys.stderr)
    return EXIT_PASS if result.passed else EXIT_FAIL


def main(argv: list[str] | None = None) -> int:
    """Run the command line and return its exit code."""
    args = _parser().parse_args(argv)
    if args.command == "modules":
        return _modules(args)
    if args.command == "decrypted":
        return _decrypted(args)
    if args.keylog is not None and not args.keylog.is_file():
        print(f"realism score failed: key log {args.keylog} is not a file", file=sys.stderr)
        return EXIT_ERROR
    target: deliver.Target | None = None
    if args.attach_run is not None:
        try:
            target = _delivery_target(args)
        except deliver.DeliveryError as exc:
            print(f"realism score failed: {exc}", file=sys.stderr)
            return EXIT_ERROR
    else:
        for flag, value in (
            ("--url", args.url),
            ("--token-file", args.token_file),
            ("--ca-file", args.ca_file),
            ("--insecure-http", args.insecure_http),
        ):
            if value is not None:
                print(f"realism score failed: {flag} needs --attach-run", file=sys.stderr)
                return EXIT_ERROR
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
            tls_probe=TsharkTlsProbe(args.tshark, args.timeout),
            keylog=args.keylog,
            max_duplicate_ratio=args.max_duplicate_ratio,
            scenario_files=args.scenario,
            ks_alpha=args.ks_alpha,
            ks_min_samples=args.ks_min_samples,
            test_config=args.test_config,
            run_report=args.run_report,
            mix_tolerance=args.mix_tolerance,
            mix_basis=args.mix_basis,
            os_profile=args.os_profile,
            os_profiles_dir=args.os_profiles_dir,
            transport_strict=args.transport_strict,
            client_hops=args.client_hops,
            server_hops=args.server_hops,
            message_probe=TsharkMessageProbe(args.tshark, args.timeout),
            host_identity=args.host_identity,
            host_count=args.host_count,
        )
    except (
        CaptureError,
        LabelsError,
        ClassifierError,
        ParserError,
        ScenarioError,
        MixError,
        TransportError,
    ) as exc:
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
    if target is not None:
        try:
            answer = deliver.attach(target, text.encode("utf-8"))
        except deliver.DeliveryError as exc:
            print(f"realism score failed: run {args.attach_run}: {exc}", file=sys.stderr)
            return EXIT_ERROR
        print(f"realism score: {answer.summary(args.attach_run)}", file=sys.stderr)
    return EXIT_PASS if report.passed else EXIT_FAIL
