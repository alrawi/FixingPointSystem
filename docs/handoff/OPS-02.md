# Handoff OPS-02 Swarm helper command

| Field | Value |
|---|---|
| Issue | #11 |
| Branch | `wp/OPS-02` |
| Pull request | #62 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-ops02-db00, lease until 2026-11-02T19:04:34Z |
| Last verified commit | 2e8a2bd |

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
- [x] 6. CI wiring, wiki update (Swarm Protocol and Log), and finish

## Done and verified

Every step is done. We verified the result with these commands.

| Command | Result |
|---|---|
| `python3 -m pytest -q tools/swarm` | 42 passed, offline |
| `python3 -m pytest -q tools/contracts tools/plan tools/swarm spikes/s4-trex-astf` | 102 passed, the CI command |
| `mypy --strict tools/swarm/fps_swarm.py` | no issues |
| `ruff format --check tools/swarm` and `ruff check tools/swarm` | clean |
| `python3 tools/plan/plan.py check`, `render --check`, and `lint` | ok |
| `fps_swarm.py heartbeat 11 --check` and `heartbeat 11 ...` against the live repository | renewed card 6583717345 |
| `fps_swarm.py list` and `sweep --dry-run` against the live repository | read only, printed the three ready items and no sweep changes |

The acceptance criteria map to tests. `test_two_agents_claiming_together_resolve_to_one_holder` interleaves both claims step by step, and `test_concurrent_claims_in_threads_resolve_to_one_holder` runs both claims in threads twenty times. Each case ends with exactly one holder.

## Remaining

Two gaps remain and suit follow up work. The `review` subcommand performs the finish transition, and posting a reviewer verdict comment still uses the raw call. Git branch preparation and pull request creation stay with the agent, because the helper covers only the API steps.

## Next action

Wait for the owner review of pull request #62.

## Blockers and open questions

none

## Decisions taken in this package

- We call `gh api -i` for every request, so each response carries the `Date` header and the command reads server time without a second clock.
- We paginate with `page` and `per_page` ourselves instead of `--paginate`, because `--paginate -i` interleaves several header blocks.
- The helper finds its card by the agent identifier, since the protocol keeps no state in agent memory.
- Blocking comments carry `<!-- fps:block agent=<id> issues=<n,...> -->`, and the promotion sweep reads the newest marker. Without a marker or with an empty issue list the sweep leaves the item for the owner. The Swarm Protocol page now documents this marker.
- `takeover` also accepts `status:in-progress` when the holder card expired, so an agent need not wait for the stale sweep.
- A card that starts with the marker but fails to parse is skipped, so one malformed comment cannot break the holder rule.
- We added `tools/swarm` to the pytest step in `.github/workflows/ci.yml`, because CI lists test directories explicitly.

## Pitfalls

- Comments posted from the Claude Code sandbox get a footer appended after the card table. The parser ignores trailing text, and the recorded fixture keeps the footer to prove it.
- The local ruff here reports `RUF100` on the `noqa: E402` imports in the test, which follow `tools/plan/test_plan.py`. Default ruff settings accept them.

## Verify the current state

```bash
python3 -m pytest -q tools/swarm   # expect 42 passed
FPS_AGENT_ID=x python3 tools/swarm/fps_swarm.py now   # prints GitHub server time
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:27:20Z | claude-ops02-db00 | claim | none | Card 6583717345 holds the lease after the settle wait |
| 2026-11-02T17:40:00Z | claude-ops02-db00 | checkpoint | pending | Steps 1 to 5 done, 42 tests pass |
| 2026-11-02T17:50:00Z | claude-ops02-db00 | finish | pending | CI wiring, wiki ingest, ready for review |
