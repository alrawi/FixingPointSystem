# Handoff AGT-09 Serve capture downloads in the agent

| Field | Value |
|---|---|
| Issue | #266 |
| Branch | `wp/AGT-09` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-9c4e1a, lease until 2026-11-09T15:12:51Z |
| Last verified commit | f8b08af |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Answer `fetch_capture` with the size, digest, and chunk size of the finished capture file, and stream the file over `UploadCapture` on the same mutually authenticated channel, from `start_offset`. Refuse unknown ids, captures that still run, and an offset beyond the size. Delete finished capture files after a configurable retention, never one with an upload in flight.

## Plan

- [x] 1. Claim commit and this handoff.
- [x] 2. Retention and transfer guard in `control.rs`, `fetch_capture` and the upload task in `session.rs`, the `--capture-retention` flag.
- [x] 3. Tests: fake controller downloads in two parts, refusals, retention.
- [ ] 4. `docs/wiki/Agent.md`, log entry, checks, pull request.

## Done and verified

- `cargo test -p fps-agent` passes, including the download in `tests/run_control.rs` (two parts, resumed at an offset, SHA-256 equals the digest) and the transfer and retention unit tests in `src/control.rs`.
- `cargo clippy -p fps-agent --all-targets -- -D warnings` and `cargo fmt --check` are clean.
- `plan.py check`, `render --check`, and `lint` pass.

## Remaining

Pull request, CI, review hand-over. No real-controller e2e step: see Decisions.

## Next action

Open the pull request and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- The session loop (not `Shared::command`) serves `fetch_capture`, because the answer must be queued before the upload call opens. The upload is a tokio task holding a clone of the session's tonic client, aborted when the session ends.
- `Captures::open_transfer` returns a `Transfer` that holds the file open, counts as in flight until dropped, and takes the one transfer slot (`RESOURCE_EXHAUSTED` otherwise). `Captures::sweep` deletes expired files and skips in-flight ones. A separate `life` mutex guards this, because `Entry::done` stays locked while a ring is joined.
- `Captures::reset` keeps finished entries of earlier plans in a `retired` list so the retention still deletes their files.
- Retention flag `--capture-retention SECONDS`, default 86400, 0 keeps files; the clock starts when the file is written.
- No `controller_e2e.sh` capture step: `POST /v1/runs/{id}/captures` needs a non-empty `port`, and the agent refuses a non-empty port (UNIMPLEMENTED, AGT-05), so the real controller cannot start a capture on the real agent. Reported in the log entry.

## Pitfalls

The first build of fps-agent takes about 10 minutes with one job. The sandbox guard refuses some shell forms next to version control commands, so keep scripts in separate calls.

## Verify the current state

```bash
cargo test -p fps-agent
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T14:45Z | claude-9c4e1a | claim | none | Claimed #266 |
| 2026-11-09T15:10Z | claude-9c4e1a | checkpoint | f8b08af | Code, tests, docs done; opening the PR |
