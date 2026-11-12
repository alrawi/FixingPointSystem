# SPDX-License-Identifier: GPL-3.0-only
"""Deliver a realism report to the controller of a run.

``python3 -m realism score --attach-run ID`` sends the report that it wrote with
``PUT /v1/runs/ID/realism`` (``attachRealismReport``), so that ``realism`` of the run report
holds its results. This module reads the run token, makes the request with the standard
library, and turns the answer of the controller into one message.

The token comes from a file or the environment and never from the command line, which a
process list shows. The order is ``--token-file``, then ``FPS_TOKEN_FILE``, then ``FPS_TOKEN``,
as for the ``fps`` command line client. No message shows the token, and the client replaces it
with ``[redacted]`` in any text that a server sends back. It never follows a redirect, so the
token reaches the configured host only, and it refuses to send the token over plain ``http`` to
a host that is not loopback unless told to.
"""

from __future__ import annotations

import http.client
import ipaddress
import json
import re
import socket
import ssl
import stat
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

DEFAULT_URL = "https://localhost:8443"
DEFAULT_TIMEOUT_S = 30.0
RUN_ID = re.compile(r"^[A-Za-z0-9_-]{1,64}$")
# The answer of a controller is a small document, so a client reads no more than this.
MAX_ANSWER_BYTES = 1 << 20
REDACTED = "[redacted]"


class DeliveryError(Exception):
    """The report could not be delivered, or the controller refused it."""


@dataclass(frozen=True)
class Target:
    """Where and how a report goes: the endpoint, the token, and the TLS settings."""

    url: str
    token: str
    ca_file: Path | None = None
    timeout_s: float = DEFAULT_TIMEOUT_S


@dataclass(frozen=True)
class Attachment:
    """The 200 answer of the controller."""

    results: int
    passed: int
    replaced: bool

    def summary(self, run: str) -> str:
        failed = self.results - self.passed
        held = "replaced the earlier report" if self.replaced else "first report"
        return (
            f"attached to run {run}: the controller holds {self.results} results, "
            f"{self.passed} passed and {failed} failed ({held})"
        )


def _is_loopback(host: str) -> bool:
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def read_token(token_file: Path | None, env: Mapping[str, str]) -> tuple[str, str | None]:
    """Return the token and a warning, from the file, ``FPS_TOKEN_FILE``, or ``FPS_TOKEN``."""
    path = token_file
    if path is None and env.get("FPS_TOKEN_FILE"):
        path = Path(env["FPS_TOKEN_FILE"])
    if path is not None:
        try:
            mode = stat.S_IMODE(path.stat().st_mode)
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as exc:
            reason = exc.strerror if isinstance(exc, OSError) and exc.strerror else "not text"
            raise DeliveryError(f"read token file {path}: {reason}") from exc
        warning = None
        if mode & 0o077:
            warning = f"token file {path} is readable by other users (mode {mode:04o})"
        return _clean(text, f"token file {path}"), warning
    if env.get("FPS_TOKEN"):
        return _clean(env["FPS_TOKEN"], "FPS_TOKEN"), None
    raise DeliveryError(
        "no run token: set --token-file, FPS_TOKEN_FILE, or FPS_TOKEN (there is no --token "
        "option, because a command line shows in the process list)"
    )


def _clean(text: str, source: str) -> str:
    token = text.strip()
    if not token:
        raise DeliveryError(f"{source} holds no token")
    if any(ch.isspace() or ord(ch) < 0x20 or ord(ch) > 0x7E for ch in token):
        raise DeliveryError(f"{source} holds a token with characters that a header cannot carry")
    return token


def endpoint(base: str, run: str, *, insecure_http: bool = False) -> str:
    """Return the URL of ``attachRealismReport`` for a run, or raise on a bad base or run."""
    if not RUN_ID.fullmatch(run):
        raise DeliveryError(
            f"run identifier {run!r} is not 1 to 64 letters, digits, underscores, or hyphens"
        )
    parts = urllib.parse.urlsplit(base)
    if parts.scheme not in ("http", "https") or not parts.hostname:
        raise DeliveryError(f"controller URL {base!r} is not an http or https URL")
    if parts.username or parts.password or parts.query or parts.fragment:
        raise DeliveryError("controller URL must not hold credentials, a query, or a fragment")
    if parts.scheme == "http" and not _is_loopback(parts.hostname) and not insecure_http:
        raise DeliveryError(
            f"refusing to send the token over http to {parts.hostname}; use https, or "
            "--insecure-http to allow it"
        )
    prefix = parts.path.rstrip("/")
    return urllib.parse.urlunsplit(
        (parts.scheme, parts.netloc, f"{prefix}/v1/runs/{run}/realism", "", "")
    )


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Refuse every redirect, so that the token reaches the configured host only."""

    def redirect_request(self, *_args: object, **_kwargs: object) -> None:
        return None


def _opener(target: Target) -> urllib.request.OpenerDirector:
    handlers: list[urllib.request.BaseHandler] = [_NoRedirect(), urllib.request.ProxyHandler({})]
    if target.url.startswith("https:"):
        context = ssl.create_default_context(
            cafile=str(target.ca_file) if target.ca_file is not None else None
        )
        handlers.append(urllib.request.HTTPSHandler(context=context))
    return urllib.request.build_opener(*handlers)


def _redact(text: str, token: str) -> str:
    return text.replace(token, REDACTED)


def _problem(status: int, body: bytes, token: str) -> str:
    """Describe an error answer from its RFC 9457 problem details."""
    try:
        doc = json.loads(body)
    except ValueError:
        doc = None
    if not isinstance(doc, dict):
        text = body[:200].decode("utf-8", "replace").strip()
        return f"HTTP {status}" + (f": {_redact(text, token)}" if text else "")
    title = _redact(str(doc.get("title") or f"HTTP {status}"), token)
    lines = [f"{status} {title}"]
    detail = doc.get("detail")
    if isinstance(detail, str) and detail:
        lines[0] += f": {_redact(detail, token)}"
    errors = doc.get("errors")
    if isinstance(errors, list):
        for finding in errors:
            if isinstance(finding, dict):
                path = _redact(str(finding.get("path", "")), token)
                message = _redact(str(finding.get("message", "")), token)
                lines.append(f"  {path or '/'}: {message}")
    return "\n".join(lines)


def attach(target: Target, body: bytes) -> Attachment:
    """PUT the report and return the controller's answer, or raise ``DeliveryError``."""
    request = urllib.request.Request(
        target.url,
        data=body,
        method="PUT",
        headers={
            "Authorization": f"Bearer {target.token}",
            "Content-Type": "application/json",
            "Accept": "application/json, application/problem+json",
        },
    )
    try:
        with _opener(target).open(request, timeout=target.timeout_s) as response:
            status = response.status
            answer = response.read(MAX_ANSWER_BYTES)
    except urllib.error.HTTPError as exc:
        answer = exc.read(MAX_ANSWER_BYTES)
        exc.close()
        reason = _problem(exc.code, answer, target.token)
        if 300 <= exc.code < 400:
            reason = f"HTTP {exc.code}: the controller redirected, which the client never follows"
        raise DeliveryError(f"the controller refused the report: {reason}") from None
    except (urllib.error.URLError, http.client.HTTPException, ssl.SSLError, OSError) as exc:
        cause: object = exc.reason if isinstance(exc, urllib.error.URLError) else exc
        if isinstance(cause, socket.timeout | TimeoutError):
            text = f"no answer within {target.timeout_s:g} seconds"
        else:
            text = _redact(str(cause), target.token)
        raise DeliveryError(f"could not reach the controller: {text}") from None
    if status != 200:
        raise DeliveryError(f"the controller answered {status}, not 200")
    try:
        doc = json.loads(answer)
        results = doc["realism"]
        replaced = doc["replaced"]
        passed = sum(1 for item in results if item["passed"] is True)
    except (ValueError, KeyError, TypeError) as exc:
        raise DeliveryError("the controller answered 200 without an attachment") from exc
    if not isinstance(results, list) or not isinstance(replaced, bool):
        raise DeliveryError("the controller answered 200 without an attachment")
    return Attachment(results=len(results), passed=passed, replaced=replaced)
