# Handoff TRN-03 Operating system profiles for the kernel backend

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #33 |
| Branch | `wp/TRN-03` |
| Pull request | #99 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-trn-03-c222, lease until 2026-11-03T13:08:07Z |
| Last verified commit | the head of `wp/TRN-03` at review |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the OS profiles windows11, linux6, and macos14 as data files in `engine/fps-backend-kernel`, apply their TTL, MSS, and window clamp per socket, and document in a limits table which options the kernel backend cannot set per socket. A capture from the QA-01 namespace harness shows the configured TTL and MSS for each profile, and the workflow `trn.yml` runs it with sudo.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, and pull request
- [x] 2. Profile data files, the profile type, and per socket options with unit tests
- [x] 3. Probe example and loopback test
- [x] 4. Namespace capture test, `mk/trn.mk`, and `trn.yml`
- [x] 5. Transport page section, log entry, checks, CI, and review

## Done and verified

Every step passed the commands below on Rust stable and Linux 6.18, as root in the sandbox.

| Step | Command | Result |
|---|---|---|
| 2 | `cargo test -p fps-backend-kernel --lib` | 15 unit tests pass, 10 of them new |
| 3 | `cargo test -p fps-backend-kernel --test loopback` | 11 tests pass, including `os_profiles_reach_client_and_accepted_sockets` |
| 4 | `make build-trn-probe` and `sudo make e2e-trn` | every profile shows its TTL, MSS, window scale, SACK, timestamp, and DF settings in SYN and SYN-ACK, and the baseline shows TTL 64 and MSS 8960 |
| 4 | `make lint-trn test-trn` | shellcheck, ruff 0.16.9, mypy 2.3.1, and 5 pytest tests pass |
| 5 | `python3 tools/plan/plan.py check`, `render --check`, `lint`, and `make lint test` | pass |

## Remaining

Nothing inside the package. We record three gaps.

- The kernel cannot reproduce the option order or the exact SYN window of windows11 and macos14. The limits table on the Transport page lists every such option.
- The profiles carry `system` sysctls, but applying them to the namespaces of host pools belongs to HST-02. The capture test writes them itself.
- The Transport page still says in its earlier socket options section that `SocketOptions` has four fields, because the brief asked us to add only a section at the end. The new section names the three new fields.

## Next action

Review pull request #99 against the Definition of Done.

## Blockers and open questions

None.

## Decisions taken in this package

- Profiles are YAML, parsed with `serde_norway` as in `fps-flow`, `fps-hosts`, and `fps-stats`, so the lock file gains no new package.
- One `ttl` field covers IPv4 TTL and IPv6 hop limit, because the three systems use the same value for both.
- The window clamp encodes the reference window scale together with `system.tcp_rmem_max`, and a unit test recomputes the Linux rule for every profile.
- The capture test uses MTU 9000, so a correct MSS of 1460 cannot come from the route, and a baseline session without a profile proves it.
- `TCP_WINDOW_CLAMP` reuses the existing `set_u32` in `sys.rs`. The only new `unsafe` block is a test only `getsockopt`.

## Pitfalls

- `sudo` drops `CARGO_TARGET_DIR`, so pass `TRN_PROBE` to `make e2e-trn` when the build used another target directory.
- The probe server log file may not exist yet when the wait loop starts, so the loop uses `grep -qs`.

## Verify the current state

```bash
cargo test -p fps-backend-kernel                 # 15 unit and 11 loopback tests pass
make build-trn-probe && sudo make e2e-trn        # ends with profile capture test passed for windows11 linux6 macos14
make lint-trn test-trn                           # pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:38Z | claude-trn-03-c222 | claim | | card 6602800166 |
| 2026-11-03T11:50Z | claude-trn-03-c222 | finish | | ready for review |
