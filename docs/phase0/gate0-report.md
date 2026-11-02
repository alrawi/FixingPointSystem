# Gate 0 report

Phase 0 is complete. Spike S3 ran on 2026-11-02 on a small KVM host and reached the 10 Gbps packet rate at 1,500 bytes with two queues, an indicative result. The kernel backend and the bytecode interpreter meet tier T0 on 2 vCPU. TRex ASTF does not qualify as the tier T2 backend, so we plan an F-Stack spike for Phase 3.

## Criteria

| Criterion | Status | Evidence |
|---|---|---|
| Contracts v0.1 validate in CI | Met locally; CI runs on the first push | `tools/contracts/validate.py` reports 0 errors; 42 validator tests pass |
| Each spike reports a measured result or a documented blocker | Met | S1, S2, S3, and S4 measured |
| The tier T2 engine path is recorded | Met | S4 recommendation; decision record 0001 updated |
| TLS performance targets are recorded | Met (proposed) | Table below, from S2 |
| The owner signs off | Pending | Approval on the Gate 0 pull request |

## Spike outcomes

| Spike | Question | Outcome |
|---|---|---|
| S1 engine | Sessions per second and overhead of the interpreter | 27,693 new sessions/s and up to 40.7 Gbit/s on 2 vCPU with both sides emulated over loopback. Overhead against hand written code is 1.1% for connection rate, 4.1% for keepalive, and 26.1% for 1 MiB bodies. |
| S2 TLS | Handshakes per core | Full TLS 1.3 with ECDSA P-256 and X25519 costs 267 µs on the client and 152 µs on the server. An RSA 2048 leaf raises server cost to 718 µs. |
| S3 AF_XDP | Packet rate on virtio-net in KVM | Indicative pass on a small host (8 threads, 3 vCPU guests). The best 1,500 byte receiver rate is 866,294 pps (10.5 Gbps) with the HWE kernel 7.0 and two queues; the GA kernel 6.8 reaches 845,141 pps. The best 64 byte rate is 1,335,363 pps. Default and forced copy modes differ by less than run to run noise of about 20%. See `spikes/s3-afxdp/RESULTS.md`. |
| S4 TRex ASTF | Lossless translation and real TLS | 73% of `http1/web_get` actions translate exactly and the rest approximately. ASTF replays recorded TLS bytes and performs no key exchange. |

## Proposed TLS targets

Full TLS 1.3 handshakes, ECDSA P-256 leaf, X25519, half of each core's time spent on TLS.

| Tier | Client only | Both sides |
|---|---|---|
| T0 (4 vCPU) | 7,500 hs/s | 4,500 hs/s |
| T1 (16 vCPU) | 30,000 hs/s | 19,000 hs/s |
| T2 (32 cores) | 60,000 hs/s | 38,000 hs/s |

## Decision D1 update

- Tiers T0 and T1 use the kernel backend, then AF_XDP. S3 shows that two AF_XDP queues on virtio-net carry 10 Gbps of 1,500 byte frames, so tier T1 remains feasible on KVM. The image should prefer the HWE kernel.
- Tier T2 does not use TRex ASTF. A Phase 3 spike evaluates F-Stack as a DPDK backend behind the interpreter's `SessionIo` trait.
- The S4 translator stays as an optional export to TRex for cleartext, deterministic throughput tests.

## Items carried into Phase 1

1. Write `fill()` bodies as vectored writes from the shared filler buffer, then rerun the S1 bulk workload against the 25% overhead criterion.
2. Done on a small host. TRN-11 must repeat S3 at the full size (16 vCPU, up to 8 queues) to measure scaling, and must verify the kernel XSK bind mode directly, because the DPDK log line that S3 counted appears in both modes.
3. Repeat S1 on a 4 vCPU KVM guest with virtio-net, because loopback hides NIC and virtualization cost.
4. Add UDP, `spawn`, and transport modules to the interpreter, which S1 left out of scope.

## Licensing check

`cargo deny check licenses` passes for every crate in the workspace. The TLS stack uses rustls with the ring provider (Apache-2.0 AND ISC). The crates.io metadata for aws-lc-sys 0.47.0 lists only licenses compatible with GPL-3.0, so a later switch of provider remains open.
