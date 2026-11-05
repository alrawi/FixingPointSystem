# Handoff RLM-11 Pinned Zeek and visible parser skips

| Field | Value |
|---|---|
| Issue | #140 |
| Branch | `wp/RLM-11` |
| Pull request | #150 |
| Status | ready for review |
| Progress | 100% (4 of 4 plan steps) |
| Current holder | claude-7e41b9, lease until 2026-11-05T20:20:37Z |
| Last verified commit | f274688 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Apply decision D12 items 5 and 23. CI installs one pinned Zeek version from the Zeek repository on the openSUSE Build Service, set in one place for both workflows, and `--parsers auto` stays but the realism report and its text summary list every parser that it skipped.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. One pinned Zeek version file, read by `realism/ci/install-tools.sh`, which both workflows call
- [x] 2. `skipped_parsers` in the report, the schema, the CLI message, and the CI summary rows, with tests
- [x] 3. Realism wiki page and log entry
- [x] 4. Pull request and green CI

## Done and verified

- CI on f274688: rlm, realism, mod-dns, mod-http1, ci, ops, ctl all green. The install step installed `zeek-core 9.0.1-0` and reported `zeek version 9.0.1`. Two realism jobs hung in the apt install step on the first attempt and passed after a rerun. Steps 1 to 3: `make lint-rlm test-rlm PYTHON=python3` passes with 140 passed and 9 skipped (the tools are not installed here). `plan.py check`, `render --check`, and `lint` pass. The pinned `zeek-core_9.0.1-0_amd64.deb` is the only Zeek package in `download.opensuse.org/repositories/security:/zeek/xUbuntu_24.04/amd64/`, read on 2026-11-05 through WebFetch.

List each finished step with the command that verified it and the result.

## Remaining

none, CI is green.

List what is still missing, including known gaps in finished steps.

## Next action

Coordinator reviews and merges pull request #150.

Write exactly one concrete step that the next agent can start immediately.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- The pin is the package version `9.0.1-0` in `realism/ci/zeek.version`, because apt needs the full package version.
- `skipped_parsers` is additive. `missing_parsers` stays so that 0.2.0 readers keep working.
- `--parsers none` adds no list, because it skips the whole check by request.

Record each local design choice with its reason. Choices that change a contract belong in a contract change issue.

## Pitfalls

The openSUSE repository `security:zeek` holds only the current Zeek release, so a new release breaks the pinned install until `zeek.version` is edited.

Record anything that cost time and that a successor should avoid.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-05T19:50Z | claude-7e41b9 | claim | | Claimed #140 with 4 plan steps |
| 2026-11-05T20:05Z | claude-7e41b9 | checkpoint | | Steps 1 to 3 done, opening the pull request |
| 2026-11-05T19:30Z | claude-7e41b9 | finish | f274688 | CI green, ready for review |
