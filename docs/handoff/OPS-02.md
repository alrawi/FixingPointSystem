# Handoff OPS-02 Swarm helper command

| Field | Value |
|---|---|
| Issue | #11 |
| Branch | `wp/OPS-02` |
| Pull request | #62 |
| Status | in progress |
| Progress | 83% (5 of 6 plan steps) |
| Current holder | claude-ops02-db00, lease until 2026-11-02T18:57:00Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We implement the claim, heartbeat, release, takeover, block, review, and sweep operations of the [Swarm Protocol](../wiki/Swarm-Protocol.md) as one command, `tools/swarm/fps_swarm.py`, over the GitHub REST API. Agents then stop typing raw API calls. Unit tests replay recorded API responses offline, and two simulated agents that claim one issue concurrently resolve to exactly one holder.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Status card model with render, parse, and the holder rule, plus unit tests
- [x] 2. API seam (one injectable `gh api` runner), server time from the Date header, and a recorded fake API for tests
- [x] 3. Subcommands list, claim, and takeover, with the concurrent claim test
- [x] 4. Subcommands heartbeat, release, block, and review
- [x] 5. Promotion sweep and stale sweep subcommands with tests
- [ ] 6. CI wiring, wiki update (Swarm Protocol and Log), and finish

## Done and verified

Steps 1 to 5 are done. `python3 -m pytest -q tools/swarm` reports 42 passed offline, and `mypy --strict tools/swarm/fps_swarm.py` reports no issues.

## Remaining

Step 6, which covers the CI pytest path, the Swarm Protocol page, the Log entry, and the finish.

## Next action

Add `tools/swarm` to the pytest step in `.github/workflows/ci.yml`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
python3 -m pytest -q tools/swarm   # expect every test to pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:27:20Z | claude-ops02-db00 | claim | none | Card 6583717345 holds the lease after the settle wait |
| 2026-11-02T17:40:00Z | claude-ops02-db00 | checkpoint | pending | Steps 1 to 5 done, 42 tests pass |
