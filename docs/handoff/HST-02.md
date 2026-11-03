# Handoff HST-02 Kernel port setup

| Field | Value |
|---|---|
| Issue | #35 |
| Branch | `wp/HST-02` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-hst-02-6b05, lease until 2026-11-03T13:08:39Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We configure a test port for the kernel backend. Each port gets its own network namespace, a local route for the pool prefix so sockets can bind any pool address, the gateway route, an optional VLAN subinterface, and a sysctl profile for the local port range, TIME_WAIT reuse, and socket buffers. Setup and teardown use netlink and never touch the management interface.

## Plan

- [ ] 1. Port specification, validation, and sysctl profile model with unit tests
- [ ] 2. Netlink setup and teardown behind a narrow trait, with a recording fake for component tests
- [ ] 3. Integration test in the QA-01 harness that binds and connects from 20,000 pool addresses
- [ ] 4. Teardown check that no namespace, route, or link remains
- [ ] 5. mk/hst.mk and .github/workflows/hst.yml that run the test as root
- [ ] 6. Hosts wiki section, log entry, and full checks

## Done and verified

Nothing yet.

## Remaining

Every plan step.

## Next action

Write the port specification and sysctl profile model in `engine/fps-hosts/src/port.rs`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-hosts
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:39Z | claude-hst-02-6b05 | claim | none | Claimed issue #35 |
