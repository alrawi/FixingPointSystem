## [2026-11-17] ingest | TRN-20 smoltcp on the TCP stack bench harness

`engine/fps-tcp` now has three stacks behind the `TcpStack` seam: the kernel loopback reference (TRN-17), the own minimal stack (TRN-18), and `SmolStack` (TRN-20), which adapts smoltcp 0.14 to `SessionIo`. `tcp_bench --stack smoltcp` runs the same three workloads and reports the same three measures, and the flags `--smol-mtu`, `--smol-checksums`, `--smol-buffer-kib`, `--smol-time-wait`, and `--smol-idle-poll-ms` change it. The adapter is the draft of the first TRN-17 worker, preserved on `wp/TRN-20`. TRN-20 merged `origin/main` into that branch with an additive merge commit (no force push), restored `src/smol.rs`, `src/link.rs`, `tests/smol.rs`, and the dependency, and added `TcpStack::notes` to the adapter. The owner allowed 0BSD on 2026-11-17, so `cargo deny check` accepts `smoltcp 0.14.0` and `managed 0.8.0`.

The results are in `engine/fps-tcp/results/sandbox-2vcpu-trn20/`, from the 2 vCPU agent sandbox with the three stacks run in one session. The fat-nuc run is not done: the relay account on fat-nuc has no Rust toolchain, and the host test with its argv is in `docs/handoff/TRN-20.md`.

Finding, the measures of smoltcp next to the own stack and the kernel. Sessions per second at 32 concurrent: smoltcp 59,000 to 66,000, kernel 42,000 to 45,000, own 35,000 to 36,000. Throughput of four uploads: smoltcp 28.1 Gbit/s, kernel 22.5, own 13.4. Sockets: smoltcp reached 1,000,584 in 64 shards at 1.59 KB per socket (with 512 B receive and transmit buffers), the own stack 228 B per socket, the kernel 0.44 to 0.49 KB until its descriptor limit at about 20,000 sockets.

Finding, the cost of smoltcp grows with the sockets of a set. One poll with idle sockets takes 24 ns per socket in one set of 16,016 sockets and 54 ns per socket over 1,000,584 sockets in 64 sets (54 ms for the pass). The probe session rate with sockets held falls from 4,800 per second at 1,016 sockets to 118 at 16,016 and 60 at 1,000,584, where the own stack stays at 33,000 to 68,000 because its timers sit in a heap. A million sockets on smoltcp need many small sets and a flow table of our own in front of them.

Finding, caveats of the smoltcp numbers. The rate uses TIME-WAIT skip, so a closed tuple is reused at once, and `--smol-time-wait keep` shows the other case. The buffers are fixed per socket and part of the 1.59 KB. Checksums are computed in software.

### Shared page notes

- [Testing](Testing.md): none beyond the sentence of TRN-17, because `test-eng` and the `rust` job run the tests of `fps-tcp`, which now include `tests/smol.rs` and the smoltcp tests of `cli.rs`.
- Plan (coordinator): the entry TRN-20 of `plan/work-packages.yaml` still has the `owner_action` about 0BSD, which the coordinator removes in a plan pull request. The summary there mentions PR #586 and the draft numbers, which the results of this package replace.
- TRN-10 (decision record 0014): the stack decision needs, from these results, the three measures per stack from one session, the finding that smoltcp costs grow with the sockets of a set while the own stack does not, the memory per socket with and without buffers, and the fat-nuc run once the relay account has a Rust toolchain. The owner action for that is a stable Rust toolchain on the relay account of fat-nuc.
- [Module Development](Module-Development.md): none, because the package changes nothing about how a module is written. [Home](Home.md) and [Implementation Plan](Implementation-Plan.md): none.
- `Cargo.lock` gains `smoltcp 0.14.0`, `managed 0.8.0`, `heapless 0.9.3`, `hash32 0.3.1`, `bitflags 1.3.2`, and the `defmt` family, which only the smoltcp adapter needs.
