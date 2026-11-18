# Handoff TRN-33 The own stack API that the datapath needs

| Field | Value |
|---|---|
| Issue | #658 |
| Branch | `wp/TRN-33` |
| Pull request | #0 |
| Status | in progress |
| Progress | 60% (3 of 5 plan steps) |
| Current holder | claude-7c2e91, lease renewed at each heartbeat |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Close the `fps-tcp-own` API gaps that TRN-27, TRN-29, and TRN-30 worked around: `OwnSession::local_addr` and `peer_addr` (so `Listener::accept` returns the peer as `KernelListener::accept` does), `OwnStack::unlisten` (a dropped listener frees its port), and `Pools::tables` (one source for the neighbor tables). The datapath and the UDP layer move onto them. An input and take_tx seam per side is optional.

## Plan

- [x] 1. Claim, read the code of the three crates
- [x] 2. OwnSession addresses and OwnStack::unlisten with tests
- [x] 3. Pools::tables, and the datapath and the UDP layer moved onto it
- [x] 4. Datapath Listener::accept returns the peer, listener drop frees the port, duplicate-table test removed
- [ ] 5. Checks, log entry, component page, PR

## Done and verified

- `cargo test -p fps-tcp-own --test api --test own` passes (6 and 33 tests); `cargo test -p fps-datapath --test memory --test listener` passes (11 and 3).
- fps-tcp-own: `Host::unlisten`/`addrs`, `OwnStack::unlisten`, `OwnSession::local_addr`/`peer_addr` (io::Result, as the kernel stream), `Pools::tables` returning `Tables`, `Pools::stations` built from it.
- fps-datapath: `responders` uses `Pools::tables`; `Listener::accept` returns `(session, peer)`; dropping a `Listener` unlistens on every lane; failed `listen` undoes lanes already listening.

## Remaining

Checks (fmt, clippy, workspace crates that call `Listener::accept`), log entry, Transport page, PR.

## Next action

Run clippy and the tests of fps-udp-own, then write the log entry and the Transport page notes.

## Blockers and open questions

none

## Decisions taken in this package

- `OwnSession::local_addr` and `peer_addr` return `io::Result<SocketAddr>` (not `Option`), like `KernelStream`, so AGT-25 can treat both sessions alike.
- `unlisten` resets queued and half open connections of the port, like closing a listening socket on Linux; accepted sessions are untouched.
- `Listener::accept` changed its return type, so the two callers outside the package paths (`engine/fps-agent/tests/afxdp_veth.rs`, `engine/fps-backend-afxdp/tests/datapath_veth.rs`) got a one line mechanical edit to compile.

## Pitfalls

none

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-TRN-33 CARGO_BUILD_JOBS=1 cargo test -p fps-tcp-own -p fps-udp-own -p fps-datapath
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T13:30Z | claude-7c2e91 | claim | none | claimed #658 with card 6979999929 |
| 2026-11-18T13:40Z | claude-7c2e91 | checkpoint | see git log | steps 2 to 4 done, tests pass |
