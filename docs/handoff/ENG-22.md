# Handoff ENG-22 Refuse tls_upgrade on UDP in the flow loader

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #340 |
| Branch | `wp/ENG-22` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-e22a7c, lease until 2026-11-10T00:39:09Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`validate.py` refuses `tls_upgrade` in a UDP flow since CON-17, but `check_action` of `fps-flow` accepts it. Add `LoadError::TlsUpgradeOnUdp`, raised at any loop and branch depth, plus verdicts.yaml cases, so both validators agree.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Add the LoadError kind and the check
- [ ] 2. Add fixtures (flat and nested) and verdicts; run Rust and Python parity tests
- [ ] 3. Update docs/wiki/Engine.md and the log entry; open the PR

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Implement step 1 in engine/fps-flow/src/check.rs and error.rs.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
CARGO_PROFILE_DEV_DEBUG=0 cargo test -p fps-flow   # expect pass
python3 -m pytest -q engine/fps-flow/tests
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T00:00Z | claude-e22a7c | claim | origin/main | claimed #340 |
