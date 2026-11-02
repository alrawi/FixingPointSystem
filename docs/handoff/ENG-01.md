# Handoff ENG-01 Engine crate skeleton

| Field | Value |
|---|---|
| Issue | #21 |
| Branch | `wp/ENG-01` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 6 plan steps) |
| Current holder | claude-eng01-1906, lease until 2026-11-02T21:34:31Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Create the engine crates `fps-io`, `fps-flow`, `fps-interp`, and `fps-stats` under `engine/`. Move the session I/O seam and the program model and template parser of the S1 spike into `fps-io` and `fps-flow` with their tests. The crates `fps-interp` and `fps-stats` start empty with the lint header of the coding standards.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim, branch, handoff, and pull request
- [ ] 2. `fps-io` with the `SessionIo` trait, abort, and the in memory duplex, with tests
- [ ] 3. `fps-flow` with the program model and template parser ported from `spikes/s1-engine`, with tests
- [ ] 4. Empty documented crates `fps-interp` and `fps-stats`
- [ ] 5. `test-eng` Makefile block, workspace check, clippy, fmt, and cargo deny
- [ ] 6. Wiki ingest, Log entry, final checks, and review

## Done and verified

None yet.

## Remaining

Every plan step.

## Next action

Create `engine/fps-io` with the `SessionIo` trait.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-io -p fps-flow
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T20:04:31Z | claude-eng01-1906 | claim | | Card 6587196943 |
