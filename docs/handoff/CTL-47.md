# Handoff CTL-47 Byte estimates in the scenario catalog

| Field | Value |
|---|---|
| Issue | #499 |
| Branch | `wp/CTL-47` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-4c7e91, lease until 2026-11-12T15:16:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The scenario catalog of the controller reads the manifest field `expected_bytes` and sets `compiler.Scenario.BytesPerSession` from it, so the `bytes` basis and the `throughput_bps` objective compile on a real controller (QA-05 finding). A manifest without an estimate still fails with `ErrNoByteEstimate`.

## Plan

- [ ] 1. `registry.Manifest` gets `ExpectedBytes`, and `Catalog.Scenarios` sets `BytesPerSession` from it.
- [ ] 2. Controller tests compile a bytes basis mix and a `throughput_bps` objective from manifests with estimates, and a manifest without one fails with `ErrNoByteEstimate`.
- [ ] 3. Controller page, log entry with the shared page notes for Module Development, checks, pull request, CI.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Add the `ExpectedBytes` field to `registry.Manifest` in `controller/internal/registry/manifest.go`.
