# FixingPointSystem Wiki

FixingPointSystem is an open source generator of realistic application traffic for testing firewalls, intrusion prevention systems, proxies, NAT devices, and load balancers. A swarm of AI coding agents builds it under review by the repository owner. This wiki is the shared record that lets any agent start, resume, or finish a piece of work without asking anyone.

## Start here

A new agent reads four pages in order before claiming work.

1. [Swarm Protocol](Swarm-Protocol.md), which explains how to find, claim, checkpoint, hand off, and finish a work package.
2. [Implementation Plan](Implementation-Plan.md), which describes the workstreams, seams, phases, and the Definition of Done.
3. [Coding Standards](Coding-Standards.md) and [Testing](Testing.md) for the workstream of the chosen package.
4. The newest entries of [Log](Log.md).

## Index

### Coordination

| Page | Summary |
|---|---|
| [Swarm Protocol](Swarm-Protocol.md) | Labels, status cards, leases, checkpoints, takeover, review, and ticket creation |
| [Project Board Setup](Project-Board-Setup.md) | One time board setup by the owner and issue seeding |
| [Wiki Maintenance](Wiki-Maintenance.md) | How agents ingest results, answer queries, and lint this wiki |
| [Log](Log.md) | Append only record of wiki operations |

### Plan

| Page | Summary |
|---|---|
| [Implementation Plan](Implementation-Plan.md) | Atomic package rules, workstreams, layout, seams, gates, Definition of Done |
| [Work Packages Phase 1](Work-Packages-Phase-1.md) | Core alpha packages up to Gate 1 |
| [Work Packages Phase 2](Work-Packages-Phase-2.md) | Library packages up to Gate 2 |
| [Work Packages Phase 3](Work-Packages-Phase-3.md) | Scale packages up to the v1 release gate |

### Engineering

| Page | Summary |
|---|---|
| [Coding Standards](Coding-Standards.md) | Rules and examples for Rust, Go, Python, TypeScript, shell, contracts, commits, and documentation |
| [Testing](Testing.md) | Test levels, modular tests, module tests, regression tests, and gate suites |

### Raw sources in the repository

| Source | Summary |
|---|---|
| [Specification](https://github.com/alrawi/FixingPointSystem/blob/main/docs/spec/specification.md) | Feasibility assessment and v1 specification |
| [Decision records](https://github.com/alrawi/FixingPointSystem/tree/main/docs/decisions) | Decisions D1 to D9, and D10 on native first party extensions as a proposal |
| [Contracts](https://github.com/alrawi/FixingPointSystem/tree/main/contracts) | OpenAPI document, schemas, extension WIT world, and examples |
| [Gate 0 report](https://github.com/alrawi/FixingPointSystem/blob/main/docs/phase0/gate0-report.md) | Spike results and items carried into Phase 1 |
| [Handoff template](https://github.com/alrawi/FixingPointSystem/blob/main/docs/handoff/TEMPLATE.md) | The progress document that every package keeps |
| [AGENTS.md](https://github.com/alrawi/FixingPointSystem/blob/main/AGENTS.md) | Schema and standing instructions for agents |

## Status

Phase 0 is complete except for spike S3 (#7), which waits for a KVM host. Phase 1 is seeded as issues #10 to #57, and eight packages are ready to claim.
