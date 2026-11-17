# Handoff CTL-52 Multi agent runs through the run orchestrator

| Field | Value |
|---|---|
| Issue | #596 |
| Branch | `wp/CTL-52` |
| Pull request | not yet opened |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-261fea, lease until 2026-11-17T03:34:27Z |
| Last verified commit | none (no local Go build; CI is the build) |

## Goal

The run orchestrator starts a run on two or more agents through `multiagent.Coordinator` with the clock source of CTL-51 and aggregates its results. An end to end test over REST runs one configuration on three fake agents, refuses it with one agent 15 ms off, and reports exact totals. Runs on one agent stay as they are.

## Plan

- [x] 1. Config fields, clock check in `Create`, REST mapping (409).
- [x] 2. Start through the coordinator in `drive`, snapshot feed, stop through `Run.Stop`.
- [x] 3. Aggregate from the collector with the crash accounting of failed agents, wiring.
- [x] 4. Tests over REST, wiki page, log entry.
- [ ] 5. CI green, review.

## Done and verified

- `gofmt -l` is clean for `controller/internal/runs` and `controller/internal/wiring`.
- Nothing else could run locally, because Go modules do not download in the sandbox.

## Remaining

CI results (`test`, `controller-e2e`) and fixes inside the package paths.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- A clock refusal is 409, like agent offline, reserved, and busy. `startRun` answers problems through `default`, so no contract change is needed.
- The check runs in `Create` after reserve and compile, so unknown agents and invalid configs keep their 422, and a refusal releases the reservations.
- `Config.Clocks == nil` keeps the legacy start. The controller always sets it.
- The coordinator gets the plans of `Create` through a `CompilerFunc`, so plan ids do not change and nothing compiles twice.
- The orchestrator keeps its per agent snapshots for calibration and for counting the open sessions of failed agents. The coordinator collector supplies the sums, so totals are counted once. `internal/metrics` is untouched.

## Pitfalls

- `Run.Stop` stops every agent, including finished and failed ones. Their errors are logged and the totals still come from the collector.
- The run state becomes running when the start commands were accepted, before the start instant.

## Verify the current state

```bash
gofmt -l controller/internal/runs controller/internal/wiring
# expected: no output; CI runs the Go tests
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17T03:04Z | claude-261fea | claim | none | issue #596 claimed |
