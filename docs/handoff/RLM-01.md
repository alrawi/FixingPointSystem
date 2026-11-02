# Handoff RLM-01 Realism runner and nDPI conformance

| Field | Value |
|---|---|
| Issue | #51 |
| Branch | `wp/RLM-01` |
| Pull request | #73 |
| Status | in progress |
| Progress | 83% (5 of 6 plan steps) |
| Current holder | claude-rlm01-7fb0, lease until 2026-11-02T21:35:33Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Score a capture against expected application labels with nDPI run out of process and write a JSON realism report. The command is `python3 -m realism score --pcap FILE --labels FILE`, and the report schema in `realism/schema/` is the seam that later checks, the controller, and QA extend. Conformance is the share of sessions that the classifier labels as the intended application, compared with the 99% threshold of the specification.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the handoff file, and open the pull request.
- [x] 2. Read pcap and pcapng captures with the standard library and build the session table.
- [x] 3. Load the labels file and add the classifier interface with the stub and the nDPI implementation.
- [x] 4. Score conformance, write the report schema, and add the command line entry point.
- [x] 5. Add tests, the `test-rlm` Makefile block, the `rlm` workflow, and the README.
- [ ] 6. Ingest into the wiki, merge `origin/main`, and finish.

## Done and verified

1. Claim card posted on #51 and branch pushed.
2. `realism/capture.py` reads pcap and pcapng and builds the session table. Verified by a manual run on a synthetic capture with 8 sessions.
3. `realism/labels.py` and `realism/classifier.py` hold the labels loader, the classifier protocol, the stub, and the nDPI runner.
4. `realism/conformance.py`, `realism/report.py`, `realism/cli.py`, and the two schemas in `realism/schema/`. Verified by `python3 -m realism score` with ndpiReader 4.2.0, which reported 5 of 5 HTTP and 3 of 3 DNS sessions. `ruff check realism` and `mypy --strict` are clean.

## Remaining

Step 6, the wiki ingest and finish.

## Next action

Ingest the package into the wiki and append the Log entry.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
git log --oneline -1
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:06:06Z | claude-rlm01-7fb0 | claim | | Card 6587219833, fresh branch from origin/main |
| 2026-11-02T20:40:00Z | claude-rlm01-7fb0 | checkpoint | | Runner code for steps 2 to 4, tests pending |
| 2026-11-02T21:00:00Z | claude-rlm01-7fb0 | checkpoint | | Tests, Makefile block, rlm workflow, README |
