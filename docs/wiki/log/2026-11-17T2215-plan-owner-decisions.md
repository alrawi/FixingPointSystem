## [2026-11-17] plan | Owner decisions on Gate 3, TRN-13, and the strike module

The owner decided four open points on 2026-11-17:

- **0BSD.** Allowed. `deny.toml` lists it since PR #612, and TRN-20 (smoltcp) is running. Its fat-nuc results follow once the relay account on fat-nuc has a Rust toolchain, which is still an owner action.
- **The Gate 3 host.** The owner first named fat-nuc. fat-nuc has 8 vCPUs (4 cores, 8 threads), so a guest of 16 vCPUs would share cores, and its throughput would not measure what tier T1 means. Asked about this, the owner kept tier T1 at 16 vCPUs and will find a larger host later. QA-07 keeps its owner action, and TRN-19 waits for the same host.
- **TRN-13.** Parked. The DPDK spike needs an SR-IOV network card or a spare network card that DPDK supports, and AF_XDP stays the fast path.
- **The strike module.** The owner's developer implements the rest by hand from a specification in an open issue, and the coordinator reviews each pull request against it and merges it. The specification is #613. It covers four parts:
  - MOD-32, to finish PR #558;
  - the rest of MOD-27: the byte estimates, the Modules page, and the strike type in Module Development;
  - MOD-29a, three raw strikes over a session;
  - MOD-29b, a written account of floods and spoofed source pools.

  PR #492 holds an older copy of the module that #538 replaced, so it can close without a merge.

### Shared page notes

None.
