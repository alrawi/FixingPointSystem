## [2026-11-18] plan | TRN-24 merged, and its acceptance compares like with like

TRN-24 merged in PR #646. It added window scaling, timestamps with PAWS, selective acknowledgment, and delayed acknowledgment to the own stack. The coordinator read its acceptance row on a hosted runner with the tcp-bench workflow, on the fat path: 50 Mbit/s, 50 ms per direction, and 0.1 percent loss, with 6 transfers of 8 MiB. Every transfer completed.

| Kernel congestion control | Kernel | Own stack | Ratio |
|---|---|---|---|
| Default (CUBIC) | 24.09 Mbit/s | 13.17 Mbit/s | 55 percent |
| Reno (`--kernel-cc reno`) | 11.39 Mbit/s | 13.38 Mbit/s | 117 percent |

smoltcp reached 4.92 Mbit/s in both runs, limited by its 64 KiB window.

The acceptance as first written did not name the congestion control of the kernel, so it mixed the options of TRN-24 with the CUBIC of TRN-25. Against the default kernel the own stack missed 80 percent. The TRN-24 worker found no timeouts and 4 to 5 SACK recoveries per transfer, which puts the own stack at the Reno loss bound. Against kernel Reno it reaches 117 percent.

The vetting agent accepted TRN-24 like for like (vetted by Fable 5.2) under four conditions:

- The TRN-24 acceptance names `--kernel-cc reno`.
- This log states the 55 percent miss against the default kernel.
- TRN-25 gains a single flow on the same fat path at 80 percent or more of the default kernel. TRN-31 depends on TRN-25, so the bar is deferred, not dropped.
- `--kernel-cc` sets `TCP_CONGESTION` on each socket and never host wide. The coordinator confirmed this: `fps_tcp::kernel` uses `set_tcp_congestion` on the socket, and the only `sysctl` in the namespace setup turns on forwarding inside the router namespace.

The scale row did not regress. The branch reached 68,152 to 68,458 probe sessions per second, against 63,018 to 63,125 for main on the same runner, at 256 B per socket for both.

### Shared page notes

None.
