# Handoff AGT-13 First party native modules in the agent

| Field | Value |
|---|---|
| Issue | #310 |
| Branch | `wp/AGT-13` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-a13e7c, lease until 2026-11-10T00:39:28Z |
| Last verified commit | none |

## Goal

The agent dispatches the native functions of every first party module crate (http1, dns, content, smtp, imap, pop3, ftp) through one registry, so a new module needs one line. The controller e2e test runs an smtp plan through the real agent, and an agent test shows each module's functions resolve.

## Plan

- [ ] 1. Registry in `engine/fps-agent/src/modules.rs` and dependencies in `Cargo.toml`.
- [ ] 2. Agent unit test that every first party manifest resolves.
- [ ] 3. smtp run in `tests/controller_e2e.sh`.
- [ ] 4. `docs/wiki/Agent.md`, log entry, checks, PR.

## Done and verified

Nothing yet.

## Remaining

Everything. FTP scenarios need spawned data flows, which `fps-runtime` does not run (the agent refuses scenarios with more than one flow), so FTP gets its functions but its scenarios need runtime work, reported in the PR.

## Next action

Write the registry.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent --lib modules
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-a13e7c | claim | none | claimed #310 |
