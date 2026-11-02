# Spike S2 results

We ran `fps-spike-tls --duration 4` on 2026-11-02 in the Phase 0 build environment (Intel Xeon at 2.10 GHz, 2 vCPU Firecracker microVM, rustls 0.23.46 with ring 0.17.14). Each figure is handshakes per second on one core. Raw JSON is in `results/phase0-container/handshakes.json`.

| TLS | Leaf key | Group | Mode | Client hs/s | Server hs/s | Both sides hs/s |
|---|---|---|---|---|---|---|
| 1.3 | ECDSA P-256 | X25519 | Full | 3,749 | 6,596 | 2,390 |
| 1.3 | ECDSA P-256 | X25519 | Resumed | 9,062 | 8,569 | 4,404 |
| 1.3 | ECDSA P-256 | secp256r1 | Full | 4,279 | 7,779 | 2,761 |
| 1.2 | ECDSA P-256 | X25519 | Full | 4,450 | 8,984 | 2,976 |
| 1.2 | ECDSA P-256 | X25519 | Resumed | 66,900 | 66,934 | 33,459 |
| 1.3 | RSA 2048 | X25519 | Full | 4,809 | 1,394 | 1,080 |
| 1.3 | RSA 2048 | X25519 | Resumed | 9,072 | 8,572 | 4,407 |
| 1.3 | RSA 2048 | secp256r1 | Full | 4,635 | 1,405 | 1,078 |
| 1.2 | RSA 2048 | X25519 | Full | 4,434 | 1,286 | 997 |
| 1.2 | RSA 2048 | X25519 | Resumed | 60,204 | 60,107 | 30,078 |

Every resumed run resumed 100% of its handshakes, and every full run resumed none.

## Findings

1. RSA 2048 signing dominates server cost. A full TLS 1.3 handshake costs the server about 718 µs with an RSA leaf and 152 µs with an ECDSA leaf, a factor of 4.7.
2. Client cost stays between 208 and 267 µs per full handshake for every leaf type, because certificate verification and key exchange dominate it.
3. TLS 1.3 resumption skips the certificate signature but still performs an ECDHE exchange. It raises the both sides rate 1.8 times with an ECDSA leaf and 4.1 times with an RSA leaf. TLS 1.2 session resumption also skips key exchange and raises the rate 11 times with ECDSA and 30 times with RSA.
4. On this CPU, secp256r1 key exchange ran 16% faster than X25519 for full TLS 1.3 handshakes with both sides emulated.

## Proposed TLS targets

We propose targets for full TLS 1.3 handshakes with an ECDSA P-256 leaf and X25519, assuming half of each core's time goes to TLS and the rest to TCP and application work.

| Tier | Cores | Client only (client simulation) | Both sides (application simulation) |
|---|---|---|---|
| T0 | 4 vCPU | 7,500 hs/s | 4,500 hs/s |
| T1 | 16 vCPU | 30,000 hs/s | 19,000 hs/s |
| T2 | 32 cores | 60,000 hs/s | 38,000 hs/s |

RSA 2048 server certificates lower the both sides targets by a factor of 2.2 (2,390 against 1,080 handshakes per second per core). The test report should therefore show the certificate type next to every TLS rate.

## Limits

- The benchmark runs in memory on one thread. It excludes TCP, system calls, and cache contention between cores.
- The CPU is a 2.10 GHz virtualized Xeon. Newer server CPUs will run faster, so Phase 3 should repeat S2 on the T1 and T2 reference hardware.
- The client verifies the server chain. An agent that skips verification would lower client cost, but it would also stop exercising the same code paths as a real client.
