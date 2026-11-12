# Handoff MOD-24 Packet gap labels in place of the module timing scripts

| Field | Value |
|---|---|
| Issue | #493 |
| Branch | `wp/MOD-24` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-6afc7f, lease until 2026-11-12T21:01:58Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Replace `rtp_check.py` (sip_rtp) and `hls_check.py` (hls) with `temporal.packets` labels (RLM-15), and remove the http2 TLS port wrapper that RLM-18 made redundant. Update the workflows. Acceptance: the sip_rtp, hls and http2 module realism jobs pass, and a jittered sip_rtp capture fails through the labels.

## Plan

- [ ] 1. Add the labels, remove the sip_rtp script and the http2 wrapper, update capture scripts, mk files, workflows
- [ ] 2. Hls decision (see findings) and the README and test updates of the three modules
- [ ] 3. Module-Development.md, log entry, checks
- [ ] 4. Open the PR, wait for CI, review

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Step 1.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

- `hls_check.py` also checks bytes (byte share, bit rate, segment body size), which no label covers.

## Verify the current state

```bash
git log --oneline origin/wp/MOD-24
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T20:35 | claude-6afc7f | claim | none | claimed issue #493 |
