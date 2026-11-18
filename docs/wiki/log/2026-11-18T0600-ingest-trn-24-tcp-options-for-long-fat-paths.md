## [2026-11-18] ingest | TRN-24 TCP options for long fat paths in the own stack

TRN-24 gave the own stack window scaling and timestamps (RFC 7323, with PAWS), selective acknowledgment (RFC 2018 receiver blocks, RFC 6675 sender recovery beside NewReno), and a delayed acknowledgment, each negotiated in the handshake with a clean fallback and none with a cost for an idle socket.

- `wire.rs` has the option codec in the kernel's order; `sack.rs` has the scoreboard (SACKed and resent ranges, `IsLost`, `pipe`, `NextSeg`); `recovery.rs` has the receiver blocks and the SACK entry into NewReno; `host.rs` does the negotiation, scaled windows, the timestamp RTT sample, PAWS, the delayed ACK on the one timer of the connection, and a restart of the timeout when the oldest segment is sent again.
- `Conn` stays at 176 bytes (6 bytes of option state paid for by a `u32` timeout and one flag moved into the option bits). The scoreboard lives in the `Recovery` box. At 1,001,129 sockets: 262 and 263 bytes per socket with the options, 262 and 262 on main, alternating runs in one session.
- `tcp_bench` has `--loss-profile fat` (0.1 percent loss, 50 ms of delay in each direction so the round trip is 100 ms, 50 Mbit/s, queue 1 MiB, 8 MiB per transfer, timeout 180 s) and `--own-wscale`, `--own-timestamps`, `--own-sack`, `--own-delayed-ack` (on or off). The default loss run is unchanged.
- Results are in `engine/fps-tcp/results/sandbox-2vcpu-trn24/` with `host.txt`. Fat path, mean of 6: own 13.40 Mbit/s, own on main 3.51, smoltcp 4.92 (capped at 64 KiB), own with only window scaling 14.07. The kernel row cannot run in the sandbox (no `sch_netem`), so the 80 percent acceptance is not verified here. Default loss run, own: 52.8, 13.8 and 3.2 Mbit/s at 0, 0.1 and 1 percent.

Tests: wire (option layout, caps, malformed), `sack.rs` (11 scoreboard tests), recovery (8 SACK tests), about 30 host level tests in `host/recovery_tests.rs` (negotiation and fallback, window scaling, timestamps, PAWS, SACK repair, delayed ACK timing, the timeout restart), 2 `cli.rs` tests, and a scaled window transfer in `tests/lossy.rs`.

### Shared page notes

- Decision record 0014 (owner or vetting agent only, not edited here): follow-up item 3 (window scaling, timestamps, SACK, delayed ACK) is done in the own stack.
- Testing page: `cargo test -p fps-tcp-own` now has about 126 unit tests; `cargo test -p fps-tcp --test lossy` has one more transfer (scaled window).
- Workflow `tcp-bench.yml` (coordinator file): the loss job runs without arguments. For the acceptance row run `tcp_bench --stack kernel --workload loss --loss-profile fat`, and the same for `own` and `smoltcp`, with `--loss-transfers 6` for steadier means. Equivalent explicit args: `--workload loss --loss-levels 0.1 --link-delay-ms 50 --link-rate-mbit 50 --link-queue-kib 1024 --loss-kib 8192 --loss-timeout-secs 180`. The kernel needs root and netem as in the existing job. The 50 ms is per direction. Add an args pass through or a step with these args.
- Transport page: this package added "Options for long fat paths in the own stack (TRN-24)" and updated the own stack table, the leaves out sentence, and the limits row. The component page for `fps-tcp-own` is the same page.
- Follow-up of the coordinator (kernel 24.09 against own 13.17 on the hosted runner): `tcp_bench --kernel-cc NAME` sets `TCP_CONGESTION` on the kernel row's sockets (client and server peers in the namespaces). For the like for like row run `--stack kernel --workload loss --loss-profile fat --loss-transfers 6 --kernel-cc reno` beside the default kernel row. The own stack's remaining gap is CUBIC (TRN-25), not an option.
- Module development: nothing changes for module authors.
