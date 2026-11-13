# Handoff RLM-23 Expected parser findings in every session of a rule

| Field | Value |
|---|---|
| Issue | #551 |
| Branch | `wp/RLM-23` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-72c43f, lease until 2026-11-13T18:06:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add an optional setting of a session rule of the realism labels under which every session of the rule must show every declared `expect_findings` item, with `missing_findings` naming each session that lacks one. Rules without the setting keep the RLM-20 behaviour. Update the realism report schema, keep every committed fixture passing, and put the text for `docs/wiki/Module-Development.md` in the Shared page notes of the log entry.

## Plan

- [x] 1. Labels: `expect_every_session` on a rule (parser, schema, refusals).
- [x] 2. Parser error check and report schema (format 0.21.0), controller schema copy, `ui/src/api/schema.d.ts`.
- [x] 3. Tests, README, Realism page, log entry with the Shared page notes.
- [ ] 4. Pull request, CI, review.

## Done and verified

- `python3 -m pytest realism -q`: 935 passed, 9 skipped (includes the real tshark test).
- `python3 -m ruff format --check realism`, `ruff check realism`, `mypy --config-file realism/pyproject.toml realism`: clean.
- Every committed labels file loads with `realism.labels.load`; the dns, http1, ftp, and hls fixtures score with exit 0 under tshark.
- `ui/src/api/schema.d.ts` regenerated with openapi-typescript 7.13.0; only the new fields changed.

## Remaining

Open the pull request and wait for CI. Go could not be built here, so the controller schema equality test runs only in CI (the copy was made with `cp` and is byte identical).

## Next action

Open the pull request with `gh api repos/alrawi/FixingPointSystem/pulls`, then poll the check runs.

## Blockers and open questions

none

## Decisions taken in this package

- Rule level boolean `expect_every_session`, not a count, so the declaration does not drift with the number of requests.
- It needs `expect_findings` in the same rule and is refused otherwise.
- Only entries of a rule with the setting carry `session`, `transport`, `client`, `server`; a rule with the setting that matches no session gives one entry per item without a session, as RLM-20 does.
- The controller schema copy and the generated UI types lie outside `realism/` and are refreshed, as RLM-20 and RLM-22 did, because the Go equality test and `check:api` need them.

## Pitfalls

- The worktree guard refuses long compound shell commands with git or heredocs; write scripts with the editor tools.
- `labels.schema.json` repeats the `match` block several times; anchor edits on the `expect_findings` block.

## Verify the current state

```bash
python3 -m pytest realism/tests/test_expected_findings.py -q   # 37 passed
python3 -m ruff check realism && python3 -m mypy --config-file realism/pyproject.toml realism
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13 | claude-72c43f | claim | none | claimed #551 |
