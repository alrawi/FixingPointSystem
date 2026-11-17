## [2026-11-17] ingest | TRN-10 User space TCP stack decision

TRN-10 proposes decision record 0014 (`docs/decisions/0014-user-space-tcp-stack.md`, status Proposed) for the TCP stack of the AF_XDP backend. It compares smoltcp, the own stack of TRN-18, and F-Stack (paper only, because the owner parked TRN-13), with a license review of all three. It changes neither D1 nor a tier target, and the two places where the evidence touches them are open questions for the owner.

The measurements come from a new run in `engine/fps-tcp/results/sandbox-2vcpu-trn10/`, where the kernel, the own stack, and smoltcp ran in one session on one 2 vCPU sandbox with the existing `tcp_bench` and no engine change. It repeats the million socket step of the own stack, which TRN-20 had taken from the TRN-18 run. One row is not measured: the kernel at a million sockets, because the sandbox allows 20,000 open files and the hard limit cannot be raised, so the kernel stops at 19,815 sockets as in TRN-17 and TRN-18.

Finding, the three stacks in one session. Sessions per second at 32 concurrent: smoltcp 55,651 to 58,148, own 34,573 to 35,518, kernel 14,404 to 44,203. Throughput of four uploads: smoltcp 27.0 Gbit/s, kernel 19.8, own 16.7. At 1,000,000 sockets in 64 shards: own 229 B per socket, a 0.72 s setup, a 36 us idle poll, and 55,819 probe sessions per second; smoltcp 1,591 B per socket, a 13.1 s setup, a 54 ms idle poll, and 55 probe sessions per second.

Finding, the recommendation. The own stack is the base of TRN-12, extended with a round trip estimator, fast retransmit, an out of order queue, window scaling, selective acknowledgment, and congestion control, with smoltcp kept as the reference and the test oracle. smoltcp behind a flow table and a timer wheel of our own is the fallback, started by a spike if the recovery work misses its acceptance. F-Stack stays a tier T2 candidate for TRN-13.

Finding, what is unmeasured. Neither user space stack ran with loss, delay, or reordering, and smoltcp's congestion control features (`socket-tcp-reno`, `socket-tcp-cubic`) are off in the harness. The first follow-up package is a loss model for the harness. The F-Stack license (expected BSD-2-Clause) and its maintenance state were not confirmed in this session, because the fetch of its repository files was not permitted, and TRN-13 must confirm them.

The fat-nuc numbers are pending, because the relay account has no Rust toolchain. Record 0014 lists when to revisit the decision once they arrive.

### Shared page notes

- Plan (coordinator): TRN-12 can be split into the children that record 0014 lists under Consequences once the vetting agent accepts the record. The accepted record should change the status line from Proposed to "Accepted (vetting agent under D13, <date>)", and the index line in `docs/decisions/README.md` the same way. The `owner_action` of TRN-10 is done by that acceptance.
- Plan (coordinator): the open question about tier T2 and F-Stack is for the owner. If the owner agrees that the own stack may serve T2 through a DPDK frame source, TRN-13 would change, so it stays parked.
- [Transport](Transport.md): the section "Decision record 0014 and the one session run" is added in this package.
- [Testing](Testing.md), [Module Development](Module-Development.md), [Home](Home.md), [Implementation Plan](Implementation-Plan.md): none.
