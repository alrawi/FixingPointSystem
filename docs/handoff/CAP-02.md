# Handoff CAP-02 Scenario generation from sessions

| Field | Value |
|---|---|
| Issue | #194 |
| Branch | `wp/CAP-02` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-8fb6e0, lease until 2026-11-10T02:37:51Z |
| Last verified commit | see the session log |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Generate a flow program from extracted sessions: `python3 -m capture scenario --sessions FILE [--session ID | --all] --out-dir DIR` writes a module directory (`manifest.yaml` and `scenarios/*.yaml`) with a client flow and a server flow, and replaces dynamic fields (lengths, identifiers, timestamps, cookies) by template expressions. A generated scenario validates with `tools/contracts/validate.py` and replays in the QA-01 harness with the same byte counts within 1%.

## Plan

- [x] 1. Claim commit, generator core (templates, framing, field detection, YAML, CLI)
- [x] 2. Unit tests with a model of the flow interpreter, validator test, lint
- [x] 3. Harness replay test (agent standalone mode) and a local loopback run
- [x] 4. Wiki page `docs/wiki/Capture.md`, `capture/README.md`, log entry
- [ ] 5. Pull request, CI, review hand-off

## Done and verified

- `make lint-cap test-cap PYTHON=python3`: ruff format and check, `mypy --strict`, 114 tests pass, 1 skipped (the replay test, which needs root, tcpdump and the agent).
- All 608 sessions of the committed module fixtures (http1, dns, smtp, ftp, imap, pop3) generate scenarios that pass `tools/contracts/validate.py` with 0 errors (`--synthetic`), and the flow model reproduces every byte count exactly with no deadlock.
- Real `fps-agent` replay in loopback mode: byte counts equal per port (8081, 8025, 8443, and UDP 8053), agent verdict pass.

## Remaining

Step 5. The namespace mode of the replay test has not run (no `ip` in the sandbox) and needs the CI job on the Capture page.

## Next action

Open the pull request, wait for CI, then set the status to ready for review.

## Blockers and open questions

none

## Decisions taken in this package

- The generator uses only the standard library, like CAP-01. The YAML writer quotes every string, so no reader reinterprets a value. PyYAML is a test dependency only (the validator needs it).
- The only random source for visible text in a template is `rand_int`, so a token of hex or base64 characters becomes a token of digits of the same length. There is no clock function, so an HTTP date becomes a scenario variable that keeps the observed date, and an epoch timestamp becomes `rand_int` inside a window of the capture time.
- A binary length prefix stays a literal (`hex(..)`) next to a `fill(N)` of the same size, because the contract has no function that writes an integer as bytes. An ASCII length (Content-Length) becomes a variable that the header and the body share.
- HTTP chunked bodies keep their size lines literal and fill the chunk data.
- Without `--synthetic` or `--consent` with `--scrubbed`, the manifest records an unscrubbed capture and fails validation until CAP-03.
- Combined scenarios use `spawn`, because the contract does not say that `after` carries variables.

## Pitfalls

- The worktree guard refuses shell lines that mix `cd`, heredocs, and `git`. Write files with the editor tools and run one plain command per call.
- The validator strips placeholders and then rejects any `{{` or `}}`, so a literal brace next to a placeholder or another brace is written as `hex(..)`.

## Verify the current state

```bash
make lint-cap test-cap PYTHON=python3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T02:07Z | claude-8fb6e0 | claim | 3bd99d4 | claimed #194, branch wp/CAP-02 |
| 2026-11-10T01:40Z | claude-8fb6e0 | checkpoint | see git log | tests, replay test, wiki, chunked bodies |
