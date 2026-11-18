# Handoff TRN-30 IPv6 in the own stack

| Field | Value |
|---|---|
| Issue | #631 |
| Branch | `wp/TRN-30` |
| Pull request | none yet |
| Status | in progress |
| Progress | 20% (1 of 6 plan steps) |
| Current holder | claude-1d4805, lease until 2026-11-18T13:31:09Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

IPv6 for the own TCP stack (`fps-tcp-own`), the user space UDP layer (`fps-udp-own`), and the datapath (`fps-datapath`, which answers ND for the host pools). The loss workload (TRN-22) and the dns scenarios (TRN-29) pass over IPv6 on a veth pair in CI.

## Plan

- [x] 1. Read the code of the three crates and decide the address and header types.
- [x] 2. IPv6 in fps-tcp-own (wire, host, pools, net).
- [ ] 3. IPv6 in fps-udp-own.
- [ ] 4. ND in fps-datapath and the IPv6 connect and listen calls.
- [ ] 5. Tests in memory and on a veth pair.
- [ ] 6. Checks, log entry, Transport page, pull request, CI.

## Done and verified

Step 2: wire, net, pools, host, lib of fps-tcp-own speak IPv4 and IPv6; `cargo test -p fps-tcp-own` passes (163 unit, 30 integration, 6 new IPv6 integration tests), clippy clean.

## Remaining

Steps 3 to 6.

## Next action

Start step 3: IPv6 in engine/fps-udp-own (wire.rs, arm.rs).

## Blockers and open questions

none

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
| 2026-11-18T12:01Z | claude-1d4805 | claim | none | claimed issue 631 |
