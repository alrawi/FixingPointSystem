# SPDX-License-Identifier: GPL-3.0-only
"""A stub controller that answers every operation of the OpenAPI document with an example.

The server reads contracts/openapi/fps-api-v1.yaml, routes each request to its
operation, and answers with the first success response. The contract carries
`examples` only on a few properties, so the stub builds the body from the response
schema and takes every documented `examples` value it finds. Tests script a
different answer for one operation with `StubServer.script`.
"""

from __future__ import annotations

import json
import re
import threading
from collections.abc import Callable
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, unquote, urlsplit

import yaml

SPEC_PATH = Path(__file__).resolve().parents[3] / "contracts" / "openapi" / "fps-api-v1.yaml"
SCOPES = ("read", "run", "admin")


def load_spec(path: Path = SPEC_PATH) -> dict[str, Any]:
    spec = yaml.safe_load(path.read_text(encoding="utf-8"))
    assert isinstance(spec, dict)
    return spec


def deref(spec: dict[str, Any], node: dict[str, Any]) -> dict[str, Any]:
    while "$ref" in node:
        ref = node["$ref"]
        if not ref.startswith("#/"):
            return {}  # a schema in another file, which the stub answers with an empty object
        target: Any = spec
        for part in ref[2:].split("/"):
            target = target[part]
        node = target
    return node


def example_for(spec: dict[str, Any], schema: dict[str, Any]) -> Any:
    """Build a value for a schema, preferring the examples that the contract documents."""
    schema = deref(spec, schema)
    if schema.get("examples"):
        return schema["examples"][0]
    if "example" in schema:
        return schema["example"]
    if "enum" in schema:
        return schema["enum"][0]
    kind = schema.get("type")
    if kind == "object" or "properties" in schema:
        return {k: example_for(spec, v) for k, v in schema.get("properties", {}).items()}
    if kind == "array":
        return [example_for(spec, schema.get("items", {}))]
    if kind == "integer":
        return schema.get("minimum", 1)
    if kind == "number":
        return 1.5
    if kind == "boolean":
        return False
    if kind == "string":
        return {"date-time": "2026-01-01T00:00:00Z", "uri-reference": "about:blank"}.get(
            schema.get("format", ""), "example"
        )
    return {}


@dataclass
class Reply:
    status: int
    body: Any = None
    content_type: str = "application/json"


@dataclass
class Operation:
    operation_id: str
    method: str
    pattern: re.Pattern[str]
    scope: str
    op: dict[str, Any]


@dataclass
class Recorded:
    method: str
    path: str
    query: dict[str, list[str]]
    headers: dict[str, str]
    body: bytes
    operation_id: str


@dataclass
class StubServer:
    """Serve the OpenAPI examples on 127.0.0.1. Use it as a context manager."""

    tokens: dict[str, str] | None = None  # token -> scope. None turns authentication off.
    spec: dict[str, Any] = field(default_factory=load_spec)

    def __post_init__(self) -> None:
        self.requests: list[Recorded] = []
        self._scripts: dict[str, list[Callable[[Recorded], Reply] | Reply]] = {}
        self._operations = self._index()
        self._base = urlsplit(self.spec["servers"][0]["url"]).path.rstrip("/")  # /v1
        stub = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: Any) -> None:
                pass

            def do_GET(self) -> None:
                stub._handle(self)

            do_POST = do_DELETE = do_PUT = do_GET

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._thread = threading.Thread(
            target=self._server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True
        )

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self._server.server_address[1]}{self._base}"

    def __enter__(self) -> StubServer:
        self._thread.start()
        return self

    def __exit__(self, *exc: object) -> None:
        self._server.shutdown()
        self._server.server_close()

    def script(self, operation_id: str, *replies: Callable[[Recorded], Reply] | Reply) -> None:
        """Answer an operation with these replies in order. The last one repeats."""
        self._scripts[operation_id] = list(replies)

    def calls(self, operation_id: str) -> list[Recorded]:
        return [r for r in self.requests if r.operation_id == operation_id]

    # Routing -------------------------------------------------------------

    def _index(self) -> list[Operation]:
        ops: list[Operation] = []
        for path, item in self.spec["paths"].items():
            regex = re.sub(r"\\\{[^}]+\\\}", "([^/:]+)", re.escape(path))
            for method in ("get", "post", "delete", "put"):
                if method in item:
                    op = item[method]
                    ops.append(
                        Operation(
                            op["operationId"],
                            method.upper(),
                            re.compile(f"^{regex}$"),
                            op.get("x-required-scope", "read"),
                            op,
                        )
                    )
        return ops

    def _problem(self, status: int, title: str, detail: str = "") -> Reply:
        body = {"type": "about:blank", "title": title, "status": status, "detail": detail}
        return Reply(status, body, "application/problem+json")

    def _example(self, operation: Operation, query: dict[str, list[str]], path_id: str) -> Reply:
        responses = operation.op["responses"]
        code = next(c for c in responses if c.startswith("2"))
        content = deref(self.spec, responses[code]).get("content", {})
        if not content:
            return Reply(int(code))
        media = "application/json"
        if media not in content:
            media = next(iter(content))
        wanted = {"csv": "text/csv", "html": "text/html"}.get(query.get("format", [""])[0])
        if wanted in content:
            media = str(wanted)
        schema = content[media].get("schema", {})
        value = example_for(self.spec, schema)
        if isinstance(value, dict) and path_id and "id" in value:
            value["id"] = path_id
        if operation.operation_id == "getRunReport" and media == "application/json":
            value["run"]["id"] = path_id
        if media == "text/csv":
            value = "property,observed,target,passed\n"
        return Reply(int(code), value, media)

    def _handle(self, h: BaseHTTPRequestHandler) -> None:
        parts = urlsplit(h.path)
        length = int(h.headers.get("Content-Length") or 0)
        raw = h.rfile.read(length) if length else b""
        path = unquote(parts.path)
        query = parse_qs(parts.query)
        found = None
        for operation in self._operations:
            m = operation.pattern.match(path.removeprefix(self._base))
            if m and operation.method == h.command:
                found = (operation, m.groups())
                break
        record = Recorded(
            h.command, path, query, {k: v for k, v in h.headers.items()}, raw,
            found[0].operation_id if found else "",
        )  # fmt: skip
        self.requests.append(record)
        if found is None:
            reply = self._problem(404, "Not Found", f"no operation for {h.command} {path}")
        else:
            operation, groups = found
            reply = self._authorize(operation, record) or self._answer(
                operation, record, groups[0] if groups else ""
            )
        payload = b""
        if reply.body is not None:
            payload = (
                json.dumps(reply.body).encode()
                if reply.content_type.endswith("json")
                else str(reply.body).encode()
            )
        h.send_response(reply.status)
        if payload:
            h.send_header("Content-Type", reply.content_type)
        h.send_header("Content-Length", str(len(payload)))
        h.end_headers()
        h.wfile.write(payload)

    def _authorize(self, operation: Operation, record: Recorded) -> Reply | None:
        if self.tokens is None:
            return None
        header = record.headers.get("Authorization", "")
        scheme, _, token = header.partition(" ")
        if scheme != "Bearer" or token not in self.tokens:
            return self._problem(401, "Unauthorized", "missing or invalid bearer token")
        if SCOPES.index(self.tokens[token]) < SCOPES.index(operation.scope):
            return self._problem(403, "Forbidden", f"the operation needs scope {operation.scope}")
        return None

    def _answer(self, operation: Operation, record: Recorded, path_id: str) -> Reply:
        queue = self._scripts.get(operation.operation_id)
        if queue:
            item = queue.pop(0) if len(queue) > 1 else queue[0]
            return item(record) if callable(item) else item
        return self._example(operation, record.query, path_id)
