# Handoff OPS-04 Project board status sync

| Field | Value |
|---|---|
| Issue | #13 |
| Branch | `wp/OPS-04` |
| Pull request | none |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-ops-04-62cd, lease until 2026-11-03T04:13:24Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We mirror the status label of each work item into the Status field of the owner's GitHub Project. A workflow runs on issue label events and on a schedule, because the scanner relabels with the repository token and such events start no workflow. The workflow exits successfully and changes nothing when the secret `FPS_PROJECT_TOKEN` is absent.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Write `tools/swarm/project_sync.py`, which finds the project, maps status labels to Status options, and edits the items over GraphQL
- [ ] 3. Add offline unit tests that replay recorded GraphQL responses
- [ ] 4. Add `.github/workflows/project-sync.yml` with label events, a schedule, and the secret guard
- [ ] 5. Write the Operations page and the log entry, pass CI, and move the package to review

## Done and verified

Nothing yet.

## Remaining

Steps 1 to 5.

## Next action

Push the claim commit and open the pull request.

## Blockers and open questions

- The owner must create the secret `FPS_PROJECT_TOKEN`, so the package keeps `needs:owner`.
