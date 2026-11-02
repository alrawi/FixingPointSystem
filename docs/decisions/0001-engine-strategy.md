# D1. Engine strategy

- Status: Accepted
- Date: 2026-10-30
- Deciders: repository owner

## Context

We need an engine that produces full TCP and TLS fidelity on every hypervisor in v1 and reaches tier T2 performance later.

## Options

- Own controller and flow programs, kernel backend first, TRex adapter spike (chosen)
- Build fully on TRex ASTF
- Own DPDK engine on F-Stack
- Own stack from scratch

## Decision

We build our own controller and flow program interpreter. The kernel socket backend ships first. Phase 0 spike S4 evaluates TRex ASTF as a scale adapter, and spike S1 measures the kernel backend.

## Consequences

We control the module contract and the realism behavior. Tier T2 depends on the outcome of spike S4 or a later F-Stack spike.

## Update after Gate 0 (2026-11-02)

Spike S4 found that TRex ASTF replays recorded TLS bytes and waits on byte watermarks, so it cannot meet the encryption, content variety, or temporal realism properties. Tier T2 therefore does not use TRex ASTF. A Phase 3 spike evaluates F-Stack as a DPDK backend behind the interpreter's `SessionIo` trait. The S4 translator remains an optional export for cleartext, deterministic throughput tests. See `docs/phase0/gate0-report.md`.
