# Handoff IMG-07 The heartbeat timeout in the image boot test

| Field | Value |
|---|---|
| Issue | #410 |
| Branch | `wp/IMG-07` |
| Pull request | #0 |
| Status | in progress |
| Progress | 5% (0 of 4 plan steps) |
| Current holder | claude-7a1c4e, lease until 2026-11-10T19:00:09Z |
| Last verified commit | none |

## Goal

The "Build, boot, and run" job of `image.yml` failed twice on 2026-11-10 (MOD-06, MOD-22 branches) and passed on rerun with the same code. Find the cause from the VM logs of the failing runs and fix it inside `image/`, so that the boot test passes 10 times in a row in CI and still fails when the agent cannot reach the controller.

## Plan

- [ ] 1. Cause from the logs of the failing jobs 129934010691 and 129666780207, compared with passing jobs
- [ ] 2. Session gate in `image/tests/vm-run.sh`, with an offline replay test
- [ ] 3. Loop and negative mode in `image/boot-test.sh`
- [ ] 4. Docs, log entry, PR, CI loop of 10 boots

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Write the session gate and its replay test.

## Blockers and open questions

none

## Decisions taken in this package

none yet

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
