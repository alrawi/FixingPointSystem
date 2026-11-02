# Log

This page is the append only record of wiki operations. [Wiki Maintenance](Wiki-Maintenance.md) defines the entry format. New entries go at the bottom.

## [2026-11-02] plan | Initial work package catalog

We created the catalog of 111 work packages for Phases 1 to 3 in `plan/work-packages.yaml` and generated the three phase pages from it.

## [2026-11-02] ingest | Swarm coordination, coding standards, and testing

We added the Swarm Protocol, the handoff template, Coding Standards, Testing, Project Board Setup, and Wiki Maintenance pages. Sources were the specification, decision records D1 to D9, `CONTRIBUTING.md`, and the Gate 0 report.

## [2026-11-02] plan | Phase 0 in the catalog and issues seeded

We added the six Phase 0 packages to the catalog and created their issues. Five are closed as done (#4, #5, #6, #8, #9). SPK-03 (#7) stays open until spike S3 runs on a KVM host with the harness from #2. We created the label set and seeded the 48 Phase 1 issues (#10 to #57), of which eight are ready.

## [2026-11-02] ingest | CON-02 Counter catalog v0.1

We added the metric catalog `contracts/metrics/catalog.yaml` with 48 metrics, its JSON Schema, and the validator rule that every metric in test criteria and in OpenAPI `x-fps-metric` annotations exists in the catalog. We added a Counter catalog section to [Implementation Plan](Implementation-Plan.md). Evidence is in `docs/handoff/CON-02.md` and `tools/contracts/test_contracts.py`.
