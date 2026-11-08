# fixingpoint

Python client for the FixingPointSystem controller API. It needs Python 3.10 or newer and has no dependencies, because it uses only `urllib` from the standard library.

```python
from fixingpoint import Client, RunFailedError

client = Client("https://controller.example:8443/v1", token="fps_...")

config = {"name": "smoke", "version": 1}      # a test configuration document, or a stored config id
run = client.start_run(config, labels={"team": "net"})
try:
    run = client.wait_for_run(run["id"], timeout=900, raise_on_failure=True)
except RunFailedError as err:
    raise SystemExit(f"run failed: {err.reason}")

report = client.get_report(run["id"])           # a dict
csv_text = client.get_report(run["id"], format="csv")
print(report["run"]["verdict"]["passed"])
```

`client.run_and_report(config)` does the three steps and returns `(run, report)`.

## What the client offers

- One method per operation of `contracts/openapi/fps-api-v1.yaml`, for example `get_capabilities()`, `list_runs(state="running")`, `stop_run(id, {"force": True})`, and `reserve_agent(id)`. Responses are plain dicts that the `fixingpoint.types` TypedDicts describe. The live metrics stream (Server Sent Events) is not generated yet. Use `get_run_metrics` for a window of samples.
- `start_run(config, agents=None, labels=None)` takes a stored configuration id or a configuration document, which it stores first.
- `wait_for_run(id, timeout=600, poll_interval=1, backoff=1.5, max_interval=15, raise_on_failure=False)` polls until the run is `completed`, `failed`, or `aborted`. The pause grows by `backoff` up to `max_interval` and never passes the deadline. It raises `WaitTimeoutError` (a `TimeoutError`) on timeout. A failed run returns with its `reason`, or raises `RunFailedError` when you ask for it.
- `get_report(id, format="json" | "csv" | "html")`.
- Errors derive from `FixingPointError`. `ApiError` carries `status`, `title`, `detail`, and the RFC 9457 `findings`, with the subclasses `AuthenticationError` (401), `ForbiddenError` (403, the token lacks the scope), and `NotFoundError` (404). `TransportError` means that no response arrived.

## Bearer tokens and TLS

Pass `token=` and the client sends `Authorization: Bearer <token>` on every request. The client refuses redirects, so the token never goes to another host, and its `repr` hides the token. Use an `https` URL outside tests, because `http` sends the token in clear text. Pass `ssl_context=` to trust a private CA.

## Keeping the client current

`fixingpoint/_generated.py` comes from the OpenAPI document. Do not edit it.

```bash
pip install -r sdk/python/requirements-dev.txt      # PyYAML is for the generator only
python3 sdk/python/scripts/generate_client.py        # regenerate after a contract change
python3 sdk/python/scripts/generate_client.py --check
```

`tests/test_currency.py` runs the check, so the test suite fails when the committed client drifts from the contract.

## Tests and linters

```bash
cd sdk/python
python3 -m pytest -q
python3 -m ruff format --check . && python3 -m ruff check . && python3 -m mypy --config-file pyproject.toml fixingpoint scripts tests
```

The tests start a standard library stub server (`tests/stub_server.py`) that routes every request through the OpenAPI document and answers with the documented `examples` of the response schema. They never reach the network beyond loopback.
