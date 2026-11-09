# Handoff AGT-05 Capture and rate update handlers

| Field | Value |
|---|---|
| Issue | #162 |
| Branch | `wp/AGT-05` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-a05c1d, lease until 2026-11-09T04:06:54Z |
| Last verified commit | none |

## Goal

Implement the CON-05 v0.2 commands UpdateRates, StartCapture, and StopCapture in the agent session (`engine/fps-agent/`). Acceptance: a fake controller test changes rates and downloads (or locates and reads) a capture.

## Plan

- [ ] 1. Rate handles: entry names in assemble.rs, UDP handles, UpdateRates handler
- [ ] 2. Capture cell, UDP tap, StartCapture and StopCapture handlers
- [ ] 3. Fake controller tests, controller-e2e check
- [ ] 4. Agent.md, log entry, PR

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Step 1.

## Blockers and open questions

none

## Decisions taken in this package

See the PR description and docs/wiki/Agent.md once written.

## Pitfalls

The sandbox worktree guard refuses compound git commands. Run git alone per call.

## Verify the current state

```bash
CARGO_BUILD_JOBS=1 cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08 | claude-a05c1d | claim | | |
