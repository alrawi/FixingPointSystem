# Handoff RLM-05 Content variety check

| Field | Value |
|---|---|
| Issue | #187 |
| Branch | `wp/RLM-05` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-d41386, lease until 2026-11-09T21:24:01Z |
| Last verified commit | 5d888cc |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the content variety check to the realism report. It reassembles application payloads per session direction (per message for HTTP/1 and per datagram for UDP), computes the duplicate payload ratio over hashes of normalized payloads and the payload size distribution, and fails when an application exceeds a configurable duplicate threshold that defaults to 1%.

## Plan

- [x] 1. Segment decoding and reassembly, normalization, hashing (`realism/content.py`, `Segment` in `realism/capture.py`)
- [x] 2. Labels `content` section, report schema 0.5.0, `--max-duplicate-ratio`
- [x] 3. Tests with generated fixtures (`realism/tests/test_content.py`)
- [ ] 4. README, wiki page, log entry, pull request, CI

## Done and verified

1 to 3: `make lint-rlm test-rlm` passes locally (252 passed, 6 skipped for Zeek and ndpiReader, which are not installed here).

## Remaining

Step 4.

## Next action

Update `realism/README.md` and `docs/wiki/Realism.md`, add the log entry, and open the pull request.

## Blockers and open questions

none

## Decisions taken in this package

- The check is opt in: it runs when the labels have a `content` section or `--max-duplicate-ratio` is given, so no existing labels file or workflow changes behavior.
- The realism package reads payloads itself (`decode_segment` in `capture.py`) and does not import `capture/`, as RLM-04 and CAP-01 decided.
- Units are HTTP/1 messages, UDP datagrams, or one whole TCP direction. The digest covers the normalized start line, headers, and decoded body, so identical repeated requests count as duplicates. Volatile headers (Date, ETag, cookies, trace ids) are dropped, and the labels add more with `content.ignore_headers`. DNS ignores the transaction id.
- Duplicates are counted per application and direction, and each application must stay at or below the threshold, so a varied application cannot hide a repeated one.
- Encrypted sessions are skipped and counted, because RLM-04 reads only Finished messages from tshark and exposes no plaintext.
- The size comparison with the labels uses the median and optionally the coefficient of variation, each within a relative tolerance (default 25%).

## Pitfalls

The sandbox guard refuses a command that mixes `git` with a heredoc, so write scripts to the scratchpad with the editor and run them in a separate call.

## Verify the current state

```bash
make lint-rlm test-rlm
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T20:54Z | claude-d41386 | claim | 6ae7965 | Claimed issue #187 |
| 2026-11-09T21:00Z | claude-d41386 | checkpoint | 5d888cc | Check, labels, schema, and tests done |
