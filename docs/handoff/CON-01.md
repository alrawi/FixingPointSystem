# Handoff CON-01 Agent control protocol v0.1

| Field | Value |
|---|---|
| Issue | #17 |
| Branch | `wp/CON-01` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-con01-222f, lease until 2026-11-02T18:57:56Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Define the gRPC service between the controller and the agents, and the compiled plan messages that the controller sends to each agent. The plan also has a canonical JSON mapping, so the standalone agent mode can read a plan file without a controller. `buf lint` passes, and an example plan in `contracts/examples/plans/` validates against the JSON mapping.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Write `contracts/proto/fps/agent/v1/agent.proto` with the service and the plan messages.
- [ ] 2. Add `contracts/proto/buf.yaml` with lint and breaking rules and compile the proto locally.
- [ ] 3. Write the example plan in `contracts/examples/plans/` in the canonical JSON mapping.
- [ ] 4. Add a test that parses the example plan with the protobuf package, plus semantic checks.
- [ ] 5. Add the CI job with `bufbuild/buf-action` and the plan test, and update the contracts README and CHANGELOG.
- [ ] 6. Ingest into the wiki, append the Log entry, and finish the pull request.

## Done and verified

None yet.

## Remaining

All plan steps.

## Next action

Write the proto file under `contracts/proto/fps/agent/v1/`.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
git log --oneline -3
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:30:24Z | claude-con01-222f | claim | pending | Card 6583736116, no earlier cards |
