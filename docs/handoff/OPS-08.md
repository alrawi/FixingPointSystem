# Handoff OPS-08 CI check that keeps self hosted runners off untrusted triggers

| Field | Value |
|---|---|
| Issue | #579 |
| Branch | `wp/OPS-08` |
| Pull request | #580 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-8c41d2, lease until 2026-11-17T00:23:00Z |
| Last verified commit | none |

## Goal

Add `tools/swarm/check_self_hosted.py` with tests. For every file of `.github/workflows/`, a job that names a self hosted label fails unless the workflow starts only on `workflow_dispatch` and `schedule`, and the job has `contents: read`, `persist-credentials: false` on every checkout, and `timeout-minutes`. Give the coordinator the exact step to add to an existing workflow, and report what `main` must change to pass.

## Plan

- [x] 1. Read the rules (Operations "Owner host", D13) and the workflows on `main`.
- [x] 2. Write the check and its tests.
- [x] 3. Document it in Operations and the swarm README, and write the log entry with the Shared page notes.
- [x] 4. Run ruff, mypy, pytest, and the plan checks.
- [ ] 5. Wait for CI on pull request #580, and send it to review.

## Done and verified

- `python3 -m pytest -q tools/swarm` passes (152 tests, 76 of them new).
- `ruff check` and `mypy --strict` pass on the two new files.
- `python3 -I tools/swarm/check_self_hosted.py` exits with 1 on `main`, with one finding: `qa-bench.yml` has `push` and `pull_request` next to its self hosted job `nightly`.

## Remaining

CI on pull request #580. The coordinator must split `nightly` out of `qa-bench.yml` and add the step to `ops.yml` (both in the Shared page notes of the log entry), in one pull request, before the check can pass on `main`.

## Next action

Poll the check runs of pull request #580.

## Blockers and open questions

none

## Decisions taken in this package

- A job level `if:` is not accepted in place of the trigger rule (Operations rule 1 and D13 say the workflow has no such trigger, and a guard can be wrong in the file that a branch edits). A missing `github.ref == 'refs/heads/main'` guard only warns, because `workflow_dispatch` runs the file of any branch.
- Fail closed on labels: only `ubuntu-*`, `windows-*`, and `macos-*` are hosted.
- A self hosted job needs at least one `actions/checkout`, and every checkout needs `persist-credentials: false`.
- The ban on secrets and rules 3 to 5 of Operations are not checked.

## Pitfalls

PyYAML reads the key `on` as `True`, so the check reads both spellings.

## Verify the current state

```bash
python3 -m pytest -q tools/swarm/test_check_self_hosted.py   # 76 passed
python3 -I tools/swarm/check_self_hosted.py                  # exit 1 until qa-bench.yml is split
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16T22:55 | claude-8c41d2 | claim | none | claimed #579 |
