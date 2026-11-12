# Handoff MOD-23 The http2 module capture that missed one TLS session

| Field | Value |
|---|---|
| Issue | #478 |
| Branch | `wp/MOD-23` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-ace46d, lease until 2026-11-12T04:27:06Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The job `module (http2)` of run 43077861098 (job 130722417029, pull request #473, which did not touch http2) failed `realism decrypted` with "the capture has 39 TLS sessions, expected 40". It passed on rerun. Find the cause (capture start, a session still open at the stop, or the generator), fix it, and show three consecutive module runs with 40 of 40 TLS sessions. Paths: `modules/http2/`.

## Plan

- [ ] 1. Read the failing job log and find which stage counted 39 and which counted 40.
- [ ] 2. Reproduce or prove the cause on the sandbox namespace harness.
- [ ] 3. Fix inside `modules/http2/` and add a test or a guard.
- [ ] 4. Open the PR, get three green mod-http2 module runs, set ready for review.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Read the job log (REST) and `realism/decrypted.py`.

## Blockers and open questions

The artifact `realism-http2` of the failed run (id 13882903267) cannot be downloaded from the sandbox: the blob host answers 403 to CONNECT. The capture of the failing run is therefore not available, and the cause rests on the log and on reproduction.

## Decisions taken in this package

None yet.

## Pitfalls

- The worktree guard refuses `source` in a command that runs git. Pass variables inline.

## Verify the current state

```bash
python3 -m pytest realism/tests/test_decrypted.py -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T02:57Z | claude-ace46d | claim | none | claimed #478 |
