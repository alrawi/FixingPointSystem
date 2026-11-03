# FixingPointSystem Wiki

FixingPointSystem is an open source generator of realistic application traffic for testing firewalls, intrusion prevention systems, proxies, NAT devices, and load balancers. A swarm of AI coding agents builds it under review by the repository owner. This wiki is the shared record that lets any agent start, resume, or finish a piece of work without asking anyone.

## Start here

A new agent reads four pages in order before claiming work.

1. [Swarm Protocol](Swarm-Protocol.md), which explains how to find, claim, checkpoint, hand off, and finish a work package.
2. [Implementation Plan](Implementation-Plan.md), which describes the workstreams, seams, phases, and the Definition of Done.
3. [Coding Standards](Coding-Standards.md) and [Testing](Testing.md) for the workstream of the chosen package.
4. The newest entries of [Log](Log.md), which live as files in `docs/wiki/log/`.

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
| [Testing](Testing.md) | Test levels, modular tests, module tests, regression tests, the namespace harness, and gate suites |
| [Controller](Controller.md) | The controller module, its flags, configuration file, logging, health endpoint, REST layer, test compiler, configuration store, module registry, agent manager, and checks |
| [Engine](Engine.md) | Engine crates, the `SessionIo` seam, and the scenario and template compiler |
| [Transport](Transport.md) | Transport backends, starting with the kernel TCP backend, its socket options, and its loopback tests |
| [Hosts](Hosts.md) | Host pool address mapping, slicing by agent and worker, and seeded host selection |
| [Contracts](Contracts.md) | Contract files and versions, the agent initiated control protocol, and the contract checks |
| [Modules](Modules.md) | Application modules, starting with http1, their native functions, fixtures, and tests |
| [Benchmarks](Benchmarks.md) | Benchmark result and baseline format, the comparison tool, and baseline refresh |
| [Web UI](Web-UI.md) | The accepted web UI stack (D11) and the UI packages |
| [Realism](Realism.md) | The realism runner, its conformance and parser error checks, and the report seam |
| [Operations](Operations.md) | Repository operations, including the Project board status sync and its owner setup |

### Raw sources in the repository

| Source | Summary |
|---|---|
| [Specification](https://github.com/alrawi/FixingPointSystem/blob/main/docs/spec/specification.md) | Feasibility assessment and v1 specification |
| [Contracts](https://github.com/alrawi/FixingPointSystem/tree/main/contracts) | OpenAPI document, schemas with scenario contract v0.2, agent control protocol, extension WIT world, metric catalog, and examples |
| [Gate 0 report](https://github.com/alrawi/FixingPointSystem/blob/main/docs/phase0/gate0-report.md) | Spike results and items carried into Phase 1 |
| [Namespace harness](https://github.com/alrawi/FixingPointSystem/blob/main/tests/harness/README.md) | Two arm network namespace harness for end to end tests and captures |
| [Handoff template](https://github.com/alrawi/FixingPointSystem/blob/main/docs/handoff/TEMPLATE.md) | The progress document that every package keeps |
| [AGENTS.md](https://github.com/alrawi/FixingPointSystem/blob/main/AGENTS.md) | Schema and standing instructions for agents |

## Status

Phase 0 is complete, including spike S3. Phase 1 has 39 packages merged after five batches, and the rest are seeded as issues. The coordinator updates this line after each batch.
