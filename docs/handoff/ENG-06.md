# Handoff ENG-06 UDP datagram sessions

| Field | Value |
|---|---|
| Issue | #26 |
| Branch | `wp/ENG-06` |
| Pull request | pending |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-eng-06-bc2f |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add a datagram variant of the session I/O seam and the UDP semantics of contract v0.2 to the interpreter, with an in memory implementation for tests.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. `DatagramIo`, `DatagramIoExt`, `MAX_DATAGRAM`, and the in memory `datagram_pair` in `fps-io`, with unit and property tests
- [x] 2. `DatagramOpener` and the UDP `send`, `expect`, and `close` in `fps-interp`, sharing the op loop with TCP
- [x] 3. Interpreter tests with the DNS lookup example, the datagram limit, and the timeouts
- [ ] 4. Engine page section, log entry, handoff, and pull request

## Done and verified

- Steps 1 to 3. `cargo test -p fps-io -p fps-interp` passes, with 15 new interpreter tests in `tests/datagrams.rs`, 3 new component and property tests in `fps-io/tests/datagram.rs`, and 6 new `fps-io` unit tests. `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo deny check` are clean. No dependency was added.

Acceptance.
- A DNS shaped request and response program passes over the in memory implementation. `the_dns_lookup_example_completes` runs `contracts/examples/modules/dns/scenarios/lookup.yaml` with both sides over `datagram_pair`, and `each_send_is_one_datagram_and_last_is_the_whole_datagram` echoes DNS shaped queries through `last`.
- Datagram size limits have tests. `a_datagram_above_max_datagram_fails_the_session`, `the_default_limit_accepts_the_largest_udp_payload`, `a_send_above_the_udp_limit_fails_the_session`, and `set_max_datagram_clamps_to_the_udp_range`.
- Timeouts have tests. `expect_times_out_when_no_datagram_arrives`, `a_lost_datagram_is_not_retransmitted` (expect and accept timeouts), and `the_timeout_bounds_each_datagram_not_the_whole_answer`.

## Remaining

- The per action `max_datagram` needs `fps-flow` to carry it in `Op::Expect`, which is outside this package's paths. The session limit applies until then, see the log entry follow ups.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- `poll_recv` returns the full datagram length and truncates into the buffer, as `MSG_TRUNC` does, so the interpreter detects oversized datagrams with one buffer.
- The datagram limit is a `Session` setter, not a `Limits` field, because adding a field to the public `Limits` struct would break struct literals in ENG-10.
- TCP and UDP share one op loop through a private `Wire` enum, so the futures stay plain async functions without trait objects.
- `ConnectionRefused` on an established UDP socket, which is ICMP port unreachable, maps to reason `connect_refused`.

## Session log

| Time (UTC) | Agent | Summary |
|---|---|---|
| 2026-11-03T12:55Z | claude-eng-06-bc2f | Claimed the package, built the seam, the interpreter changes, and the tests |
