## [2026-11-28] ingest | SPK-05: a smoltcp socket runs behind our own flow table and timer wheel

Issue #620. Spike under `spikes/s5-smoltcp-flow-table/` (prototype in `flowtable/`, answer in `RESULTS.md`).

Answer. Yes, a smoltcp 0.14 socket processes frames outside its interface loop, with one condition: the socket methods are `pub(crate)`, so the way in is `Interface::poll_ingress_single`, `poll_egress`, and `poll_at`, which take a `SocketSet`. The prototype shares one `Interface` and gives every flow a `SocketSet` of one socket, parses the 4-tuple into a hash map of its own, and arms a hashed timer wheel from `poll_at`. Handshake, echo, loss with timer driven retransmission, and close with TIME-WAIT passed at 1 to 64,000 connections with every byte checked.

Cost. At 16,000 connections (32,000 sockets, both ends, one core, sandbox) a frame event costs 0.67 to 0.85 us mean and 0.37 to 0.44 us p50; a timer fire 0.1 to 0.2 us. The interface loop over one set of 16,000 sockets costs 927 to 935 us per event. The flow table cost is not flat: 0.13 us at 100 connections, 0.69 us at 16,000.

Findings. Memory is the open risk: about 4.0 KB resident per socket with 1 KiB buffers (2.6 KB explained), against 229 B for the own stack. Sets of one socket make the "sets of a few hundred" of record 0014 unnecessary. Lazy timer cancellation makes 80 percent of fires spurious (cheap). Synchronized timers make a 3 ms tick at 16,000 flows.

### Shared page notes

Decision record 0014 (coordinator or vetting agent, outside the paths of SPK-05): item 3 says "we have not checked that smoltcp's socket can run outside its interface loop" and item 9 of Consequences lists the spike. Suggested text: "SPK-05 (spikes/s5-smoltcp-flow-table) verified it: one shared Interface and one SocketSet of one socket per flow, with our own flow table and timer wheel, run without Interface::poll at about 0.7 us per event at 16,000 sockets. Sets of a few hundred sockets are not needed. Memory per socket (about 4 KB measured) is the open risk of the fallback." Also item 3 of the decision: replace "sets of at most a few hundred sockets" by "one socket per set".

Root `Cargo.toml` (required before CI can pass): change the exclude line to `exclude = ["spikes/s3-afxdp", "spikes/s4-trex-astf", "spikes/s5-smoltcp-flow-table"]`. Without it the glob `spikes/*` matches the spike directory, which has no manifest, and every cargo command of the workspace fails. Checked locally: with the line, `cargo metadata --locked` passes and `Cargo.lock` is unchanged.

Makefile or `mk/spk.mk`: the spike crate is outside the root workspace, so `make test-spk` does not build it. To test it in CI add to `test-spk`: `$(CARGO) test --release --manifest-path spikes/s5-smoltcp-flow-table/flowtable/Cargo.toml --locked`. The crate has its own `Cargo.lock`; `cargo deny` does not see it (all its dependencies are in the licenses of `deny.toml`).

Module Development, "Watch out for": none.
