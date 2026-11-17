# Handoff TRN-10 User space TCP stack decision

| Field | Value |
|---|---|
| Issue | #561 |
| Branch | `wp/TRN-10` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-9d41e7, lease until 2026-11-18T01:16:47Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Propose decision record 0014 for the TCP stack of the AF_XDP backend (TRN-12): compare smoltcp, the own stack of TRN-18, and F-Stack (paper only), with the measurements of TRN-17, TRN-18, and TRN-20, a license review, and the follow-up packages for TRN-12. The record must not alter D1 or a tier target.

## Plan

- [ ] 1. Re-run the missing rows in one session in the sandbox and commit the results under `engine/fps-tcp/results/`.
- [ ] 2. Write `docs/decisions/0014-user-space-tcp-stack.md` and add it to the decisions index.
- [ ] 3. Write the log entry and run the checks.
- [ ] 4. Open the pull request, wait for CI, and move to review.

## Done and verified

Nothing yet.

## Remaining

All steps.

## Next action

Run the same session benchmark of the three stacks.
