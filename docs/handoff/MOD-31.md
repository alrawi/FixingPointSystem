# Handoff MOD-31 HLS timing and bytes as labels

| Field | Value |
|---|---|
| Issue | #547 |
| Branch | `wp/MOD-31` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-7f1c9e, lease until 2026-11-13T18:06:32Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Move what `modules/hls/fixtures/hls_check.py` checks into the `temporal` labels of `modules/hls/fixtures/expected.yaml`, using the labels of RLM-22 (a packet gap target selected by request path, `min_samples` per target, and `temporal.rungs`), at equal strength. Then remove the script, its tests, `hls.playbacks.json`, and their calls in `capture.sh`, `mk/mod-hls.mk`, and the workflow.

## Plan

- [ ] 1. Claim, branch, handoff file.
- [ ] 2. Labels in `expected.yaml`; tests in `modules/hls/fixtures/test_labels.py` (committed capture passes, retimed and wrong rate captures fail, labels match `params.json`).
- [ ] 3. Remove `hls_check.py`, `test_hls_check.py`, `hls.playbacks.json` and their calls; update the README of the module and the module development page.
- [ ] 4. Checks, log entry, PR, CI.

## Done and verified

none yet

## Remaining

Everything after the claim.

## Next action

Add the temporal block of the RLM-22 log entry to `expected.yaml` and write the tests.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
python3 -m pytest -q modules/hls/fixtures
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13T17:36Z | claude-7f1c9e | claim | pending | claimed #547 with card 6858978262 |
