# Handoff TRN-01 Kernel TCP backend

| Field | Value |
|---|---|
| Issue | #31 |
| Branch | `wp/TRN-01` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-trn-01-0b1a, lease until 2026-11-03T04:13:37Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Implement `SessionIo` over kernel TCP sockets in the new crate `engine/fps-backend-kernel`. Clients connect from host pool source addresses with `IP_BIND_ADDRESS_NO_PORT` and a configured local port range, servers accept on pool addresses with a configurable backlog, and both apply buffer size, Nagle, and linger options. A loopback test opens 10,000 sessions from 100 source addresses in 127.0.0.0/8 without address errors.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim, branch, handoff, and pull request
- [ ] 2. Crate skeleton with options, errors, and the socket option helper
- [ ] 3. Connector, listener, and `KernelStream` with abort, with unit tests
- [ ] 4. Loopback component tests, including the 10,000 session acceptance test
- [ ] 5. Transport wiki page, log entry, `make lint test`, CI, and review

## Done and verified

Step 1 needs no verification.

## Remaining

Steps 2 to 5.

## Next action

Create `engine/fps-backend-kernel/Cargo.toml` and `src/lib.rs`.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
git log --oneline -1
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T02:44Z | claude-trn-01-0b1a | claim | | card 6594096558 |
