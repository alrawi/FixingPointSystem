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
| [Module Development](Module-Development.md) | The guide for third party module authors, from an empty directory to a signed module that a controller accepts |
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

Phases 0 and 1 are complete, and the owner signed off Gate 1 on 2026-11-05. Phase 2 has delivered TLS, content generators, the WebAssembly host, session capture, run time rate changes, profiles and distributions, the qcow2 image (built and boot tested on the owner host), the HTML report, the Python SDK, and the contract changes CON-05, CON-10, CON-11, and CON-12. Batch 34 merged a session as one scenario session (CON-31, approved by the owner with `after` flows included) and its counting in the engine (ENG-30), STARTTLS ports in the decrypted view (RLM-21), request cadence and bytes per rung for HLS in the labels (RLM-22, report format 0.20.0), and CTL-49, which removed the last waiver of Gate 2: the CI run passes on the bytes basis with all 15 applications, the largest mix difference 1.35 points. Gate 2 waits for the owner's sign-off. Batch 33 merged the raw action in the engine for strike and fuzzer modules (ENG-29), expected parser findings in the realism labels (RLM-20, report format 0.19.0), registry rules as strict as the validator (CTL-50), and packet gap labels in place of the sip_rtp timing script and the http2 TLS wrapper (MOD-24). A rustfmt change in the stable toolchain had stopped the `rust` job of CI at its format step on every pull request; the coordinator fixed the format in PR #533, and the job runs clippy and the workspace tests again. CTL-49 is blocked: a spawned flow is a session of its own in the engine, and changing that is a contract change for the owner (ENG-30, CON-31). Batch 32 merged measured byte estimates in every first party manifest with a guard test per module (MOD-25), manifest contract v0.5 with a validator that checks every rule the registry enforces (CON-30), TLS on named ports in the decrypted view (RLM-18), the port collision rule in the realism runner (RLM-19, report format 0.18.0), and calibration in the Gate 2 report (QA-12). The Gate 2 byte basis now waits only for CTL-49, because the run counts a spawned flow as a session of its own. The strike module of issue #491 is in the catalog as MOD-27 to MOD-29, which the owner implements. Batch 31 merged the [Module Development](Module-Development.md) guide for third party authors and a template that validates (MOD-26), byte estimates read by the controller (CTL-47), packet gap KS series in the metric catalog (CON-28, catalog 0.8.0, report format 0.17.0), report delivery from the realism runner (RLM-16), the CON-23 wording of every realism metric (CTL-48), and named Gate 2 parser findings with a port collision rule (QA-11). A pull request that changes how a module is written now updates the guide in the same pull request. Batch 30 merged the Gate 2 suite QA-05 (verdict PASS WITH WAIVERS: the byte basis waits for CTL-47, and AGT-21 ended the mix waiver of hls, ldap, mqtt and ntp), NTP, MQTT, LDAP and HLS in the agent AGT-21, packet gaps inside a session RLM-15 (report format 0.16.0), the host identity check in the metric catalog CON-27 (catalog 0.7.0, report format 0.15.0), the KS property wording CTL-46, and the flake fixes MOD-23 and ENG-28. Batch 29 merged the host identity check RLM-10 (report format 0.13.0), the CON-23 mapping table in the controller CTL-42, Gate 1 without its own reuse rule QA-10, the enrollment test fix AGT-15, the CON-25 contract addition (metric catalog 0.6.0, the messages check carries its failures series, report format 0.14.0), and CON-26 (REST 0.12.0: a read-scoped capture download answers 409 for a running capture). The owner's security review found four issues, all fixed: the renewal and revocation race in the controller PKI CTL-43, a JSON bypass of the capture scrubber CAP-06 (scrubber 1.1.0), a read-scoped download that stopped a capture CTL-44, and the order of long numeric prerelease versions CTL-45. Batch 28 merged SIP with RTP in the agent AGT-20, the CON-23 metrics in every realism check RLM-17 (report format 0.12.0), reservation holders in the agents view UI-09, the HLS video MOD-15, NTP MOD-16, LDAP MOD-17 and MQTT MOD-18 modules, and the CON-24 contract additions (agent protocol 0.2.4 and REST 0.11.0: capture drops split into overflow, oversize and cut at the stop). Batch 27 merged the controller sending the first response frame of an agent session at once CTL-40, realism reports attached to a run CTL-41, datagram flows in multi flow instances and shared data ports in the runtime ENG-26, split drop counts in the capture ENG-27, SMB2 in the agent AGT-19, the scenario table and criterion filters in the report views UI-08, message counts in the decrypted view RLM-14, the cause of the Gate 1 parser error flake QA-09, and the CON-23 contract additions (metric catalog 0.5.0, a catalog metric for every realism check). Batch 26 merged the scenario totals and criterion filters of the REST report CTL-39, classifier mix series RLM-13, the SMB2 module MOD-13 (18 of 18 sessions classified as SMB), the SIP with RTP module MOD-14 (not yet runnable through the agent), per profile captures scored with the transport check TRN-16, the causes of the agent test flakes AGT-18, a session gate in the image boot test IMG-07, and the CON-22 contract additions (REST 0.10.0: attachRealismReport, a documented 409 for reserveAgent, Agent.reserved_by). Batch 25 merged ssh in the agent with TLS profiles from the TLS module and a digested manifest AGT-17, distributions in the compiler and the scenarios table of the run report CTL-38, the SMB2 functions MOD-12, the transport behavior check RLM-09, provenance fields from the scrub record CAP-05, the shared decrypted check in the module capture scripts MOD-22, the agents and ports view UI-06, the CON-21 contract additions (REST 0.9.0), and the owner's CI fix that bounds apt and gives every hosted job a timeout. Batch 24 merged multi flow scenarios and digested extension bytes in the agent AGT-16 (FTP data connections now run through the agent), nDPI 4.14 and a shared decrypted view in the realism runner RLM-12, TLS backpressure and the mail and h2 profiles TRN-15, int_bytes and now in the engine ENG-25, the SSH module MOD-11, the profile extractor CAP-04, the report and run comparison views UI-05, the CON-20 contract additions (REST 0.8.0, metric catalog 0.4.0, module manifest v0.4), and the HTTP/2 module MOD-06. Batch 23 merged port filtered captures through the REST API on the real agent AGT-12, per session TLS connectors and tls metrics ENG-24, STARTTLS for smtp and imap MOD-20, the fix of the http1 parser error flake MOD-21, the mix accuracy check RLM-07, scrubbing and provenance for captured scenarios CAP-03, the run view with live metrics UI-04, and the CON-19 contract additions (scenario contract v0.6, test configuration v0.3). Batch 22 delivered TLS in the agent AGT-14, multi flow instances ENG-23, HTTP/2 framing MOD-05, the temporal check RLM-06, the configuration editor UI-03, scenario generation CAP-02, and CON-18. The coordinator updates this line after each batch.
