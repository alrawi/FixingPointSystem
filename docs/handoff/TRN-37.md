# Handoff TRN-37 DHCP leases on the datapath

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #720 |
| Branch | `wp/TRN-37` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (6 of 7 plan steps) |
| Current holder | claude-00af87, lease until 2026-11-28T07:39:53Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Plug `fps_hosts::DhcpHosts` into the dhcp hook of the arm whose pool leases, with a lease phase the caller waits on. The own stacks, the ARP responder, and the route of the arm read the lease table and follow its generation. Issue #720, bug #708.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Lease board in fps-tcp-own (shared lease table with generation), Station and Pools follow it
- [x] 2. Stack reads leased addresses (connect by index, next pair)
- [x] 3. DHCP hook, lease phase, DatapathConfig in fps-datapath
- [x] 4. Splitter responder and Route follow the lease generation
- [x] 5. In memory tests (no root), timeout error test
- [x] 6. Harness test and CI
- [ ] 7. Docs, log entry, PR, review

## Done and verified

- `cargo test -p fps-datapath -p fps-tcp-own -p fps-udp-own --offline`: all pass, including `dhcp_memory` (5 tests).
- `cargo test -p fps-datapath --test dhcp_harness -- --nocapture` as root here: 8 of 8 bound, 8 of 8 sessions ok, 0 failed.
- `cargo clippy -p fps-datapath -p fps-tcp-own -p fps-udp-own --all-targets -- -D warnings` and `cargo fmt`: clean. `cargo check -p fps-agent --all-targets`: builds.

## Remaining

PR, CI, review hand off.

## Next action

Open the PR and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

See the log entry docs/wiki/log/2026-11-28T0900-ingest-trn-37-*.md. Key one: no new fields on HostSlice, Pools, or Lane because fps-agent builds them by literal.

## Pitfalls

The harness arms are in two namespaces; the test moves the server arm interface into the client namespace.

## Verify the current state

```bash
cargo test -p fps-datapath --test dhcp_memory   # 5 passed
cargo test -p fps-datapath --test dhcp_harness  # needs root
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-28T06:10 | claude-00af87 | claim | 502f726 | claimed #720 |
| 2026-11-28T06:55 | claude-00af87 | checkpoint | see git | steps 1 to 6 done, docs written |
