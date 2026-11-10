# Handoff MOD-14 SIP with RTP module

| Field | Value |
|---|---|
| Issue | #181 |
| Branch | `wp/MOD-14` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-4d1e7a (the lease on the status card renews at every checkpoint) |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship `sip_rtp` scenarios with an INVITE dialog, SDP negotiation, and paced RTP streams in both directions. Deliverable: SIP over UDP, spawned RTP flows at codec packet intervals, and BYE teardown. Acceptance: the temporal check accepts RTP inter arrival times at the configured interval, and Zeek logs every call.

## Plan

- [ ] 1. Write the manifest and the native crate (SIP messages, SDP offer and answer, RTP packets) with unit tests
- [ ] 2. Write the scenarios and the interpreter, catalog, and golden transcript tests over an in memory UDP network
- [ ] 3. Write `fixture_peer` and `capture.sh`, and record and score the fixture in the QA-01 harness
- [ ] 4. Write the RTP interval check and the Zeek check with their tests
- [ ] 5. Write the README, the log entry with the shared page notes, and the Modules page section
- [ ] 6. Run the checks, open the PR, and wait for CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Write `modules/sip_rtp/manifest.yaml` and the native crate.

## Blockers and open questions

The agent refuses a scenario with several flows of which one is UDP (AGT-16 left the check in place, the runtime has no datagram opener for instances). The scenarios of this package therefore run through the interpreter and the fixture peer, and not yet through the agent.

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:25Z | claude-4d1e7a | claim | | Claimed #181 with a 6 step plan |
