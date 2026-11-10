# Handoff MOD-14 SIP with RTP module

| Field | Value |
|---|---|
| Issue | #181 |
| Branch | `wp/MOD-14` |
| Pull request | #417 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-4d1e7a (the lease on the status card renews at every checkpoint) |
| Last verified commit | 3f97b8a |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship `sip_rtp` scenarios with an INVITE dialog, SDP negotiation, and paced RTP streams in both directions. Deliverable: SIP over UDP, spawned RTP flows at codec packet intervals, and BYE teardown. Acceptance: the temporal check accepts RTP inter arrival times at the configured interval, and Zeek logs every call.

## Plan

- [x] 1. Write the manifest and the native crate (SIP messages, SDP offer and answer, RTP packets) with unit tests
- [x] 2. Write the scenarios and the interpreter, catalog, and golden transcript tests over an in memory UDP network
- [x] 3. Write `fixture_peer` and `capture.sh`, and record and score the fixture in the QA-01 harness
- [x] 4. Write the RTP interval check and the Zeek check with their tests
- [x] 5. Write the README, the log entry with the shared page notes, and the Modules page section
- [x] 6. Run the checks, open the PR, and wait for CI

## Done and verified

PR #417 passes all 32 CI checks at 3f97b8a, including the realism job of `sip_rtp` with a fresh capture, tshark 4.2.2, nDPI 4.2 from apt, and Zeek 9.0.1. Manifest, native crate, five scenarios, interpreter tests over an in memory UDP network, golden transcripts, `fixture_peer`, `capture.sh`, the recorded fixture `fixtures/sip_rtp.pcapng` (27 sessions: 15 SIP, 12 RTP), `rtp_check.py`, `zeek_check.py`, and their unit tests. `capture.sh` passes in the harness: nDPI 4.2.0 labels all 15 SIP and 12 RTP sessions, tshark reports no parser error, and all 24 RTP streams have a mean gap within 0.1 ms of the packet time. Zeek is not installed here, so `zeek_check.py` is tested only against synthetic logs.

## Remaining

The coordinator adds `mk/mod-sip.mk` and `.github/workflows/mod-sip.yml` from the log entry. The runtime datagram opener, the `sip_rtp` registry entry, and a per packet inter arrival property in the realism runner are follow ups.

## Next action

Review PR #417.

## Blockers and open questions

The agent refuses a scenario with several flows of which one is UDP (AGT-16 left the check in place, the runtime has no datagram opener for instances). The scenarios of this package therefore run through the interpreter and the fixture peer, and not yet through the agent.

## Decisions taken in this package

- Lock step media: both sides of a call send RTP on one pair of ports in one flow, because the scenario language has no concurrency inside a side and nDPI needs both directions on one 5-tuple.
- The `pace` function schedules each packet against an absolute due time. A plain `delay` per packet drifted to 21.5 ms for a 20 ms packet time in the harness.
- The Opus payload starts with a valid table of contents byte, because tshark rejects random Opus frames as parser errors.

## Pitfalls

- The server spawns its media flow before the ring time. When it spawned after the ring time, the first RTP packet of the client reached the port before the server bound it, and the call failed.
- The realism temporal check measures session start gaps and payload sizes, not the gaps between the packets of one session, so `rtp_check.py` does that in this module.

## Verify the current state

```bash
git log --oneline -3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:25Z | claude-4d1e7a | claim | | Claimed #181 with a 6 step plan |
| 2026-11-10T17:50Z | claude-4d1e7a | checkpoint | fa2f84c | Steps 1 and 2: manifest, native crate, scenarios, tests |
| 2026-11-10T18:25Z | claude-4d1e7a | checkpoint | 541804a | Steps 3 and 4: fixture peer, recorded fixture, RTP interval and Zeek checks |
| 2026-11-10T18:45Z | claude-4d1e7a | checkpoint | 9b7fa8d | Step 5: README, log entry, Modules page section |
| 2026-11-10T19:05Z | claude-4d1e7a | review | 3f97b8a | PR #417 opened, all 32 CI checks green (the realism job ran Zeek 9.0.1: sip.log 57 rows, no weird entry) |
