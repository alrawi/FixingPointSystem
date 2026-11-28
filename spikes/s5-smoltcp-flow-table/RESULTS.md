# Spike S5 results: smoltcp behind our own flow table

Package SPK-05, issue #620. Question from decision record 0014, item 3: the fallback "smoltcp behind a flow table and a timer wheel of our own" is unverified. This spike answers two things and nothing else.

1. Does a smoltcp socket process frames outside its interface loop, behind a flow table and a timer wheel of our own?
2. What does one event cost at 16,000 sockets?

All numbers are from the 2 vCPU agent sandbox (Xeon at 2.30 GHz, kernel 6.18.44, rustc 1.97.0, load average 1.2 to 3.4 from other workers, `results/host.txt`). They are not from the GitHub runner and not from fat-nuc. Read them as the cost of the state machine, the flow table, and the frame copies on one core, with both ends of every connection in one process over a link without delay.

## Answer

**1. Yes, with one condition that the decision record did not name.** smoltcp 0.14 makes the socket methods that process a segment and build a segment (`process`, `dispatch`, `accepts`, `poll_at` of `tcp::Socket`) `pub(crate)`. A socket cannot be called directly. The public way into the socket state machine is `Interface::poll_ingress_single`, `poll_egress`, and `poll_at`, and each takes a `SocketSet`. What works, and what the prototype does, is one `Interface` per worker shared by all flows, and **one `SocketSet` per flow that holds exactly one socket**. `Interface::poll` (the loop) is never called. Our code does the work of the loop:

- **Ingress.** Parse the 4-tuple from the IPv4 and TCP header, look it up in a hash map of our own, and give the frame to the device. A SYN for a listened port with no flow creates a flow with a listening socket first. Then call `poll_ingress_single` on the set of that flow. smoltcp looks at one socket.
- **Egress.** After every event on a flow (a frame, a write or read of the application, a timer), call `poll_egress` on the set of that flow until it returns `None`. Frames that smoltcp writes, including the replies of the interface itself, go to the output queue of the device.
- **Timers.** After egress, call `poll_at` on the set of that flow and arm one entry in a hashed timer wheel (1 ms tick, 1,024 slots, a round count for deadlines further away, such as TIME-WAIT at 10 s). A due entry calls the same egress path for that flow, with no frame arriving. A flow whose socket reaches `Closed` leaves the table.

So the interface type still exists in the fallback, and the shared `Interface` carries the addresses and the random state for initial sequence numbers. What does not exist is a loop that walks sockets. The cost of an event no longer depends on the number of sockets that a set holds, because a set holds one.

Verified by running both ends of N connections through five phases with every response checked byte for byte (`tests/flow_table.rs`, N = 1 and 64, and the runs below to N = 64,000):

| Phase | What it proves |
|---|---|
| Handshake | The SYN creates a flow, SYN, SYN-ACK, and ACK run through `poll_ingress_single` only. Every flow reaches ESTABLISHED on both ends. |
| Burst | Three rounds of a 64 byte request and a 64 byte echo on every flow, in random order. |
| Single | One round trip at a time with all flows resident. |
| Loss | The first transmission of one request on every flow is dropped. The retransmission timer of every flow fires from the wheel with no frame arriving, the retransmitted segment is delivered, and the echo is correct. 16,000 of 16,000 recovered. |
| Close | The client closes. FIN, ACK, and TIME-WAIT (10 s of virtual time through the wheel) run to the end, every flow leaves both tables, and no frame was an orphan. |

## Cost per event at 16,000 sockets

**What an event is.** One call into the table: a frame given to a flow (parse, lookup, copy, `poll_ingress_single`, `poll_egress` until idle, `poll_at`, wheel arm), a write or a read of the application on a flow (the same egress and arm), or a timer entry that fires for a live flow (`poll_egress`, `poll_at`, arm). Each is timed with `Instant::now` before and after, so every figure includes about 22 ns of clock (a pair costs 22 to 23 ns on this host, measured at the start of each run).

**The state machine.** smoltcp 0.14 TCP with the harness settings: Nagle off, no delayed acknowledgment, 1 KiB receive and 1 KiB transmit buffer per socket (no window scaling), medium `ip`, IPv4, checksums computed on both directions. The Cargo features `socket-tcp-reno` and `socket-tcp-cubic` are off, as in the harness.

**The method.** 16,000 connections give 32,000 sockets (the client table and the server table, each with 16,000 flows). The client and server sit in one thread and exchange `Vec<u8>` packets through two queues, so frames are copied once per hop and buffers are recycled from a pool. Flows are touched in random order to keep the 4 KB per flow working set (about 130 MB) from sitting in cache. The three full runs (`results/run-16000-{1,2,3}.txt`) agree within 6 percent, and the numbers below are their range.

| Event at 16,000 connections | Count per run | Mean | p50 | p99 |
|---|---|---|---|---|
| Frame given to a flow, burst of requests | 192,000 | 0.80 to 0.85 us | 0.42 to 0.44 us | 2.2 us |
| Frame given to a flow, isolated round trips | 8,000 | 0.67 to 0.69 us | 0.37 to 0.38 us | 2.2 us |
| Write or read of the application | 144,000 (burst) | 0.26 to 0.27 us | 0.18 us | 0.8 to 0.9 us |
| Timer fire: retransmission and spurious (loss phase) | 48,000 | 0.09 to 0.11 us | 0.08 to 0.09 us | 0.2 us |
| Timer fire: TIME-WAIT expiry and spurious (close phase) | 32,000 | 0.16 to 0.17 us | 0.12 us | 0.5 us |
| Idle tick of the wheel (nothing due) | 20,000 | below the clock floor | 0.023 us | 0.04 to 0.08 us |
| Handshake: first frame of a flow (SYN, allocates the flow) and the others | 48,000 | 1.8 to 1.9 us | 1.1 us | 5.6 to 6.0 us |
| Handshake: `connect` | 16,000 | 1.6 to 1.7 us | 1.4 us | 2.9 us |

The same pattern through the interface loop of smoltcp (`Interface::poll` and `poll_at` on one set of 16,000 sockets per host, the layout of the harness in `engine/fps-tcp`), measured on isolated round trips in the same binary: **927 and 935 us per event** (p50 944 us, p99 1.2 ms), two runs. That is about 1,400 times the mean of the flow table for the same isolated round trips, and about 2,600 times the p50.

### How the cost moves with the number of connections

Frame given to a flow (isolated round trips) and one turn of the loop (isolated round trips), sandbox, one run each:

| Connections | Flow table, mean | Flow table, p50 | Interface loop, mean | Loop cost per socket |
|---|---|---|---|---|
| 100 | 0.13 us | 0.14 us | 2.2 us | 22 ns |
| 1,000 | 0.19 us | 0.17 us | 21.1 us | 21 ns |
| 4,000 | 0.32 us | 0.21 us | 154 us | 19 ns |
| 16,000 | 0.67 to 0.69 us | 0.37 us | 927 to 935 us | 58 ns |
| 64,000 | 1.22 us | 0.42 us | not run | |

The loop grows with the sockets, as decision record 0014 found (22 to 24 ns per socket up to 4,000, and more once the set no longer fits in cache). **The flow table does not stay flat. Its cost grows by a factor of five from 100 to 16,000 connections** (0.13 to 0.69 us mean), and by almost ten at 64,000. We did not isolate the cause. The likely one is cache misses, because the working set grows from under a megabyte to about 130 MB, and the p50 grows less than the mean. The hash map, the allocator, and the wheel are the other candidates. The growth is slow compared with the loop, but it is not the flat cost that the own stack of the record shows, and the spike does not claim it.

### Whole session, by arithmetic

From the phase walls of the 16,000 run (both ends, all costs included, so an upper bound on the cost of the stack): handshake 7.2 to 7.7 us per session, a request and response 4.4 to 4.7 us, close with TIME-WAIT 3.7 to 3.8 us. A session with one request and response is about 16 us on one core for both ends, which is about 60,000 sessions per second. This is derived and not a separate measurement, and it is the same order as the 56,000 to 58,000 sessions per second of smoltcp in the record (a different workload and a different load on the host, so not a comparison).

## Findings that the decision record does not have

1. **One socket per set removes the need to size sets.** Item 3 of the record proposes "sets of at most a few hundred sockets and a scheduler that polls only the sets that have work". At 22 ns per socket, a set of 300 sockets costs about 6.6 us per event, ten times the cost of a one-socket set at 16,000 flows. The spike supports one socket per set.
2. **Memory is the larger risk of the fallback, not the cost of an event.** The resident memory grew by about 4,040 B per socket (both ends counted) at 16,000 and at 64,000 connections, with 2 KiB of buffers. smoltcp's `SocketStorage` is 432 B. The structures of the prototype add up to about 2.6 KB per socket by my estimate (buffers with allocator rounding about 2.1 KB, storage 0.45 KB, flow slot and map entry about 0.1 KB). **The remaining 1.4 KB is not explained**: it may be transient wire buffers and allocator behavior, and I did not separate it. For the 2,000,000 sessions of tier T1 this is 4,000,000 sockets, which is 10 to 16 GB at these sizes against about 0.9 GB for the own stack at 229 B (record, item 1). Smaller buffers (smoltcp needs at least a window per socket) lower it, and a pool of buffers that are given to a socket on demand would need changes inside the socket, which `SocketBuffer::new` does not support. This needs a decision before the fallback is taken.
3. **Lazy cancellation costs spurious fires.** The wheel never removes an entry. When a deadline moves later (an acknowledgment cancels a retransmission timer), the early entry stays, fires, finds no deadline, and the flow is not armed again. Of 80,000 fires in a run at 16,000 connections, 16,000 sent a frame and 64,000 were spurious, at 0.1 to 0.2 us each. That is cheap here, and it is one wheel entry per flow instead of one per event. A scheduler that fires only real deadlines would need an intrusive wheel.
4. **Synchronized timers make a long tick.** All 16,000 retransmission timers armed in the same millisecond fall due in the same tick. The tick that runs them takes 3.0 to 3.6 ms (max of the tick samples, 16,000 fires at 0.1 to 0.2 us each), so the mean tick of the loss phase is 3.1 to 3.6 us even though the idle tick is 23 ns. A real scheduler needs a limit on fires per tick, or jitter in the deadlines.
5. **The first SYN of a flow and the growth of the map are the slow events.** The handshake frame mean is 1.8 us with a p99 of 5.6 to 6 us, and the single slowest event of each run (3.5 ms at 16,000) is the resize of the hash map. Pre-sizing the map removes it. A SYN allocates a listening socket and its buffers before any check, so a flood of SYNs allocates per frame. The prototype has no backlog limit and no SYN cookies.
6. **Time resolution.** Deadlines round up to the 1 ms tick of the wheel. smoltcp's own retransmission floor is 10 ms, so the rounding is not visible in the recovery, but a profile that wants a finer timer needs a finer wheel.

## What this spike did not test

- No loss other than the one dropped request per flow, no delay, no reordering, no slow receiver, and no window larger than 1 KiB. The recovery code of smoltcp ran only for a first retransmission. Congestion control (Reno, CUBIC), window scaling, selective acknowledgment, and the out of order queue did not run.
- No real link. The wire is two in memory queues with a copy per hop, no Ethernet medium, no ARP, no AF_XDP, no NIC. Both ends share one thread and one cache.
- Unmatched segments are dropped by the flow table (an orphan counter, zero in all runs). The prototype never makes smoltcp send a RST for an unknown flow, so a real stack's behavior toward stray segments is not modeled.
- IPv4 only, and no fragmentation.
- Virtual time. The clock moves in steps of 1 ms between events and the wheel runs on it. The costs are real CPU time, but no run waited on real timers.
- One thread. The record's design runs one stack per worker, and the spike did not measure several workers or the effect of a shared cache between them.
- The numbers come from a shared sandbox with other workers running, one run per row of the sweep table, and three runs at 16,000 connections. The 64,000 run is one run and holds 512 MB.
- The memory figure is resident set growth around the handshake, not an allocator report, and the unexplained part is stated above.
- OS profile options (initial window, option order) were not looked at.

## Verdict for the fallback

The fallback of decision record 0014 works as a design: smoltcp sockets run behind a flow table and a timer wheel of our own with no interface loop, loss recovery driven by the wheel works, and the cost of an event at 16,000 sockets is about 0.7 us (p50 0.4 us), which is about a thousand times less than the loop on one set. What the spike does not settle is memory per socket (finding 2), which would have to be solved or accepted before TRN-12 switches, and behavior under real loss, delay, and congestion, which the loss link of the follow-up package 1 measures.

## Files

- `flowtable/`: the prototype crate `fps-spike-smoltcp-flow-table`. `src/table.rs` is the flow table, `src/wheel.rs` the timer wheel, `src/dev.rs` the frame device, `src/scenario.rs` the five phases and their checks, `src/baseline.rs` the interface loop for comparison, `src/main.rs` the benchmark. `tests/flow_table.rs` has the tests.
- `results/`: the raw output of every run (`host.txt`, `run-sweep-baseline.txt`, `run-16000-{1,2,3}.txt`, `run-64000.txt`).

## Run it

The crate is standalone. It has its own `[workspace]` table and `Cargo.lock`, and it sits in `flowtable/` so that its manifest is not at the spike root. **The root `Cargo.toml` needs one change that this package may not make**: its member glob `spikes/*` matches `spikes/s5-smoltcp-flow-table/`, which has no `Cargo.toml`, and Cargo then fails to load the whole workspace ("failed to load manifest for workspace member"). Add the directory to the exclude list, as for S3 and S4:

```toml
exclude = ["spikes/s3-afxdp", "spikes/s4-trex-astf", "spikes/s5-smoltcp-flow-table"]
```

With that line, `cargo metadata --locked` on the root workspace passes and `Cargo.lock` does not change (checked locally). The dependencies are smoltcp 0.14 and `managed` 0.8 (0BSD) with `heapless`, `hash32`, `byteorder`, `bitflags`, `stable_deref_trait`, and `cfg-if` (MIT or Apache-2.0, or Unlicense or MIT), all licenses of `deny.toml`. `cargo deny` runs on the root workspace and does not see this crate.

```bash
cd spikes/s5-smoltcp-flow-table/flowtable
cargo test --release
cargo run --release -- --n 16000 --baseline          # one size, with the loop baseline
cargo run --release -- --n 1000 --n 4000 --n 16000   # a sweep; defaults are 1000, 4000, 16000, 64000
```

Options: `--rounds R` (burst rounds, default 3), `--single K` (isolated round trips, default 2,000; the baseline uses at most 500), `--baseline`. A run exits with status 1 when a check fails. The baseline takes a few seconds at 16,000 connections and is too slow for 64,000.
