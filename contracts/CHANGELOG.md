# Contracts changelog

## Extension interface 0.1.0 (2026-11-02)

- First draft of the WIT package `fps:extension@0.1.0` in `extension/fps-extension.wit`. The world `extension` exports the hooks `on-open`, `on-data`, `on-timer`, `on-close`, and `render`, and a catalog of template functions with typed parameters. It imports the engine calls `send`, `set-timer`, `cancel-timer`, `log`, `random-u64`, and `random-bytes`.
- `extension/README.md` defines the session lifecycle, the engine calls allowed in each hook, and the mapping from WIT types to the Rust traits that native extensions implement (decision record 0010, proposed).
## Unreleased

- Agent control protocol 0.1.0 in `proto/fps/agent/v1/agent.proto` with `AgentService` (Register, Heartbeat, Prepare, Start, Stop, StreamCounters), the `Plan` message, and `Capabilities` (CON-01).
- buf module configuration in `proto/buf.yaml` with the STANDARD and COMMENTS lint categories and FILE breaking rules.
- Example agent plan `http_dns_smoke` in the canonical JSON mapping, and the plan checker `tools/contracts/check_plans.py`.

## 0.1.0 (2026-11-02)

- First draft of the controller OpenAPI document, the module manifest schema, the scenario schema, and the test configuration schema.
- Example modules `http1` and `dns`, and example tests `enterprise_mix_10g` and `http_smoke`.
