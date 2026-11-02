# Phase 0: Spikes and Contracts

Phase 0 decides the engine path and fixes the v1 contracts before parallel work starts. It closes at Gate 0, which requires a recorded engine decision.

## Deliverables

1. Contracts v0.1 in `contracts/`, validated in CI.
2. Four spikes, each with a brief, runnable code, and a results file.
3. A Gate 0 report in `docs/phase0/gate0-report.md`.

## Spike briefs

### S1. Engine benchmark (kernel backend)

- Question. How many flow program sessions per second, concurrent sessions, and bits per second does one core of a Rust kernel socket backend sustain?
- Method. A single binary runs the flow program interpreter as client and server over loopback. We compare the interpreter against a hand written baseline that sends the same bytes, so the gap measures interpreter overhead.
- Metrics. Sessions per second per core, session latency p50 and p99, throughput, failed sessions.
- Pass. The kernel backend meets tier T0 (5,000 sessions per second and 1 Gbps on 4 vCPU), and interpreter overhead stays under 25% of the baseline session rate.
- Location. `spikes/s1-engine/`

### S2. TLS cost per core

- Question. How many TLS handshakes per second does one core complete for each client and server profile?
- Method. An in memory rustls client and server pair runs full and resumed handshakes for TLS 1.2 and 1.3 with ECDSA P-256 and RSA 2048 certificates.
- Metrics. Handshakes per second per core for each combination, and the resumption speedup.
- Pass. The results set the TLS rows of the performance targets in the specification.
- Location. `spikes/s2-tls/`

### S3. AF_XDP on virtio-net

- Question. What packet rate does AF_XDP reach inside a KVM guest with virtio-net, in copy mode and in zero copy mode, and which guest kernel enables zero copy?
- Method. A harness boots an Ubuntu LTS cloud image under KVM with vhost-net and multiqueue virtio-net, then runs `xdp-bench` and `xdpsock` style transmit and receive tests.
- Metrics. Packets per second at 64 and 1,500 bytes, CPU per Mpps, zero copy availability.
- Pass. AF_XDP reaches 10 Gbps of 1,500 byte frames on 16 vCPU, which tier T1 needs. Otherwise we plan a DPDK virtio path for T1.
- Location. `spikes/s3-afxdp/`. This spike needs a KVM host and does not run in CI.

### S4. TRex ASTF adapter

- Question. Can a TRex ASTF profile express our scenario model without loss, and does ASTF perform real TLS key exchange?
- Method. A translator converts our test configuration and HTTP scenarios into ASTF Python profiles. A source review of TRex answers the TLS question.
- Metrics. The share of flow program actions that translate without loss, and the TLS finding with source references.
- Pass. The translator round trips every v1 HTTP scenario, and the TLS finding cites TRex source.
- Location. `spikes/s4-trex-astf/`

## Gate 0 criteria

| Criterion | Evidence |
|---|---|
| Contracts v0.1 validate in CI | Passing `contracts` CI job |
| Each spike reports a measured result or a documented blocker | `spikes/*/RESULTS.md` |
| The tier T2 engine path is recorded | Gate 0 report, decision D1 update |
| TLS performance targets are recorded | Gate 0 report |
| The owner signs off | Approval on the Gate 0 pull request |
