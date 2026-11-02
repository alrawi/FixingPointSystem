# Handoff CTL-01 Controller skeleton

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #39 |
| Branch | `wp/CTL-01` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-ctl01-a67c, lease until 2026-11-02T21:34:21Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We create the Go module `github.com/alrawi/FixingPointSystem/controller` with the `fps-controller` binary. The binary reads flags and a YAML configuration file, logs JSON through `log/slog`, serves `/healthz`, and shuts down gracefully on SIGTERM. A golangci-lint configuration, a path filtered CI workflow, a `test-ctl` make target, and the `go.work` line complete the package.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Create the module, the configuration loader in `internal/app`, and its unit tests
- [ ] 3. Add the HTTP server with `/healthz`, the run loop with graceful shutdown, and component tests
- [ ] 4. Add `cmd/fps-controller` with flags, signal handling, and its tests
- [ ] 5. Add `.golangci.yml`, the `go.work` line, the make targets, and `.github/workflows/controller.yml`
- [ ] 6. Ingest the result into the wiki, pass the tests and CI, and finish

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Create `controller/go.mod` and the configuration loader in `controller/internal/app/config.go`.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cd controller && go test -race ./...
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:04:21Z | claude-ctl01-a67c | claim | none | Claimed #39 with card 6587193148, branch created from `origin/main` at 2921645 |
