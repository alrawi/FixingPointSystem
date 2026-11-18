# Handoff AGT-27 The standalone agent writes a TLS key log

| Field | Value |
|---|---|
| Issue | #674 |
| Branch | `wp/AGT-27` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-6032c3, lease until 2026-11-18T18:47:37Z |
| Last verified commit | none |

## Goal

`fps-agent run --keylog FILE` writes the TLS secrets of a standalone run in the NSS key log format, so the afxdp job of `mod-http1` decrypts every TLS session of its capture and classifies the decrypted view as HTTP, with no change to the job.

## Plan

- [ ] 1. The flag in `run.rs` and `main.rs`, from the diff in the TRN-12 log entry.
- [ ] 2. The test in `tests/standalone.rs` and the line on the Agent page.
- [ ] 3. Checks, pull request, CI.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Apply the diff to `engine/fps-agent/src/run.rs` and `main.rs`.

## Blockers and open questions

none

## Decisions taken in this package

- The key log file is created with mode 0600, because it holds session secrets.

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_BUILD_JOBS=1 cargo test -p fps-agent --test standalone
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-18T18:17Z | claude-6032c3 | claim | none | claimed #674 |
