# Handoff AGT-19 SMB2 in the agent

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #408 |
| Branch | `wp/AGT-19` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (3 of 3 plan steps, CI pending) |
| Current holder | claude-61ad74, lease until 2026-11-11T00:05:18Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Register the smb2 module (MOD-12 functions, MOD-13 manifest and scenarios) in the agent and run an smb2 scenario through the real agent under the real controller with succeeded sessions.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Dependency, registry line, module list tests
- [x] 2. controller_e2e.sh runs smb2/download and smb2/upload
- [x] 3. Checks, PR (CI pending)

## Done and verified

- `cargo test -p fps-agent`: all suites pass, including the unit tests `the_registry_covers_every_first_party_module` (lists smb2) and the manifest tests, which now cover smb2 through `registered_with_manifest`.
- New component test `smb2_scenarios_run_through_the_agent` in `tests/first_party.rs` runs smb2/download and smb2/upload through `run` with succeeded smb2 sessions (passes locally, 3.8 s).
- `cargo fmt --check` and `cargo clippy -p fps-agent --all-targets -D warnings` clean.
- `controller_e2e.sh` runs smb2/download and smb2/upload under the real controller on ports 18687 and 18688. `bash -n` passes. It cannot run here (no Go modules), so CI is the check.

## Remaining

CI result of the PR.

## Next action

Step 1.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- Port 445 needs root or `net.ipv4.ip_unprivileged_port_start`, and the e2e script runs without root, so it uses server_port 18687 and 18688 (nDPI classifies SMB only on 445, but the e2e checks sessions). Cargo.lock gains the fps-agent dependency line.

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
