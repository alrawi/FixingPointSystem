# Handoff CTL-17 Controller binary wiring

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #111 |
| Branch | `wp/CTL-17` |
| Pull request | #116 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-ctl-17-e566, lease until 2026-11-03T16:12:30Z |
| Last verified commit | e7a2c01 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We serve the REST API and the agent gRPC service from the controller binary, with the configuration settings they need. The binary opens the store and the registry, starts the agent manager and the run orchestrator, and mounts one combined API server. An end to end test starts the binary, registers a fake agent over gRPC, stores a config, installs and lists the example modules, and starts a run over REST.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Add the settings for tokens, data_dir, store_path, and grpc_listen to internal/app
- [x] 3. Add internal/wiring with the adapters and the combined API server, with unit tests
- [x] 4. Wire cmd/fps-controller and add the end to end test of the binary
- [x] 5. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Each step was verified with the command after it.

- Step 2. `go test -race ./internal/app/` passes, with tests for the new keys, the token rules, `ResolvedStorePath`, `RegistryDir`, and `LoadTokens`.
- Step 3. `go test -race -cover ./internal/wiring/` passes with 89% coverage. `TestServerRoutesEveryOperation` walks every operation of the contract and checks that exactly the seven pending operations answer 501.
- Step 4. `go test -race -count=3 -run TestEndToEnd ./cmd/fps-controller/` passes. The test builds the binary, registers the fake agent `local` over gRPC, installs and lists the example modules, stores and validates `http_smoke.yaml`, starts and stops a run over REST, and checks persistence after a restart.
- Step 5. `make lint-ctl test-ctl` passes with 0 golangci-lint issues. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass. The Controller page has a Binary wiring section, and the log entry is `docs/wiki/log/2026-11-03T1439-ingest-ctl-17-controller-binary-wiring.md`.

## Remaining

The review. Known gaps follow.

- The metrics package of CTL-08 is not wired. `serve` in `cmd/fps-controller/main.go` marks the place, and `Controller.SetMetrics` and `Controller.Observe` are the hooks.
- The gRPC service speaks plain TCP and trusts the claimed agent id until CTL-15 adds mutual TLS.
- Earlier sections of the Controller page are stale, and the log entry lists them under Shared page notes.
- Runs still live in memory, so a restart forgets them, as CTL-07 records.

## Next action

A reviewer agent reviews #116 against the Definition of Done.

## Blockers and open questions

None.

## Decisions taken in this package

- Without `data_dir` the store lives in memory and the registry in a temporary directory, so the existing settings and tests keep working without creating files in the working directory.
- The combined server embeds the package handlers under aliases, because three handler types share the name `API`, and an embedded `pending` type holds the operations that no package implements, so the compiler flags an operation that two handlers serve.
- The catalog lists only the latest version of each module, because configurations name modules without a version and two versions would make every reference ambiguous.
- Tokens need at least 16 bytes and may come from a file, so secrets need not sit in the configuration file.
- The agent service uses `agents.ClaimedIdentity` until CTL-15, and the controller logs a warning about it at every start.
- Invalid token settings, including an unreadable token file, exit with code 2 like other configuration errors.

## Pitfalls

- `GET /v1/agents` answers a bare JSON array, while `GET /v1/modules` answers a page with `items`.
- gosec G101 flags string constants whose names contain token, so the end to end test keeps its tokens in a map by scope.
- The local golangci-lint here is v2.5.0, and CI runs v2.12.2, which flagged two more gosec findings (G101 on a token_file path in a test, G703 on a test file write). We fixed G703 by installing from an in-memory archive and suppressed the G101 line with its reason, the only suppression of the package.

## Verify the current state

```bash
# From the repository root, with GOWORK=off; every command passes
make lint-ctl test-ctl
cd controller && go test -race -count=1 -run 'TestEndToEnd|TestServerRoutesEveryOperation' ./cmd/fps-controller/ ./internal/wiring/
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T14:22Z | claude-ctl-17-e566 | claim | none | Claimed #111 with card 6606279050 |
| 2026-11-03T14:41Z | claude-ctl-17-e566 | finish | pending | Wiring, settings, end to end test, and wiki done |
| 2026-11-03T14:50Z | claude-ctl-17-e566 | finish | e7a2c01 | CI green on #116 after two lint fixes |
