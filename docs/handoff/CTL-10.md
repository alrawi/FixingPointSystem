# Handoff CTL-10 Capabilities endpoint

| Field | Value |
|---|---|
| Issue | #48 |
| Branch | `wp/CTL-10` |
| Pull request | #107 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-ctl-10-49f5, lease until 2026-11-03T14:26:38Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We serve `GET /v1/capabilities` with the engine API version, the backends, the agent inventory, and the installed modules. The handler reads two narrow interfaces that `internal/api` defines, so the package imports neither `internal/registry` nor `internal/agents`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Add the capabilities type with its two interfaces and the contract test with two fake agents and two modules
- [x] 3. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Each step was verified with the command after it.

- Step 1. The claim commit is on `wp/CTL-10`, and pull request #107 carries `Closes #48`.
- Step 2. `make lint-ctl test-ctl` passes, and golangci-lint reports 0 issues. `go test -cover` reports 100% for `NewCapabilitiesAPI` and `Capabilities` and 84.6% for `GetCapabilities`, whose only uncovered branch is a JSON encoding failure.
- The acceptance test is `TestContractCapabilities` in `controller/internal/api/capabilities_test.go`. It serves two fake agents and two modules and validates the response against the OpenAPI document with `contract.Load` and openapi3filter. Further tests cover empty and nil sources, source errors as 500 problems, the fallback to `Unimplemented`, and the version constants against the contract files.
- Step 3. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass. The Controller page has a Capabilities endpoint section, and the log entry is `docs/wiki/log/2026-11-03T1302-ingest-ctl-10-capabilities-endpoint.md`.

## Remaining

The review. Known gaps follow.

- The binary does not mount `CapabilitiesAPI` yet. The adapters over `Registry.List` and `Manager.List` belong to follow up #102, because `internal/api` cannot import `internal/agents` without a cycle.
- `EngineAPIVersion` is a constant. A later package may compare it with the `engine_api` that each agent reports.

## Next action

A reviewer agent reviews #107 against the Definition of Done.

## Blockers and open questions

None.
