## [2026-11-28] ingest | bug 738: the QA harness capture no longer drops frames from its tcpdump ring

Issue #738 (audit of the kernel-run capture scripts after bug #735).

Where. Every `modules/*/fixtures/capture.sh` and the gate scripts record through `capture_start` in `tests/harness/netns.sh`, which ran `tcpdump -n -s 0 -U --immediate-mode` with the default buffer, the small ring of bug 735. `capture_stop` did not look at the tcpdump statistics, so a drop went unnoticed (the gates only copied the count into context.json).

Fix. `capture_start` runs tcpdump with `-s 9216 -B 65536`: 9216 covers the largest harness frame (MTU 9000 with an 802.1Q tag, or the cooked header of `all`), and 65536 KB leaves thousands of slots. `capture_stop` reads the statistics from the capture log and fails with "tcpdump dropped N packets" when N is not zero, or with "did not report its drop count" when the line is missing; it writes no pcapng then. The gate scripts call `capture stop` under `set -e`, so a drop now stops the gate before the report; their `capture_dropped` field stays as it is. Test: `tests/harness/test_capture_drops.py` (root, ip, tcpdump; a stopped tcpdump with the old options dropped 4699 of 6009 frames, with the new ones none; a stub tcpdump checks the failure messages). No module `capture.sh` needed a change, and `modules/strike/` was not touched.

### Shared page notes

Module Development, "Watch out for": the entry of bug 735 also holds for the kernel run; add at its end: "`tests/harness/netns.sh capture start` uses these options, and `capture stop` fails with the drop count (bug 738), so a module's capture.sh needs no options of its own."
