# Handoff AGT-06 Kernel requirement and redial policy

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #138 |
| Branch | `wp/AGT-06` |
| Pull request | none yet |
| Status | in progress |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-d5d4e4, lease until 2026-11-05T20:20:39Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 3 and 11. The agent refuses to start on Linux older than 6.3 with an error that names IP_LOCAL_PORT_RANGE, and it redials the controller with a backoff that restarts at 200 ms with jitter after an abort, and with a limit on failed redialing that defaults to five minutes.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Kernel version check through SystemRoot, with tests
- [x] 2. Backoff reset after abort, with jitter, with tests
- [x] 3. Redial limit flag and nonzero exit, with tests
- [x] 4. Make targets, wiki Agent page, crate README, log entry, checks

## Done and verified

- Step 1: `capabilities::require_kernel(&SystemRoot)` and `KernelVersion::require_ip_local_port_range`, called first by `fps-agent connect` and `run`. `cargo test -p fps-agent --test detect` passes with the 5.4 fixture refused and 6.3 and newer accepted.
- Step 2 and 3: `session::RedialPolicy` with jitter, reset after abort, and the limit. `Config.redial_limit` and `jitter_seed`, `SessionError::RedialLimit`, flag `--redial-limit SECONDS` (default 300, 0 for none), exit code 3. `cargo test -p fps-agent --test redial --test exit_codes` passes, including `serve` against a closed port.
- Step 4: `make test-agt-redial`, [Agent](../wiki/Agent.md) sections, `engine/fps-agent/README.md`, log entry. `make test-agt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and the three `plan.py` checks pass locally.

## Remaining

The real controller end to end test (`make test-agt-controller`) could not run in the sandbox, because the Go module proxy is blocked. CI runs it.

## Next action

Wait for CI on the pull request and fix failures inside engine/fps-agent/.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- The kernel check compares the release version and does not probe the option, because the decision names Linux 6.3. A distribution that backports the option is refused.
- Jitter draws from the upper half of each step, so a wait never exceeds the maximum. The seed comes from the clock in `main.rs`, the process edge.
- An abort restarts the backoff but keeps the failure window running, so two agents of one identity that replace each other cannot redial forever. Only a session that opened and ended without an abort clears the window.
- `--redial-limit 0` means no limit. A distinct exit code 3 lets a supervisor tell a given up agent from a configuration fault (code 2).

## Pitfalls

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
export CARGO_TARGET_DIR=/tmp/fps-target
make test-agt-redial   # 20 tests pass
make test-agt          # every test of the crate passes
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-05T19:50Z | claude-d5d4e4 | claim | none | Claimed issue #138, card 6666407169 |
| 2026-11-05T19:10Z | claude-d5d4e4 | checkpoint | see branch | Kernel check, redial policy, flag, docs, tests all done locally |
