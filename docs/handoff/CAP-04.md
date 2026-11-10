# Handoff CAP-04 Profile extractor

| Field | Value |
|---|---|
| Issue | #196 |
| Branch | `wp/CAP-04` |
| Pull request | #381 |
| Status | in progress |
| Progress | 90% (3 of 4 plan steps) |
| Current holder | claude-211a73, lease until 2026-11-10T14:16:51Z |
| Last verified commit | none |

## Goal

Derive an application mix and temporal models from a capture and write a test profile. Deliverable: mix shares, session arrival models, and object size models fitted per application. Acceptance: a profile extracted from a generated run reproduces its configured mix within 2 points.

## Plan

- [x] 1. Read the sessions document, scrub record and fixtures; design the profile format
- [x] 2. Implement classification, mix shares, arrival and size models
- [x] 3. Add the `profile` command, tests with literal values from committed fixtures
- [ ] 4. Docs, log entry, checks, PR

## Done and verified

Steps 1 to 3: `capture/profile.py`, the `profile` command in `capture/cli.py`, and `capture/tests/test_profile.py` (28 tests). `ruff format --check`, `ruff check`, `mypy` and `pytest -q capture` pass locally (167 passed).

## Remaining

Capture wiki page, README, log entry, PR, CI.

## Next action

Update `docs/wiki/Capture.md` and `capture/README.md`, add the log entry, run the plan checks, open the PR.

## Blockers and open questions

none

## Decisions taken in this package

- The mix names applications by server port (table plus `--app` rules) because the compiler reads no protocol. Unnamed sessions stay out of the mix and are reported, as in the realism labels (FTP data connections).
- Models: exponential, uniform, lognormal, pareto, constant; the smallest KS distance wins. Lognormal arrivals are written in seconds with `unit: s`.
- TCP size unit is a whole direction of a session, UDP is a datagram, as the contract allows.
- UDP sessions that continue after an idle split add bytes but are not new sessions.

## Pitfalls

none

## Verify the current state

```bash
python3 -m pytest capture/tests -q
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-10T12:47Z | claude-211a73 | claim | none | Claimed #196 with card 6767705449 |
| 2026-11-10T13:20Z | claude-211a73 | checkpoint | see git log | Steps 1 to 3 done |
| 2026-11-10T13:40Z | claude-211a73 | checkpoint | 2f4a8c8 | Docs, log entry, PR #381 opened; waiting for CI |
