# SDK

The Python SDK in [`sdk/python/`](https://github.com/alrawi/FixingPointSystem/tree/main/sdk/python) is a client for the controller REST API in `contracts/openapi/fps-api-v1.yaml`. The package is `fixingpoint`. It needs Python 3.10 or newer and no runtime dependency, because its HTTP layer uses `urllib`. CLI-01 delivered it. The Go command line client of CLI-02 is a separate package.

## Layout

| Path | Purpose |
|---|---|
| `fixingpoint/_generated.py` | TypedDicts for the schemas and one method per operation. The generator writes it, and nobody edits it |
| `fixingpoint/client.py` | The `Client` class with the transport, the bearer token, the error classes, and the run helpers |
| `scripts/generate_client.py` | The generator, about 300 lines, which needs PyYAML and runs `--check` for the currency test |
| `tests/stub_server.py` | A standard library stub controller that answers each operation with the examples of the OpenAPI document |
| `requirements-dev.txt` | PyYAML, pytest, ruff, mypy, and the stubs, for development only |

## Generated client and currency check

The generator reads the OpenAPI document and writes plain `TypedDict` classes (required keys apart from optional ones), `Literal` types for enums, and one method per operation. Server Sent Events operations are skipped, which today means `streamRunMetrics`. A schema in another file, such as the test configuration, is `dict[str, Any]`, because the SDK leaves validation to the controller. The file header records the API version and the SHA-256 of the OpenAPI document.

`python3 sdk/python/scripts/generate_client.py --check` exits with 1 when the committed client differs from what the generator makes of the current document. `tests/test_currency.py` runs the same check, so a contract change fails the SDK tests until someone regenerates the client. The tests also prove that the check fails for a new operation, a changed schema, and a hand edited client.

## Helpers

| Call | Behavior |
|---|---|
| `start_run(config, agents=None, labels=None)` | Takes a stored config id or a configuration document, stores a document first, and returns the accepted run |
| `wait_for_run(id, timeout, poll_interval, backoff, max_interval, raise_on_failure)` | Polls with exponential backoff until `completed`, `failed`, or `aborted`. It never sleeps past the deadline and raises `WaitTimeoutError` on timeout. A failed run returns with its `reason`, or raises `RunFailedError` |
| `get_report(id, format)` | The JSON report as a dict, or the CSV or HTML text |
| `run_and_report(config, timeout=...)` | The three calls above, returning `(run, report)` and raising for a failed run |

Errors follow the RFC 9457 problem details of the controller. `AuthenticationError` is 401, `ForbiddenError` is 403 because the token lacks the scope in `x-required-scope`, `NotFoundError` is 404, and `ApiError` covers the rest.

## Bearer tokens

`Client(url, token=...)` sends `Authorization: Bearer <token>`. The client refuses HTTP redirects, so the token never reaches another host, and it hides the token in `repr`. Plain `http` URLs work for loopback tests, and the README tells users to use `https` elsewhere.

## Tests

`python3 -m pytest -q` in `sdk/python/` runs 30 tests in about two seconds. The stub server routes each request through the OpenAPI paths, checks the bearer token and its scope when a test gives it a token table, and answers with the first success response. The contract documents `examples` only on a few properties, so the stub builds each body from the response schema and uses every documented example it finds, such as the failure reason of a run. A test scripts other answers, for example a run that goes from `running` to `failed` with a reason. The tests cover start, wait with backoff, timeout, and failure, reports in JSON and CSV, the authentication and scope errors, problem details, redirects, and the currency check.
