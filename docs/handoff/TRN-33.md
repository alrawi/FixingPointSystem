# Handoff TRN-33 The own stack API that the datapath needs

| Field | Value |
|---|---|
| Issue | #658 |
| Branch | `wp/TRN-33` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-7c2e91, lease renewed at each heartbeat |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Close the `fps-tcp-own` API gaps that TRN-27, TRN-29, and TRN-30 worked around: `OwnSession::local_addr` and `peer_addr` (so `Listener::accept` returns the peer as `KernelListener::accept` does), `OwnStack::unlisten` (a dropped listener frees its port), and `Pools::tables` (one source for the neighbor tables). The datapath and the UDP layer move onto them. An input and take_tx seam per side is optional.

## Plan

- [ ] 1. Claim, read the code of the three crates
- [ ] 2. OwnSession addresses and OwnStack::unlisten with tests
- [ ] 3. Pools::tables, and the datapath and the UDP layer moved onto it
- [ ] 4. Datapath Listener::accept returns the peer, listener drop frees the port, duplicate-table test removed
- [ ] 5. Checks, log entry, component page, PR

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read `engine/fps-tcp-own/src/lib.rs`, `host.rs` (listen, Tuple), `pools.rs`, and `engine/fps-datapath/src/lib.rs`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-TRN-33 CARGO_BUILD_JOBS=1 cargo test -p fps-tcp-own -p fps-udp-own -p fps-datapath
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T13:30Z | claude-7c2e91 | claim | none | claimed #658 with card 6979999929 |
