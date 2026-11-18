## [2026-11-18] ingest | TRN-34 pacing and faster loss detection in the own stack

TRN-34 gives the own TCP stack HyStart for CUBIC, RACK with a reorder timer, tail loss probes, and sender pacing, each behind a switch of `OwnConfig` (`hystart`, `rack`, `tlp`, `pacing`) and a `tcp_bench` flag (`--own-hystart`, `--own-rack`, `--own-tlp`, `--own-pacing`, each `on` or `off`). It edits `engine/fps-tcp-own/` (new `rack.rs`; `cubic.rs`, `recovery.rs`, `sack.rs`, `host.rs`, `lib.rs`), `engine/fps-tcp/` (`cli.rs`, `interop_kernel.rs`, `tests/bottleneck.rs`, `tests/interop_kernel.rs`, `results/sandbox-2vcpu-trn34/`), `docs/handoff/TRN-34.md`, this entry, and the Transport page (section "Pacing, HyStart, RACK and tail loss probes in the own stack (TRN-34)" and one row of its Limits table).

- Defaults: RACK on, tail loss probes on, HyStart off, pacing off. HyStart gives the whole drop in retransmissions in the short queue (60 to 8 to 15 per 2 MiB, the kernel sends 8 to 9) but lowers the fairness of four flows (TRN-25 row) from a Jain index of 0.97 to 0.90 to 0.91, on the bar, so it is a switch for the owner after the hosted run. Pacing does not reduce retransmissions and costs a little on the fat row.
- Without HyStart the own stack does not meet "at most twice the kernel's retransmissions" in the bottleneck test; with `hystart: true` it does (5 against 51 in memory, best of three). The test `the_loss_switches_keep_the_retransmissions_near_the_kernels` sets the switches itself.
- RACK and tail loss probes end the stalls of the own sender against smoltcp (11.4 Mbit/s with two timeouts in 4 of 6 runs, 19.0 Mbit/s every time with them). The probe and the reorder timer use the one per connection timer (`TimerKind`), the send log and the RACK state live in the recovery box, and `Conn` stays 176 bytes (bit 15 of `opts` is the pacing flag).
- Memory per socket, main against this branch in one session: 349 and 361 bytes at 100,000 and 500,000 sockets on both (0 to 1 byte). The idle poll rose from 19 to 20 us to 23 to 26 us (one more heap in `next_deadline`).

Evidence (sandbox, 2 vCPU, root, release build, `engine/fps-tcp/results/sandbox-2vcpu-trn34/`): `cargo test -p fps-tcp-own -p fps-tcp` passes (220 lib tests of the own stack, with new ones for the log, RACK, probes, pacing and the switches off; the bottleneck test, the kernel interop test with two new rows per pairing, and the cli test), clippy with `-D warnings` is clean. The default loss row reads 52.6, 25.9, 3.12 Mbit/s with the defaults against 52.7, 25.8, 3.11 with all four off (0, 0.1, 1 percent at 20 ms each way). The fat row (x24) reads 18.3 to 19.0 with the defaults against 18.1 to 19.1 for the unchanged stack, which is the run to run spread. The four flow bottleneck row reads 891 Mbit/s with Jain 0.97 with the defaults (HyStart off) and 0.91, 0.90 and 0.91 with HyStart on.

Not verified here: every kernel row at loss and delay (the sandbox kernel has no `sch_netem`, and its default congestion control is BBR, so the kernel rows used `--kernel-cc cubic` or are marked), the hosted runner, and the 80 percent of the kernel on the fat row of TRN-24 and TRN-25.

### Shared page notes

**Dispatches wanted from the coordinator** (workflow `tcp-bench`, `job: loss`, `ref: wp/TRN-34`; each is one dispatch, `loss_args` as given):

1. Default loss row: `--stack own` default flags (the workflow's own defaults) and the kernel row.
2. Fat row, defaults: `--loss-profile fat --loss-transfers 6`, and the same with `--own-hystart on`, to compare with 24.09 Mbit/s of the kernel.
3. Fairness row: `--loss-profile bottleneck --loss-transfers 6`, defaults and `--own-hystart on`. If the Jain index with HyStart stays at or above 0.9 and the kernel ratio holds, the owner may set `hystart: true` in `OwnConfig::default`.
4. Short queue: `--link-rate-mbit 20 --link-queue-kib 32 --link-delay-ms 0 --loss-levels 0 --loss-kib 2048 --loss-transfers 12` with `--own-hystart on --own-rack on --own-tlp on`, with `--kernel-cc cubic` for the kernel row.

**Transport page.** The section "A short tail-drop queue under SACK and CUBIC (bug 652)" still lists pacing, HyStart and RACK as open; the new TRN-34 section answers both findings, and the section of bug 652 was left as written (it is an earlier ingest).
