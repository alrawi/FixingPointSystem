# Contracts

The contracts in [`contracts/`](https://github.com/alrawi/FixingPointSystem/tree/main/contracts) define the interfaces between the workstreams. This page summarizes what each contract covers today and grows with each CON package. The [contracts README](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md) is the normative text, and this page never overrides it.

## Contract files

| File | Contract |
|---|---|
| `openapi/fps-api-v1.yaml` | Controller REST API, OpenAPI 0.2.0 |
| `proto/fps/agent/v1/agent.proto` | Agent control protocol 0.1.0, unreleased |
| `schemas/` | Module manifest, scenario, and test configuration schemas, scenario contract v0.2 |
| `extension/fps-extension.wit` | Extension interface `fps:extension@0.1.0` |
| `metrics/catalog.yaml` | Metric catalog 0.1.0 |
| `examples/` | Example modules, tests, and plans that every validator run checks |

## Agent control protocol

Agents dial the controller, which the owner decided on 2026-11-02 and CON-06 applied. An agent behind NAT or in a cloud therefore needs no inbound port. The controller serves `ControllerService` over gRPC with mutual TLS.

1. The agent calls `Register` with its certificate identity, its capabilities, its state, and any loaded plan. The controller returns a session token and the heartbeat interval and timeout.
2. The agent opens the bidirectional `Session` stream and sends `open` with the token.
3. The controller sends `prepare`, `start`, and `stop` commands, and acknowledges each heartbeat.
4. The agent answers every command with one `result`, sends a heartbeat per interval and after every state change, and sends counter snapshots during the run.

A failed command carries an `ErrorCode` whose numbers equal the gRPC status codes. When the agent hears nothing from the controller within the heartbeat timeout, it aborts the run. When it reconnects within the timeout, it registers again with its state and the run continues.

| Consumer | Package | Uses |
|---|---|---|
| Agent | AGT-02 | The client loop that registers and serves commands on the session |
| Controller | CTL-06 | The `ControllerService` server, enrollment, and heartbeat timeouts |
| Controller | CTL-07 | Commands to prepare, start, and stop runs |
| Standalone mode | AGT-01 | A `Plan` file in the canonical JSON mapping, with no service |

## Checks

```bash
python3 tools/contracts/validate.py
python3 tools/contracts/check_plans.py
python3 -m pytest -q tools/contracts
buf format --diff --exit-code contracts/proto
buf lint contracts/proto
```

`tools/contracts/test_plans.py` also checks the shape of the protocol. It confirms that `ControllerService` is the only service, that `Session` streams both ways, that the session envelopes carry the expected messages, and that `ErrorCode` matches the gRPC codes.

## Changing a contract

A CON package changes a contract and adds one entry file under `contracts/changes/`. The coordinator folds the entries into `contracts/CHANGELOG.md` when a version ships.

## Test configuration and manifest additions

CON-09 applied decision D12 items 7, 9, 10, 14, and 22 to the test configuration schema v0.2 and the module manifest schema v0.3. The entry `contracts/changes/CON-09.md` lists every change.

| Field | Contract | Rule | Read by |
|---|---|---|---|
| `timeline.seed` | Test configuration | At least 1, and the controller chooses a random nonzero seed when the field is absent | CTL-18 |
| `network.ports[].location` | Test configuration | `<agent>/<interface>` pins the port to one agent, and a bare `<interface>` or `*/<interface>` spreads it across agents | CTL-18 |
| `criteria[].where` | Test configuration | Label names and values that select the series a criterion checks | CTL-20 |
| `modules.<name>.settings` | Test configuration | Settings that hold for every scenario of the module | CTL-18, MOD-19 |
| `settings` | Module manifest | JSON Schema with an object root that describes the module settings | CTL-18, MOD-19 |
| `expected_bytes` | Module manifest | Estimated application payload bytes per session of each scenario with default parameters | CTL-21 |

The examples that use `where`, `modules`, and the new location forms live under `contracts/examples/tests-pending/`, which neither the validator default nor the controller tests read. We check them with the command below. CTL-18 and CTL-20 taught the compiler these fields, but the store validation and the catalog adapter (CTL-22) and the `ModuleRef.settings` field of the agent protocol (CON-11) are still missing, so CON-11 moves them into `contracts/examples/tests/`. Until then a test with module settings compiles but fails at dispatch.

```bash
python3 tools/contracts/validate.py --tests contracts/examples/tests/*.yaml contracts/examples/tests-pending/*.yaml
```

## Metric catalog views, reasons, and side label

CON-08 applies decision D12 items 13, 15, 17, and 21 to the metric catalog, which moves to version 0.2.0.

| Change | Rule |
|---|---|
| `view` on every metric | One value of the OpenAPI `MetricView` enum, which the schema checks against the level |
| `histogram_statistics` | The fixed suffix list `p50`, `p90`, `p99`, `p999`, `min`, `max`, `mean`, and `count` |
| New reasons | `datagram_rejected`, `template_error`, and `extension_error` |
| `side` label | `client` or `server` on every scenario level metric and every transport metric |

A total over both sides counts each session twice, so the API views and test criteria report the client side unless a criterion selects the server side. The controller embeds a byte copy of the catalog in `controller/internal/metrics/catalog.yaml`, which must follow every catalog change.

## Failure reasons and the metrics endpoint

CON-07 applied decision D12 items 19 and 20 to the controller API, which is now at version 0.3.0.

| Field or document | Meaning |
|---|---|
| `Run.reason` | Why a run failed or was aborted, absent in every other state |
| `RunVerdict` | Whether the run passed, and one `CriterionResult` per criterion in configuration order |
| `CriterionResult.observed` | The value of the metric over the whole run, absent when the metric has no value |
| `CriterionResult.passed` | The outcome of the comparison, absent when the metric has no value and false when the criterion cannot be evaluated |
| `CriterionResult.reason` | Why the criterion passed, failed, or has no result |
| `MetricSample.values` | Metric values in double precision |
| `openapi/fps-metrics.yaml` | `GET /metrics` on the controller root, open by default, with the optional bearer scheme `metricsToken` |

Every enum of the API names `type: string`. The metrics endpoint has its own document, because every operation of `fps-api-v1.yaml` lies under `/v1` and names a token scope. CTL-20 fills the reasons, and CTL-19 adds the metrics token setting.
