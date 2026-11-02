# Log

This page is the append only record of wiki operations. [Wiki Maintenance](Wiki-Maintenance.md) defines the entry format. New entries go at the bottom.

## [2026-11-02] plan | Initial work package catalog

We created the catalog of 111 work packages for Phases 1 to 3 in `plan/work-packages.yaml` and generated the three phase pages from it.

## [2026-11-02] ingest | Swarm coordination, coding standards, and testing

We added the Swarm Protocol, the handoff template, Coding Standards, Testing, Project Board Setup, and Wiki Maintenance pages. Sources were the specification, decision records D1 to D9, `CONTRIBUTING.md`, and the Gate 0 report.

## [2026-11-02] plan | Phase 0 in the catalog and issues seeded

We added the six Phase 0 packages to the catalog and created their issues. Five are closed as done (#4, #5, #6, #8, #9). SPK-03 (#7) stays open until spike S3 runs on a KVM host with the harness from #2. We created the label set and seeded the 48 Phase 1 issues (#10 to #57), of which eight are ready.

## [2026-11-02] ingest | CON-01 Agent control protocol v0.1

We drafted the agent control protocol in `contracts/proto/fps/agent/v1/agent.proto`. The agent serves `AgentService` with Register, Heartbeat, Prepare, Start, Stop, and StreamCounters, and the controller is its only client. The `Plan` message carries scenario sources, module digests, host pool slices, per worker rates, load phases, and the seed. Its canonical JSON form is the proto3 JSON mapping, which the standalone agent mode reads. The [contracts README](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md) records the state machine, the error codes, the module tree digest, and the host address rule. `tools/contracts/check_plans.py` validates the example plan, and the CI job `proto` runs `buf lint`, `buf breaking`, and the plan checker. Counter fields name metrics from the CON-02 catalog. We updated the Contracts row of [Home](Home.md) and the seam table of the [Implementation Plan](Implementation-Plan.md). The package waits for owner approval, and the handoff is `docs/handoff/CON-01.md`.
