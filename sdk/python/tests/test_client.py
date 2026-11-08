# SPDX-License-Identifier: GPL-3.0-only
"""The SDK against a stub controller that answers with the OpenAPI examples."""

from __future__ import annotations

import json
from collections.abc import Iterator
from typing import Any

import pytest

from fixingpoint import (
    ApiError,
    AuthenticationError,
    Client,
    ForbiddenError,
    NotFoundError,
    RunFailedError,
    TransportError,
    WaitTimeoutError,
)

from .stub_server import Recorded, Reply, StubServer, load_spec

TOKENS = {"reader": "read", "runner": "run", "root": "admin"}
CONFIG = {"name": "smoke", "version": 1}
REASON = "criterion sessions.failed_ratio < 0.01 failed with 0.042"


@pytest.fixture
def stub() -> Iterator[StubServer]:
    with StubServer(tokens=TOKENS) as server:
        yield server


@pytest.fixture
def client(stub: StubServer) -> Client:
    return Client(stub.url, token="runner", timeout=5)


def run_in(state: str, **extra: Any) -> Reply:
    return Reply(200, {"id": "r1", "config_id": "c1", "state": state, **extra})


class Clock:
    """A fake clock whose sleep advances time, so wait tests take no real time."""

    def __init__(self) -> None:
        self.now = 0.0
        self.sleeps: list[float] = []

    def clock(self) -> float:
        return self.now

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.now += seconds


def test_every_generated_operation_answers_with_its_example(stub: StubServer) -> None:
    admin = Client(stub.url, token="root", timeout=5)
    assert admin.get_capabilities()["engine_api"] == "0.1.0"
    assert admin.list_modules(page_size=5)["items"][0]["name"] == "example"
    assert admin.install_module(b"archive")["license"] == "example"
    admin.delete_module("http1", "1.0.0")
    assert admin.create_config(CONFIG)["id"] == "example"
    assert admin.validate_config("c1")["valid"] is False
    assert admin.list_runs(state="running")["items"][0]["state"] == "pending"
    assert admin.stop_run("r1", {"force": True})["id"] == "r1"
    assert admin.get_run_metrics("r1", "tcp", step_s=2)["view"] == "application"
    assert admin.list_captures("r1")[0]["port"] == "example"
    assert admin.start_capture("r1", {"port": "p0"})["state"] == "running"
    assert isinstance(admin.list_agents()[0]["id"], str)
    assert admin.register_agent({"id": "a1"})["state"] == "online"
    assert admin.reserve_agent("a1")["id"] == "a1"
    assert admin.release_agent("a1")["id"] == "a1"
    # The query and body reach the server as the contract describes.
    assert stub.calls("listModules")[0].query == {"page_size": ["5"]}
    assert json.loads(stub.calls("stopRun")[0].body) == {"force": True}
    assert stub.calls("installModule")[0].headers["Content-Type"] == "application/octet-stream"
    assert stub.calls("getRunMetrics")[0].query == {"view": ["tcp"], "step_s": ["2"]}


def test_every_operation_of_the_contract_is_covered_by_the_client() -> None:
    spec = load_spec()
    ids = {
        op["operationId"]
        for item in spec["paths"].values()
        for method, op in item.items()
        if method in ("get", "post", "put", "delete")
    }
    streaming = {"streamRunMetrics"}
    names = {"".join("_" + c.lower() if c.isupper() else c for c in i) for i in ids - streaming}
    assert names <= set(dir(Client))


def test_start_run_from_a_stored_config_id(stub: StubServer, client: Client) -> None:
    run = client.start_run("c1", agents=["a1"], labels={"team": "net"})
    assert run["state"] == "pending"
    assert stub.calls("createConfig") == []
    sent = json.loads(stub.calls("startRun")[0].body)
    assert sent == {"config_id": "c1", "agents": ["a1"], "labels": {"team": "net"}}


def test_start_run_stores_a_config_document_first(stub: StubServer, client: Client) -> None:
    stub.script("createConfig", Reply(201, {"id": "stored-7", "name": "smoke"}))
    client.start_run(CONFIG)
    assert json.loads(stub.calls("createConfig")[0].body) == CONFIG
    assert json.loads(stub.calls("startRun")[0].body) == {"config_id": "stored-7"}


def test_wait_polls_with_backoff_until_the_run_completes(stub: StubServer, client: Client) -> None:
    stub.script(
        "getRun", run_in("pending"), run_in("running"), run_in("running"), run_in("completed")
    )
    t = Clock()
    run = client.wait_for_run(
        "r1", timeout=60, poll_interval=1, backoff=2, max_interval=3, sleep=t.sleep, clock=t.clock
    )
    assert run["state"] == "completed"
    assert t.sleeps == [1, 2, 3]  # doubles, then stops at max_interval
    assert len(stub.calls("getRun")) == 4


def test_wait_returns_a_failed_run_with_its_reason(stub: StubServer, client: Client) -> None:
    stub.script("getRun", run_in("running"), run_in("failed", reason=REASON, degraded=True))
    t = Clock()
    run = client.wait_for_run("r1", sleep=t.sleep, clock=t.clock)
    assert run["state"] == "failed"
    assert run["reason"] == REASON


def test_wait_can_raise_on_a_failed_run(stub: StubServer, client: Client) -> None:
    stub.script("getRun", run_in("aborted", reason="stop requested by operator"))
    t = Clock()
    with pytest.raises(RunFailedError) as caught:
        client.wait_for_run("r1", raise_on_failure=True, sleep=t.sleep, clock=t.clock)
    assert caught.value.reason == "stop requested by operator"
    assert caught.value.run["state"] == "aborted"
    assert "stop requested by operator" in str(caught.value)


def test_wait_times_out_without_sleeping_past_the_deadline(
    stub: StubServer, client: Client
) -> None:
    stub.script("getRun", run_in("running"))
    t = Clock()
    with pytest.raises(WaitTimeoutError) as caught:
        client.wait_for_run(
            "r1", timeout=10, poll_interval=4, backoff=2, sleep=t.sleep, clock=t.clock
        )
    assert caught.value.run["state"] == "running"
    assert isinstance(caught.value, TimeoutError)
    assert t.now == 10  # 4, then 6 of the 8 that backoff asked for
    assert t.sleeps == [4, 6]


def test_wait_rejects_bad_arguments(client: Client) -> None:
    with pytest.raises(ValueError, match="out of range"):
        client.wait_for_run("r1", poll_interval=0)


def test_report_json_and_csv(stub: StubServer, client: Client) -> None:
    report = client.get_report("r1")
    assert isinstance(report, dict)
    assert report["run"]["id"] == "r1"
    assert report["realism"][0]["passed"] is False
    assert stub.calls("getRunReport")[0].query == {"format": ["json"]}
    csv = client.get_report("r1", format="csv")
    assert csv.startswith("property,observed")  # type: ignore[union-attr]
    assert stub.calls("getRunReport")[1].query == {"format": ["csv"]}
    with pytest.raises(ValueError):
        client.get_report("r1", format="pdf")  # type: ignore[arg-type]


def test_run_and_report_end_to_end(stub: StubServer, client: Client) -> None:
    stub.script("getRun", run_in("running"), run_in("completed"))
    t = Clock()
    run, report = client.run_and_report("c1", sleep=t.sleep, clock=t.clock)
    assert run["state"] == "completed"
    assert report["run"]["id"] == "r1"


def test_run_and_report_raises_for_a_failed_run_and_fetches_no_report(
    stub: StubServer, client: Client
) -> None:
    stub.script("getRun", run_in("failed", reason=REASON))
    t = Clock()
    with pytest.raises(RunFailedError, match="failed_ratio"):
        client.run_and_report("c1", sleep=t.sleep, clock=t.clock)
    assert stub.calls("getRunReport") == []


def test_missing_token_is_an_authentication_error(stub: StubServer) -> None:
    anonymous = Client(stub.url)
    with pytest.raises(AuthenticationError) as caught:
        anonymous.list_runs()
    assert caught.value.status == 401
    assert caught.value.title == "Unauthorized"
    assert "Authorization" not in stub.requests[0].headers


def test_wrong_token_is_an_authentication_error(stub: StubServer) -> None:
    with pytest.raises(AuthenticationError):
        Client(stub.url, token="nope").get_capabilities()


def test_token_is_sent_as_a_bearer_header(stub: StubServer, client: Client) -> None:
    client.list_runs()
    assert stub.requests[0].headers["Authorization"] == "Bearer runner"


def test_insufficient_scope_is_a_forbidden_error(stub: StubServer) -> None:
    reader = Client(stub.url, token="reader")
    assert reader.get_capabilities()
    with pytest.raises(ForbiddenError) as caught:
        reader.start_run("c1")
    assert caught.value.status == 403
    assert "scope run" in caught.value.detail


def test_problem_details_and_not_found(stub: StubServer, client: Client) -> None:
    stub.script(
        "getRun",
        Reply(
            404,
            {"title": "Not Found", "status": 404, "detail": "no run r9"},
            "application/problem+json",
        ),
    )
    with pytest.raises(NotFoundError) as caught:
        client.get_run("r9")
    assert caught.value.detail == "no run r9"
    stub.script(
        "createConfig",
        Reply(
            422,
            {
                "title": "Invalid",
                "status": 422,
                "errors": [{"severity": "error", "path": "/ports", "message": "required"}],
            },
            "application/problem+json",
        ),
    )
    with pytest.raises(ApiError) as bad:
        client.create_config(CONFIG)
    assert type(bad.value) is ApiError
    assert bad.value.findings[0]["path"] == "/ports"


def test_error_without_problem_details_keeps_the_status(stub: StubServer, client: Client) -> None:
    stub.script("getRun", Reply(502, "bad gateway", "text/plain"))
    with pytest.raises(ApiError) as caught:
        client.get_run("r1")
    assert caught.value.status == 502
    assert caught.value.detail == "bad gateway"


def test_path_values_are_quoted(stub: StubServer, client: Client) -> None:
    with pytest.raises(NotFoundError):  # "/" never matches {id}, so the stub says 404
        client.get_run("a/b c")
    assert stub.requests[0].path == "/v1/runs/a/b c"  # the server decodes %2F and %20
    client.get_run("a b")
    assert stub.requests[1].operation_id == "getRun"


def test_connection_failure_is_a_transport_error() -> None:
    dead = Client("http://127.0.0.1:1/v1", timeout=1)
    with pytest.raises(TransportError):
        dead.get_capabilities()


def test_redirects_are_not_followed(stub: StubServer, client: Client) -> None:
    stub.script("getRun", Reply(302, None))
    with pytest.raises(ApiError) as caught:
        client.get_run("r1")
    assert caught.value.status == 302


def test_base_url_must_be_http(stub: StubServer) -> None:
    with pytest.raises(ValueError):
        Client("file:///etc/passwd")
    assert "runner" not in repr(Client(stub.url, token="runner"))


def test_scripted_callable_sees_the_request(stub: StubServer, client: Client) -> None:
    seen: list[Recorded] = []

    def reply(request: Recorded) -> Reply:
        seen.append(request)
        return run_in("running")

    stub.script("getRun", reply)
    assert client.get_run("r1")["state"] == "running"
    assert seen[0].path == "/v1/runs/r1"
