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

- Registry, dependencies, required-module registration: `cargo test -p fps-agent` passes (61 lib tests, `tests/first_party.rs`, all other agent tests). `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt --check` are clean.
- `tests/first_party.rs` negative control: with `content` out of the registry the run fails every session.
- `controller_e2e.sh` smtp run added; `bash -n` passes. It needs the Go controller, so CI is its first run.
- `docs/wiki/Agent.md` and the log entry updated; `plan.py check`, `render --check` pass.

## Remaining

CI result, then review. FTP scenarios need spawned data flows, which `fps-runtime` does not run (the agent refuses scenarios with more than one flow), so FTP has its functions registered and its scenarios need runtime work, reported in the PR.

## Next action

Poll the CI of the PR and fix failures inside `engine/fps-agent/`.

## Blockers and open questions

none

## Decisions taken in this package

- The plan lists only the modules of the mix scenarios, so `content` is absent from an smtp plan. The agent registers required modules that the plan omits (no settings), checking the catalog against the installed manifest when there is one. Reason: otherwise every content call of a mail scenario fails in the agent.
- `for_modules` takes the extension module names (AGT-11 note) instead of `load` filtering notes afterwards.

## Pitfalls

- The worktree guard refuses long heredocs and `export` in one command; write files with the editor tools.

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent --lib modules
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-a13e7c | claim | none | claimed #310 |
