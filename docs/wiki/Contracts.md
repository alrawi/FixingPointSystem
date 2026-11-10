# Contracts

The contracts in [`contracts/`](https://github.com/alrawi/FixingPointSystem/tree/main/contracts) define the interfaces between the workstreams. This page summarizes what each contract covers today and grows with each CON package. The [contracts README](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md) is the normative text, and this page never overrides it.

## Contract files

| File | Contract |
|---|---|
| `openapi/fps-api-v1.yaml` | Controller REST API, OpenAPI 0.7.0 |
| `proto/fps/agent/v1/agent.proto` | Agent control protocol 0.2.3, unreleased |
| `schemas/` | Module manifest, scenario, and test configuration schemas, scenario contract v0.6, test configuration v0.3 |
| `extension/fps-extension.wit` | Extension interface `fps:extension@0.1.0` |
| `metrics/catalog.yaml` | Metric catalog 0.3.0 |
| `examples/` | Example modules, tests, and plans that every validator run checks |

## Agent control protocol

Agents dial the controller, which the owner decided on 2026-11-02 and CON-06 applied. An agent behind NAT or in a cloud therefore needs no inbound port. The controller serves `ControllerService` over gRPC with mutual TLS.

1. The agent calls `Register` with its certificate identity, its capabilities, its state, and any loaded plan. The controller returns a session token and the heartbeat interval and timeout.
2. The agent opens the bidirectional `Session` stream and sends `open` with the token.
3. The controller sends `prepare`, `start`, and `stop` commands, and acknowledges each heartbeat. Version 0.2.0 adds the commands `update_rates`, `start_capture`, and `stop_capture`, which the controller sends only to an agent whose `engine_api` is 0.2.0 or later.
4. The agent answers every command with one `result`, sends a heartbeat per interval and after every state change, and sends counter snapshots during the run.

A failed command carries an `ErrorCode` whose numbers equal the gRPC status codes. When the agent hears nothing from the controller within the heartbeat timeout, it aborts the run. When it reconnects within the timeout, it registers again with its state and the run continues.

| Consumer | Package | Uses |
|---|---|---|
| Agent | AGT-02 | The client loop that registers and serves commands on the session |
| Controller | CTL-06 | The `ControllerService` server, enrollment, and heartbeat timeouts |
| Controller | CTL-07 | Commands to prepare, start, and stop runs |
| Standalone mode | AGT-01 | A `Plan` file in the canonical JSON mapping, with no service |

## Version 0.2.0 additions

CON-05 adds three commands and the signature fields of module references. Every change is additive, and `buf breaking` reports nothing against 0.1.0. The entry `contracts/changes/CON-05.md` states the compatibility in full.

| Addition | Rule | Read by |
|---|---|---|
| `UpdateRatesCommand` | Gives the new steady state target of the whole agent for each named mix entry. The agent splits it over its workers in proportion to their current rates, applies all updates or none, and accepts the command in the running state only | AGT-05, CTL-12 |
| `StartCaptureCommand` and `StopCaptureCommand` | A controller chosen `capture_id` names the capture. The start command takes an optional port, a pcap-filter expression, a snap length, and byte and time limits, and the stop command is idempotent | AGT-05 |
| `StopCaptureResult` | Packets, bytes, dropped packets, and a `sha256:` digest of the pcapng file. [CON-13](#capture-download-over-the-agent-connection) defines how the controller fetches the file | AGT-05 |
| `ModuleRef.signature` and `ModuleRef.signature_key_id` | An Ed25519 signature over the UTF-8 bytes of the digest, and the name of the registry key that made it. Both are empty for an unsigned module | CTL-11, AGT-04 |

CON-11 adds `ModuleRef.settings` (field 6) in version 0.2.1, described under [Module settings in the agent protocol](#module-settings-in-the-agent-protocol). The new commands use the existing error codes, so the error code list is unchanged.

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

The examples that use `where`, `modules`, and the new location forms, `port_locations.yaml`, `dns_zone.yaml`, and `filtered_criteria.yaml`, moved from `contracts/examples/tests-pending/` to `contracts/examples/tests/` with CON-11, so the validator default and the controller tests read them. CTL-18, CTL-20, and CTL-22 taught the compiler, the store validation, and the catalog adapter their fields.

```bash
python3 tools/contracts/validate.py
```

## Module settings in the agent protocol

CON-11 adds `ModuleRef.settings` (field 6, `google.protobuf.Struct`) and moves the protocol from 0.2.0 to 0.2.1. The field carries `modules.<name>.settings` of the test configuration, which the compiler has already validated against the settings schema of the module manifest. In a plan file it appears as `modules[].settings`, an object, and it is left out when the test sets nothing. It is not part of the module tree, so no digest changes.

The change takes a patch number because it adds one field and no command. The controller admits agents by the major and minor part of `engine_api`, so 0.3.0 would have locked out every agent until `DefaultEngineAPIs` changed. An agent of 0.2.0 skips the field and would run without the settings, so the controller sends a plan with module settings only to an agent that reports 0.2.1 or later. The Rust agent carries the settings through its plan parser and its conversions to and from the generated types. AGT-07 applies them. The entry `contracts/changes/CON-11.md` lists the compatibility checks.

## Capture download over the agent connection

CON-13 moves the protocol from 0.2.1 to 0.2.2 and lets the controller fetch a finished capture file, which stays on the agent after `StopCaptureResult`. The owner chose to stream it over the existing mutually authenticated connection. The agent dials the controller, so the transfer starts from the agent: the controller sends the command `fetch_capture` on `Session`, and the agent opens the new client streaming call `UploadCapture` on the same connection. The entry `contracts/changes/CON-13.md` records the decision and the compatibility checks.

| Message | Rule | Read by |
|---|---|---|
| `FetchCaptureCommand` | Names the plan, the finished capture, a `start_offset`, a single use `transfer_id` chosen by the controller, and an optional `max_chunk_bytes`. The agent answers at once with `FetchCaptureResult` (total size, digest, chunk size) and then uploads | CTL-14, AGT-09 |
| `UploadCaptureRequest` | A stream of one `open` (transfer id and session token), `CaptureChunk` messages with an `offset` and 1 to 262144 bytes in ascending order from `start_offset`, and one `end` with `total_bytes` and the `sha256:` digest of the whole file, which equals the digest of `StopCaptureResult` | CTL-14, AGT-09 |
| `UploadCaptureResponse` | `received_offset` and `verified`, sent once after `end` | CTL-14 |

The upload is a separate HTTP/2 stream, so its flow control does not hold up heartbeats on `Session`, and the contract defines no credit messages. A chunk is at most 256 KiB. To resume, the controller keeps the bytes it received and sends a new command with a new `transfer_id` and the offset it holds. The file is immutable, so the digest covers the bytes of every attempt together. The new RPC and command are additive, `buf breaking` reports nothing, and the version takes a patch number so that the controller, which admits agents by major and minor, keeps accepting every agent. The controller sends `fetch_capture` only to an agent that reports 0.2.2 or later. The Rust agent reports 0.2.3 (CON-15) and serves the download since AGT-09.

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

CON-07 applied decision D12 items 19 and 20 to the controller API and moved it to version 0.3.0. [CON-12](#side-selection-in-the-run-metrics-api) moved it on to 0.4.0.

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

## Behavior profiles and distributions

CON-10 applied decision D12 item 18 and moved the scenario contract to v0.3. The entry `contracts/changes/CON-10.md` lists every change and states the compatibility. The change is additive, and the only incompatibility is that module names `profiles` and `dist` are now reserved.

| Feature | Rule |
|---|---|
| `profiles` | Optional top level object of named header sets. `profiles.<name>` has type `text` and renders each entry as `Name: value` and CRLF. A value holds no CR or LF |
| `dist.pareto`, `dist.lognormal`, `dist.exponential`, `dist.uniform` | Draws from the seeded generator that return an `int`, rounded and limited to an optional `cap` that defaults to 2,147,483,647 |
| `http1.request` | New optional parameter `headers` of type `text` |

The specification example lives at `contracts/examples/modules/http1/scenarios-pending/web_profile.yaml`. The validator checks it, and the engine and the controller do not compile `profiles` or `dist` yet, so the manifest does not list it. `contracts/README.md` still shows the v0.2 grammar until the coordinator folds the text that the change entry carries.

## Side selection in the run metrics API

CON-12 adds the optional query parameter `side` to `GET /v1/runs/{id}/metrics` and to `GET /v1/runs/{id}/metrics:stream`, and moves the controller API from 0.3.0 to 0.4.0. The values are `client` and `server`, and the default is `client`, so a request without the parameter gets the answer that it got before. Any other value answers 400. On the stream, one `side` applies to every requested view.

The change takes a minor number because it adds a parameter that a reader can see in the document, as CON-07 did for its fields. The REST version has no admission rule like the `engine_api` rule that made CON-11 choose a patch number. A response does not echo the side, and no value selects both sides, because a total over both counts each session twice. The server side holds values only for the metrics that carry the `side` label of CON-08. The controller serves it from `Window.Side` and `Aggregator.LatestSide` of CTL-23. The entry `contracts/changes/CON-12.md` records the change, and `api.gen.go` is regenerated with `go generate ./internal/api`.

## Signature headers and calibration history

CON-14 documents two behaviors of the controller that the REST contract did not name, and moves the controller API from 0.4.0 to 0.5.0. The change is additive, and the entry `contracts/changes/CON-14.md` records it.

| Field or response | Rule |
|---|---|
| `installModule` header `X-Module-Signature-Key-Id` | Optional. The id of the trust store key that made the signature |
| `installModule` header `X-Module-Signature` | Optional. The 64 byte Ed25519 signature over the module tree digest, in standard base64. A request sends both headers or neither |
| `installModule` 400 | Only one header, a signature that is not base64, or a broken archive |
| `installModule` 403 | A refused signature, which includes a missing signature when the controller has a trust store and does not allow unsigned modules, or a token scope that is too low. The problem detail gives the reason |
| `Run.calibration` | Optional array of at most 256 `CalibrationStep` objects, oldest first, for a run with a `throughput_bps` objective. Absent while the factor stays at one |
| `CalibrationStep` | `time`, `factor`, `target`, and `sessions_per_second`, plus the optional `entries` (`id`, `bytes_per_session`, `measured`) and `multipliers` (byte mix multiplier by mix entry id) |

The Go struct of a step has no JSON tags, so `runs.ToAPI` produces the contract names. The controller keeps the newest 256 steps of a run. The generated Go code is regenerated with `go generate ./internal/api`, and the Python SDK and the UI schema follow the document.

## Capture and criteria gaps

CON-15 closes gaps that CTL-14, AGT-07, and AGT-09 found, and moves the controller API from 0.5.0 to 0.6.0 and the agent protocol from 0.2.2 to 0.2.3. Every change is additive, and the entry `contracts/changes/CON-15.md` records it.

| Change | Rule |
|---|---|
| `stopCapture`, `POST /runs/{id}/captures/{capture_id}:stop` | Ends the capture on every agent without a download and answers 200 with the `Capture`. The state is then `completed` (or `failed`), and `completed` means that the files are ready to download. The operation is idempotent. A download of a running capture still ends it |
| `Capture.files` | Optional array of `CaptureFile`, ordered by agent identity. Each entry has the required `agent` and the optional `started`, `error`, `packets`, `bytes`, `dropped`, and `digest` |
| `startCapture` `port` | Stays required. AGT-12 makes the agent filter by the port, and its end to end script starts a capture with a port, so no contract change is needed. Making it optional later would be additive |
| `StartCaptureCommand.export_tls_keys` (8) | When true, the agent puts the TLS key log lines into the file. An agent of 0.2.2 skips the field and decides on its own |
| `Criterion.labels` (4) | A label filter like `where` in the test configuration, for example `side: server`. Without a `side` label a criterion counts the client side |
| `examples/tests/dns_zone.yaml` | Uses the first party scenario `dns/lookup_a`. `tools/contracts/validate.py` validates it against `modules/dns` |

An agent of 0.2.2 skips `labels` and would count every series, so the controller sends a labeled criterion only to an agent that reports 0.2.3 or later. The Rust agent reports 0.2.3 and applies labeled criteria since AGT-14; before that it refused them, because a silent skip would measure another rule. The generated Go code is regenerated by the coordinator, and the Python SDK and the UI schema follow the document.

## Version, configuration listing, and breaking check

CON-16 moves the controller API from 0.6.0 to 0.7.0. The change is additive, and the entry `contracts/changes/CON-16.md` records it.

| Change | Rule |
|---|---|
| `Capabilities.controller_version` | Optional string with the version of the controller build. It is not the REST contract version, which `api_version` gives. The field is absent when the build carries no version |
| `listConfigs`, `GET /configs` | Scope `read`. Pages like `listRuns` with `page_size` and `page_token`, and answers `{items: ConfigRef[], next_page_token}`, newest first. `fps config list` can use it instead of the uploads of the local machine |
| `MixEntry.weight` comment | Says that the weight is the session share (0 to 1, the shares of a plan sum to 1). The compiler converts the byte weights of the bytes basis (CTL-34), and the agent uses the field as it is. This is a comment only: no field, number, or wire behavior changes, so the protocol stays at 0.2.3 |
| Breaking check | The workflow `.github/workflows/openapi-breaking.yml` runs oasdiff on `contracts/openapi/*.yaml` against the base of the pull request (or the previous commit of a push to `main`) and fails on a breaking change. `tools/contracts/test_openapi_breaking.py` proves with the fixture pair in `tools/contracts/fixtures/openapi-breaking/` that the check passes an additive change and fails a breaking one |

The proto files keep `buf breaking` with the `FILE` rules. The REST documents now have the same automatic check, so the additive rule holds for both. A deliberate breaking change needs a new major path (`/v2`) and the owner's approval, not a way around the check. The Python SDK and the UI schema follow the document, and the coordinator regenerates the Go code.

## The tls_upgrade action

CON-17 moves the scenario contract from v0.3 to v0.4. The change is additive, so every v0.3 scenario stays valid, and the entry `contracts/changes/CON-17.md` records it.

| Change | Rule |
|---|---|
| `tls_upgrade` | New flow action for TCP flows. It wraps the transport of the running session in TLS in place, as STARTTLS does, and every later action of the side runs over TLS. `profile` (required) names a TLS profile of the run, `role` is `client` or `server` and defaults to the role of the side, and `server_name` is verified and sent as SNI by a client handshake |
| UDP rule | `tools/contracts/validate.py` reports `tls_upgrade` in a flow with `l4: udp` as an error, at any depth of `loop` and `branch`. The validator does not check that the profile exists, because the profiles belong to the run |
| Embedded copy | The controller embeds the scenario schema, so the copy under `controller/internal/registry/schema/` follows the contract and the controller accepts the keyword |

The engine has parsed the action since ENG-20. `fps-flow` does not reject it in a UDP flow yet, and the runtime fails such a session with reason `internal`, so the engine needs the same rule as an ENG change. The agent builds no TLS profiles until AGT-14, so a session of a plan with the action fails until then. MOD-20 can add the STARTTLS variants of SMTP and IMAP once this change merges.

## The transport keyword

CON-18 moves the scenario contract from v0.4 to v0.5 and decides the flow keyword `transport`. The schema had `transport: {module, profile}` since v0.1, but `Scenario::load` answered `unsupported: a transport module` and the validator ignored it, so MOD-04 wrote `tls_upgrade` as the first action after the opener instead. We chose to keep the keyword and define it as sugar, because the engine cost is one op per side and removing it would leave the specification example without a contract. The entry `contracts/changes/CON-18.md` records the reasons and the compatibility.

| Change | Rule |
|---|---|
| `transport` | `{module: tls, profile, server_name?}` on a TCP flow. It is exactly a `tls_upgrade` right after the opener of each side that the flow declares: the client side upgrades as the client with `server_name`, and the server side upgrades as the server. `module` takes only `tls`, and `profile` is required and applies to both sides, so a flow whose sides need different profiles keeps writing `tls_upgrade` on each side |
| Composition | Explicit `tls_upgrade` actions run after the sugar in declaration order, and the `tls_upgrade` rules apply to it |
| UDP rule | `tools/contracts/validate.py` reports `transport` in a flow with `l4: udp` as an error at `/flows/<i>/transport`. `fps-flow` reports the same case as `TlsUpgradeOnUdp` |
| Loader | `Scenario::load` desugars the keyword into `Op::UpgradeTls` in front of each side's body, so the runtime and the agent see the same ops as for explicit actions. CON-18 makes this small `fps-flow` change itself, because the validator and the loader must agree. The shared fixtures `transport_tcp.yaml` and `transport_udp.yaml` pin both verdicts in the Rust test and in `test_fixtures.py` |
| Embedded copy | The controller embeds the scenario schema, so the copy under `controller/internal/registry/schema/` follows the contract byte for byte |

The tightened body breaks nothing, because no loader ever accepted the old shape. The agent builds no TLS profiles until AGT-14, so a session of a plan that uses the keyword fails until then, as it does for `tls_upgrade`.

## Gaps found by CAP-02 and RLM-06

CON-19 moves the scenario contract from v0.5 to v0.6, the test configuration from v0.2 to v0.3, and the metric catalog from 0.2.0 to 0.3.0. Every change is additive, and the entry `contracts/changes/CON-19.md` records the reasons, the compatibility, and the follow up work.

| Change | Rule |
|---|---|
| `int_bytes(value, width, endianness)` | Core function. An unsigned integer as exactly `width` bytes (1 to 8), `'big'` by default or `'little'`. A negative value or a value that does not fit is an error, never a truncation. The validator checks literal arguments, and a computed bad value fails the session with `template_error` |
| `now(format)` | Core function. `'http_date'` (text, IMF fixdate in UTC, 29 bytes), `'epoch_s'` and `'epoch_ms'` (int). It reads the clock at each call, so it is not a function of the run seed. Replays and goldens pin the clock, and the output length does not depend on the instant, so byte counts replay without pinning. The compiler never folds it. Neither function draws from the seeded generator |
| Variables across `after` | A flow sees the variables that flows before it (directly or through a chain) bound with `set`, as they stood when it started, and a spawned flow sees its parent up to the `spawn`. The two roles run on different agents and share no variables, so a value that both need is bound by each role from its own `last`. Flows that `after` does not order have no defined order. The engine already behaves this way, so the change states the rule |
| `profile.arrival`, `profile.size`, and the same fields in a mix entry | Optional declarations of the intended distributions, which the realism check follows. `arrival` takes `constant`, `exponential`, `uniform`, `lognormal`, and `pareto` with the parameters of the scenario `delay`, and `size` takes the same names in bytes for `request`, `response`, and `both`. They add no command for agents, so arrivals still follow the load objective and sizes follow the scenarios |
| `application: all` and `property: both` | The KS metrics `realism.ks_statistic` and `realism.ks_p_value` can total every application and pool both directions. The module name `all` is reserved, and the `LABEL_VALUES` mirror in `fps-stats` gains `both` |

The engine does not implement `int_bytes` and `now` yet, the controller compiler does not accept `arrival` and `size` yet, and the realism runner does not read them yet. Until an ENG, a CTL, and an RLM package land, a scenario that calls the functions fails to load in the engine, and a configuration that declares the distributions fails in the compiler. The follow up list is in the change entry.
