# Handoff CAP-02 Scenario generation from sessions

| Field | Value |
|---|---|
| Issue | #194 |
| Branch | `wp/CAP-02` |
| Pull request | #0 |
| Status | in progress |
| Progress | 20% (1 of 5 plan steps) |
| Current holder | claude-8fb6e0, lease until 2026-11-10T02:37:51Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Generate a flow program from extracted sessions: `python3 -m capture scenario --sessions FILE [--session ID | --all] --out-dir DIR` writes a module directory (`manifest.yaml` and `scenarios/*.yaml`) with a client flow and a server flow, and replaces dynamic fields (lengths, identifiers, timestamps, cookies) by template expressions. A generated scenario validates with `tools/contracts/validate.py` and replays in the QA-01 harness with the same byte counts within 1%.

## Plan

- [x] 1. Claim commit, generator core (templates, framing, field detection, YAML, CLI)
- [ ] 2. Unit tests with a model of the flow interpreter, validator test, lint
- [ ] 3. Harness replay test (agent standalone mode in the namespace harness) and a local loopback run
- [ ] 4. Wiki page `docs/wiki/Capture.md`, `capture/README.md`, log entry
- [ ] 5. Pull request, CI, review hand-off

## Done and verified

Nothing verified yet.

## Remaining

Steps 2 to 5.

## Next action

Step 2: write `capture/tests/test_scenario.py`.

## Blockers and open questions

none

## Decisions taken in this package

- The generator uses only the standard library, like CAP-01. The YAML writer quotes every string, so no reader reinterprets a value. PyYAML is a test dependency only (the validator needs it).
- The only random source for visible text in a template is `rand_int`, so a token of hex or base64 characters becomes a token of digits of the same length. There is no clock function, so an HTTP date becomes a scenario variable that keeps the observed date, and an epoch timestamp becomes `rand_int` inside a window of the capture time.
- A binary length prefix stays a literal (`hex(..)`) next to a `fill(N)` of the same size, because the contract has no function that writes an integer as bytes. An ASCII length (Content-Length) becomes a variable that the header and the body share.

## Pitfalls

- The worktree guard refuses shell lines that mix `cd`, heredocs, and `git`. Write files with the editor tools and run one plain command per call.

## Verify the current state

```bash
make lint-cap test-cap PYTHON=python3
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T02:07Z | claude-8fb6e0 | claim | none | claimed #194, branch wp/CAP-02 |
