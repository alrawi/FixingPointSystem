# Handoff CTL-19 Optional token for the metrics endpoint

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #136 |
| Branch | `wp/CTL-19` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (3 of 3 plan steps, CI pending) |
| Current holder | claude-d5763a, lease until 2026-11-05T20:20:40Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 item 2. `GET /metrics` stays open by default, and a configuration setting can require a bearer token for it.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Setting `metrics_token_file` and flag `-metrics-token-file`, loader, and constant time bearer middleware in `internal/app`
- [x] 2. Wire it in `cmd/fps-controller` and add tests (open, required, wrong token)
- [x] 3. Document in the Controller wiki page, log entry, and run the checks

## Done and verified

- Steps 1 to 3 written. `gofmt -l controller` is clean, `plan.py check`, `render --check`, and `lint` pass.
- The middleware logic passes its table test in a scratch module (stdlib only).

## Remaining

CI result of `make lint-ctl test-ctl`. The sandbox cannot reach proxy.golang.org, so the controller module could not build locally.

## Next action

Wait for CI, fix failures inside `controller/internal/app/` and `controller/cmd/`.

## Blockers and open questions

none

## Decisions taken in this package

- The setting is `metrics_token_file` only, with the flag `-metrics-token-file`. A file keeps the secret out of config, flags, and logs. The repo has no environment variable convention, so there is none.
- Constant time comparison over SHA-256 digests, which also hides the token length.
- The metrics token is separate from `tokens`.

## Pitfalls

No Go module access in the sandbox. Do not use GOPROXY=direct, the sandbox policy denies it.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-05T19:50Z | claude-d5763a | claim | | Claimed #136 |
