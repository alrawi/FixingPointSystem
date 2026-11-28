# Handoff AGT-23 The agent leases the addresses of a DHCP pool

| Field | Value |
|---|---|
| Issue | #635 |
| Branch | `wp/AGT-23` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
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

Nothing yet.

## Remaining

Everything.

## Next action

Read engine/fps-agent/src/backend.rs layout and transport.rs, then start step 1.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cd engine/fps-agent && cargo test -p fps-agent --lib
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-28T08:28Z | claude-a23c7e | claim | none | claimed #635 with 5 plan steps |
