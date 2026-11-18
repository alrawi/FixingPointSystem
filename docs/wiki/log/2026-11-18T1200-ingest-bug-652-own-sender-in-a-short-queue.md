## [2026-11-18] ingest | 652 own sender collapses into a short tail-drop queue with options on

Bug 652 (found by TRN-26): the own client with window scale, SACK, timestamps, and CUBIC reached 3.61 Mbit/s with 517 retransmissions through a 20 Mbit/s bottleneck with a 32 KiB tail-drop queue, against 19.4 Mbit/s with the options off. It edits only `engine/fps-tcp-own/`, `engine/fps-tcp/tests/`, its handoff, this entry, and `Transport.md`.

- Regression test `engine/fps-tcp/tests/bottleneck.rs` (in memory, no root): own to own on that link, best of three runs each, options on at least 80 percent of options off, retransmissions within twice those of the run without options plus 40, at most one timeout. Before the fix: 5.31 against 18.75 Mbit/s, 746 retransmissions, 7 timeouts (the test fails). After: 18.75 against 18.63, 48 against 37, 0 timeouts.
- Cause 1, `sack.rs` and `host.rs`: a retransmission that the full queue dropped again was counted in the pipe until the timer fired, while the recovery kept sending new data, so the flight grew to the receive window. The scoreboard now finds a lost copy when data sent more than two segments after it was selectively acknowledged (`mark_rexmit_at`, `detect_lost_rexmit`, counter `sack_lost_rexmits`).
- Cause 2, `cubic.rs` and `recovery.rs`: the reduction used the flight, which a SACK recovery grows beyond the window. It now uses `min(cwnd, flight)`, and a timeout in a recovery never raises the threshold of that recovery.
- Unit tests: two in `sack.rs` (a lost copy, a copy counted once) and one in `cubic.rs` (the reduction from the window).
- Kernel row (sandbox, root, no netem), own client to kernel server, bottleneck: 19.2 Mbit/s with 43 to 67 retransmissions after, 7.4 Mbit/s with 192 to 254 before.
- TRN-23/24/25 rows, A/B in one sandbox session (own stack, release build, main against the fix, means): default loss row 0 / 0.1 / 1 percent 52.79 / 19.68 / 2.85 against 53.01 / 19.63 / 2.83 Mbit/s; `--loss-profile fat --loss-transfers 6` 19.80 against 19.75 Mbit/s; `--loss-profile bottleneck` goodput 891.0 for both, Jain index 0.978, 0.933, 0.957 on main against 0.910, 0.964, 0.979 with the fix (run to run spread). No regression. The kernel rows need netem and the hosted runner.

Findings:

- The own sender with options on still stalls twice against smoltcp 0.14 on this link: the assembler of smoltcp keeps four out of order ranges, drops the rest without a SACK block, and that tail stays in the pipe until the timer fires. Time based loss detection (RACK, RFC 8985) would end it. Not in this bug.
- The own stack sends 40 to 70 segments again on this link where the kernel sends 9: no pacing, no HyStart, no tail loss probe. The expected value of the issue (at most twice the kernel's) is therefore not met in count; the goodput and the collapse are.

### Shared page notes

- Module development, "Watch out for": nothing for module authors.
- Testing page: `cargo test -p fps-tcp --test bottleneck` runs in memory in about 6 s and needs no root.
