# Handoff HST-01 Host pool allocator

| Field | Value |
|---|---|
| Issue | #34 |
| Branch | `wp/HST-01` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-hst-01-477c, lease until 2026-11-03T04:14:00Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We map sessions to source hosts deterministically and split host pools across agents and workers. The crate `engine/fps-hosts` offers round robin, uniform, and Zipf host selection from the run seed, and slicing of a pool by agent and by worker without overlap for IPv4 and IPv6 prefixes.

## Plan

- [ ] 1. Crate skeleton, pool model, and host index to address mapping
- [ ] 2. Slicing by agent and by worker with property tests
- [ ] 3. Round robin, uniform, and Zipf selection with tests
- [ ] 4. Hosts wiki page, log entry, and full checks

## Done and verified

Nothing yet.

## Remaining

Every plan step.

## Next action

Create the crate skeleton under engine/fps-hosts.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

none yet

## Verify the current state

```bash
cargo test -p fps-hosts
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-hst-01-477c | claim | none | Card 6594101938 |
