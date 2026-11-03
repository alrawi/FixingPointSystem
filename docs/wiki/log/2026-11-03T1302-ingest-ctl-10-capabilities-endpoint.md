## [2026-11-03] ingest | CTL-10 Capabilities endpoint

We added `controller/internal/api/capabilities.go` and its test, and a Capabilities endpoint section at the end of the [Controller](Controller.md) page. The type `CapabilitiesAPI` serves `GET /v1/capabilities` with the engine API version 0.2.0, the REST contract version, the union of the agent backends, the agents, and the modules. It reads the two interfaces `ModuleLister` and `AgentInventory`, which the same file defines, so `internal/api` imports neither the registry nor the agent manager. `TestContractCapabilities` serves two fake agents and two modules and validates the response against the OpenAPI document with the existing contract loader. No dependency changed. The evidence is `make lint-ctl test-ctl` and `docs/handoff/CTL-10.md` on the branch `wp/CTL-10`.

## Shared page notes

The coordinator may apply three notes.

1. Follow up #102 can name the two adapters that the binary needs, which are a `ModuleLister` over `Registry.List` and an `AgentInventory` over `Manager.List` that keeps every backend of the latest registration.
2. [Testing](Testing.md) can name the capabilities contract test in the `test-ctl` row next to the REST layer contract test.
3. [Home](Home.md) can mark CTL-10 as ready for review.
