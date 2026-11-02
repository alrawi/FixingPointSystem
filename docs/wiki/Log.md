# Log

This page is the append only record of wiki operations. [Wiki Maintenance](Wiki-Maintenance.md) defines the entry format. New entries go at the bottom.

## [2026-11-02] plan | Initial work package catalog

We created the catalog of 111 work packages for Phases 1 to 3 in `plan/work-packages.yaml` and generated the three phase pages from it.

## [2026-11-02] ingest | Swarm coordination, coding standards, and testing

We added the Swarm Protocol, the handoff template, Coding Standards, Testing, Project Board Setup, and Wiki Maintenance pages. Sources were the specification, decision records D1 to D9, `CONTRIBUTING.md`, and the Gate 0 report.

## [2026-11-02] plan | Phase 0 in the catalog and issues seeded

We added the six Phase 0 packages to the catalog and created their issues. Five are closed as done (#4, #5, #6, #8, #9). SPK-03 (#7) stays open until spike S3 runs on a KVM host with the harness from #2. We created the label set and seeded the 48 Phase 1 issues (#10 to #57), of which eight are ready.

## [2026-11-02] ingest | CON-03 Extension interface v0.1

We added the WIT package `fps:extension@0.1.0` with the five hooks, a typed template function catalog, and six engine calls, together with its semantics and the mapping to the Rust traits in `contracts/extension/README.md`. Decision record D10 proposes native first party extensions behind the same traits and waits for the owner. We updated Home, the seam table in Implementation Plan, and the module layout in Coding Standards. The evidence is `docs/handoff/CON-03.md` on the branch `wp/CON-03`.
## [2026-11-02] ingest | OPS-01 Repository scaffolding for Phase 1

We added the root `Makefile` with `lint`, `test`, and per workstream targets, switched the root `Cargo.toml` to a glob member list, and added `go.work`, `CODEOWNERS`, and the first workstream workflow `.github/workflows/ops.yml`. We rewrote the Commands section of [Testing](Testing.md), which had described the `Makefile` as future work, and added its CI workflows section. Cargo rejects a member glob that matches nothing, so the list uses one character class glob for `spikes/` and `engine/` and defers `modules/*/native` to the first native module crate. The evidence is `make test-ops-workspace` and the handoff file `docs/handoff/OPS-01.md`.
## [2026-11-02] ingest | CON-02 Counter catalog v0.1

We added the metric catalog `contracts/metrics/catalog.yaml` with 48 metrics, its JSON Schema, and the validator rule that every metric in test criteria and in OpenAPI `x-fps-metric` annotations exists in the catalog. We added a Counter catalog section to [Implementation Plan](Implementation-Plan.md). Evidence is in `docs/handoff/CON-02.md` and `tools/contracts/test_contracts.py`.
## [2026-11-02] ingest | CON-01 Agent control protocol v0.1

We drafted the agent control protocol in `contracts/proto/fps/agent/v1/agent.proto`. The agent serves `AgentService` with Register, Heartbeat, Prepare, Start, Stop, and StreamCounters, and the controller is its only client. The `Plan` message carries scenario sources, module digests, host pool slices, per worker rates, load phases, and the seed. Its canonical JSON form is the proto3 JSON mapping, which the standalone agent mode reads. The [contracts README](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md) records the state machine, the error codes, the module tree digest, and the host address rule. `tools/contracts/check_plans.py` validates the example plan, and the CI job `proto` runs `buf lint`, `buf breaking`, and the plan checker. Counter fields name metrics from the CON-02 catalog. We updated the Contracts row of [Home](Home.md) and the seam table of the [Implementation Plan](Implementation-Plan.md). The package waits for owner approval, and the handoff is `docs/handoff/CON-01.md`.
## [2026-11-02] ingest | OPS-02 Swarm helper command

We added `tools/swarm/fps_swarm.py`, which runs every step of the [Swarm Protocol](Swarm-Protocol.md) over the REST API, and updated that page to show each helper command next to its raw calls. The page now also defines the `fps:block` marker that the promotion sweep reads. The evidence is `docs/handoff/OPS-02.md` and the offline tests in `tools/swarm/test_fps_swarm.py`, which include two agents that claim one issue at the same time and resolve to one holder.

## [2026-11-02] ingest | CTL-01 Controller skeleton

We added the Go module `github.com/alrawi/FixingPointSystem/controller` with the `fps-controller` binary, YAML configuration with strict keys, `log/slog` JSON logging, `GET /healthz`, and graceful shutdown on SIGINT and SIGTERM. We added the page [Controller](Controller.md) and linked it from [Home](Home.md). We corrected two stale claims in [Testing](Testing.md), which said that `go.work` lists no modules and named the CTL workflow `ctl.yml`, and added the `lint-ctl` and `test-ctl` targets to its table. The workflow `.github/workflows/controller.yml` pins golangci-lint v2.12.2 through its action. The evidence is `make lint-ctl test-ctl` and the handoff file `docs/handoff/CTL-01.md`.
## [2026-11-02] ingest | ENG-01 Engine crate skeleton

We created the engine crates `fps-io`, `fps-flow`, `fps-interp`, and `fps-stats` under `engine/` and added the [Engine](Engine.md) page, which records the four rules of the `SessionIo` seam, the in memory duplex, and the four changes that the port of the S1 program model and template parser made. `fps-interp` and `fps-stats` stay empty until ENG-04 and ENG-09. Spike S1 keeps its own copy of the ported files, and the Engine page records that duplication. We added the `test-eng` target to [Testing](Testing.md) and linked the new page from [Home](Home.md). The root `Cargo.toml` glob picked up the new crates without an edit. The evidence is `make test-eng` and `docs/handoff/ENG-01.md` on the branch `wp/ENG-01`.
## [2026-11-02] ingest | OPS-03 Stale lease scanner and dependency promoter

We added the workflow `.github/workflows/swarm-scan.yml`, which runs `fps_swarm.py sweep` every 30 minutes with only the repository token, and a manual dispatch with a `dry_run` input that defaults to true. The stale sweep now posts one comment with the `fps:stale` marker and the expiry time when it marks an item stale. We added the section The scanner to [Swarm Protocol](Swarm-Protocol.md) and a note to [Project Board Setup](Project-Board-Setup.md) that relabels made with the repository token start no OPS-04 workflow. The evidence is `docs/handoff/OPS-03.md` and the `test_scan_*` tests in `tools/swarm/test_fps_swarm.py`.
## [2026-11-02] ingest | RLM-01 Realism runner and nDPI conformance

We added the realism runner in `realism/`, which scores a pcap or pcapng capture against a labels file and writes a JSON report that follows `realism/schema/realism-report.schema.json`. The conformance check runs `ndpiReader -d` out of process and compares the share of sessions classified as the intended application with the 99% threshold. We added a Realism report section and a schema link to the seam table of [Implementation Plan](Implementation-Plan.md), and the `lint-rlm` and `test-rlm` rows to [Testing](Testing.md). The evidence is `docs/handoff/RLM-01.md` and the tests in `realism/tests/`, which include a run of the real ndpiReader 4.2.0 on synthetic HTTP and DNS captures.
## [2026-11-02] decision | Agents register with the controller

The owner decided that agents dial the controller and register themselves, which reverses the direction in the CON-01 draft. We added CON-06 to the catalog to change the agent control protocol, and AGT-02 and CTL-06 now depend on it.
