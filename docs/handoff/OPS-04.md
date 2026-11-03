# Handoff OPS-04 Project board status sync

| Field | Value |
|---|---|
| Issue | #13 |
| Branch | `wp/OPS-04` |
| Pull request | #80 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-ops-04-62cd, lease until 2026-11-03T04:13:24Z |
| Last verified commit | see pull request #80 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We mirror the status label of each work item into the Status field of the owner's GitHub Project. A workflow runs on issue label events and on a schedule, because the scanner relabels with the repository token and such events start no workflow. The workflow exits successfully and changes nothing when the secret `FPS_PROJECT_TOKEN` is absent.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Write `tools/swarm/project_sync.py`, which finds the project, maps status labels to Status options, and edits the items over GraphQL
- [x] 3. Add offline unit tests that replay recorded GraphQL responses
- [x] 4. Add `.github/workflows/project-sync.yml` with label events, a schedule, and the secret guard
- [x] 5. Write the Operations page and the log entry, pass CI, and move the package to review

## Done and verified

1. The claim commit is on `origin/wp/OPS-04`, and pull request #80 carries `wip`.
2. `tools/swarm/project_sync.py` finds the one project of `alrawi` linked to the repository, or the project in `FPS_PROJECT_NUMBER`, reads the Status options, and adds or edits items with `addProjectV2ItemById` and `updateProjectV2ItemFieldValue`. It sends no mutation when the option already matches, and it exits with 0 without any request when `FPS_PROJECT_TOKEN` is absent.
3. `tools/swarm/test_project_sync.py` replays the responses in `tools/swarm/testdata/project_sync/`. We wrote those files by hand in the shape of GitHub GraphQL replies, because the sandbox cannot reach the API to record real ones. Verified by `python3 -m pytest -q tools/swarm`, 68 passed.
4. `.github/workflows/project-sync.yml` runs on issue events, at minutes 7 and 37, and on dispatch with `dry_run`. `test_workflow_runs_on_label_events_and_on_a_schedule_with_the_secret` checks it.
5. We created `docs/wiki/Operations.md`, the log entry with Shared page notes, a link line in Project Board Setup that the coordinator approved, and two rows in `tools/swarm/README.md`. Verified by `python3 tools/plan/plan.py lint`, which printed `ok: wiki links`.

## Remaining

Review by an agent that did not write the change. Nobody has run the sync against the live GraphQL API. The acceptance criterion, a card that moves within one minute of a relabel, can only be checked after the owner creates the secret and the change merges.

## Next action

After the merge, the owner follows the Owner setup steps in Operations, runs **Run workflow** on `project-sync` with `dry_run` set, and then relabels a test issue.

## Blockers and open questions

- The owner must create the secret `FPS_PROJECT_TOKEN`, so the package keeps `needs:owner`.
