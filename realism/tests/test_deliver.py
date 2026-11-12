# SPDX-License-Identifier: GPL-3.0-only
"""Tests of the delivery of a realism report to a local fake controller."""

from __future__ import annotations

import json
import threading
from collections.abc import Callable, Iterator
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from realism import deliver
from realism.cli import EXIT_ERROR, EXIT_FAIL, EXIT_PASS, main
from realism.tests import pcapgen
from realism.tests.conftest import NDPI_42_HEADER

CSV_PASS = NDPI_42_HEADER + "\n" + "0,6,1,1,0,10.0.0.10,40000,10.0.1.80,80,7,HTTP,,6,414\n"
TOKEN = "run-token-0123456789abcdef"
OK_ANSWER = {
    "realism": [
        {"property": "a", "observed": 1, "target": ">= 0.99", "passed": True},
        {"property": "b", "observed": 0, "target": "== 0", "passed": False},
    ],
    "replaced": False,
}


@dataclass
class Seen:
    """What the fake controller received and what it answers next."""

    requests: list[dict[str, object]] = field(default_factory=list)
    status: int = 200
    body: bytes = json.dumps(OK_ANSWER).encode()
    content_type: str = "application/json"
    location: str | None = None


@pytest.fixture
def controller() -> Iterator[tuple[str, Seen]]:
    """Serve PUT requests on loopback and record them."""
    seen = Seen()

    class Handler(BaseHTTPRequestHandler):
        def do_PUT(self) -> None:
            length = int(self.headers.get("Content-Length", "0"))
            seen.requests.append(
                {
                    "method": self.command,
                    "path": self.path,
                    "authorization": self.headers.get("Authorization"),
                    "content_type": self.headers.get("Content-Type"),
                    "body": self.rfile.read(length),
                }
            )
            self.send_response(seen.status)
            if seen.location is not None:
                self.send_header("Location", seen.location)
            self.send_header("Content-Type", seen.content_type)
            self.send_header("Content-Length", str(len(seen.body)))
            self.end_headers()
            self.wfile.write(seen.body)

        def log_message(self, *_args: object) -> None:
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", seen
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


def _problem(
    status: int, title: str, detail: str, errors: list[dict[str, str]] | None = None
) -> bytes:
    doc: dict[str, object] = {"title": title, "status": status, "detail": detail}
    if errors is not None:
        doc["errors"] = errors
    return json.dumps(doc).encode()


def _target(base: str, token: str = TOKEN) -> deliver.Target:
    return deliver.Target(url=deliver.endpoint(base, "run1"), token=token)


def test_attach_sends_the_report_with_the_bearer_token(controller: tuple[str, Seen]) -> None:
    base, seen = controller
    answer = deliver.attach(_target(base), b'{"report": "fps-realism"}')
    assert answer == deliver.Attachment(results=2, passed=1, replaced=False)
    assert seen.requests == [
        {
            "method": "PUT",
            "path": "/v1/runs/run1/realism",
            "authorization": f"Bearer {TOKEN}",
            "content_type": "application/json",
            "body": b'{"report": "fps-realism"}',
        }
    ]


def test_attach_reports_a_replaced_report(controller: tuple[str, Seen]) -> None:
    base, seen = controller
    seen.body = json.dumps({**OK_ANSWER, "replaced": True}).encode()
    answer = deliver.attach(_target(base), b"{}")
    assert answer.replaced is True
    assert "replaced the earlier report" in answer.summary("run1")


def test_409_names_the_state_of_the_run(controller: tuple[str, Seen]) -> None:
    base, seen = controller
    seen.status = 409
    seen.content_type = "application/problem+json"
    seen.body = _problem(409, "Conflict", "run run1 is running; only an ended run takes a report")
    with pytest.raises(deliver.DeliveryError) as info:
        deliver.attach(_target(base), b"{}")
    assert str(info.value) == (
        "the controller refused the report: 409 Conflict: run run1 is running; "
        "only an ended run takes a report"
    )


def test_422_lists_every_finding_with_its_pointer(controller: tuple[str, Seen]) -> None:
    base, seen = controller
    seen.status = 422
    seen.content_type = "application/problem+json"
    seen.body = _problem(
        422,
        "Unprocessable Entity",
        "the report breaks the schema",
        [
            {"severity": "error", "path": "/checks/0/passed", "message": "must be boolean"},
            {"severity": "error", "path": "/version", "message": "unsupported"},
        ],
    )
    with pytest.raises(deliver.DeliveryError) as info:
        deliver.attach(_target(base), b"{}")
    assert str(info.value).splitlines() == [
        "the controller refused the report: 422 Unprocessable Entity: the report breaks the schema",
        "  /checks/0/passed: must be boolean",
        "  /version: unsupported",
    ]


@pytest.mark.parametrize("status", [401, 403, 404, 413, 415, 500])
def test_other_refusals_carry_their_status(controller: tuple[str, Seen], status: int) -> None:
    base, seen = controller
    seen.status = status
    seen.body = b"not a problem document"
    with pytest.raises(deliver.DeliveryError, match=f"HTTP {status}: not a problem document"):
        deliver.attach(_target(base), b"{}")


def test_an_answer_never_shows_the_token(controller: tuple[str, Seen]) -> None:
    base, seen = controller
    seen.status = 401
    seen.content_type = "application/problem+json"
    seen.body = _problem(401, "Unauthorized", f"bad token {TOKEN}", [])
    with pytest.raises(deliver.DeliveryError) as info:
        deliver.attach(_target(base), b"{}")
    assert TOKEN not in str(info.value)
    assert "bad token [redacted]" in str(info.value)


def test_a_redirect_is_refused_and_not_followed(controller: tuple[str, Seen]) -> None:
    base, seen = controller
    other = Seen()
    seen.status = 307
    seen.location = "http://127.0.0.1:9/elsewhere"
    seen.body = b""
    with pytest.raises(deliver.DeliveryError, match="redirected, which the client never follows"):
        deliver.attach(_target(base), b"{}")
    assert len(seen.requests) == 1
    assert other.requests == []


def test_a_200_without_an_attachment_is_an_error(controller: tuple[str, Seen]) -> None:
    base, seen = controller
    seen.body = b'{"ok": true}'
    with pytest.raises(deliver.DeliveryError, match="without an attachment"):
        deliver.attach(_target(base), b"{}")


def test_an_unreachable_controller_is_an_error() -> None:
    with pytest.raises(deliver.DeliveryError, match="could not reach the controller"):
        deliver.attach(_target("http://127.0.0.1:9"), b"{}")


def test_token_file_beats_the_environment(tmp_path: Path) -> None:
    named = tmp_path / "named"
    named.write_text("from-flag\n")
    named.chmod(0o600)
    via_env = tmp_path / "via-env"
    via_env.write_text("from-env-file\n")
    via_env.chmod(0o600)
    env = {"FPS_TOKEN_FILE": str(via_env), "FPS_TOKEN": "from-variable"}
    assert deliver.read_token(named, env) == ("from-flag", None)
    assert deliver.read_token(None, env) == ("from-env-file", None)
    assert deliver.read_token(None, {"FPS_TOKEN": " from-variable\n"}) == ("from-variable", None)


def test_a_token_file_that_others_can_read_warns(tmp_path: Path) -> None:
    path = tmp_path / "token"
    path.write_text(TOKEN)
    path.chmod(0o644)
    token, warning = deliver.read_token(path, {})
    assert token == TOKEN
    assert warning is not None
    assert "readable by other users" in warning
    assert TOKEN not in warning


@pytest.mark.parametrize(
    ("content", "message"),
    [("\n", "holds no token"), ("two words", "characters that a header cannot carry")],
)
def test_a_bad_token_file_is_an_error(tmp_path: Path, content: str, message: str) -> None:
    path = tmp_path / "token"
    path.write_text(content)
    with pytest.raises(deliver.DeliveryError, match=message):
        deliver.read_token(path, {})


def test_a_missing_token_is_an_error(tmp_path: Path) -> None:
    with pytest.raises(deliver.DeliveryError, match="no run token"):
        deliver.read_token(None, {})
    with pytest.raises(deliver.DeliveryError, match="read token file"):
        deliver.read_token(tmp_path / "absent", {})


@pytest.mark.parametrize(
    ("base", "run", "message"),
    [
        ("https://c.example", "../x", "run identifier"),
        ("https://c.example", "", "run identifier"),
        ("ftp://c.example", "r", "not an http or https URL"),
        ("https://user:pw@c.example", "r", "credentials"),
        ("http://c.example:8080", "r", "refusing to send the token over http"),
    ],
)
def test_endpoint_refuses_bad_input(base: str, run: str, message: str) -> None:
    with pytest.raises(deliver.DeliveryError, match=message):
        deliver.endpoint(base, run)


def test_endpoint_builds_the_path() -> None:
    assert deliver.endpoint("https://c.example:8443/", "r_1") == (
        "https://c.example:8443/v1/runs/r_1/realism"
    )
    assert (
        deliver.endpoint("https://c.example/fps", "r") == "https://c.example/fps/v1/runs/r/realism"
    )
    assert deliver.endpoint("http://localhost:1", "r") == "http://localhost:1/v1/runs/r/realism"
    assert deliver.endpoint("http://[::1]:1", "r") == "http://[::1]:1/v1/runs/r/realism"
    assert deliver.endpoint("http://c.example", "r", insecure_http=True).startswith("http://c.")


# The command line.


def _capture(tmp_path: Path) -> Path:
    cap = pcapgen.Capture()
    cap.http_get(("10.0.0.10", 40000), ("10.0.1.80", 80))
    return cap.write_pcap(tmp_path / "one.pcap")


@pytest.fixture
def score_argv(
    tmp_path: Path, fake_reader: Callable[..., Path], labels_file: Callable[..., Path]
) -> list[str]:
    argv = ["score", "--pcap", str(_capture(tmp_path)), "--labels", str(labels_file())]
    return [*argv, "--parsers", "none", "--ndpi-reader", str(fake_reader(CSV_PASS))]


@pytest.fixture(autouse=True)
def clean_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in ("FPS_URL", "FPS_TOKEN", "FPS_TOKEN_FILE", "FPS_CA_FILE", "FPS_INSECURE_HTTP"):
        monkeypatch.delenv(name, raising=False)


def test_score_delivers_the_report_it_wrote(
    tmp_path: Path,
    score_argv: list[str],
    controller: tuple[str, Seen],
    capsys: pytest.CaptureFixture[str],
) -> None:
    base, seen = controller
    token_file = tmp_path / "token"
    token_file.write_text(TOKEN + "\n")
    token_file.chmod(0o600)
    out = tmp_path / "report.json"
    argv = [*score_argv, "--output", str(out), "--attach-run", "run1", "--url", base]
    assert main([*argv, "--token-file", str(token_file)]) == EXIT_PASS
    assert len(seen.requests) == 1
    request = seen.requests[0]
    assert request["path"] == "/v1/runs/run1/realism"
    assert request["authorization"] == f"Bearer {TOKEN}"
    assert request["body"] == out.read_bytes()
    err = capsys.readouterr().err
    assert "attached to run run1: the controller holds 2 results, 1 passed and 1 failed" in err
    assert TOKEN not in err


def test_score_reads_the_controller_and_token_from_the_environment(
    score_argv: list[str],
    controller: tuple[str, Seen],
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    base, seen = controller
    monkeypatch.setenv("FPS_URL", base)
    monkeypatch.setenv("FPS_TOKEN", TOKEN)
    assert main([*score_argv, "--attach-run", "run1"]) == EXIT_PASS
    assert seen.requests[0]["authorization"] == f"Bearer {TOKEN}"
    assert json.loads(capsys.readouterr().out)["passed"] is True


def test_score_still_delivers_a_failing_report_and_keeps_its_exit_code(
    tmp_path: Path,
    controller: tuple[str, Seen],
    fake_reader: Callable[..., Path],
    labels_file: Callable[..., Path],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    base, seen = controller
    monkeypatch.setenv("FPS_TOKEN", TOKEN)
    argv = ["score", "--pcap", str(_capture(tmp_path)), "--labels", str(labels_file())]
    argv += ["--parsers", "none", "--ndpi-reader", str(fake_reader())]
    argv += ["--attach-run", "run1", "--url", base, "--output", str(tmp_path / "r.json")]
    assert main(argv) == EXIT_FAIL
    assert json.loads(seen.requests[0]["body"])["passed"] is False  # type: ignore[arg-type]


@pytest.mark.parametrize(
    ("status", "title", "detail", "findings"),
    [
        (409, "Conflict", "run run1 is running", None),
        (
            422,
            "Unprocessable Entity",
            "the report breaks the schema",
            [{"severity": "error", "path": "/checks/0", "message": "unknown kind"}],
        ),
    ],
)
def test_score_exits_2_when_the_controller_refuses_but_keeps_the_report(
    tmp_path: Path,
    score_argv: list[str],
    controller: tuple[str, Seen],
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    status: int,
    title: str,
    detail: str,
    findings: list[dict[str, str]] | None,
) -> None:
    base, seen = controller
    seen.status = status
    seen.content_type = "application/problem+json"
    seen.body = _problem(status, title, detail, findings)
    monkeypatch.setenv("FPS_TOKEN", TOKEN)
    out = tmp_path / "report.json"
    argv = [*score_argv, "--output", str(out), "--attach-run", "run1", "--url", base]
    assert main(argv) == EXIT_ERROR
    assert json.loads(out.read_text())["passed"] is True
    err = capsys.readouterr().err
    assert (
        f"realism score failed: run run1: the controller refused the report: {status} {title}"
        in err
    )
    assert detail in err
    if findings is not None:
        assert "  /checks/0: unknown kind" in err
    assert TOKEN not in err


def test_score_checks_the_token_before_it_scores(
    score_argv: list[str], controller: tuple[str, Seen], capsys: pytest.CaptureFixture[str]
) -> None:
    base, seen = controller
    assert main([*score_argv, "--attach-run", "run1", "--url", base]) == EXIT_ERROR
    assert "no run token" in capsys.readouterr().err
    assert seen.requests == []


def test_score_has_no_token_option(
    score_argv: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    with pytest.raises(SystemExit) as info:
        main([*score_argv, "--attach-run", "run1", "--token", TOKEN])
    assert info.value.code == 2
    assert "unrecognized arguments" in capsys.readouterr().err


def test_score_refuses_delivery_options_without_attach_run(
    score_argv: list[str], capsys: pytest.CaptureFixture[str]
) -> None:
    assert main([*score_argv, "--url", "https://c.example"]) == EXIT_ERROR
    assert "--url needs --attach-run" in capsys.readouterr().err


def test_score_refuses_a_token_over_plain_http_to_another_host(
    score_argv: list[str], monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("FPS_TOKEN", TOKEN)
    argv = [*score_argv, "--attach-run", "run1", "--url", "http://controller.example:8080"]
    assert main(argv) == EXIT_ERROR
    err = capsys.readouterr().err
    assert "refusing to send the token over http" in err
    assert TOKEN not in err
