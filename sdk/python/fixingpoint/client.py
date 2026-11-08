# SPDX-License-Identifier: GPL-3.0-only
"""HTTP layer and helpers of the FixingPointSystem Python SDK.

The layer uses only the standard library. `_generated.GeneratedOperations` supplies
one method per API operation, and `Client` adds the transport, the bearer token,
error mapping, and the run helpers.
"""

from __future__ import annotations

import json
import ssl
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Mapping, Sequence
from typing import Any, Literal
from urllib.parse import quote, urlsplit

from ._generated import GeneratedOperations, Run, RunReport

TERMINAL_STATES = frozenset({"completed", "failed", "aborted"})
FAILED_STATES = frozenset({"failed", "aborted"})


class FixingPointError(Exception):
    """Base class of every error that the SDK raises."""


class TransportError(FixingPointError):
    """The request never produced an HTTP response, for example a refused connection."""


class ApiError(FixingPointError):
    """The controller answered with an error status.

    `problem` holds the RFC 9457 problem details when the controller sent them.
    """

    def __init__(self, status: int, problem: Mapping[str, Any] | None, body: str = "") -> None:
        self.status = status
        self.problem: dict[str, Any] = dict(problem or {})
        self.title: str = str(self.problem.get("title") or "")
        self.detail: str = str(self.problem.get("detail") or body.strip()[:200])
        self.findings: list[dict[str, Any]] = list(self.problem.get("errors") or [])
        text = f"HTTP {status}"
        if self.title:
            text += f" {self.title}"
        if self.detail:
            text += f": {self.detail}"
        super().__init__(text)


class AuthenticationError(ApiError):
    """HTTP 401. The token is missing, invalid, or expired."""


class ForbiddenError(ApiError):
    """HTTP 403. The token lacks the scope that the operation needs."""


class NotFoundError(ApiError):
    """HTTP 404."""


class RunFailedError(FixingPointError):
    """A run ended in the state failed or aborted. `reason` says why."""

    def __init__(self, run: Run) -> None:
        self.run = run
        self.reason: str = run.get("reason", "")
        text = f"run {run['id']} {run['state']}"
        super().__init__(f"{text}: {self.reason}" if self.reason else text)


class WaitTimeoutError(FixingPointError, TimeoutError):
    """The run did not reach a final state in time. `run` is the last state seen."""

    def __init__(self, run: Run, timeout: float) -> None:
        self.run = run
        super().__init__(f"run {run['id']} is still {run['state']} after {timeout:g} seconds")


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Refuse redirects, so the bearer token never travels to another host."""

    def redirect_request(self, *args: Any, **kwargs: Any) -> None:
        return None


def _problem(raw: bytes, content_type: str) -> dict[str, Any] | None:
    if "json" not in content_type:
        return None
    try:
        data = json.loads(raw)
    except ValueError:
        return None
    return data if isinstance(data, dict) else None


class Client(GeneratedOperations):
    """Client of the controller REST API.

    `base_url` includes the version path, for example `https://host:8443/v1`.
    `token` is a scoped API token that the client sends as a bearer token.
    `ssl_context` customizes TLS, for example to trust a private CA.
    """

    def __init__(
        self,
        base_url: str,
        token: str | None = None,
        *,
        timeout: float = 30.0,
        ssl_context: ssl.SSLContext | None = None,
    ) -> None:
        parts = urlsplit(base_url)
        if parts.scheme not in ("http", "https") or not parts.netloc:
            raise ValueError("base_url must be an http or https URL")
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        self._token = token
        handlers: list[Any] = [_NoRedirect()]
        if ssl_context is not None:
            handlers.append(urllib.request.HTTPSHandler(context=ssl_context))
        self._opener = urllib.request.build_opener(*handlers)

    def __repr__(self) -> str:
        return f"Client({self.base_url!r}, token={'set' if self._token else 'none'})"

    # Transport -----------------------------------------------------------

    def _call(
        self,
        method: str,
        path: str,
        *,
        path_params: dict[str, str],
        query: dict[str, Any],
        body: Any,
        body_type: str | None,
        expected: tuple[int, ...],
    ) -> Any:
        url = self.base_url + path.format(**{k: quote(v, safe="") for k, v in path_params.items()})
        pairs = [(k, _query_value(v)) for k, v in query.items() if v is not None]
        if pairs:
            url += "?" + urllib.parse.urlencode(pairs)
        headers = {"Accept": "application/json, text/*;q=0.9, */*;q=0.5"}
        data: bytes | None = None
        if body is not None:
            data = json.dumps(body).encode() if body_type == "application/json" else bytes(body)
            headers["Content-Type"] = body_type or "application/octet-stream"
        if self._token:
            headers["Authorization"] = f"Bearer {self._token}"
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with self._opener.open(request, timeout=self.timeout) as response:
                status = response.status
                content_type = response.headers.get("Content-Type", "")
                raw = response.read()
        except urllib.error.HTTPError as err:
            raw = err.read()
            raise _error_for(err.code, raw, err.headers.get("Content-Type", "")) from None
        except (urllib.error.URLError, OSError) as err:
            raise TransportError(f"{method} {path}: {err}") from err
        if status not in expected:
            raise _error_for(status, raw, content_type)
        return _decode(raw, content_type)

    # Run helpers ---------------------------------------------------------

    def start_run(
        self,
        config: Mapping[str, Any] | str,
        *,
        agents: Sequence[str] | None = None,
        labels: Mapping[str, str] | None = None,
    ) -> Run:
        """Start a run.

        `config` is a stored configuration id, or a configuration document that the
        client stores first. The call returns at once with the accepted run.
        """
        config_id = config if isinstance(config, str) else self.create_config(dict(config))["id"]
        request: dict[str, Any] = {"config_id": config_id}
        if agents is not None:
            request["agents"] = list(agents)
        if labels is not None:
            request["labels"] = dict(labels)
        return super().start_run(request)  # type: ignore[arg-type]

    def wait_for_run(
        self,
        run_id: str,
        *,
        timeout: float = 600.0,
        poll_interval: float = 1.0,
        max_interval: float = 15.0,
        backoff: float = 1.5,
        raise_on_failure: bool = False,
        sleep: Callable[[float], None] = time.sleep,
        clock: Callable[[], float] = time.monotonic,
    ) -> Run:
        """Poll a run until it completes, fails, or is aborted.

        The wait between polls starts at `poll_interval` and grows by `backoff`
        up to `max_interval`. It never sleeps past the deadline. The call raises
        `WaitTimeoutError` when `timeout` seconds pass first. A failed or aborted
        run returns normally, with its `reason`, unless `raise_on_failure` is true,
        and then the call raises `RunFailedError`.
        """
        if timeout < 0 or poll_interval <= 0 or backoff < 1:
            raise ValueError("timeout, poll_interval, and backoff are out of range")
        deadline = clock() + timeout
        interval = poll_interval
        while True:
            run = self.get_run(run_id)
            if run["state"] in TERMINAL_STATES:
                if raise_on_failure and run["state"] in FAILED_STATES:
                    raise RunFailedError(run)
                return run
            remaining = deadline - clock()
            if remaining <= 0:
                raise WaitTimeoutError(run, timeout)
            sleep(min(interval, remaining))
            interval = min(interval * backoff, max_interval)

    def get_report(
        self, run_id: str, format: Literal["json", "csv", "html"] = "json"
    ) -> RunReport | str:
        """Fetch the report of a run, as a dict for json and as text for csv or html."""
        if format not in ("json", "csv", "html"):
            raise ValueError("format must be json, csv, or html")
        return self.get_run_report(run_id, format=format)

    def run_and_report(
        self,
        config: Mapping[str, Any] | str,
        *,
        timeout: float = 600.0,
        agents: Sequence[str] | None = None,
        labels: Mapping[str, str] | None = None,
        **wait: Any,
    ) -> tuple[Run, RunReport]:
        """Start a run, wait for it, and return the final run with its JSON report.

        A failed or aborted run raises `RunFailedError`, so the report of a good
        run is the only one that this call returns.
        """
        run = self.start_run(config, agents=agents, labels=labels)
        run = self.wait_for_run(run["id"], timeout=timeout, raise_on_failure=True, **wait)
        report = self.get_report(run["id"])
        assert isinstance(report, dict)
        return run, report


def _query_value(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def _decode(raw: bytes, content_type: str) -> Any:
    if not raw:
        return None
    media = content_type.split(";")[0].strip().lower()
    if media == "application/json" or media.endswith("+json"):
        return json.loads(raw)
    if media.startswith("text/"):
        return raw.decode("utf-8", errors="replace")
    return raw


def _error_for(status: int, raw: bytes, content_type: str) -> ApiError:
    text = raw.decode("utf-8", errors="replace")
    cls = {401: AuthenticationError, 403: ForbiddenError, 404: NotFoundError}.get(status, ApiError)
    return cls(status, _problem(raw, content_type), text)
