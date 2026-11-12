# Handoff RLM-15 Packet inter arrival times inside a session

| Field | Value |
|---|---|
| Issue | #425 |
| Branch | `wp/RLM-15` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-a3e893, lease until 2026-11-12T04:27:19Z |
| Last verified commit | none |

## Goal

The temporal check gets a per application property for the gaps between the packets of one session, compared with a distribution of the labels, with the constant distribution for paced media. The sip_rtp fixture passes with 20 ms and 30 ms packet times, and a capture with jittered packets fails with a named reason.

## Plan

- [ ] 1. Labels: a `packets` section of `temporal` (parser, dataclasses, labels schema)
- [ ] 2. The check: per session gaps, the paced constant test, report entry, schema, report format bump
- [ ] 3. Tests: the sip_rtp fixture at 20 ms and 30 ms, jitter, labels, report schema
- [ ] 4. README, Realism page, log entry, run the checks, open the pull request

## Done and verified

none yet

## Remaining

Everything in the plan.

## Next action

Add the `packets` section to `realism/labels.py` and `realism/schema/labels.schema.json`.

## Blockers and open questions

none

## Decisions taken in this package

none yet

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
