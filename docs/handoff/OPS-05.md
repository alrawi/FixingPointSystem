# Handoff OPS-05 Release workflow

| Field | Value |
|---|---|
| Issue | #155 |
| Branch | `wp/OPS-05` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-7a6ec6, lease until 2026-11-09T15:12:20Z |
| Last verified commit | none |

## Goal

A tag triggered workflow builds the agent, controller, module archives, and Python package, writes SHA256SUMS and release notes, and uploads them to a GitHub release. A dry run produces every artifact without a release.

## Plan

- [x] 1. Write `.github/workflows/release.yml` and lint it with actionlint
- [x] 2. Document it in `docs/wiki/Operations.md` with the manual image attachment
- [x] 3. Dry run on the branch (push run 42628407496, dispatch run 42629459202)
- [ ] 4. Open the PR and wait for CI

## Done and verified

- actionlint passes. Dry run 42628407496 built agent, controller, 3 module archives, sdist and wheel, SHA256SUMS (all OK), notes; publish job skipped.

## Remaining

PR and CI.

## Next action

Open the PR, wait for CI, then run fps_swarm.py review.

## Blockers and open questions

none

## Decisions taken in this package

- The `publish` job is separate from `bundle` so only it holds `contents: write` and it runs no repository code.
- No version stamping into the controller, because `version()` reads Go build info and has no variable to set.
- Only `actions/*` and `dtolnay/rust-toolchain` are used, with the tags the other workflows use.

## Pitfalls

- The sandbox guard refuses commands that mix git and complex shell. Use plain commands.

## Verify the current state

```bash
actionlint .github/workflows/release.yml
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-7a6ec6 | claim and first push | none | workflow and docs written |

A workflow_dispatch workflow is unknown to the REST dispatch until a run exists. The dispatch 404ed until a temporary `push: branches` trigger registered it; that trigger is removed again.
