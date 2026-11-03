# Handoff TRN-01 Kernel TCP backend

| Field | Value |
|---|---|
| Issue | #31 |
| Branch | `wp/TRN-01` |
| Pull request | #88 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-trn-01-0b1a, lease until 2026-11-03T04:13:37Z |
| Last verified commit | the head of `wp/TRN-01` at review |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement `SessionIo` over kernel TCP sockets in the new crate `engine/fps-backend-kernel`. Clients connect from host pool source addresses with `IP_BIND_ADDRESS_NO_PORT` and a configured local port range, servers accept on pool addresses with a configurable backlog, and both apply buffer size, Nagle, and linger options. A loopback test opens 10,000 sessions from 100 source addresses in 127.0.0.0/8 without address errors.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, and pull request
- [x] 2. Crate skeleton with options, errors, and the socket option helper
- [x] 3. Connector, listener, and `KernelStream` with abort, with unit tests
- [x] 4. Loopback component tests, including the 10,000 session acceptance test
- [x] 5. Transport wiki page, log entry, `make lint test`, CI, and review

## Done and verified

Every step passed the commands below on Rust stable and Linux 6.18.

| Step | Command | Result |
|---|---|---|
| 2, 3 | `cargo test -p fps-backend-kernel --lib` | 5 unit tests pass |
| 4 | `cargo test -p fps-backend-kernel --test loopback` | 10 component tests pass in about one second, three repeated runs |
| 5 | `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` | pass |
| 5 | `python3 tools/plan/plan.py check`, `render --check`, and `lint` | pass |
| 5 | `make lint` and every `TEST_TARGETS` entry except `test-spk` | pass; locally `test-spk` hung in the S1 test `pattern_split_across_reads` under load from other agents, and the CI `rust` job, which runs it, passed |
| 5 | PR #88 CI at 887124b | all nine checks pass |

## Remaining

Nothing inside the package. We record three gaps.

- The IPv6 test skipped itself locally because the sandbox has no IPv6 loopback. CI on ubuntu-24.04 runs it.
- `IP_LOCAL_PORT_RANGE` needs Linux 6.3 or newer, and older kernels fail every connect that names a range.
- The crate uses `unsafe` in `src/sys.rs` for two socket options, which the brief did not expect. The owner may prefer another route.

## Next action

Review pull request #88 against the Definition of Done.

## Blockers and open questions

None.

## Decisions taken in this package

- `socket2` 0.6.5, Tokio, `rustix`, and `nix` have no setter for `IP_LOCAL_PORT_RANGE`, and only `nix` has one for `IP_BIND_ADDRESS_NO_PORT`. We call `setsockopt` through `libc` in one private module under `#![deny(unsafe_code)]` instead of adding `nix` for half of the need.
- The port range is per socket through `IP_LOCAL_PORT_RANGE`, so it combines with `IP_BIND_ADDRESS_NO_PORT` and the kernel shares a port across 4-tuples. A user space allocator would bind explicit ports and lose that sharing.
- Accepted sessions inherit options from the listener, which the options test verifies, so `accept` sets nothing per session.
- `SocketOptions::default()` turns Nagle off, as spike S1 did.
- `Linger::Timeout` exists but its documentation warns that close then blocks the worker thread.

## Pitfalls

- Parallel agents share `/tmp`, and golangci-lint refuses to run while another copy holds its lock there. Set `TMPDIR` to a private directory before `make lint`.
- Tokio deprecates `TcpStream::set_linger`, so the crate sets linger through `socket2::SockRef`.

## Verify the current state

```bash
git log --oneline -1
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-trn-01-0b1a | claim | | card 6594096558 |
| 2026-11-03T02:49Z | claude-trn-01-0b1a | checkpoint | | crate and tests pushed |
| 2026-11-03T03:05Z | claude-trn-01-0b1a | finish | | ready for review |
| 2026-11-03T03:30Z | claude-trn-01-0b1a | checkpoint | 887124b | CI green, local checks recorded |
