# Log

This page is the append only record of wiki operations. [Wiki Maintenance](Wiki-Maintenance.md) defines the entry format. New entries go at the bottom.

## [2026-11-02] plan | Initial work package catalog

We created the catalog of 111 work packages for Phases 1 to 3 in `plan/work-packages.yaml` and generated the three phase pages from it.

## [2026-11-02] ingest | Swarm coordination, coding standards, and testing

We added the Swarm Protocol, the handoff template, Coding Standards, Testing, Project Board Setup, and Wiki Maintenance pages. Sources were the specification, decision records D1 to D9, `CONTRIBUTING.md`, and the Gate 0 report.

## [2026-11-02] plan | Phase 0 in the catalog and issues seeded

We added the six Phase 0 packages to the catalog and created their issues. Five are closed as done (#4, #5, #6, #8, #9). SPK-03 (#7) stays open until spike S3 runs on a KVM host with the harness from #2. We created the label set and seeded the 48 Phase 1 issues (#10 to #57), of which eight are ready.

## [2026-11-02] ingest | OPS-01 Repository scaffolding for Phase 1

We added the root `Makefile` with `lint`, `test`, and per workstream targets, switched the root `Cargo.toml` to a glob member list, and added `go.work`, `CODEOWNERS`, and the first workstream workflow `.github/workflows/ops.yml`. We rewrote the Commands section of [Testing](Testing.md), which had described the `Makefile` as future work, and added its CI workflows section. Cargo rejects a member glob that matches nothing, so the list uses one character class glob for `spikes/` and `engine/` and defers `modules/*/native` to the first native module crate. The evidence is `make test-ops-workspace` and the handoff file `docs/handoff/OPS-01.md`.
