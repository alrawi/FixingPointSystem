# Handoff IMG-07 The heartbeat timeout in the image boot test

| Field | Value |
|---|---|
| Issue | #410 |
| Branch | `wp/IMG-07` |
| Pull request | #0 |
| Status | in progress |
| Progress | 70% (3 of 4 plan steps) |
| Current holder | claude-7a1c4e, lease until 2026-11-10T19:00:09Z |
| Last verified commit | none |

## Goal

The "Build, boot, and run" job of `image.yml` failed twice on 2026-11-10 (MOD-06, MOD-22 branches) and passed on rerun with the same code. Find the cause from the VM logs of the failing runs and fix it inside `image/`, so that the boot test passes 10 times in a row in CI and still fails when the agent cannot reach the controller.

## Plan

- [x] 1. Cause from the logs of the failing jobs 129934010691 and 129666780207, compared with passing jobs
- [x] 2. Session gate in `image/tests/vm-run.sh`, with an offline replay test
- [x] 3. Loop and negative mode in `image/boot-test.sh`
- [ ] 4. Docs, log entry, PR, CI loop of 10 boots

## Done and verified

- Cause, from the journals of jobs 129666780207 and 129934010691 (both KVM): the controller marks the agent online at the open message and sends nothing until a command or a heartbeat; the Rust agent heartbeats only after the first response frame. Idle sessions end every 5 to 6 s. A first command in the last second before the sweep (5.02 s and 5.08 s after the session began) meets an agent whose watchdog already fired. The fix of the cause is `stream.SendHeader` in the controller, outside `image/`.
- `image/tests/session-gate-test.sh` (22 checks, replays the failed job and refuses its moment) and `image/tests/boot-loop-test.sh` (26 checks) pass; both run in `image/tests/static-check.sh`. ShellCheck is clean on every changed script. A mutation (`FPS_RUN_FRESH_S=10`) makes the replay test fail.
- `make lint-img test-img`, `plan.py check`, `render --check`, and `lint` pass.

## Remaining

CI: the gate verdict in a real VM, ten boots in a row, and the negative boot (needs a temporary loop default, see Decisions).

## Next action

Open the PR, read the gate line in the CI log, then push the temporary commit that sets the loop defaults, and read the result.

## Blockers and open questions

none

## Decisions taken in this package

- The gate is a workaround that the boot test needs while the controller sends no response headers on open. It names the gap in a NOTE when it takes the `fresh` path, so that CI logs show it until the controller fix lands.
- The loop lives in `boot-test.sh` behind `FPS_TEST_REPEAT`, because `image.yml` is a coordinator file. A temporary commit sets the defaults to 10 boots and the negative boot for the PR run, and a later commit reverts it, so the head of the PR keeps the defaults 1 and none.

## Pitfalls

The sandbox has no QEMU and no KVM, so no VM step runs locally. The CI job is the only place the boot test runs.

## Verify the current state

```bash
image/tests/static-check.sh
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T17:00Z | claude-7a1c4e | claim | none | claimed #410, card 6774392966 |
| 2026-11-10T18:05Z | claude-7a1c4e | checkpoint | see git log | cause found, session gate, loop and negative boot, offline tests, docs |
