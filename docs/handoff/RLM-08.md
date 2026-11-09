# Handoff RLM-08 Session structure check

| Field | Value |
|---|---|
| Issue | #190 |
| Branch | `wp/RLM-08` |
| Pull request | #343 |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-36eea3, lease until 2026-11-10T00:38:58Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the session structure check to the realism report. It reads the flow graph of a scenario (flows, `after` edges, `spawn` edges) from a module scenario file, reconstructs the connections of each scenario instance from the capture, and compares the connection count and the start and end ordering with the graph. A failure names the instance and the violated edge.

## Plan

- [x] 1. Scenario graph reader (`realism/scenario_graph.py`)
- [x] 2. Instance reconstruction and checks (`realism/structure.py`), labels `structure` section, report schema 0.6.0, `--scenario`
- [x] 3. Tests with generated fixtures (`realism/tests/test_structure.py`): web_browse passes, page before lookup fails, ftp shaped scenario
- [ ] 4. README, wiki page, log entry, pull request, CI

## Done and verified

1 to 3: `make lint-rlm test-rlm` passes locally (306 passed, 3 skipped for tools that are not installed). `plan.py check`, `render --check`, and `lint` pass. Step 4 is the README, the wiki page, and the log entry, done, and then the pull request.

## Remaining

The pull request and CI.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- The check is opt in: a `structure` section in the labels or `--scenario FILE`.
- Instances come from slot filling per client address, not from opening an instance at each root flow, so a page fetch before its lookup joins the instance of the lookup and the failure names the edge.
- A session completes at its last packet with payload, FIN, or RST, and the closing ACK does not count.
- `web_browse` of the specification (lookup, then page) is copied to `realism/examples/web_browse.scenario.yaml`, because the repository held it only as a string in an ENG-07 test and `contracts/` is out of scope.
- `modules/ftp/fixtures/structure_check.py` stays, because the new check does not read the announced data port or the FINs, and `ftp.pcapng` mixes five scenarios on one address.

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
