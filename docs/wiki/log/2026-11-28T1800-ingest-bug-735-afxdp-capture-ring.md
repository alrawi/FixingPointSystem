## [2026-11-28] ingest | bug 735: the AF_XDP realism capture no longer drops frames from its tcpdump ring

Issue #735: http1 on AF_XDP decrypted 79 of 80 TLS sessions (run 46153734448, PR #734).

Where. Session 79 of the report, 10.1.0.4:10078 to 10.2.0.1:8445 (TLS 1.2). The capture holds its SYN exchange, ClientHello and the server flight, then nothing: tshark read 0 of 2 Finished messages. It is the only one of 196 TCP streams without a FIN or RST. The key log had all 80 sessions and the session sits at 3.1 s of 24.4 s, so neither the key log nor the capture boundary. The agent counters say the session completed (client tx equals server rx for all 26 TLS 1.2 sessions), so the frames were on the wire and tcpdump on wc missed them: a hole of about 2 ms.

Cause. `tcpdump -s 0 --immediate-mode` with the default 2 MB buffer has about 30 ring slots (buffer divided by the 262144 byte snaplen). A tcpdump that is not scheduled for a few milliseconds while the agent polls loses the rest of a burst. A stopped tcpdump with these options dropped 28 of 60 frames in a test; with `-s 9216 -B 65536` none.

Fix. `module-afxdp.sh` runs tcpdump with `-s 9216 -B 65536`, prints its log, and fails with the drop count if tcpdump dropped any packet or did not report one. The encryption threshold is unchanged. Test: `engine/fps-datapath/realism/test_capture_ring.py`. The kernel run's `capture.sh` uses its own tcpdump options (not checked here).

### Shared page notes

Module Development, "Watch out for", add: "**A capture check reports a session cut in the middle when tcpdump drops frames.** tcpdump in immediate mode holds buffer / snaplen frames in its ring, so `-s 0` with the default 2 MB leaves about 30 slots, and a tcpdump that is not scheduled for a few milliseconds drops a burst. One TLS session then lacked its Finished messages and the encryption check decrypted 79 of 80 sessions. Use a snaplen near the frame size and `-B` of tens of MB, and fail the run when tcpdump reports dropped packets (bug 735)."
