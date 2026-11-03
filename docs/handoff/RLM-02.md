# Handoff RLM-02 Parser error checks with Zeek and tshark

| Field | Value |
|---|---|
| Issue | #52 |
| Branch | `wp/RLM-02` |
| Pull request | #82 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-rlm-02-f66d, lease until 2026-11-03T04:13:46Z |
| Last verified commit | b6f6031 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Count parser errors and malformed packets per application with tshark and Zeek, each run as a separate process, and add the counts to the realism report as a new check. A capture with one malformed DNS message reports exactly one parser error.

## Plan

- [x] 1. Parser runner interface with tshark and Zeek runners and stub runners
- [x] 2. Parser error check that attributes errors to sessions and applications, with its report schema entry
- [x] 3. Command integration and unit tests, including the malformed DNS acceptance test
- [x] 4. Optional real tool tests, make targets, and the rlm workflow
- [x] 5. README, component page, log entry, and pull request

## Done and verified

1. `realism/parsers.py` holds `ParserRunner`, `StubParser`, `TsharkRunner`, `ZeekRunner`, and the output parsers. Verified with `python3 -m pytest -q realism/tests/test_parsers.py`, which uses fake programs.
2. `realism/parser_errors.py` holds the check, and `realism/schema/realism-report.schema.json` gains `$defs.parser_errors` with its `allOf` entry. The report format is now 0.2.0.
3. `python3 -m realism score` takes `--parsers`, `--tshark`, and `--zeek`. `realism/tests/test_parser_errors.py` validates every report entry against the schema and checks that one malformed DNS message reports one parser error.
4. `realism/tests/test_parser_tools.py` runs the real tshark 4.2.2 locally and passes. Zeek was not installed locally, so its tests skipped here and run in the `rlm` workflow.
5. `make lint-rlm test-rlm` passes with 104 tests and 3 skips. `plan.py check`, `render --check`, and `lint` pass.

## Remaining

The metric catalog has no parser error metric, so the check leaves `metrics` empty. Issue #89 tracks the catalog entry.

## Next action

Review pull request #82.

## Blockers and open questions

none

## Decisions taken in this package

- The application count is the largest count of any one parser, so two parsers that see one fault count it once.
- tshark counts frames with expert information of error severity. Zeek counts weirds per connection and packet time, and leaves out the state weirds in `ZEEK_STATE_WEIRDS`.
- Zeek runs with `-C`, because checksum offload makes bad checksums in captures from the sending host.
- `--parsers auto` is the default, and the check fails when no parser ran. Explicit lists make a missing tool an input error with exit code 2.
- The workflow installs Zeek from the package repository of the Zeek project, because Ubuntu 24.04 has no Zeek package.

## Pitfalls

- tshark prints expert severity and group as numbers with `-T fields`, so the parser compares with the constants of `epan/proto.h`.
- RLM-01 used `parser_errors` as the example of an unknown check kind in `test_conformance.py`. We renamed that example to `distribution`.

## Verify the current state

```bash
make lint-rlm test-rlm
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-rlm-02-f66d | claim | none | Session start, branch created from origin/main |
| 2026-11-03T03:00Z | claude-rlm-02-f66d | finish | see branch head | All plan steps done, CI passed with real tshark and Zeek |
