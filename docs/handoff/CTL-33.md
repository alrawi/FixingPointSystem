# Handoff CTL-33 Version stamp in fps-controller

| Field | Value |
|---|---|
| Issue | #298 |
| Branch | `wp/CTL-33` |
| Pull request | none yet |
| Status | in progress |
| Progress | 60% (2 of 3 plan steps) |
| Current holder | claude-c33a1f |
| Last verified commit | none |

## Goal

`fps-controller -version` and the startup log report a version stamped with `-ldflags -X main.version=...`, and the release workflow passes the tag version.

## Plan

- [x] 1. `var version` and `buildVersion()` in `cmd/fps-controller/main.go`, tests, release.yml stamp and check.
- [x] 2. Wiki: Controller.md, Operations.md, log entry.
- [ ] 3. CI green and a release dry run on `wp/CTL-33`.

## Done and verified

Step 1 and 2 written. `gofmt -l` clean and `actionlint` clean on release.yml. Go cannot build in this sandbox, so CI is the build loop.

## Remaining

CI result and dry run id.

## Next action

Open the PR and dispatch `release.yml` on `wp/CTL-33` with `dry_run` true.

## Blockers and open questions

none

## Decisions taken in this package

- `GET /v1/capabilities` is not changed. The contract `Capabilities` has no controller version field, and `api_version` is the REST contract version, so reusing it would be wrong. A contract change (CON) would expose the controller version there.
- The release job keeps VERSION in env and the meta job validates it, so no untrusted value reaches a `run:` script through `${{ }}`.

## Pitfalls

none

## Verify the current state

```bash
cd controller && GOWORK=off go test ./cmd/fps-controller -run 'TestVersionStamp|TestBuildVersionFallback'
```

## Session log

- Claimed and implemented.
