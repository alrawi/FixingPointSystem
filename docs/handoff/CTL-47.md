# Handoff CTL-47 Byte estimates in the scenario catalog

| Field | Value |
|---|---|
| Issue | #499 |
| Branch | `wp/CTL-47` |
| Pull request | #504 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-4c7e91, lease until 2026-11-12T15:16:07Z |
| Last verified commit | 75e5075 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The scenario catalog of the controller reads the manifest field `expected_bytes` and sets `compiler.Scenario.BytesPerSession` from it, so the `bytes` basis and the `throughput_bps` objective compile on a real controller (QA-05 finding). A manifest without an estimate still fails with `ErrNoByteEstimate`.

## Plan

- [x] 1. `registry.Manifest` gets `ExpectedBytes`, and `Catalog.Scenarios` sets `BytesPerSession` from it.
- [x] 2. Controller tests compile a bytes basis mix and a `throughput_bps` objective from manifests with estimates, and a manifest without one fails with `ErrNoByteEstimate`.
- [x] 3. Controller page, log entry with the shared page notes for Module Development, checks, pull request, CI .

## Done and verified

- `registry.Manifest.ExpectedBytes` and `Catalog.Scenarios` setting `BytesPerSession` (`gofmt -l controller` clean).
- Tests in `controller/internal/wiring/bytes_test.go`; they could not run locally because Go modules cannot be downloaded, so CI is the first run.
- Controller page updated, log entry written with the shared page notes (exact text for Module Development).
- `plan.py check` and `render --check` pass.
- CI on 75e5075: all 19 check runs succeeded (ctl, ci, qa-gate1, qa-gate2, image, containers, and the others). The first run failed on gosec G703 in the test helper, fixed in 75e5075.

## Remaining

Nothing. The coordinator applies the Module Development text from the log entry and merges.

## Next action

Review pull request #504.
