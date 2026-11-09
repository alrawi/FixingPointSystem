# Handoff AGT-09 Serve capture downloads in the agent

| Field | Value |
|---|---|
| Issue | #266 |
| Branch | `wp/AGT-09` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-9c4e1a, lease until 2026-11-09T15:12:51Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Answer `fetch_capture` with the size, digest, and chunk size of the finished capture file, and stream the file over `UploadCapture` on the same mutually authenticated channel, from `start_offset`. Refuse unknown ids, captures that still run, and an offset beyond the size. Delete finished capture files after a configurable retention, never one with an upload in flight.

## Plan

- [ ] 1. Claim commit and this handoff.
- [ ] 2. Retention and transfer guard in `control.rs`, `fetch_capture` and the upload task in `session.rs`, the `--capture-retention` flag.
- [ ] 3. Tests: fake controller downloads in two parts, refusals, retention.
- [ ] 4. `docs/wiki/Agent.md`, log entry, checks, pull request.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Implement step 2.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T14:45Z | claude-9c4e1a | claim | none | Claimed #266 |
