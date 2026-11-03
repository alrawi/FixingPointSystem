# Handoff CTL-04 Module registry

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #42 |
| Branch | `wp/CTL-04` |
| Pull request | #93 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-ctl-04-70e2, lease until 2026-11-03T13:25:25Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

List, install, inspect, and delete module packages with manifest validation and content digests. The registry keeps a local directory with an index file and serves the /v1/modules operations.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Read the manifest schema, examples, digest rule, and generated API
- [x] 2. Registry store with an index file, the tree digest, and manifest validation
- [x] 3. License allow list and the lookup interface for CTL-03
- [x] 4. The /v1/modules handlers that wrap api.Unimplemented, with tests
- [x] 5. Docs, log entry, lint, test, CI

## Done and verified

- Steps 1 to 4. `cd controller && go test -race ./internal/registry/` passes. `TestInstallExampleModules` and `TestAPIInstallExamplesAndManage` install the example http1 and dns modules from a directory and from tar and gzip archives, and both match the digests of `contracts/examples/plans/http_dns_smoke.json`. `TestAPIInstallRejectsLicenses` and `TestInstallRejects` show that unknown and incompatible licenses fail with a 422 problem whose only finding names `/license`. Every API response validates against the OpenAPI contract with kin-openapi.
- Fuzz targets `FuzzParseManifest` and `FuzzCheckLicense` ran for 15 and 10 seconds without a failure.
- Step 5. `make lint-ctl test-ctl` passes with golangci-lint 2.5.0 locally (0 issues), and `plan.py check`, `render --check`, and `lint` pass.

## Remaining

- The binary does not mount the registry, because the configuration has no data directory or token setting. The log entry names the wiring for the CTL-01 paths.
- Archives are unsigned. CTL-11 adds signing under `controller/internal/registry/signing/` and fills `signed_by`.
- Scenario files are checked only for a unique `scenario` name. Full scenario validation belongs to CTL-03 and the contract validator.

## Next action

A reviewer checks pull request #93 against the Definition of Done.

## Blockers and open questions

none

## Decisions taken in this package

- The archive format is tar, optionally gzip compressed (detected by magic bytes), with `manifest.yaml` at the root, because the specification says "a signed archive with its manifest at the root" and the standard library reads tar and gzip.
- The manifest schema is embedded as a copy, with a test that compares it to the contract byte for byte, because a deployed controller has no `contracts/` directory.
- `jsonschema/v6` v6.0.2 (Apache-2.0) validates 2020-12, and `regexp2` v1.11.5 (MIT) runs the `relpath` pattern, whose lookahead RE2 cannot express.
- Every license in an expression must be known, even in an unneeded OR operand, so typos never pass. The deprecated `+` and bare `GPL-3.0` forms are unknown. AGPL-3.0 is on the allow list because GPL-3.0 section 13 permits the combination; the owner may want to drop it.
- A module version is immutable, so a second install of the same name and version answers 409.
- `Lookup` has `Manifest(name, version)` and `Latest(name)`, because test configurations name modules without a version.
- The page token is the base64url of `name/version` of the last item, so pages stay stable when modules are installed or deleted.

## Pitfalls

- The shared disk filled up during fuzzing. Clear `$GOCACHE/fuzz` after fuzz runs.

## Verify the current state

```bash
export GOWORK=off
(cd controller && go test -race ./internal/registry/)   # ok
make lint-ctl test-ctl                                  # 0 issues, all packages ok
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:38Z | claude-ctl-04-70e2 | claim | none | session start |
| 2026-11-03T11:57Z | claude-ctl-04-70e2 | checkpoint | a | registry, tests, docs |
