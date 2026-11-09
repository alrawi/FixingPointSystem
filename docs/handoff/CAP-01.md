# Handoff CAP-01 Capture session extraction

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #193 |
| Branch | `wp/CAP-01` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-c4a901, lease until 2026-11-09T17:42:01Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Reassemble TCP streams and group UDP flows from a capture into sessions with timing: `python3 -m capture sessions --pcap FILE` writes session JSON. Tests use synthetic captures that cover retransmission, out of order segments, and missing handshakes.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim commit and handoff
- [ ] 2. Readers (pcap, pcapng) and packet decoder
- [ ] 3. TCP reassembly and UDP flows, session builder, CLI, JSON Schema
- [ ] 4. Tests with synthetic captures, ruff, mypy, mk/cap.mk
- [ ] 5. Wiki page, log entry, PR, CI

## Done and verified

List each finished step with the command that verified it and the result.

## Remaining

List what is still missing, including known gaps in finished steps.

## Next action

Step 2: write capture/pcapio.py.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T17:12Z | claude-c4a901 | claim | none | claimed #193 |
