# Handoff RLM-22 Request cadence and bytes for HLS in the realism labels

| Field | Value |
|---|---|
| Issue | #536 |
| Branch | `wp/RLM-22` |
| Pull request | #541 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-r22a7c, lease until 2026-11-13T06:48:40Z |
| Last verified commit | d71b9c5 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Let the `temporal` labels of the realism runner express what `modules/hls/fixtures/hls_check.py` checks: a packet gap target that selects the requests of one kind by HTTP request path, a minimum sample count per target, and a byte property per rung. A MOD follow up then moves `hls_check.py` into labels. This package does not touch `modules/hls/`; its tests prove the labels against the committed hls fixture.

## Plan

- [x] 1. Read the labels parser, the temporal check, the report schema, and the hls fixture; decide the label syntax.
- [x] 2. Labels parser: `path` selector, `join_ms`, `min_samples` per target, `mean_tolerance_ms`, `rungs` section.
- [x] 3. Reader of HTTP exchanges (`realism/exchanges.py`), rung check (`realism/rungs.py`), wiring in `temporal.py`.
- [x] 4. Both schemas, report format 0.20.0, controller schema copy, `ui/src/api/schema.d.ts`, `realism/tests/test_http_rungs.py`.
- [x] 5. Docs, log entry with Shared page notes, checks, PR, CI.

## Done and verified

- `python3 -m pytest -q realism`: 921 passed, 9 skipped. `ruff format --check`, `ruff check`, `mypy --strict` of `realism`: clean.
- `plan.py check`, `render --check`, `lint`: clean. `tools/contracts/validate.py`: 0 errors.
- `node ui/scripts/gen-api.mjs --check`: matches. `cmp` of the realism schema and the controller copy: equal.
- The committed hls capture passes with the labels of the log entry, and its bit rates and rung shares equal `hls.playbacks.json`.

## Remaining

none

## Next action

Review the pull request (RLM-22, #541) against the Definition of Done.

## Blockers and open questions

none

## Decisions taken in this package

- Report format 0.20.0 (next after 0.19.0 of main). New fields are optional, so a 0.19.0 report validates.
- `path` is allowed only under `applications` with `direction: client`, because the sessions of an application are the ones that speak HTTP, and the server sends no request.
- A comparison with a `path` and the `rungs` add no `realism.*` series: the catalog has none for bytes, and two paths of one application and direction would give two series with equal labels. The Go mapping of the controller reads comparisons by application and property, so no Go change is needed.
- The per target `min_samples` wins over the section value and over `--ks-min-samples`, because it is the more specific setting.
- Bytes on the wire are the octets of the response head and body, as `hls_check.py` counted them, not TCP or IP overhead.
- `join_ms` (800 in the hls labels) stands for the two segment durations that `hls_check.py` used to group the connections of `vod_segments`.

## Pitfalls

- The sandbox guard refuses heredocs and complex commands that mix `cd` with git. Write scripts to the scratch folder and run them.
- `ui/node_modules` came from `npm ci` for `gen-api.mjs` and is deleted again (it was untracked and ignored).

## Verify the current state

```bash
python3 -m pytest -q realism/tests/test_http_rungs.py realism/tests/test_temporal.py realism/tests/test_packet_gaps.py
python3 tools/plan/plan.py lint
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13T05:09Z | claude-r22a7c | claim | 4489226 | claimed #536 with card 6844739470 |
| 2026-11-13T05:19Z | claude-r22a7c | checkpoint | c2246b4 | labels, check, schemas, version 0.20.0 |
| 2026-11-13T05:32Z | claude-r22a7c | checkpoint | d71b9c5 | tests, schema copies, docs, log entry; PR 541 opened, all 30 check runs green |
| 2026-11-13T05:48Z | claude-r22a7c | finish | pending | handoff set to ready for review; the review step follows green CI |
