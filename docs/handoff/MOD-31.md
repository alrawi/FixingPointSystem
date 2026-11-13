# Handoff MOD-31 HLS timing and bytes as labels

| Field | Value |
|---|---|
| Issue | #547 |
| Branch | `wp/MOD-31` |
| Pull request | #555 |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps, CI pending the one outside test) |
| Current holder | claude-7f1c9e, lease until 2026-11-13T18:06:32Z |
| Last verified commit | 7b735ce |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Move what `modules/hls/fixtures/hls_check.py` checks into the `temporal` labels of `modules/hls/fixtures/expected.yaml`, using the labels of RLM-22 (a packet gap target selected by request path, `min_samples` per target, and `temporal.rungs`), at equal strength. Then remove the script, its tests, `hls.playbacks.json`, and their calls in `capture.sh`, `mk/mod-hls.mk`, and the workflow.

## Plan

- [x] 1. Claim, branch, handoff file.
- [x] 2. Labels in `expected.yaml`; tests in `modules/hls/fixtures/test_temporal_labels.py` (committed capture passes, retimed and wrong rate captures fail, labels match `params.json`).
- [x] 3. Remove `hls_check.py`, `test_hls_check.py`, `hls.playbacks.json` and their calls; update the README of the module and the module development page.
- [ ] 4. Checks, log entry, PR, CI.

## Done and verified

- `python3 -m unittest discover -s modules/hls/fixtures`: 23 tests pass (committed capture passes; retimed 1.15, 0.8 and 1.06 fail; wrong rate, drift, burst, missing playback fail; labels match `params.json`).
- `python3 -m realism score` of the committed capture with the labels (nDPI 4.2.0, tshark 4.2.2 here): passes, exit 0. The same score of the 1.15 and 1.06 retimed copies exits 1.
- `plan.py check`, `render --check`, `lint`: clean. `make lint-mod-hls`: 0 errors.
- `python3 -m pytest -q realism`: 920 passed, 1 failed (`test_http_rungs.py::test_the_byte_numbers_equal_those_of_hls_check`, which reads the deleted `hls.playbacks.json`; outside this package's paths, reported in the log entry).

## Remaining

The coordinator's change to `realism/tests/test_http_rungs.py`.

## Next action

The coordinator changes or deletes `realism/tests/test_http_rungs.py::test_the_byte_numbers_equal_those_of_hls_check` (the only red check that this package causes), then CI is green and the package goes to review.

## Blockers and open questions

none

## Decisions taken in this package

- The labels are the block of the RLM-22 log entry, unchanged, so they have the strength of `hls_check.py` (mean 20 ms, 90% within 40 ms, 188 octet body, 5% share and rate, walk up for vod).
- The new tests are `unittest` (`test_temporal_labels.py`), because the mod-hls workflow and `make test-mod-hls` run `unittest discover` and install no pytest.
- `hls.report.json` is scored again so that it holds the new comparisons (it was at format 0.11.0).
- `mk/mod-hls.mk` and `.github/workflows/mod-hls.yml` are edited although the catalog path is `modules/hls/`, because the deliverable names them (MOD-24 did the same).

## Pitfalls

- `capture.sh` has an EXIT trap that tears the harness down, so the final score command must not use `exec`.
- The worktree guard refuses commands that mix `git` with other commands in one call.

## Verify the current state

```bash
python3 -m unittest discover -s modules/hls/fixtures
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13T17:36Z | claude-7f1c9e | claim | 8f7aad3 | claimed #547 with card 6858978262 |
| 2026-11-13T17:58Z | claude-7f1c9e | checkpoint | 7b735ce | PR 555 opened; CI: module (hls) green; test (realism) fails only on the test that reads the deleted hls.playbacks.json; mix fails on an unrelated Zeek weird (event_queue_overflow_drain) |
