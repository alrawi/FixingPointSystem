# Handoff ENG-04 Flow program interpreter

| Field | Value |
|---|---|
| Issue | #24 |
| Branch | `wp/ENG-04` |
| Pull request | to be opened |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-eng-04-6d65, lease until 2026-11-03T13:08:31Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Execute compiled flow programs of `fps-flow` over any `fps_io::SessionIo` for TCP sessions, with timeouts, the three expect modes, and seeded delay distributions. Per session generators derive from the run seed and the session id, and every failure maps to a `reason` value of `contracts/metrics/catalog.yaml`.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Crate skeleton with dependencies, `session_rng`, and the `Reason` and `Failure` types
- [ ] 2. Session runner with send, the three expect modes, timeouts, and `last`
- [ ] 3. Delays of the five distributions, set, loop, branch, and the three close modes
- [ ] 4. Client and server tests over the duplex, one test per reachable failure reason
- [ ] 5. Engine page section, log entry, handoff, and pull request

## Done and verified

None yet.

## Remaining

Everything in the plan.

## Next action

Add the dependencies to `engine/fps-interp/Cargo.toml` and write `session_rng` with its collision test.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
cargo test -p fps-interp
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:40:17Z | claude-eng-04-6d65 | claim | none | Claimed with card 6602808216 |
