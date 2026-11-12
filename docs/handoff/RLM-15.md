# Handoff RLM-15 Packet inter arrival times inside a session

| Field | Value |
|---|---|
| Issue | #425 |
| Branch | `wp/RLM-15` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-a3e893, lease until 2026-11-12T04:27:19Z |
| Last verified commit | none |

## Goal

The temporal check gets a per application property for the gaps between the packets of one session, compared with a distribution of the labels, with the constant distribution for paced media. The sip_rtp fixture passes with 20 ms and 30 ms packet times, and a capture with jittered packets fails with a named reason.

## Plan

- [x] 1. Labels: a `packets` section of `temporal` (parser, dataclasses, labels schema)
- [x] 2. The check: per session gaps, the paced constant test, report entry, schema, report format bump
- [x] 3. Tests: the sip_rtp fixture at 20 ms and 30 ms, jitter, labels, report schema
- [ ] 4. README, Realism page, log entry, run the checks, open the pull request

## Done and verified

1. Labels: `temporal.packets` in `realism/labels.py` and `labels.schema.json`. `python3 -m pytest realism/tests/test_packet_gaps.py` covers the parser and the schema (36 passed).
2. Check: `packet_gap` in `realism/temporal.py`, schema and report format 0.15.0, controller schema copy refreshed with `cp`.
3. Tests: `realism/tests/test_packet_gaps.py`, including the committed sip_rtp fixture (passes with `ms: [20, 30]`, fails with either time alone) and jittered, bursty, and wrong packet time captures. `ruff format --check`, `ruff check`, and `mypy` of `make lint-rlm` pass.

## Remaining

Full `pytest realism`, the pull request, and CI.

## Next action

Open the pull request and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

1. The property is `packet_gap`, the labels section `temporal.packets`. A stream is one direction of a session, and only packets with payload count, so lock step RTP directions and TCP acknowledgments do not break the gaps.
2. The constant of `packets` is paced: `ms` takes a list, `tolerance_ms` is required, and `within` (default 0.9) is the share of on time gaps per stream. The committed fixture has gaps from 0.09 ms to 63.7 ms in a 20 ms stream, so the exact constant of `arrivals` cannot pass a real capture.
3. Each stream with at least `min_samples` gaps is judged alone, so one jittered call cannot hide among good ones.
4. Packet gaps carry no `realism.ks_*` series, because the catalog 0.6.0 has no property label for them. This needs a CON follow up (see the log entry).
5. The controller embeds a copy of the report schema, which a Go test compares, and `ui/src/api/schema.d.ts` is generated from the OpenAPI file that references it (ui.yml checks it). We refreshed the copy with `cp` and regenerated the types with openapi-typescript 7.13.0, so those two are the only files outside `realism/` and the wiki.
6. The fields that 0.15.0 added (`target.values`, `streams`) are optional in the schema, so the controller still takes reports of older formats. Report format 0.15.0. CON-27 also bumps it, so the second merge renumbers.

## Pitfalls

The controller embeds a byte copy of the report schema (`controller/internal/report/schema/realism-report.schema.json`), and a Go test fails when it differs. Any schema change needs the copy refreshed.

## Verify the current state

```bash
python3 -m pytest realism/tests/test_temporal.py -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T03:57Z | claude-a3e893 | claim | none | Claimed #425, branch from origin/main f3ffedc |
| 2026-11-12T04:10Z | claude-a3e893 | checkpoint | see git log | Labels, check, schema, tests, docs, log entry |
