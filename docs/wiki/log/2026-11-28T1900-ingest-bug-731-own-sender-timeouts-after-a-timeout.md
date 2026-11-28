## [2026-11-28] ingest | bug 731 The own sender without SACK ran into a second and third timeout

Issue #731: the row "own client to kernel server, own options off, bottleneck" of `interop_kernel` ran at 4.56 Mbit/s once in about 100 runs (858 data segments for 720) and passed, because it had no goodput floor.

Where. Sandbox with `ip` and `tc` (tbf, no netem), the row alone in a loop, two to four busy loops as load, and a trace of the sender events written to a file (timeouts, fast retransmits, every ACK, duplicate ACK and data segment). A run takes 0.4 s; without load about 1 in 500 runs had a timeout, with four busy loops about 1 in 80, and in about 500 runs under load 3 had two timeouts (8.1 to 8.2 Mbit/s) and one three (4.56 Mbit/s, 858 data segments: the same run as the issue).

Cause, in the 4.56 Mbit/s run. The sender is NewReno on CUBIC without SACK, and the receive window of 64 KiB (44 segments) is twice the queue of 22 segments.
1. Slow start overshoots, 22 segments drop. The fast retransmit goes out at 21.7 ms, and the segment that was sent again is dropped too (the queue is still full). No partial ACK comes, and the first timeout fires at 222 ms. That one is inherent to NewReno (a run with this timeout alone takes 0.63 s, 13 Mbit/s).
2. The timeout goes back N. The kernel holds half of the old flight out of order, so the ACK of the first segment sent again jumps over 17 segments. Slow start added the bytes acknowledged, 17 segments, so the window doubled from 24.8 KB to 44 KB and the sender sent 29 segments at once into the queue of 22. Bytes that the peer already held were counted as news.
3. Segments of that burst were dropped, and the duplicate ACKs (15 to 22 of them, every ACK with the same number) did not start a fast retransmit, because the ACK number stood below `recover`, the end of the old flight (RFC 6582 section 3.2 step 2). The rule is there because the old segments sent again draw duplicates from a peer that holds them. In this case the hole was the last old segment, or one of the last ones, and the duplicates came from new data.
4. Second timeout at 641 ms (the timeout was 400 ms, doubled at the first and kept until a segment that was never sent again is acknowledged, RFC 6298 5.7), the same again, third at 1497 ms (800 ms).

Fix (engine/fps-tcp-own/src/recovery.rs).
- `NewReno::on_new_ack_at`: an ACK that stays within the old flight after a timeout (`ack <= recover`) is worth at most two segments of window growth (RFC 3465 L=2). Unit test `an_acknowledgment_that_jumps_over_the_old_flight_grows_the_window_by_two_segments`.
- `NewReno::on_dupack`: below `recover` the duplicates of the old segments above the hole (at most `ceil((recover - ack) / mss) - 1`) are not counted; the third duplicate beyond them starts the fast retransmit. They can only come from data that was sent after the timeout. Unit test `duplicates_beyond_those_of_the_old_flight_start_a_recovery_below_recover`. A peer that sends fewer duplicates than that waits for the timeout as before.

After the fix, same load (four busy loops), about 1000 runs of the row: no run with more than one timeout, the slowest 13.1 Mbit/s (single timeout), the rest 17 to 19.5. Before, at that load, 3 of about 500 below 9.

Floor. The floor of 9 Mbit/s (bug #699, own server rows without SACK) now covers every row that pins the kernel to CUBIC, the own client rows without SACK included. A single timeout (13 Mbit/s) passes, two or more fail. 100 runs of the whole binary: see the pull request.

Not fixed: the first timeout (step 1, the retransmission lost in a full queue) is inherent to NewReno without SACK, and the row still has it about once in 100 runs.

### Shared page notes

- Testing: the floor of 9 Mbit/s of `interop_kernel` applies to every row that pins the kernel to CUBIC (the rows without SACK and the TRN-34 baseline), in both directions.
- Module Development, "Watch out for": nothing for module authors.
