# Handoff CTL-53 The controller reports kernel release, selected backend, and XDP mode

| Field | Value |
|---|---|
| Issue | #597 |
| Branch | `wp/CTL-53` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 66% (2 of 3 plan steps) |
| Current holder | claude-c53a7e, lease until 2026-11-17T17:22:10Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`controller/internal/agents/rest.go` fills `Agent.kernel_release`, `Agent.backend` from `selected_backend`, and `Agent.ports[].xdp_driver_mode` from the capabilities of CON-32 and AGT-22. An old agent (empty or unspecified values) falls back to the first entry of `backends`, and its kernel release and XDP mode stay absent.

## Plan

- [x] 1. Fill the three fields in `ToAPI` with the fallback.
- [x] 2. Add `TestRESTAgentCapabilitiesReport` (new agent, AF_XDP with per-port modes, old agent, no backend), validated against the OpenAPI contract.
- [ ] 3. Push, open the PR, wait for CI, then move to review.

## Done and verified

- Steps 1 and 2: `gofmt -l controller/internal/agents` prints nothing. Go cannot build here (no module proxy), so CI is the build and test loop.

## Remaining

CI result for the Go build and tests.

## Next action

Push, open the PR, and poll the check runs of the head commit every 3 minutes or more.

## Blockers and open questions

none

## Decisions taken in this package

- `selected_backend` wins when it maps to a known REST backend. Otherwise the first known entry of `backends` is used, as before. The controller cannot tell an old agent from a new one by engine_api, so the fallback keys on the value.
- Absent means a missing JSON key: empty `kernel_release` and `XDP_MODE_UNSPECIFIED` produce no property.
- The XDP mode is copied as a hint. Nothing selects or refuses on it.
- `wiring.Inventory` (outside these paths) still reads the first known backend for the orchestrator. Reported in the log entry, not changed here.

## Pitfalls

The sandbox cannot download Go modules, so nothing compiled locally.

## Verify the current state

```bash
gofmt -l controller/internal/agents   # expect no output
# CI: go test ./internal/agents/ -run TestREST
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17 | claude-c53a7e | claim | none | claimed #597 |
