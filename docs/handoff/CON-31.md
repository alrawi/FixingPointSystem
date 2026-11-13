# Handoff CON-31 A session is a scenario session, and its spawned flows add bytes

| Field | Value |
|---|---|
| Issue | #537 |
| Branch | `wp/CON-31` |
| Pull request | #540 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-10fd70, lease until 2026-11-13T05:40:12Z |
| Last verified commit | 34ec147 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

A session becomes one run of the entry flow of a scenario instance. Every other flow of the instance, spawned or ordered with `after`, adds its bytes to that session, and its failure fails it. No other flow counts as a session. The owner approved this on 2026-11-12 and decided the `after` question in the comment on #537. This package changes the contract only. ENG-30 changes the engine afterwards.

## Plan

- [x] 1. Find every contract statement and consumer that counts sessions per flow
- [x] 2. Change contracts/README.md and the sessions series descriptions in the metric catalog, decide the version question
- [x] 3. Write contracts/changes/CON-31.md with the consumer and test list
- [x] 4. Write the log entry with the Shared page notes (Module-Development expected_bytes text)
- [x] 5. Run checks, open the PR, wait for CI, review step

## Done and verified

- Steps 1 to 4: contracts/README.md (Sessions and flows), catalog descriptions, controller byte copy, contracts/changes/CON-31.md, Contracts.md, log entry with Shared page notes.
- `python3 tools/contracts/validate.py`: 0 errors. `make test-con`: 350 passed. `plan.py check`, `render --check`, `lint`: ok. CI on 34ec147: 22 of 22 check runs succeeded.

## Remaining

Nothing. The coordinator applies the Module Development text from the Shared page notes of the log entry, merges after the owner approves, and ENG-30 follows.

## Next action

Coordinator review and merge.

## Blockers and open questions

none

## Decisions taken in this package

- No version bump (catalog 0.8.0, scenario v0.6): text only, precedent CON-16 and CON-23.
- Descriptions of sessions.attempted, concurrent, failed, failed_ratio unchanged, because the controller prints them in testdata/prometheus.txt, which no authorized follow up covers.
- A session spans its whole instance (duration, concurrent) and fails once with the reason of the first failed flow.

## Pitfalls

The controller golden prometheus.txt holds the HELP text of four sessions series. Go cannot build here.

## Verify the current state

```bash
python3 tools/contracts/validate.py
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13T04:12Z | claude-10fd70 | claim | none | Claimed #537, branch from origin/main ec730b4 |
| 2026-11-13T04:22Z | claude-10fd70 | checkpoint | 34ec147 | Contract, entry, wiki and log entry written, checks pass, PR #540 opened |
| 2026-11-13T04:35Z | claude-10fd70 | finish | | CI green on 34ec147, ready for review |
