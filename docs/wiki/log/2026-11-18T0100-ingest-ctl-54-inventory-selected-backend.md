## [2026-11-18] ingest | CTL-54 The orchestrator inventory uses the selected backend

CTL-54 (issue #634) changes `wiring.Inventory.AvailableAgents` in `controller/internal/wiring/adapters.go`. The backend that the orchestrator compiles for an agent is now the backend that the agent selected (`selected_backend`). An agent that sends `BACKEND_UNSPECIFIED` gets the first known entry of `backends`, as before. Both are the kernel backend for every current agent, so nothing changes today. [Controller](Controller.md) has the table row.

Decisions.

- The inventory calls the exported `agents.ToAPI(ag).Backend` instead of a second copy of the rule, so the REST view and the compiler cannot drift apart. `agents.ToAPI` is read only here, and no change in `controller/internal/agents` was needed.
- `AgentCapabilities` keeps listing every backend of the latest registration, because it answers what an agent can run, not what it runs.

Findings.

- The fallback and the selection are covered by `TestInventoryUsesSelectedBackend`: a selected AF_XDP agent whose first backend is kernel, an old agent that falls back to its first known backend, an old agent whose first entry is `BACKEND_UNSPECIFIED`, a selected kernel agent, an agent with no backends, and an agent without capabilities.

### Shared page notes

- Implementation Plan: CTL-54 is complete once merged, which unblocks the CTL-54 dependency of TRN-31 and TRN-12 follow-ups.
- Testing: add `controller/internal/wiring/wiring_test.go` (`TestInventoryUsesSelectedBackend`) to the controller test list.
- Module Development: no change, and no "Watch out for" entry.
