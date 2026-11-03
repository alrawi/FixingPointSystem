# Handoff CTL-02 REST layer from the OpenAPI contract

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #40 |
| Branch | `wp/CTL-02` |
| Pull request | #81 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-ctl-02-fb93, lease until 2026-11-03T04:37:17Z |
| Last verified commit | c1515f4 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Generate the server interface from contracts/openapi/fps-api-v1.yaml and add RFC 9457 problem details, read, run, and admin token scopes, and 501 stubs. A contract test calls every operation and validates each response against the OpenAPI document.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Pin oapi-codegen, generate types and the server interface, add the freshness check
- [x] 2. Problem responses and the problem helper
- [x] 3. Token scopes middleware
- [x] 4. Stub server that returns 501 problems and the public constructor
- [x] 5. Contract test with kin-openapi over every operation
- [x] 6. Docs, log entry, make targets, CI

## Done and verified

- Steps 1 to 5. `cd controller && go test -race ./...` passes, including `TestContractEveryOperation` (98 subtests over the 22 operations), `TestContractParameterErrors`, `TestUnroutedRequests`, and `TestGeneratedCodeIsCurrent`.
- Step 6. `make lint test` passes with golangci-lint 2.5.0 locally, with 0 issues.

## Remaining

- The binary does not mount the API, because the configuration has no token setting. The log entry names the one line follow up for the CTL-01 paths.
- Bodies that point at `contracts/schemas/*.json` are open objects in Go and in the contract test. CTL-03 validates configurations against their JSON Schema.

## Next action

A reviewer checks the pull request against the Definition of Done.

## Blockers and open questions

none

## Decisions taken in this package

- oapi-codegen v2.6.0, kin-openapi v0.133.0, and oapi-codegen/runtime v1.3.1, because later releases need Go 1.25 and `controller/go.mod` says Go 1.24.
- We call the oapi-codegen library from `internal/api/gen` instead of a `tool` directive, so the freshness test regenerates in memory without a subprocess. The generated header names the pinned version, and a test checks it against go.mod.
- `contract.Load` rewrites external `$ref`s to open objects and types untyped string enums, because kin-openapi reads the 3.0 dialect and the external files use JSON Schema 2020-12.
- The std-http server of oapi-codegen registers `{id}:validate` patterns that `net/http` rejects, so we do not call its `HandlerWithOptions`. `Server.Mount` registers `{id}` and dispatches on the colon suffix. A colon always marks an action.
- Scopes nest (admin over run over read). Each token has one scope. `StaticTokens` stores SHA-256 digests.
- The scope check runs before parameter binding. Problems use type about:blank with the status text as title.
- `Unimplemented` is hand written with a compile time assertion against `ServerInterface`, so a new contract operation fails the build until someone adds its stub.

## Pitfalls

- `go get` of the newest oapi-codegen silently raises the `go` line of go.mod to 1.25. Use `GOTOOLCHAIN=local` to see the failure.
- kin-openapi rejects the 3.1 fields `identifier`, `summary`, `examples`, and `contentMediaType` unless `contract.Validate` allows them.

## Verify the current state

```bash
(cd controller && GOWORK=off go test -race ./internal/api/...)   # ok for api, contract, gen
(cd controller/internal/api && GOWORK=off go generate . && git diff --exit-code)   # no diff
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:43Z | claude-ctl-02-fb93 | claim | | Session start |
| 2026-11-03T03:00Z | claude-ctl-02-fb93 | checkpoint | c1515f4 | Generated server, scopes, problems, stubs, contract test |
| 2026-11-03T03:10Z | claude-ctl-02-fb93 | finish | | Docs, log entry, ready for review |
