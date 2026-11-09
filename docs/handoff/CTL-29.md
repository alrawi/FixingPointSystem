# Handoff CTL-29 Rebuild the agent inventory after a restart

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #263 |
| Branch | `wp/CTL-29` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-c29a7e, lease until 2026-11-09T05:22:21Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

At controller start every identity with a valid, unrevoked, unexpired certificate in the PKI state joins the agent inventory, so agents register again after a restart with agent_auto_enroll off.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. pki: Authority.ValidIdentities lists the identities with a live certificate
- [x] 2. agents: Manager.Seed adds identities to the inventory
- [x] 3. Tests: unit tests plus restart acceptance test; one-line call in wiring/controller.go
- [ ] 4. Docs: Controller.md, log entry; PR and CI

## Done and verified

- Steps 1 to 3: `go test ./internal/pki` passes locally (GOPROXY=off, stdlib only); the agents and wiring packages cannot build here, so CI is the loop. `gofmt -l controller/internal` is clean.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Open the PR and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

Seed lives on the manager and the one-line call is in wiring/controller.go (outside the paths, the owner of the brief asked for it). Seeded entries are enrolled with no labels. Tokens without a certificate are not seeded.

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
| 2026-11-08 | claude-c29a7e | claim | none | Claimed #263 |
| 2026-11-08 | claude-c29a7e | checkpoint | pending | Code, tests, docs written |
