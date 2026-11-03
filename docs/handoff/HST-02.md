# Handoff HST-02 Kernel port setup

| Field | Value |
|---|---|
| Issue | #35 |
| Branch | `wp/HST-02` |
| Pull request | #95 |
| Status | ready for review |
| Progress | 100% (6 of 6 plan steps) |
| Current holder | claude-hst-02-6b05, lease until 2026-11-03T13:08:39Z |
| Last verified commit | see the finish row of the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We configure a test port for the kernel backend. Each port gets its own network namespace, a local route for the pool prefix so sockets can bind any pool address, the gateway route, an optional VLAN subinterface, and a sysctl profile for the local port range, TIME_WAIT reuse, and socket buffers. Setup and teardown use netlink and never touch the management interface.

## Plan

- [x] 1. Port specification, validation, and sysctl profile model with unit tests
- [x] 2. Netlink setup and teardown behind a narrow trait, with a recording fake for component tests
- [x] 3. Integration test in the QA-01 harness that binds and connects from 20,000 pool addresses
- [x] 4. Teardown check that no namespace, route, or link remains
- [x] 5. mk/hst.mk and .github/workflows/hst.yml that run the test as root
- [x] 6. Hosts wiki section, log entry, and full checks

## Done and verified

- Steps 1 and 2 are verified by `cargo test -p fps-hosts`, with unit tests in `src/port.rs`, `src/sysctl.rs`, and `src/ipcmd.rs` and 8 component tests in `tests/port_component.rs` against `FakeIp`.
- Steps 3 and 4 are verified by `make e2e-hst` as root. The routed test opens 20,000 TCP sessions from 20,000 distinct pool addresses in about one second, and every test compares snapshots of links, addresses, and routes before setup and after teardown.
- Step 5 is verified by the `kernel-port` job of `.github/workflows/hst.yml` on pull request #95, which also ran the VLAN test with `FPS_REQUIRE_VLAN=1`.
- Step 6 is verified by `make lint test` and `tools/plan/plan.py lint`.

## Remaining

Nothing in scope. Known limits follow.

- The integration tests cover IPv4 only, because the QA-01 harness has no IPv6 arm. IPv6 setup is covered by unit and component tests.
- The on-link tests use 500 hosts, because the host wide neighbor limit `gc_thresh3` is 1,024 entries. The 20,000 host test uses a routed port.
- `net.core.rmem_max` and `net.core.wmem_max` are optional, because Linux 6.x refuses to raise them above the host value in a child namespace.
- There is no formal performance baseline yet. The integration test prints setup, session, and teardown times.

## Next action

Review pull request #95.

## Blockers and open questions

none

## Decisions taken in this package

- We drive iproute2 through the narrow trait `IpRunner` instead of the `rtnetlink` crate. Namespace creation and in namespace sysctls need `setns`, which needs unsafe code that the crate forbids, and `rtnetlink` would add tokio to the crate. The Hosts page records the reasons.
- Local routes cover exactly the slice addresses through minimal aligned prefixes and skip the gateway. A `throw` route in the local table does not work, because the kernel merges the local and main tables without policy rules.
- Setup refuses an interface with a global address or a default route, so the caller must hand over a bare interface.
- Optional sysctls that are missing or clamped are skipped and reported by `KernelPort::skipped_sysctls`.
- Teardown keeps the namespace when the interface cannot move back, because deleting it would destroy a veth.
- The integration test runs its client and server as child processes of the test binary under `ip netns exec`, so it needs no `setns`. It adds `socket2` as a development dependency for bind before connect, which changes `Cargo.lock`.

## Pitfalls

- libtest prints helper output on the same line as `test helper_process ...`, so the parent searches for the marker anywhere in a line.
- A veth that moves back reports no carrier for a moment, so the snapshot comparison polls for up to three seconds.
- The local development container has no 8021q module, so the VLAN test skips there and runs only in CI.

## Verify the current state

```bash
cargo test -p fps-hosts
sudo make e2e-hst
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:39Z | claude-hst-02-6b05 | claim | none | Claimed issue #35 |
| 2026-11-03T12:10Z | claude-hst-02-6b05 | finish | see branch head | Ready for review, CI green |
