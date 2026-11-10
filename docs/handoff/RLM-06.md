# Handoff RLM-06 Temporal distribution check

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #188 |
| Branch | `wp/RLM-06` |
| Pull request | #353 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-9c41e7, lease until 2026-11-10T02:39:45Z |
| Last verified commit | d7b75fc |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Compare session inter-arrival times and object sizes with target distributions given in the labels, through the KS test with alpha as a parameter. Samples from the target pass and samples from a shifted distribution fail. Opt-in check, report format 0.7.0.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Pure Python KS (one and two sample, exact and asymptotic p value) with tests against known values
- [x] 2. Labels section `temporal` and its schema, target distributions
- [x] 3. The check, report schema entry, CLI flags, metrics
- [x] 4. Tests with direct samples and pcapgen captures
- [x] 5. Docs: Realism.md, README, log entry, CI checks

## Done and verified

- Steps 1 to 5: `python3 -m pytest -q realism` (389 passed, 3 skipped for tools that are not installed), `make lint-rlm`, `python3 tools/plan/plan.py check`, `render --check`, and `lint` all pass locally.
- The KS tests match the critical value tables and `scipy.stats` (installed only in the scratchpad to read reference values, not a dependency).

## Remaining

Nothing for this package. CI on commit 75635b1 is green. Known gaps: the check does not compare think times inside a session, and comparisons over all sessions or over both directions have no catalog metric label.

## Next action

The coordinator reviews and merges pull request #353.

## Blockers and open questions

none

## Decisions taken in this package

- Pure Python KS (`realism/ks.py`), because the runner has no numeric dependency. Exact p values for one sample up to 100 observations (Marsaglia, Tsang, Wang) and for two samples up to 10000 pairs (lattice paths), Stephens corrected asymptotic otherwise.
- Targets live in the labels (`temporal` section), in the vocabulary of a scenario `delay` for arrivals and in bytes for sizes, because the test configuration has no arrival process or size distribution.
- Insufficient data (default fewer than 30 observations) fails the check, and so does a section without targets, so the check never passes on nothing.
- Default alpha 0.01 as the brief says, although the specification gate says 0.05, so the gate sets alpha explicitly.
- Metrics only for per application comparisons whose property the catalog names (inter_arrival, request_bytes, response_bytes).

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

The worktree guard refuses a command that mixes a heredoc and git, so write helper scripts with the editor and run them alone. pcapgen sessions start at `clock + 0.001`, so tests set `cap.clock` to place a session start.

## Verify the current state

```bash
python3 -m pytest -q realism   # 389 passed, 3 skipped
make lint-rlm                  # clean
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T02:10Z | claude-9c41e7 | claim | | claimed #188 |
| 2026-11-10T02:30Z | claude-9c41e7 | checkpoint | d7b75fc | code, tests, docs pushed, pull request #353 opened |
| 2026-11-10T01:40Z | claude-9c41e7 | review | 75635b1 | CI green on pull request #353, status ready for review |
