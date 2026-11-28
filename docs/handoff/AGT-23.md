# Handoff AGT-23 The agent leases the addresses of a DHCP pool

| Field | Value |
|---|---|
| Issue | #635 |
| Branch | `wp/AGT-23` |
| Pull request | #729 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-a23c7e, lease until 2026-11-28T08:28:34Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent leases the addresses of a DHCP host pool through the lease phase of TRN-37, on the user space backend, filters a DHCP port by the leased addresses, and refuses dhcp4 on the kernel backend with the cause of an unforced fallback. dhcp_clients.yaml runs through a forced AF_XDP agent in the namespace harness (bug #708).

## Plan

- [ ] 1. assemble builds HostPool::dhcp; check_af_xdp accepts a DHCP client pool; kernel refusal names dhcp4.
- [ ] 2. AF_XDP backend waits for the lease phase (timeout error names dhcp4, pool, bound count).
- [ ] 3. Pools::addresses reads the lease table; capture filter keeps a session only when an address is leased; neighbor table follows the lease generation.
- [ ] 4. tests/fallback.rs EXPECTED row and the dhcp test; fallback cause in the kernel refusal error.
- [ ] 5. Namespace harness test, docs (Agent page), log entry, PR, CI.

## Done and verified

- Kernel refusal with the fallback cause (`backend::check_kernel_dhcp`), the virtual source pool in 240.0.0.0/4, `LeaseMap` in the transport, the lease phase and `DatapathConfig::dhcp` in `backend::launch`, the live-board capture filter (`portfilter::LeaseCell`), the stricter `EXPECTED` row, the Agent page, and `tests/afxdp_dhcp.rs`.
- `cargo test -p fps-agent` (each target, as root): all pass, including `afxdp_dhcp` (200 leases, 84 of 84 sessions, every server-side client address leased, none in 240.0.0.0/4). `cargo clippy -p fps-agent --all-targets -D warnings` and `cargo fmt --check` clean.

## Remaining

Review and merge. CI is green on fcfcbc0e.

Known debt: the virtual pool (240.0.0.0/4) encodes a host index as an address because SourceHosts and the transport trait live in engine/fps-runtime/.

## Next action

Open the PR and poll CI.

## Blockers and open questions

none (bug #724 merged)

## Decisions taken in this package

- Virtual source pool 240.0.0.0/4 for a leasing client pool, mapped on the live lease boards at each connect by the transport (vetting agent, log 2026-11-28T0730). Known debt: the encoding exists because `SourceHosts` and the transport trait live in `engine/fps-runtime/`.
- `socket2` dev dependency for the harness test, so Cargo.lock gains one edge.

## Pitfalls

- `LeaseBoard::addr_of` takes the absolute pool index.
- The disk of the machine fills when all test binaries of fps-agent link at once; build one `--test` target at a time.
- A `LeaseTable` cannot be filled from outside `fps-hosts`, so unit tests use a fake `LeaseLookup` and `LeaseIndex`.

## Host test

`cargo test -p fps-agent --test afxdp_dhcp -- --nocapture`, needs root (or sudo), `ip`, AF_XDP, timeout 300 s, expected: `test result: ok. 1 passed`, and the log lines `run outcome=pass`, `server leased=200`.

## Verify the current state

```bash
cd engine/fps-agent && cargo test -p fps-agent --lib
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-28T08:28Z | claude-a23c7e | claim | none | claimed #635 with 5 plan steps |
| 2026-11-28T08:50Z | claude-a23c7e | block | see git log | deliverable 2 done; the rest needs LeaseMonitor::boards in fps-datapath |
| 2026-11-28T09:24Z | claude-a23c7e | takeover | see git log | bug #724 merged, design approved; implemented all deliverables |
| 2026-11-28T10:00Z | claude-a23c7e | review | see git log | PR #729, CI green, ready for review |
