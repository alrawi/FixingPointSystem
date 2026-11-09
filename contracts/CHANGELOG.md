# Contracts changelog

## Controller API 0.7.0 (2026-11-09)

Controller version, configuration listing, and a REST breaking check (CON-16). All changes are additive. See `changes/CON-16.md`.

- OpenAPI 0.7.0 adds the optional `Capabilities.controller_version` and `listConfigs` (`GET /configs`, paged like `listRuns`).
- The comment of `MixEntry.weight` in the agent protocol says session share. The protocol stays at 0.2.3, because only a comment changed.
- The workflow `openapi-breaking.yml` runs oasdiff on the OpenAPI documents and fails on a breaking change. `tools/contracts/test_openapi_breaking.py` checks it against a deliberately breaking fixture.

## Controller API 0.6.0 and agent protocol 0.2.3 (2026-11-09)

Capture and criteria gaps (CON-15). All changes are additive. See `changes/CON-15.md`.

- OpenAPI 0.6.0 adds `stopCapture` (`POST /runs/{id}/captures/{capture_id}:stop`), which ends a capture without downloading it, and the optional `Capture.files` (schema `CaptureFile`), which names the agent that produced each capture file. `port` of `startCapture` stays required.
- Agent protocol 0.2.3 adds `StartCaptureCommand.export_tls_keys` (8) and `Criterion.labels` (4), a label filter such as `side: server`.
- The example test `dns_zone.yaml` uses the first party scenario `dns/lookup_a`.

## 0.2.0 (2026-11-02)

Scenario contract v0.2 (CON-04).

- The template grammar adds module function calls with keyword arguments, such as `dns.query(name=vars.host, qtype='A')`, nested calls as arguments, float and boolean literals, and core functions with keyword arguments. Every v0.1 template keeps its meaning.
- The reference `last` holds the bytes that the previous `expect` of the same side consumed. A template may use it only after an `expect` that always runs earlier on that side.
- Datagram rules for `expect` on UDP. One `expect` reads exactly one datagram, the stop rule checks that datagram, and the new field `max_datagram` (1 to 65,507 bytes, default 65,507) bounds its size. A longer datagram fails the session.
- The module manifest adds `functions`, the template function catalog with typed parameters and defaults. It must equal the catalog that the extension returns, and the validator checks every module call against it without running extension code.
- Template values and parameters carry the types of the extension `value-type` enum, and the validator checks argument types.
- The example modules declare `engine_api: ">=0.2, <0.3"` and their function catalogs. `dns` adds the scenario `resolve`, which uses `dns.query`, `dns.answer_ip(last)`, and `dns.response(last, ...)`, and `http1` adds the scenario `web_page`, which builds its messages with `http1.request` and `http1.response`. The v0.1 examples stay unchanged, because spikes S1 and S4 and the example plan read them. The example plan uses engine API 0.2.0 and the new module digests.
- The reserved action `raw` is unchanged.

## Extension interface 0.1.0 (2026-11-02)

- First draft of the WIT package `fps:extension@0.1.0` in `extension/fps-extension.wit`. The world `extension` exports the hooks `on-open`, `on-data`, `on-timer`, `on-close`, and `render`, and a catalog of template functions with typed parameters. It imports the engine calls `send`, `set-timer`, `cancel-timer`, `log`, `random-u64`, and `random-bytes`.
- `extension/README.md` defines the session lifecycle, the engine calls allowed in each hook, and the mapping from WIT types to the Rust traits that native extensions implement (decision record 0010, proposed).

## 0.1.1 (2026-11-02)

- Added the metric catalog `metrics/catalog.yaml` (catalog 0.1.0) and its schema `metrics/catalog.schema.json`. The catalog names 48 metrics across the application, scenario, flow action, transport, and port and host levels (CON-02).
- OpenAPI 0.1.1 names the catalog metric of every `RunSummary` property through the `x-fps-metric` keyword and points criteria and metric sample values at the catalog. No field was added, removed, or renamed.
- The validator rejects metric names in test criteria and in `x-fps-metric` annotations that the catalog does not define, and checks the catalog itself.

## Unreleased

- Agent control protocol 0.1.0 in `proto/fps/agent/v1/agent.proto` with `AgentService` (Register, Heartbeat, Prepare, Start, Stop, StreamCounters), the `Plan` message, and `Capabilities` (CON-01).
- buf module configuration in `proto/buf.yaml` with the STANDARD and COMMENTS lint categories and FILE breaking rules.
- Example agent plan `http_dns_smoke` in the canonical JSON mapping, and the plan checker `tools/contracts/check_plans.py`.

## 0.1.0 (2026-11-02)

- First draft of the controller OpenAPI document, the module manifest schema, the scenario schema, and the test configuration schema.
- Example modules `http1` and `dns`, and example tests `enterprise_mix_10g` and `http_smoke`.
