# Handoff CTL-27 Mutual TLS in the controller

| Field | Value |
|---|---|
| Issue | #251 |
| Branch | `wp/CTL-27` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-ctl27-a1, lease until 2026-11-09T04:08:27Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The controller opens the CTL-15 authority from its data directory, serves agent gRPC with mutual TLS, serves enrollment and renewal on a second HTTPS listener, ends the session of a revoked or expired identity, and gives operators token and revoke commands without a second writer of the PKI state.

## Plan

- [ ] 1. Settings in internal/app, session end and expiry in internal/agents
- [ ] 2. Wiring of the authority, the TLS gRPC server, the enrollment listener, and the admin socket
- [ ] 3. Binary: flags, the token and revoke subcommands, existing tests moved to the explicit plain flag
- [ ] 4. Mutual TLS tests (wiring and end to end with a Go fake agent)
- [ ] 5. Docs, make targets, log entry, pull request, CI

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Step 1.

## Blockers and open questions

none

## Decisions taken in this package

See the Controller page, section Mutual TLS.

## Pitfalls

Go cannot build in this sandbox (no module proxy), so CI is the build loop.

## Verify the current state

```bash
make lint-ctl test-ctl
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08 | claude-ctl27-a1 | claim | | claimed issue 251 |
