# Handoff CTL-11 Module signing and signed index

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #163 |
| Branch | `wp/CTL-11` |
| Pull request | #256 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-ctl11-a7c3e1, lease until 2026-11-09T04:07:11Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ed25519 signing and verification for module archives (the signature covers the digest string, as ModuleRef.signature in CON-05 says) and for a canonical registry index, key ids derived from public keys, and a trust store with active, retired (until a cutoff), and revoked keys. The package is `controller/internal/registry/signing/`, standard library only. Install and the agent do not use it yet.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff
- [x] 2. Keys, trust store, digest signing and archive digest, with tests
- [x] 3. Canonical index and its signature, with tests; make targets
- [x] 4. Controller page, log entry, checks, PR, CI

## Done and verified

- Steps 1 to 3. The package is `controller/internal/registry/signing/`. Go modules cannot be downloaded here, so a scratch module with the package and a copy of `registry/digest.go` ran `go test -race -cover` (all pass, 95.2% coverage), `go vet`, and `golangci-lint run` with the controller config (0 issues). The CI run of `make test-ctl` is the real check, because the cross-check test imports the full `registry` package.
- The golden signature and key id were also produced with Python `cryptography` and match.

## Remaining

CI passed on commit 3f379f9 (workflow ctl and ci). Install, compiler, and agent wiring are outside this package and are listed in the Controller page and the log entry.

## Next action

Review.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- The archive signature covers the digest string (CON-05). The index signature covers a canonical JSON form with sorted modules, fixed key order, and no escapes, which the manifest name and semver patterns make possible.
- The index carries a `serial` for replay protection; the package does not store serials.
- A retired key verifies strictly before its cutoff, using the verifier's injected clock, because signatures carry no time.
- `ArchiveDigest` repeats the digest rule so the agent side can follow it; a test ties it to `registry.TreeDigest`.

## Pitfalls

The sandbox worktree guard refuses heredocs and `source` in one command with git; use the Write tool and small scripts.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08T22:40Z | claude-ctl11-a7c3e1 | claim | none | Claimed issue #163, branch from origin/main 82a5953 |
| 2026-11-08T23:50Z | claude-ctl11-a7c3e1 | checkpoint | see branch | Package, tests, Controller page, log entry written; checks pass locally |
| 2026-11-09T02:40Z | claude-ctl11-a7c3e1 | ready for review | 3f379f9 | PR #256, CI green |
