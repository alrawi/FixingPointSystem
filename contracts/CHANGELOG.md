# Contracts changelog

## Unreleased

- Agent control protocol 0.1.0 in `proto/fps/agent/v1/agent.proto` with `AgentService` (Register, Heartbeat, Prepare, Start, Stop, StreamCounters), the `Plan` message, and `Capabilities` (CON-01).
- buf module configuration in `proto/buf.yaml` with the STANDARD and COMMENTS lint categories and FILE breaking rules.
- Example agent plan `http_dns_smoke` in the canonical JSON mapping, and the plan checker `tools/contracts/check_plans.py`.

## 0.1.0 (2026-11-02)

- First draft of the controller OpenAPI document, the module manifest schema, the scenario schema, and the test configuration schema.
- Example modules `http1` and `dns`, and example tests `enterprise_mix_10g` and `http_smoke`.
