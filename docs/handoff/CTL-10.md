# Handoff CTL-10 Capabilities endpoint

| Field | Value |
|---|---|
| Issue | #48 |
| Branch | `wp/CTL-10` |
| Pull request | none |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-ctl-10-49f5, lease until 2026-11-03T14:26:38Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We serve `GET /v1/capabilities` with the engine API version, the backends, the agent inventory, and the installed modules. The handler reads two narrow interfaces that `internal/api` defines, so the package imports neither `internal/registry` nor `internal/agents`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Add the capabilities type with its two interfaces and the contract test with two fake agents and two modules
- [ ] 3. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Nothing yet.

## Remaining

Steps 2 and 3.

## Next action

Write `controller/internal/api/capabilities.go` and its test.

## Blockers and open questions

None.
