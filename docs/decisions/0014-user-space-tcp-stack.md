# D14. User space TCP stack for the AF_XDP backend

- Status: Accepted (vetting agent under D13, 2026-11-17). Proposed by TRN-10.
- Date: 2026-11-17
- Deciders: vetting agent under decision D13 (technical decision record), unless the open questions at the end are answered "defer to owner"

## Context

Decision D1 puts the kernel socket backend first and the AF_XDP backend (tier T1: 10 Gbps, 100,000 new sessions per second, and 2,000,000 concurrent sessions per agent on a 16 vCPU guest) behind it. AF_XDP hands the engine raw frames, so a TCP stack must run in user space behind the `SessionIo` trait. TRN-12 builds that path and is a large package that waits for this record, because the stack decides how TRN-12 splits. D1 rejected "an own stack from scratch" as the engine strategy, and that stands: the kernel backend comes first. This record only picks the TCP layer behind `SessionIo` for the AF_XDP backend.

Three candidates exist.

- **smoltcp 0.14**, a Rust TCP/IP stack for embedded systems, licensed 0BSD. The owner allowed 0BSD in `deny.toml` on 2026-11-17 (PR #612).
- **The own stack** of TRN-18 (`engine/fps-tcp/src/own/`), a minimal stack of ours over `FrameIo` queues of `fps-backend-afxdp`.
- **F-Stack**, the FreeBSD TCP stack on DPDK. TRN-13 would measure it on SR-IOV or bare metal, and the owner parked TRN-13 on 2026-11-17 because no host has such a network card. We assess it on paper.

The kernel loopback TCP is the reference that a user space stack must justify itself against. TRN-17 built the bench harness and TRN-18 and TRN-20 plugged the stacks into it, so all three run the same three workloads through the same `SessionIo` adapter.

This record must not change decision D1 or a tier target. Where the evidence touches either, it is an open question for the owner at the end.

## Options

- smoltcp, with the engine owning sharding (proposed as the fallback)
- The own stack, extended with loss recovery and congestion control (proposed)
- F-Stack behind the DPDK backend (not measured, paper assessment only)
- A hybrid that takes the connection logic of one and the flow table and timers of the other

## Measurements

### Host and what is the same

All three stacks ran in **one session on one host**, the 2 vCPU agent sandbox (Xeon at 2.10 GHz, 8 GiB, kernel 6.18.44, rustc 1.97.0, load average 0.5 at the start), with the same binary, in the order recorded in `engine/fps-tcp/results/sandbox-2vcpu-trn10/host.txt`. That run is the one for this record. It repeats the rows that TRN-20 measured (rate, throughput, the smoltcp scaling) and also repeats the million socket row of the own stack, which the TRN-20 table took from the earlier TRN-18 run. The older runs agree with it within the noise of the shared host: smoltcp 59,000 to 66,000 against 56,000 to 58,000 sessions per second, the own stack 228 against 229 B per socket at 1,000,629 sockets.

**One row is not from the same session.** The kernel cannot reach a million sockets in this sandbox, because the open files limit is 20,000 soft and hard and a non root user cannot raise it. The kernel run stopped at 19,815 sockets in this session as it did in TRN-17 and TRN-18, so the kernel ceiling is a limit of the host and not of the kernel, and the kernel row at 1,000,000 sockets is **not measured**. The kernel has one file descriptor per socket, so reaching 1,000,000 needs a limit above 1,000,100 and a high `fs.nr_open`, which fat-nuc can give. The relay account of fat-nuc has no Rust toolchain yet, so no number of this record is from fat-nuc.

All results are sandbox results. They are not from the GitHub runner and not from fat-nuc. A stack runs both hosts of a pair on one core in one process over a link without delay, loss, or reordering, and no NIC or AF_XDP is involved. Read a rate as the cost of the state machine, the checksums, the frame copies, and the adapter, and not as the rate of an agent that talks to a remote peer. Differences below a factor of two between stacks are noise on a shared host.

### Sessions per second

32 and 256 concurrent sessions, a 64 byte request and a 64 byte response, three alternating runs per cell, from `rate-sweep.txt`.

| Stack | 32 concurrent | p50 | 256 concurrent | p50 |
|---|---|---|---|---|
| smoltcp | 55,651 to 58,148 | 0.51 to 0.56 ms | 43,373 to 47,932 | 4.9 to 5.4 ms |
| Own stack | 34,573 to 35,518 | 0.86 to 0.87 ms | 33,619 to 35,555 | 6.8 to 7.0 ms |
| Kernel loopback | 14,404 to 44,203 (one run hit 14,404 with a p99 of 66 ms) | 0.65 to 0.71 ms | 30,604 to 32,570 | 7.3 to 7.6 ms |

smoltcp is the fastest with few sockets. Its rate uses TIME-WAIT skip, so a closed tuple is reused at once, which the kernel and the own stack do not do, so its rate is an upper bound. The own stack is as fast as the kernel and flat between 32 and 256 sessions.

### Throughput

Four concurrent uploads of 64 MiB, MTU 1500, one core, application Gbit per second. The workload moves 256 MiB in about 0.1 s, so each figure is one noisy sample (the small profile gave 26.2, 14.8, and 20.1 in the same session).

| Stack | Throughput |
|---|---|
| smoltcp | 27.0 Gbit/s |
| Kernel loopback | 19.8 Gbit/s |
| Own stack | 16.7 Gbit/s |

All three are above the 10 Gbps of tier T1 for one core, with no NIC cost counted. The own stack limits one flow to its 65,535 byte window, which at a 1 ms round trip is about 0.5 Gbit/s per session, so the own stack needs window scaling before a real path (see the follow-up packages). smoltcp has window scaling.

### Socket scaling

Sockets are counted for both hosts of a pair, spread over 64 shards (one independent pair per shard, as one engine worker owns its sessions), run on one thread. The probe is the session rate that a probe sees while all sockets are held.

| Sockets | smoltcp B/socket | smoltcp idle poll | smoltcp probe per s | Own B/socket | Own idle poll | Own probe per s |
|---|---|---|---|---|---|---|
| 1,000 | 1,724 | 16 us | 26,003 | 440 | 25 us | 61,412 |
| 10,000 | 1,905 | 393 us | 14,284 | 411 | 25 us | 43,465 |
| 100,000 | 1,688 | 4.4 ms | 2,706 | 260 | 25 us | 55,675 |
| 250,000 | 1,645 | 13.4 ms | 777 | 249 | 34 us | 51,225 |
| 500,000 | 1,626 | 27.7 ms | 196 | 236 | 37 us | 51,280 |
| 750,000 | 1,615 | 56.2 ms | 111 | 261 | 25 us | 57,449 |
| 1,000,000 | 1,591 | 53.9 ms | 55 | 229 | 36 us | 55,819 |
| Setup of the last step | 13.1 s (19,082 sockets/s) | | | 0.72 s (348,371 sockets/s) | | |

The kernel in the same session: 1,089 sockets at 489 B per socket and 45,177 probe sessions per second, 10,177 sockets at 464 B and 44,466 per second, then a stop at 19,815 sockets. Its bytes per socket are user space only, because kernel socket memory is not in the resident set of the process. Its idle poll is zero by construction.

A single shard with no sharding (the small profile) shows the same effect on smoltcp without the 64 way split: 1,016 sockets give 4,616 probe sessions per second, 4,016 give 864, and 16,016 give 119, a factor of 40 from 1,000 to 16,000 sockets in one set. The idle poll of one smoltcp set grows from 10 us to 359 us (22 to 24 ns per socket). The own stack in one shard stays between 48,000 and 63,000 probe sessions per second at 1,002 to 16,202 sockets, and the kernel gives about 40,000 per second at 16,002 sockets.

What the numbers cannot show: neither user space stack was tested with loss, delay, reordering, or a slow receiver. The own stack never retransmitted in these runs, and smoltcp's recovery code never ran. A comparison on a lossy path needs the work that the own stack leaves out, and that work is the main cost of choosing it.

## F-Stack, paper assessment

TRN-13 is parked, so nothing below is measured. We read the project README on 2026-11-17 and recall the rest; the items that we could not confirm in this session are marked.

- **Architecture.** F-Stack is the FreeBSD TCP/IP stack (the README names 11.0 and 13.0-STABLE) ported to user space on top of DPDK, with a multi process model in which each process owns a NIC queue and a stack instance, and an epoll and kqueue compatible API (`ff_*` functions, plus a micro thread interface). It is aimed at servers that accept connections (Nginx and Redis ports exist). It claims 10 million concurrent connections, 5 million requests per second, and 1 million connections per second on suitable hardware, which are the project's figures and not ours.
- **Fit with AF_XDP.** Poor. F-Stack takes frames from the DPDK poll mode drivers, bundles its own DPDK build, needs hugepages, NIC binding (`igb_uio` or `vfio`) or a KNI interface, and (per the README) ASLR disabled for its multi process mode. It has no AF_XDP frame source in the README. A port to our `FrameIo` trait would replace its DPDK ethdev layer, and its kqueue style interface and global stack state would need an adapter to `SessionIo`, with the stack owning the thread. It fits tier T2 (DPDK on SR-IOV or bare metal), which is what the Gate 0 update of D1 chose it for, and not tier T1.
- **Features.** It has the full FreeBSD TCP with loss recovery, selective acknowledgment, window scaling, timestamps, and a choice of congestion control, which is the one thing that both of our candidates lack or have untested. The cost is a C codebase of a size that dwarfs the other two, linked into a Rust workspace through FFI.
- **Realism.** The engine needs per profile TCP behavior (initial window, options order, timestamps, window scaling values) for operating system profiles. The FreeBSD stack mimics FreeBSD, and bending it to a Windows or Linux profile means patching a large stack. The own stack and smoltcp are small enough to shape.
- **License.** The project describes its stack as BSD licensed and its repository carries a `LICENSE` file; we expect BSD-2-Clause, and we could not read the file in this session (the fetch was not permitted), so TRN-13 must confirm it before any code lands. FreeBSD sources are BSD-2-Clause or BSD-3-Clause, and DPDK's core is BSD-3-Clause. DPDK's Linux kernel modules (`igb_uio`, and KNI in older releases) are GPL-2.0, but they are separate kernel modules and not linked into the engine. BSD-2-Clause and BSD-3-Clause are GPL-3.0 compatible. `deny.toml` checks only Cargo crates, so F-Stack, a C library, would not pass through `cargo deny` and needs a review of its own license files, with `deny.toml` unchanged. `deny.toml` already allows BSD-2-Clause and BSD-3-Clause, so a `-sys` crate that wraps it would pass.
- **Maintenance.** The project is a Tencent open source project with releases and commits over several years. We did not read its recent history in this session, so we do not claim its current state. TRN-13 should record the date of the last release, the supported DPDK version, and the open issues, because an old DPDK pin would tie the build to an old kernel and toolchain.
- **Verdict on paper.** F-Stack stays a T2 candidate for TRN-13 and is not a candidate for the AF_XDP backend.

## License review

| Candidate | License | Compatible with GPL-3.0 (AGENTS.md rule) | `deny.toml` | Notes |
|---|---|---|---|---|
| smoltcp 0.14 and `managed` 0.8 | 0BSD | Yes. 0BSD is a public domain style license with no conditions | Allowed since 2026-11-17 (PR #612), and `cargo deny check` passes with the harness dependency | 0BSD permits copying code into our stack with or without attribution. Its transitive crates (`heapless`, `hash32`, `bitflags`, `defmt` family) are in `Cargo.lock` already and pass |
| Own stack | Ours, GPL-3.0 like the workspace (D4) | Yes | No new dependency. The only path crate is `fps-backend-afxdp` | We own the code and its realism behavior |
| F-Stack | BSD style, expected BSD-2-Clause (unconfirmed, see above). DPDK core BSD-3-Clause. DPDK kernel modules GPL-2.0, not linked | Yes for the BSD parts | Not seen by `cargo deny`, so a license review of the C tree is part of TRN-13 | Needs hugepages and NIC binding on the host, which is a host change that D13 keeps with the owner |

No candidate needs a change to `deny.toml`.

## Decision

We propose the following, in this order of preference.

1. **The AF_XDP backend (TRN-12) runs the own stack of TRN-18, extended with loss recovery and congestion control.** The own stack keeps one flat cost per event: a slab connection of about 230 B, a timer heap with one entry per active connection, and an idle poll that does not walk the sockets. It holds 1,000,629 sockets at 229 B each and still gives 56,000 probe sessions per second with all of them open. The tier T1 target of 2,000,000 concurrent sessions per agent is 125,000 sockets per core at 16 vCPU, which costs about 29 MB per core on the own stack and 200 MB per core on smoltcp with 512 B buffers. The T1 rate of 100,000 new sessions per second is about 6,250 per core, which the own stack passes by a factor of five with all million sockets open, while one smoltcp set with 1,000 sockets gives 4,600 probe sessions per second. The numbers are for a lossless in memory link, so they do not prove the target, and TRN-12 measures it.
2. **smoltcp stays in the harness as a reference and a test oracle, not in the engine.** It is the fastest stack with few sockets, it already has the features that the own stack lacks (window scaling, selective acknowledgment, an out of order queue, delayed acknowledgment, a round trip estimator, fast retransmit, and optional Reno and CUBIC through the Cargo features `socket-tcp-reno` and `socket-tcp-cubic`, which the harness does not enable), and the interop tests of the own stack can run against it over a lossy link. Because 0BSD allows copying, the follow-up packages may port its self contained parts (the congestion control modules and the segment assembler) into the own stack, after a read of each file, instead of writing them again.
3. **The hybrid in the other direction is the fallback.** If the loss recovery package for the own stack does not pass its interop and loss tests within its size, TRN-12 switches to smoltcp behind a flow table and a timer wheel of our own, with sets of at most a few hundred sockets and a scheduler that polls only the sets that have work. This record does not choose it, because the measurements show that smoltcp needs that machinery around it to reach the T1 socket count. The fallback is unverified: we have not checked that smoltcp's socket can run outside its interface loop. A one day spike under `spikes/` answers that question early, in parallel with the loss recovery work, so the fallback is known to work before it is needed.
4. **F-Stack is not a candidate for the AF_XDP backend** and stays with TRN-13 for tier T2, when a host exists.

### Why the own stack and not smoltcp

| Criterion | Own stack | smoltcp |
|---|---|---|
| Cost of one event as sockets grow | Flat from 1,000 to 1,000,000 sockets | Grows with the sockets of a set: 24 ns per socket per poll, a factor of 40 from 1,000 to 16,000 sockets in one set |
| Memory per socket | 229 B, buffers on demand | 1.6 KB with 512 B buffers fixed per socket |
| Million sockets | Setup in 0.72 s | Setup in 13.1 s, and only with 64 sets and a flow table of our own |
| Few sockets, rate and throughput | 34,000 to 35,000 sessions per second, 16.7 Gbit/s | 56,000 to 58,000 sessions per second, 27.0 Gbit/s |
| Loss recovery and congestion control | Minimal: a fixed timeout of 200 ms that doubles, go back N, no fast retransmit, no congestion control, no window scaling (64 KiB window), no out of order queue. Untested with loss | Present, and untested by us. Congestion control needs a Cargo feature that the harness leaves off |
| Operating system profile control | Ours, with every option and its order under our control | A library with its own option order and defaults, which the profile layer would patch or fork |
| Maintenance | Ours | Upstream, 0BSD, a small project aimed at embedded systems |

The own stack wins on the property that the tier targets depend on and loses on the property that makes a network correct under loss. We choose the property that is hard to add later (the flat cost) and price the one that we can add and test (recovery) in the follow-up packages. We accept the risk that the recovery work is larger than we expect, and the fallback in item 3 is the limit on that risk.

## Consequences

- TRN-12 splits after this record is accepted. The proposed children (the plan update belongs to the coordinator, and TRN-12 lists connection setup, data transfer, timers and retransmission, OS profile options, and the agent wiring child):
  1. **Loss model for the harness** (`engine/fps-tcp/`): a lossy, delaying, and reordering link for the user space stacks and a `tc netem` counterpart for the kernel, with a loss workload that reports goodput and recovery time at 0.1, 1, and 5 percent loss and 1, 10, and 50 ms of delay. It measures what this record could not, for the own stack and for smoltcp.
  2. **Own stack, RTT estimator and fast recovery**: RFC 6298 timers, fast retransmit, and an out of order queue.
  3. **Own stack, window scaling, timestamps, selective acknowledgment, and delayed acknowledgment.**
  4. **Own stack, congestion control**: New Reno first, CUBIC second, ported from smoltcp where the code fits.
  5. **Own stack, interop and conformance against smoltcp and the kernel**, over the lossy link of item 1, and with `packetdrill` style scripts when the tool is available.
  6. **Own stack on AF_XDP queues** through `OwnStack::with_links` (TRN-12 proper, after TRN-11), with one stack per worker and the engine's flow steering to the right queue.
  7. **OS profile options** for the own stack (initial window, MSS, options order, timestamps, window scale), against the capture checks that the TLS and profile tests already use.
  8. **Agent wiring** (`engine/fps-agent/`, `controller/internal/wiring/`), as TRN-12 already lists.
  9. **Fallback spike**, a one day spike under `spikes/` that runs early: can a smoltcp socket run behind our flow table and timer wheel, outside its interface loop?
- The first item of the list needs no decision beyond this record and unlocks the others, and items 2 to 4 are independent of each other once the loss link exists.
- The decision does not touch the kernel backend, which stays first and which is the reference of every comparison.
- smoltcp stays a dependency of `fps-tcp` (the harness), so `cargo deny check` keeps passing and the license note above holds.
- The own stack is about 2,300 lines today (TRN-18) and grows by the amount that items 2 to 5 add. We carry its security fixes ourselves, which includes initial sequence number generation, which TRN-18 documents as a mixing function and not a keyed cryptographic one.

## Revisit when fat-nuc numbers arrive

The numbers of this record are from the sandbox. The relay account of fat-nuc has no Rust toolchain, so the host tests of TRN-17, TRN-18, and TRN-20 have not run there. When the owner installs one, the same `tcp_bench` argv should run there, with the open files limit above 1,000,100 for the kernel row. **Revisit this decision when those numbers arrive** if any of these holds on fat-nuc:

- the own stack falls below the kernel by a factor of two or more on sessions per second, or below 50,000 probe sessions per second at 1,000,000 sockets;
- smoltcp holds a probe rate above 6,250 per second with sets of more than 1,000 sockets, which would remove the main reason against it;
- the kernel reaches 1,000,000 sockets with a rate and a memory cost that make the user space stack hard to justify outside the AF_XDP case (a question for the owner, see below).

Otherwise the decision stands, and the loss measurements of the follow-up package 1 are the next evidence.

## Open questions for the owner

These touch D1 or a tier target, so this record only asks them.

1. **Does the own stack replace F-Stack for tier T2?** The Gate 0 update of D1 says a Phase 3 spike evaluates F-Stack as a DPDK backend. If the own stack takes frames from a DPDK poll mode driver through the same `FrameIo` trait that AF_XDP uses, tier T2 might need no second TCP stack, and TRN-13 could end in a decision to extend the own stack and not to port F-Stack. That would change what D1 says about T2, so this record does not decide it. TRN-13 stays parked. We suggest the owner decides after the loss measurements, not now.
2. **Does the T1 target of 2,000,000 concurrent sessions per agent still hold in the user space path?** With both ends of each session on one agent, 2,000,000 sessions are 4,000,000 sockets, so the own stack needs about 0.9 GB of connection state before buffers, and a connection that holds unread data costs more. This record does not change the target, and the estimate fits a 16 vCPU guest. We list it because the target is the owner's, and the first real number comes from TRN-12 on AF_XDP.
