## [2026-11-18] plan | Batch 39 wrap-up: the own stack packages merged, four follow-ups added

Since the TRN-24 entry the coordinator merged, each after all of CI passed and Jev scored every rule below 0.2:

- TRN-25 in PR #649: CUBIC as the default congestion control of the own stack. On the hosted runner the fat path reached 21.06 Mbit/s against 20.46 for the kernel with CUBIC, and four flows shared the link with a Jain index of 0.919 to 0.975.
- TRN-27 in PR #650: `engine/fps-datapath`, the per queue loop of the AF_XDP backend. A veth pair carried 6,908 sessions/s.
- TRN-26 in PR #651: the interop matrix of the own stack with smoltcp and the kernel.
- Bug #652 in PR #654: the own sender in a short bottleneck queue went from 3.6 Mbit/s with 517 retransmissions to 19.2 Mbit/s with 46.
- TRN-29 in PR #653: user space UDP for the AF_XDP backend. The dns module completes its scenarios over it on a veth pair.

AGT-24 (PR #655) is in review: a forced AF_XDP run starts the datapath and reports the mode each socket bound in.

The coordinator added six packages to the plan from what these packages found:

- TRN-33: the fps-tcp-own API that TRN-27 and TRN-29 worked around (`OwnSession::local_addr` and `peer_addr`, `OwnStack::unlisten`, `Pools::tables`, and optionally an input and take_tx seam per side). TRN-12 and TRN-31 depend on it.
- TRN-34: pacing, HyStart, TLP and RACK in the own stack, because after bug #652 the own sender still retransmits 40 to 70 segments where the kernel retransmits 9. It does not block the flip of TRN-31; the vetting of TRN-31 decides whether it should.
- TRN-35: ARP requests and neighbor solicitations from the user space stacks, because TRN-26 found that the own stack sends none, so a peer behind a router needs the next hop MAC address by hand. TRN-31 depends on it.
- ENG-31: a Transport trait in `fps-runtime`, because the workers open sessions only through the kernel backend, so a forced AF_XDP run binds its queues and then stops with `Unimplemented` (AGT-24 finding).
- AGT-25: the agent runs its TCP and UDP workers on the datapath through that trait (AGT-24 and TRN-29 findings). TRN-12 and TRN-31 depend on it.
- CON-35: a contract field for the XDP mode that each port really bound in, which today reaches only the agent log because Register comes before any plan (AGT-24 finding). It does not block TRN-31.

The UDP layer reads no ICMP, so a module that waits for a refused UDP port times out on the AF_XDP backend; Module Development says so, and an ICMP hook waits until a module needs it.

The coordinator also added the `interop` job and the dns step to `trn.yml`, and the AF_XDP test of the agent to its `afxdp` job, so they keep their numbers as artifacts and run when only `engine/fps-tcp` or `engine/fps-agent` changes.

TRN-13 stays parked by the owner; the promote sweep marked it ready twice, and the coordinator returned it to backlog both times.

### Shared page notes

Applied by the coordinator in the same pull request: [Testing](../Testing.md) ("Since batch 39"), `.github/workflows/trn.yml`, and the catalog entries TRN-33, TRN-34, TRN-35, ENG-31, AGT-25, CON-35, TRN-12 and TRN-31. The Transport page change of AGT-24 went into PR #655.
