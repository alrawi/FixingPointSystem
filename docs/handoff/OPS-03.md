# Handoff OPS-03 Stale lease scanner and dependency promoter

| Field | Value |
|---|---|
| Issue | #12 |
| Branch | `wp/OPS-03` |
| Pull request | #72 |
| Status | in progress |
| Progress | 67% (4 of 6 plan steps) |
| Current holder | claude-ops03-edfd, lease until 2026-11-02T21:35:06Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We run a scheduled workflow that marks expired leases stale and promotes backlog packages whose dependencies have closed. The workflow runs every 30 minutes with only the repository token, and a manual dispatch with `dry_run` set prints the planned changes and changes nothing. The stale sweep posts one comment with the expiry time when it marks an issue stale.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Post one stale comment with the expiry time from the existing stale sweep in `fps_swarm.py`
- [x] 3. Add `.github/workflows/swarm-scan.yml` with the schedule, the dispatch input, and the token permissions
- [x] 4. Add unit tests for expiry, a renewed lease, a blocked issue, and dependency promotion
- [ ] 5. Ingest the result into the wiki and append the Log entry
- [ ] 6. Merge `origin/main`, pass CI, and move the package to review

## Done and verified

1. The claim commit is on `origin/wp/OPS-03`, and pull request #72 carries `wip`.
2. `Swarm.stale_sweep` posts one comment per expired lease after the relabel. Verified by `python3 -m pytest -q tools/swarm`, 49 passed.
3. `.github/workflows/swarm-scan.yml` runs `fps_swarm.py sweep` every 30 minutes and on dispatch. Verified by `test_scan_workflow_runs_the_sweep_on_schedule_with_the_repository_token`.
4. The tests named `test_scan_*` cover expiry, a renewed lease, a blocked issue, dependency promotion, a card free item, and the dry run. Verified by the same pytest run.

## Remaining

Steps 5 and 6. The acceptance criterion for a manual dispatch with `dry_run` can only run on GitHub after the owner merges, because GitHub dispatches only workflows on the default branch.

## Next action

Ingest the scanner into Swarm Protocol, Project Board Setup, and the swarm README, then append the Log entry.

## Blockers and open questions

none

## Decisions taken in this package

- The workflow calls `fps_swarm.py sweep` and holds no sweep logic of its own, so the scanner and agents apply the same rules.
- The stale comment starts with `<!-- fps:stale lease=<time> -->`. The sweep skips the comment when one with the same lease exists, so a sweep that races another sweep or a relabel does not repeat it. The relabel runs first, so a failed comment never leaves an item in progress.
- The expiry time is the latest lease among active cards. An item without any active card gets `lease=none`.
- A manual dispatch defaults to `dry_run` true, so an accidental dispatch changes nothing. Scheduled runs carry no inputs and always apply.
- The job uses `concurrency` so two runs never sweep at the same time.

## Pitfalls

- PyYAML reads the bare workflow key `on` as the boolean `true`, so the workflow test indexes `flow[True]`.
- `ruff check tools/swarm` reports two RUF100 findings on `main` already when a user level ruff config enables RUF. The repository has no ruff config, and we left those lines alone.

## Verify the current state

```bash
python3 -m pytest -q tools/swarm     # 49 passed
ruff format --check tools/swarm      # 4 files already formatted
mypy --strict tools/swarm/fps_swarm.py
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:05:06Z | claude-ops03-edfd | claim | none | Card 6587210287, six plan steps |
| 2026-11-02T20:08:21Z | claude-ops03-edfd | checkpoint | see branch | Steps 2 to 4, stale comment, workflow, and tests |
