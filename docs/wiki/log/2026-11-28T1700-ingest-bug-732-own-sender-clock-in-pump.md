## [2026-11-28] ingest | bug 732 The own stack reads the clock before every step of a pump pass

Issue #732: the TRN-34 bound (own retransmissions <= 2 x kernel + 8) failed on the short queue under CUBIC.

Where. Rows on tbf, no netem (sandbox). Own client with the TRN-34 switches against the kernel server, baseline: kernel client to own server, bottleneck with tail drop. Kernel CUBIC: 7 to 13 segments sent again (median 8, 3 percent of runs 24 to 38). Own before the fix: median 6, but 5 of 60 runs at 31 to 51 (bound about 22). Under BBR (sandbox default) the kernel sends 100 to 140 again and the bound means nothing, so the baseline row now pins CUBIC.

Not spurious. In a bad run the router's tbf dropped 41 packets and the own sender sent 43 again. The bad runs overshoot the queue (flight of 40 segments into a queue of 22).

Cause. `Inner::pump` read the clock once and ran passes until nothing moved. A pass runs for milliseconds in the unoptimized test build (65 us per segment), and the ACKs of a burst arrive during it. Segment and ACK got one instant, so round trip samples read 1 us (`max(1)`), and the smallest round trip of HyStart became 1 us, which made the delay rule useless. The ACK train rule also broke when two ACK batches were more than 2 ms apart. The exit then came at a window of 40 segments instead of 17.

Fix. `Inner::pump` takes the clock and reads it before each of the four steps. The test `a_pump_pass_reads_the_clock_before_each_of_its_steps` fails without it (1 read, not 4).

After the fix (CUBIC, rows only): own median 5, 9 at the 99th percentile; 2 outliers in about 485 runs (131; 27, where a 2.6 ms gap between ACK batches ended the ACK train late). Whole binary, sandbox with BBR host default: 2 failures in 105 runs, one the bound (own 92, kernel 25) and one the existing floor of the own server without SACK (3.09 Mbit/s, bug #699 again, not touched here). The bound is not changed. If the vetting agent wants it tighter in practice, the evidence supports comparing with the larger of two kernel runs, because the kernel baseline has the same heavy tail.

### Shared page notes

- Testing: the TRN-34 baseline row (kernel client to own server, bottleneck with tail drop) pins the kernel to CUBIC.
- Module Development, "Watch out for": nothing for module authors.
