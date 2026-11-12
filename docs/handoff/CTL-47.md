# Handoff CTL-47 Byte estimates in the scenario catalog

| Field | Value |
|---|---|
| Issue | #499 |
| Branch | `wp/CTL-47` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (3 of 3 plan steps, CI pending) |
| Current holder | claude-4c7e91, lease until 2026-11-12T15:16:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The scenario catalog of the controller reads the manifest field `expected_bytes` and sets `compiler.Scenario.BytesPerSession` from it, so the `bytes` basis and the `throughput_bps` objective compile on a real controller (QA-05 finding). A manifest without an estimate still fails with `ErrNoByteEstimate`.

## Plan

- [x] 1. `registry.Manifest` gets `ExpectedBytes`, and `Catalog.Scenarios` sets `BytesPerSession` from it.
- [x] 2. Controller tests compile a bytes basis mix and a `throughput_bps` objective from manifests with estimates, and a manifest without one fails with `ErrNoByteEstimate`.
- [x] 3. Controller page, log entry with the shared page notes for Module Development, checks, pull request, CI (pull request and CI still open).

## Done and verified

- `registry.Manifest.ExpectedBytes` and `Catalog.Scenarios` setting `BytesPerSession` (`gofmt -l controller` clean).
- Tests in `controller/internal/wiring/bytes_test.go`; they could not run locally because Go modules cannot be downloaded, so CI is the first run.
- Controller page updated, log entry written with the shared page notes (exact text for Module Development).
- `plan.py check` and `render --check` pass.

## Remaining

Open the pull request, wait for CI, fix failures inside the paths, then `fps_swarm.py review`.

## Next action

Open the pull request against main and poll its check runs.
