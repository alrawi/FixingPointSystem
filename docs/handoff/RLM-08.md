# Handoff RLM-08 Session structure check

| Field | Value |
|---|---|
| Issue | #190 |
| Branch | `wp/RLM-08` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-36eea3, lease until 2026-11-10T00:38:58Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the session structure check to the realism report. It reads the flow graph of a scenario (flows, `after` edges, `spawn` edges) from a module scenario file, reconstructs the connections of each scenario instance from the capture, and compares the connection count and the start and end ordering with the graph. A failure names the instance and the violated edge.

## Plan

- [ ] 1. Scenario graph reader (`realism/scenario_graph.py`)
- [ ] 2. Instance reconstruction and checks (`realism/structure.py`), labels `structure` section, report schema 0.6.0, `--scenario`
- [ ] 3. Tests with generated fixtures (`realism/tests/test_structure.py`): web_browse passes, page before lookup fails, ftp shaped scenario
- [ ] 4. README, wiki page, log entry, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything in the plan.

## Next action

Write `realism/scenario_graph.py`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

The sandbox guard refuses a command that mixes `git` with `source`, `export`, or a heredoc, so pass variables inline and write files with the editor tools.

## Verify the current state

```bash
make lint-rlm test-rlm
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T00:09Z | claude-36eea3 | claim | none | claimed #190, branch wp/RLM-08 |
