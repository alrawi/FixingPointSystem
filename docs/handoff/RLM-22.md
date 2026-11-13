# Handoff RLM-22 Request cadence and bytes for HLS in the realism labels

| Field | Value |
|---|---|
| Issue | #536 |
| Branch | `wp/RLM-22` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-r22a7c, lease until 2026-11-13T06:39:28Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Let the `temporal.packets` labels of the realism runner express what `modules/hls/fixtures/hls_check.py` checks: a packet gap target that selects the requests of one kind by HTTP request path, a minimum sample count per target, and a byte property per rung. A MOD follow up then moves `hls_check.py` into labels. This package does not touch `modules/hls/`; its tests prove the labels against the committed hls fixture.

## Plan

- [ ] 1. Read the labels parser, the temporal check, the report schema, and the hls fixture; decide the label syntax.
- [ ] 2. Labels parser: `path` selector, `min_samples` per target, byte target.
- [ ] 3. Temporal check: request streams, per target minimum, byte shares and bit rate.
- [ ] 4. Report schema (0.20.0), controller schema copy, ui schema.d.ts, tests against the hls fixture.
- [ ] 5. Docs, log entry with Shared page notes, checks, PR, CI.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Read `realism/labels.py` around `_temporal` and decide the syntax.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
python3 -m pytest realism/tests/test_temporal.py realism/tests/test_packet_gaps.py -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-13T05:09Z | claude-r22a7c | claim | none | claimed #536 with card 6844739470 |
