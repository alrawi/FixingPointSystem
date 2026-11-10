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
| [Gates](Gates.md) | The gate acceptance suites, starting with the Gate 1 suite of QA-04, and how to run and read them |
| [Image](Image.md) | The qcow2 base image of IMG-01: how to build it with Packer, boot test it under KVM, and its management interface isolation |
| [SDK](SDK.md) | The Python SDK of CLI-01: the generated client, its currency check, and the start, wait, and report helpers |
| [CLI](CLI.md) | The `fps` command line client of CLI-02: configurations, runs, live metrics, reports, authentication, and exit codes |
| [Controller](Controller.md) | The controller module, its flags, configuration file, logging, health endpoint, REST layer, test compiler, configuration store, module registry, agent manager, and checks |
| [Engine](Engine.md) | Engine crates, the `SessionIo` seam, and the scenario and template compiler |
| [Transport](Transport.md) | Transport backends, starting with the kernel TCP backend, its socket options, and its loopback tests, and the TLS module of TRN-04 |
| [Hosts](Hosts.md) | Host pool address mapping, slicing by agent and worker, and seeded host selection |
| [Contracts](Contracts.md) | Contract files and versions, the agent initiated control protocol, and the contract checks |
| [Agent](Agent.md) | The agent binary, its standalone `run` mode, plan loading, and module digests |
| [Modules](Modules.md) | Application modules, starting with http1, their native functions, fixtures, and tests |
| [Benchmarks](Benchmarks.md) | Benchmark result and baseline format, the comparison tool, and baseline refresh |
| [Web UI](Web-UI.md) | The accepted web UI stack (D11) and the UI packages |
| [Realism](Realism.md) | The realism runner, its conformance and parser error checks, and the report seam |
| [Capture](Capture.md) | Capture analysis of CAP-01: the pcap and pcapng reader, TCP reassembly, UDP flows, and the session JSON schema |
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

Phases 0 and 1 are complete, and the owner signed off Gate 1 on 2026-11-05. Phase 2 has delivered TLS, content generators, the WebAssembly host, session capture, run time rate changes, profiles and distributions, the qcow2 image (built and boot tested on the owner host), the HTML report, the Python SDK, and the contract changes CON-05, CON-10, CON-11, and CON-12. Batch 27 merged the controller sending the first response frame of an agent session at once CTL-40, realism reports attached to a run CTL-41, datagram flows in multi flow instances and shared data ports in the runtime ENG-26, split drop counts in the capture ENG-27, SMB2 in the agent AGT-19, the scenario table and criterion filters in the report views UI-08, message counts in the decrypted view RLM-14, the cause of the Gate 1 parser error flake QA-09, and the CON-23 contract additions (metric catalog 0.5.0, a catalog metric for every realism check). Batch 26 merged the scenario totals and criterion filters of the REST report CTL-39, classifier mix series RLM-13, the SMB2 module MOD-13 (18 of 18 sessions classified as SMB), the SIP with RTP module MOD-14 (not yet runnable through the agent), per profile captures scored with the transport check TRN-16, the causes of the agent test flakes AGT-18, a session gate in the image boot test IMG-07, and the CON-22 contract additions (REST 0.10.0: attachRealismReport, a documented 409 for reserveAgent, Agent.reserved_by). Batch 25 merged ssh in the agent with TLS profiles from the TLS module and a digested manifest AGT-17, distributions in the compiler and the scenarios table of the run report CTL-38, the SMB2 functions MOD-12, the transport behavior check RLM-09, provenance fields from the scrub record CAP-05, the shared decrypted check in the module capture scripts MOD-22, the agents and ports view UI-06, the CON-21 contract additions (REST 0.9.0), and the owner's CI fix that bounds apt and gives every hosted job a timeout. Batch 24 merged multi flow scenarios and digested extension bytes in the agent AGT-16 (FTP data connections now run through the agent), nDPI 4.14 and a shared decrypted view in the realism runner RLM-12, TLS backpressure and the mail and h2 profiles TRN-15, int_bytes and now in the engine ENG-25, the SSH module MOD-11, the profile extractor CAP-04, the report and run comparison views UI-05, the CON-20 contract additions (REST 0.8.0, metric catalog 0.4.0, module manifest v0.4), and the HTTP/2 module MOD-06. Batch 23 merged port filtered captures through the REST API on the real agent AGT-12, per session TLS connectors and tls metrics ENG-24, STARTTLS for smtp and imap MOD-20, the fix of the http1 parser error flake MOD-21, the mix accuracy check RLM-07, scrubbing and provenance for captured scenarios CAP-03, the run view with live metrics UI-04, and the CON-19 contract additions (scenario contract v0.6, test configuration v0.3). Batch 22 delivered TLS in the agent AGT-14, multi flow instances ENG-23, HTTP/2 framing MOD-05, the temporal check RLM-06, the configuration editor UI-03, scenario generation CAP-02, and CON-18. The coordinator updates this line after each batch.
