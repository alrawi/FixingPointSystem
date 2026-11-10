# Handoff AGT-17 SSH in the agent and TLS profiles from the TLS module

| Field | Value |
|---|---|
| Issue | #388 |
| Branch | `wp/AGT-17` |
| Pull request | #0 |
| Status | in progress |
| Progress | 60% (3 of 5 plan steps) |
| Current holder | claude-1c4e6a |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The agent registers the ssh module (MOD-11) with one `Ssh` value per session side, takes the profiles `mail-client`, `server-tls13-h2`, and `server-tls13-noalpn` from the TLS module registry instead of local copies, and parses the `manifest.yaml` bytes that the digest pass read, closing the check-then-use gap that AGT-16 closed for `extension.wasm`. A controller e2e plan runs ssh/interactive_shell and ssh/scp_upload with succeeded sessions.

## Plan

- [x] 1. Parse the digested manifest bytes in `modules.rs` and add the swap tests
- [x] 2. Look the TLS profiles up in the TLS module registry and test that they are the module's
- [x] 3. Register ssh, drop the http2 test exception, run ssh through the agent in `first_party.rs`
- [ ] 4. Extend `controller_e2e.sh`, wiki page, and log entry; run the checks of AGENTS.md
- [ ] 5. Open the PR, wait for CI, hand over for review

## Done and verified

- Steps 1 to 3: `cargo test -p fps-agent --lib` (78 passed) and `--test extensions --test first_party --test tls` (all passed), with `CARGO_PROFILE_DEV_DEBUG=0`, one job.
- Step 4 partly: the script edit is in place and cannot run here (Go does not build in this sandbox), so the controller e2e is left to CI.

## Remaining

Clippy and the repository checks, the PR, and CI.

## Next action

Run `cargo clippy -p fps-agent --all-targets -- -D warnings`, the plan checks, then push and open the PR.

## Blockers and open questions

none

## Decisions taken in this package

- `SshSide` wraps `Ssh` and clones to a fresh value, because `Ssh` is not `Clone` and the registry copies its value for every session side. A clone of a used value is never a copy of its state, which is the property that sessions need.
- `server_profile` sets `ServerResumption::Tickets` on the module's `server-tls13-h2` and `server-tls13-noalpn`, so the agent's behaviour does not change (the previous local copies used `ServerProfile::default()`, which issues stateless tickets).
- The digest pass keeps both `manifest.yaml` and `extension.wasm`. The manifest is parsed before the name, version, and digest comparisons, so the order of the errors of a bad tree is as before, with a missing manifest still a `ModuleError::Manifest`.
- `modules::installed` uses the same single pass, so the name, version, and digest that the agent reports come from one read.

## Pitfalls

- A swap after the digest cannot show the gap that was closed here, because the old code also parsed before the hook ran. The race test with a writer thread covers the window before the digest.
- The worktree guard refuses heredocs in the same command as `git`; use the editor tools.

## Verify the current state

```bash
CARGO_TARGET_DIR=$PWD/target-AGT-17 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-agent --lib --test extensions --test first_party --test tls
```

Expected: every test passes.

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T14:25Z | claude-1c4e6a | claim | n/a | Claimed card 6769807358 |
| 2026-11-10T15:00Z | claude-1c4e6a | checkpoint | see branch | Manifest from the digest pass, profiles from the TLS module, ssh registered |
