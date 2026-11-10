# Handoff CON-18 Decide the transport keyword of the scenario contract

| Field | Value |
|---|---|
| Issue | #351 |
| Branch | `wp/CON-18` |
| Pull request | none yet |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-8b5636, lease until 2026-11-10T02:37:47Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The schema's `transport` (module tls with a profile) is rejected by `Scenario::load` and ignored by the validator. Decide it: define it as sugar for a leading `tls_upgrade`, or remove it. The owner approved the contract change. The loader and the validator must agree on the keyword.

## Plan

- [x] 1. Decide (A, sugar) and change the schema, validator, tests, embedded copy
- [x] 2. Desugar in `Scenario::load` of `fps-flow` with parity fixtures
- [x] 3. README, CHANGELOG, `contracts/changes/CON-18.md`, wiki Contracts page
- [ ] 4. Run the checks, open the PR, wait for CI

## Done and verified

- `python3 -m pytest tools/contracts engine/fps-flow/tests/test_fixtures.py -q`: 248 passed, 1 skipped.
- `cargo test -p fps-flow` and `cargo clippy -p fps-flow --all-targets -- -D warnings`: pass.

## Remaining

The PR and CI.

## Next action

Run `plan.py check`, `render --check`, `lint`, then open the PR.

## Blockers and open questions

none

## Decisions taken in this package

- Option A, because `fps-flow` already builds `Op::UpgradeTls`, so the sugar is one op per side. See `contracts/changes/CON-18.md`.
- `server_name` goes to the client side only, because only a client handshake uses it.
- `module` is the constant `tls` and `profile` is required. The old shape never ran.

## Pitfalls

- The `fx` fixture module lists every scenario in its `manifest.yaml`, so a new fixture needs a line there.
- The sandbox guard refuses compound shell commands in a worktree. Use the editor tools.

## Verify the current state

```bash
python3 -m pytest tools/contracts engine/fps-flow/tests/test_fixtures.py -q
CARGO_TARGET_DIR=$PWD/target-CON-18 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-flow
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T01:10Z | claude-8b5636 | claim | none | Claimed #351 |
