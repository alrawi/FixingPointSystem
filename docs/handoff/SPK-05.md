# Handoff SPK-05 Spike smoltcp behind our own flow table

| Field | Value |
|---|---|
| Issue | #620 |
| Branch | `wp/SPK-05` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-03a72e |
| Last verified commit | none |

## Goal

Decision record 0014 names smoltcp behind a flow table and a timer wheel of our own as the fallback of the own TCP stack, and says it is unverified. This spike answers one question: does a smoltcp socket process frames outside its interface loop, and what does one event cost at 16,000 sockets. Deliverable: `spikes/s5-smoltcp-flow-table/` with a prototype and `RESULTS.md`.

## Plan

- [ ] 1. Claim, handoff, read smoltcp 0.14 API for ingress, egress, and timers.
- [ ] 2. Prototype: flow table, one socket set per flow, timer wheel, frame device.
- [ ] 3. Scenarios and verification: handshake, echo, loss and retransmission, close and TIME-WAIT.
- [ ] 4. Measure at 1,000 to 64,000 sockets, with the interface loop as baseline; write RESULTS.md.
- [ ] 5. Lints, log entry, pull request, CI.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Write the prototype crate in `spikes/s5-smoltcp-flow-table/flowtable/`.

## Blockers and open questions

none

## Decisions taken in this package

- The crate sits one directory down (`flowtable/`) with its own `[workspace]` table and `Cargo.lock`, because the root `members` glob `spikes/*` would otherwise pull a crate at the spike root into the workspace and change the shared `Cargo.lock`.

## Pitfalls

none yet

## Verify the current state

```bash
cd spikes/s5-smoltcp-flow-table/flowtable && cargo test
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-28 | claude-03a72e | claim | none | Lease taken on #620 |
