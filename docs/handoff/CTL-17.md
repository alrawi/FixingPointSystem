# Handoff CTL-17 Controller binary wiring

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #111 |
| Branch | `wp/CTL-17` |
| Pull request | #0 |
| Status | in progress |
| Progress | 20% (1 of 5 plan steps) |
| Current holder | claude-ctl-17-e566, lease until 2026-11-03T15:51:41Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We serve the REST API and the agent gRPC service from the controller binary, with the configuration settings they need. The binary opens the store and the registry, starts the agent manager and the run orchestrator, and mounts one combined API server. An end to end test starts the binary, registers a fake agent over gRPC, stores a config, installs and lists the example modules, and starts a run over REST.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Add the settings for tokens, data_dir, store_path, and grpc_listen to internal/app
- [ ] 3. Add internal/wiring with the adapters and the combined API server, with unit tests
- [ ] 4. Wire cmd/fps-controller and add the end to end test of the binary
- [ ] 5. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Write exactly one concrete step that the next agent can start immediately.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T14:22Z | claude-ctl-17-e566 | claim | none | Claimed #111 with card 6606279050 |
