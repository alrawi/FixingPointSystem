# Handoff CTL-03 Configuration store and validation

| Field | Value |
|---|---|
| Issue | #41 |
| Branch | `wp/CTL-03` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-ctl-03-78e4, lease until 2026-11-03T13:09:14Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We store test configurations in SQLite behind a repository interface and validate them against the JSON Schema, the installed module references, and the agent capacity. The package serves POST and GET /v1/configs and POST /v1/configs/{id}:validate.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Add the repository interface and its SQLite implementation
- [ ] 3. Add schema validation, module reference checks, and capacity checks
- [ ] 4. Add the handlers and their tests with every example and invalid fixtures
- [ ] 5. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Nothing yet.

## Remaining

Steps 2 to 5.

## Next action

Add the repository interface and the SQLite implementation in controller/internal/store/.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/store/
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:40Z | claude-ctl-03-78e4 | claim | none | Claimed with card 6602821433 |
