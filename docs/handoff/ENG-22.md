# Handoff ENG-22 Refuse tls_upgrade on UDP in the flow loader

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #340 |
| Branch | `wp/ENG-22` |
| Pull request | #341 |
| Status | ready for review |
| Progress | 100% (3 of 3 plan steps) |
| Current holder | claude-e22a7c, lease until 2026-11-10T00:39:09Z |
| Last verified commit | 9bf7a21 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`validate.py` refuses `tls_upgrade` in a UDP flow since CON-17, but `check_action` of `fps-flow` accepts it. Add `LoadError::TlsUpgradeOnUdp`, raised at any loop and branch depth, plus verdicts.yaml cases, so both validators agree.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Add the LoadError kind and the check
- [x] 2. Add fixtures (flat and nested) and verdicts; run Rust and Python parity tests
- [x] 3. Update docs/wiki/Engine.md and the log entry; open the PR

## Done and verified

- `cargo test -p fps-flow` and `cargo clippy -p fps-flow --all-targets -D warnings`, `cargo fmt --check`: pass.
- `python3 -m pytest -q engine/fps-flow/tests`: 49 passed.
- plan.py check, render --check, lint: ok.

## Remaining

Nothing.

## Next action

Coordinator reviews and merges PR #341.

## Blockers and open questions

none

## Decisions taken in this package

The check is a match arm in `check_action`, so every action that `walk` visits is covered, as in validate.py. The nested fixture needs two branch arms because the schema requires at least two.

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
| 2026-11-09T00:00Z | claude-e22a7c | finish | 9bf7a21 | PR #341 opened, CI green, ready for review |
