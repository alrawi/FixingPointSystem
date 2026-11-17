# Handoff TRN-18 A minimal own TCP stack on the bench harness

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #572 |
| Branch | `wp/TRN-18` |
| Pull request | #601 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-d57e44, lease until 2026-11-17T03:18:09Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Build a minimal TCP stack of our own behind the `TcpStack` seam of TRN-17 (connection setup with SYN, SYN-ACK, and ACK and an ISN, data transfer with sequence and acknowledgment numbers, a receive window, FIN and RST teardown, minimal retransmission documented as such), running over an in process frame pair so it needs no root, and measure it on the same harness and host as the kernel reference stack. The decision record 0014 belongs to TRN-10, not to this package.

## Plan

- [x] 1. Wire format and state machine (`src/own/wire.rs`, `src/own/host.rs`)
- [x] 2. The stack over `FrameIo`, the driver, and `SessionIo` sessions (`src/own/mod.rs`)
- [x] 3. `tcp_bench --stack own` and the tests over a lossy link (`tests/own.rs`)
- [x] 4. Sandbox results next to the kernel reference (`results/sandbox-2vcpu-trn18/`)
- [ ] 5. Wiki page, log entry, checks, pull request, and CI

## Done and verified

- `cargo test -p fps-tcp` (debug): 29 unit, 8 kernel, 23 own, and 5 workload tests pass.
- `cargo clippy -p fps-tcp --all-targets -- -D warnings` and `cargo fmt -p fps-tcp --check` are clean.
- `cargo deny check`: advisories, bans, licenses, and sources ok. The only new dependency is the path crate `fps-backend-afxdp`.
- Sandbox runs of `tcp_bench` for `own` and `kernel` are in `engine/fps-tcp/results/sandbox-2vcpu-trn18/` with `host.txt`: 0 failed sessions, 1,000,629 sockets for the own stack, no retransmission on the lossless link.

## Remaining

- The pull request, CI, and the review.
- The fat-nuc run (host test below), which only the relay can start.
- Known gaps of the stack, on purpose: no round trip estimate, fast retransmit, SACK, congestion control, window scaling, out of order queue, simultaneous open, or IPv6. See the module documentation of `src/own/mod.rs`.

## Next action

Open the pull request, wait for CI, fix failures inside `engine/fps-tcp/`, then run `fps_swarm.py review 572 --pr N`.

## Blockers and open questions

none

## Decisions taken in this package

- The link is `fps_backend_afxdp::FrameIo`, with the in memory pair as the default, so TRN-12 can run the same code over an AF_XDP queue and the stack is measured with the per frame costs that it will have there.
- Frames are raw IPv4 with TCP and no Ethernet header, with both checksums computed and verified, so the checksum cost is in the numbers.
- Timers are a heap with at most one entry per connection (lazy re-arming), so an idle poll does not walk the sockets. This is why the idle poll is flat, and it is a property of this design and not of own stacks in general.
- TIME-WAIT lasts one second by default (RFC 9293 says minutes), because a rate run would otherwise run out of 55,000 client ports. `--own-time-wait-ms` changes it.
- Connections are keyed by the two ports only, because a host has one peer.
- Unread data stays readable after the FIN of the peer and is freed when the session drops, after a bug where TIME-WAIT dropped it.
- The kernel reference was run again in the same session, because the sandbox booted differently since TRN-17 and TRN-10 compares only runs of one host. The TRN-17 files stay as they were.

## Pitfalls

- `sockets_open` of a user space stack includes TIME-WAIT connections, so a test cannot expect only the listener right after a rate run.
- CI runs `cargo fmt` and clippy with warnings as errors (`manual_is_multiple_of` and `unusual_byte_groupings` fired here), so run them before a push.
- The worktree guard refuses commands that mix `git` with other tools or heredocs in one line, so run `git` alone and write files with the editor tools.

## Verify the current state

```bash
cargo test -p fps-tcp                     # all tests pass, the own tests take about 3 s
cargo run --release -p fps-tcp --example tcp_bench -- --stack own --profile small
```

## Host test

Run on fat-nuc through the relay, with the same flags as TRN-17 so that TRN-10 compares one host. It needs no sudo, no namespaces, and no change to the host, and it builds the release profile in the clone.

| Item | Value |
|---|---|
| Commit | the head of `wp/TRN-18` after CI passed, or `main` after the merge |
| Argv 1 | `cargo run --release -p fps-tcp --example tcp_bench -- --stack own --profile full --label fat-nuc-own --out -` |
| Argv 2 | `cargo run --release -p fps-tcp --example tcp_bench -- --stack own --workload rate --threads 4 --label fat-nuc-own-4-threads --out -` |
| Argv 3 | `cargo run --release -p fps-tcp --example tcp_bench -- --stack own --workload throughput --threads 4 --label fat-nuc-own-4-threads --out -` |
| Sudo | no |
| Timeout | 1800 s for argv 1 (the build takes 1 to 3 minutes), 600 s for argv 2 and 3 |
| Memory | about 230 MB of resident growth for 1,000,000 sockets in the sandbox run, because the stack keeps about 230 B per socket in user space and uses no descriptors |
| Start window | not between 00:50 and 03:30 UTC, as decision D13 says |

The expected result of each argv is exit code 0 and a report on standard output that ends with an `fps-bench/v1` JSON document. Argv 1 prints `0 failed` for the session rate, the throughput, and a scaling table that reaches 1,000,000 sockets with an `idle_poll_us` that does not grow with the socket count and no `stopped` line. A nonzero exit code means failed sessions or a harness error, and the message names the cause. The coordinator posts the reports on the pull request, and the kernel reference runs of TRN-17 on the same host are the comparison. Nothing is committed from the host.

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-17 | claude-d57e44 | claim | bf32ef3 | Claimed #572 with 5 steps |
| 2026-11-17 | claude-d57e44 | checkpoint | f188564 | The own stack, tests, and tcp_bench --stack own. A container restart stopped the first session and the work resumed from this commit |
