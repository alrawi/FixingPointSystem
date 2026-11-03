# Handoff TRN-02 Kernel UDP backend

| Field | Value |
|---|---|
| Issue | #32 |
| Branch | `wp/TRN-02` |
| Pull request | #114 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-trn-02-d41d, lease until 2026-11-03T16:20:14Z |
| Last verified commit | the head of `wp/TRN-02` at review |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement `DatagramIo` over kernel UDP sockets bound to pool addresses in `engine/fps-backend-kernel`, with connected sockets for clients, bound sockets for servers, and a `DatagramOpener` for the interpreter. A loopback test exchanges 10,000 request and response pairs from 100 source addresses in 127.0.0.0/8.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, and pull request
- [x] 2. Datagram socket, connector, and listener with unit tests
- [x] 3. Interpreter openers and loopback component tests, including the acceptance test
- [x] 4. Transport wiki section, log entry, `make lint test`, CI, and review

## Done and verified

Every step passed the commands below on Rust stable and Linux 6.18.

| Step | Command | Result |
|---|---|---|
| 2 | `cargo test -p fps-backend-kernel --lib` | 18 unit tests pass, three of them new |
| 3 | `cargo test -p fps-backend-kernel --test udp_loopback` | 9 component tests pass in about 0.6 seconds, and the binary passed 50 repeated runs |
| 3 | `cargo test -p fps-backend-kernel` | the TCP loopback tests still pass |
| 4 | `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` | pass |
| 4 | `python3 tools/plan/plan.py check`, `render --check`, and `lint` | pass |

## Remaining

Nothing inside the package. We record four gaps.

- A client that reuses the five tuple of a session that the server still holds open talks to the old session. UDP has no close, so the backend cannot tell the two apart. The acceptance test avoids it by keeping each client socket open until the next one is bound.
- A stray datagram that reaches a server session socket after its close drain is lost. The window lasts microseconds, and no test run hit it.
- `MSG_TRUNC` and the stray handling assume Linux.
- No runtime wires UDP flows yet. The two openers are ready for the worker runtime of `fps-runtime`.

## Next action

Review pull request #114 against the Definition of Done.

## Blockers and open questions

None.

## Decisions taken in this package

- Server sessions are connected sockets in a reuse port group with the listener, so the kernel filters each peer and reports ICMP errors per session. The alternative, one shared unconnected socket with a user space demultiplexer, would need one task to drive all receives and could not report ICMP errors per peer.
- Linux delivers datagrams of new peers to a session socket between its bind and its connect, and a classic BPF reuse port program that always picks the listener did not stop that once the group held connected sockets. We dropped the program and let server sessions hand such strays back to the listener.
- The Tokio poll path does not wake a reader on `EPOLLERR`, which is the only event of ICMP port unreachable on a connected UDP socket, so each session keeps one boxed `ready(Interest::ERROR)` future while a receive waits.
- `socket2` receives into `MaybeUninit` buffers, so the existing `sys` module gained one cast with a `SAFETY` comment in place of a raw `recv` call.

## Session log

- 2026-11-03 claude-trn-02-d41d claimed the package, implemented the UDP sessions, the listener, and the openers, found and fixed the stray delivery and the missing error wakeup with the acceptance test, and wrote the Transport section and the log entry.
