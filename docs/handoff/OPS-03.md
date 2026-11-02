# Handoff OPS-03 Stale lease scanner and dependency promoter

| Field | Value |
|---|---|
| Issue | #12 |
| Branch | `wp/OPS-03` |
| Pull request | none yet |
| Status | in progress |
| Progress | 17% (1 of 6 plan steps) |
| Current holder | claude-ops03-edfd, lease until 2026-11-02T21:35:06Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We run a scheduled workflow that marks expired leases stale and promotes backlog packages whose dependencies have closed. The workflow runs every 30 minutes with only the repository token, and a manual dispatch with `dry_run` set prints the planned changes and changes nothing. The stale sweep posts one comment with the expiry time when it marks an issue stale.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Post one stale comment with the expiry time from the existing stale sweep in `fps_swarm.py`
- [ ] 3. Add `.github/workflows/swarm-scan.yml` with the schedule, the dispatch input, and the token permissions
- [ ] 4. Add unit tests for expiry, a renewed lease, a blocked issue, and dependency promotion
- [ ] 5. Ingest the result into the wiki and append the Log entry
- [ ] 6. Merge `origin/main`, pass CI, and move the package to review

## Done and verified

Step 1 is done when the claim commit is on `origin/wp/OPS-03`.

## Remaining

Steps 2 to 6.

## Next action

Add the stale comment to `Swarm.stale_sweep` in `tools/swarm/fps_swarm.py`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 -m pytest -q tools/swarm     # every test passes
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:05:06Z | claude-ops03-edfd | claim | none | Card 6587210287, six plan steps |
