## [2026-11-18] ingest | TRN-25 CUBIC in the own stack

TRN-25 gave the own stack CUBIC (RFC 9438) beside the NewReno of TRN-23, as the default of `OwnConfig::congestion_control` (`tcp_bench --own-cc newreno|cubic`). The loss recovery (duplicate acknowledgments, `recover`, the SACK scoreboard, partial acknowledgments, the timeout) is shared; only the reduction at an event and the growth in congestion avoidance differ.

- `cubic.rs` is a port of parts of smoltcp 0.14.0 (0BSD), `src/socket/tcp/congestion/cubic.rs` (constants, `cube_root`, window function, reno friendly estimate, 1.5 clamp, idle absorption, event rules); its header comment lists every ported part and the differences. `recovery.rs` calls it from `NewReno`, `host.rs` passes time, smoothed RTT and flight per acknowledgment.
- The state (72 bytes) lives in the in-flight `Recovery` box. Scale row of the full profile: 263 bytes per socket at 1,001,129 sockets (262 and 263 in TRN-24). `Conn` is unchanged.
- CUBIC is the default because the Linux kernel, the reference of every row, runs it. No HyStart and no PRR: slow start leaves on a loss, and a recovery sets the window to the threshold at once.
- `tcp_bench` has `--own-cc`, `--own-sndbuf-kib`, and a fairness variant of the loss workload: `--loss-flows N`, `--loss-warmup-secs`, `--loss-window-secs`, `--loss-profile bottleneck` (4 flows, 1000 Mbit/s, 20 ms in each direction, 4 MiB queue, 4 MiB own windows). Every flow sends without end, the server counts per flow in a window, the report has the sum and the Jain index. The kernel runs the flows as one client and one server peer process in the namespaces.
- Results are in `engine/fps-tcp/results/sandbox-2vcpu-trn25/` with `host.txt`. Fat path, mean of 6: own CUBIC 20.00 Mbit/s, own NewReno 14.18, smoltcp 4.91 (mean of 24, seed 7: 19.58 and 13.54); 19.3 is the bar of the acceptance (80 percent of the 24.09 of the hosted runner), so the margin is thin and the spread per transfer is 6 to 32 Mbit/s. Four flows: 891 Mbit/s of payload for both controls (the link), Jain index mean 0.941 for CUBIC (lowest run 0.776) and 0.962 for NewReno, smoltcp 51 Mbit/s and 1.000. The kernel rows cannot run in the sandbox (no `sch_netem`).

Tests: `cubic.rs` (13), CUBIC cases in `recovery.rs` (9) and `host/recovery_tests.rs` (5), `jain` and a two flow transfer over a rate limited link, 2 `cli.rs` tests, a `report.rs` test, and the fairness result line in `netns.rs`.

### Shared page notes

- Workflow `tcp-bench.yml` needs no change: the loss job passes `loss_args` to every stack. Acceptance row 2 (one flow, fat path): `--loss-profile fat --loss-transfers 6` (kernel, own with CUBIC, smoltcp), and for the like for like Reno row `--loss-profile fat --loss-transfers 6 --kernel-cc reno --own-cc newreno`. Acceptance row 1 (four flows): `--loss-profile bottleneck --loss-transfers 3`; each transfer takes about 16 s per stack. Equivalent explicit args for the bottleneck: `--workload loss --loss-levels 0 --loss-flows 4 --link-delay-ms 20 --link-rate-mbit 1000 --link-queue-kib 4096 --own-window-kib 4096 --own-sndbuf-kib 4096`.
- Decision record 0014 (owner or vetting agent only, not edited here): the follow-up item for CUBIC is done in the own stack.
- Testing page: `cargo test -p fps-tcp-own` now has about 150 unit tests including `cubic.rs`, and `cargo test -p fps-tcp --test lossy` has one more test (two flows over a rate limited link).
- Transport page: this package added "CUBIC in the own stack (TRN-25)", a row in the own stack table, and updated the limits row and the leaves out sentence. The component page for `fps-tcp-own` is the same page.
- Module development: nothing changes for module authors.
