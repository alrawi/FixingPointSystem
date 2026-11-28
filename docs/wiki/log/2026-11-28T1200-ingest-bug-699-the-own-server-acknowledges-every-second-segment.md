## [2026-11-28] ingest | bug 699 The own server acknowledges every second segment on arrival

Issue #699: the rows "kernel client to own server, kernel options off, bottleneck" and "own options off, bottleneck" of `interop_kernel` sometimes ran at 0.6 Mbit/s (25 s instead of 1 s) and passed, because they had no goodput floor. In the sandbox (`ip` and `tc` installed, veth and tbf) the rows were slow in 18 of 40 runs.

Cause. The server did not stall: no zero window (the field read 65535 all the time, the buffer is 1 MiB), no persist state, no delayed ACK timer (`delayed_acks` 0), and it answered every segment. The ACK cadence was wrong. `Host` acknowledged in order data once per flush, and the flush runs once per batch of frames that a pump pass takes from the link, so one ACK covered 4 to 8 segments, and after a hole 20 to 38. A stretch ACK opens the window of a sender (44 segments without window scaling) at once, the sender sends the burst back to back, and the 32 KiB tail drop queue drops it (205 of 955 segments in one run, read at the router). A kernel sender without SACK then repairs one hole per round trip, loses a retransmission into the full queue (`ss -ti`: `retrans:0/4 lost:1 sacked:22`, then silence), and waits for the retransmission timeout, which doubles (200 ms, 400, 800, up to 6.9 s in all). In the slow run the own server held 160 to 270 segments out of order, in a fast run 0 to 60.

Fix. `Host::handle` sends the ACK at once for a segment that arrives while an ACK is owed (`O_UNACKED` or `ack_pending`), so at least every second full sized segment gets its ACK (RFC 5681 section 4.2) inside one batch too. The unit test `a_batch_of_segments_gets_an_acknowledgment_for_every_second_one` feeds five segments before the flush and fails without the fix (no ACK before the flush). Kernel client rows without SACK, sandbox, before and after: 18 of 40 rows below 6 Mbit/s, and 8 of 60.

What is left is the kernel sender. Kernel to kernel with the options off on the same link (no own stack) ran at 1 Mbit/s in 30 of 100 runs, because the sandbox kernel sends with BBR. With CUBIC the rows ran at 19.4 Mbit/s in 40 of 40 runs even before the fix, and 43 of 43 after it. The rows without SACK therefore pin the kernel peer to CUBIC (`KernelInteropConfig::kernel_cc`, `--cc` of the peer), as the TRN-34 results already do, and the rows of the own server without SACK have a goodput floor of 9 Mbit/s (they run at 13 to 19.5). The test also removes stale `fpsix*` namespaces at its start.

Evidence: 100 runs of the whole binary in the sandbox with BBR as the host default: see the pull request. The two own server rows ran at 19.3 to 19.5 Mbit/s in every run, and the run took 9.0 s (minimum 8.9, maximum 9.1).

Not fixed, found on the way: the own sender without SACK ("own client to kernel server, own options off, bottleneck") ran at 4.56 Mbit/s once in about 100 runs (858 data segments for 720). It has no floor, and the floor above is only for the own server.

### Shared page notes

- Testing: `interop_kernel` rows without SACK pin the kernel peer to CUBIC and the rows of the own server without SACK need 9 Mbit/s; the test removes stale `fpsix*` namespaces at its start, so two instances must not run at once.
- Module Development, "Watch out for": nothing for module authors.
