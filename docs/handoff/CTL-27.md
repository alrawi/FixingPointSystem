# Handoff CTL-27 Mutual TLS in the controller

| Field | Value |
|---|---|
| Issue | #251 |
| Branch | `wp/CTL-27` |
| Pull request | #257 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-ctl27-a1, lease until 2026-11-09T04:08:27Z |
| Last verified commit | 1965c1f |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The controller opens the CTL-15 authority from its data directory, serves agent gRPC with mutual TLS, serves enrollment and renewal on a second HTTPS listener, ends the session of a revoked or expired identity, and gives operators token and revoke commands without a second writer of the PKI state.

## Plan

- [x] 1. Settings in internal/app, session end and expiry in internal/agents
- [x] 2. Wiring of the authority, the TLS gRPC server, the enrollment listener, and the admin socket
- [x] 3. Binary: flags, the token and revoke subcommands, existing tests and the two Rust agent scripts moved to the explicit plain flag
- [x] 4. Mutual TLS tests (wiring and end to end with a Go fake agent)
- [x] 5. Docs, make targets, log entry, CI green, review

## Done and verified

- Steps 1 to 4 pushed in 8f31914. The `ctl` workflow (gofmt, go vet, golangci-lint, go test -race) and the agt and qa-gate1 workflows passed on that commit.

## Remaining

Nothing. The ci, ctl, agt, and qa-gate1 workflows passed on 1965c1f.

## Next action

Coordinator review of pull request 257.

## Blockers and open questions

none

## Decisions taken in this package

- Operators use the admin unix socket and two subcommands (`token`, `revoke`) because the OpenAPI has no operation for them and a command that opens the state itself would be a second writer that cannot end the open session. The socket check also refuses a second controller on the same PKI directory.
- Minting a token enrolls the identity in the inventory.
- `agent_auto_enroll` means that a valid certificate for an identity outside the inventory is admitted. It never admits an agent without a certificate.
- The five settings live in `controller/internal/app/config.go`, outside the catalog paths, because the YAML keys live in `app.Config` and an unknown key is an error. The two scripts that start the controller with the real Rust agent set `insecure_plain_grpc: true`.
- The controller of the image now requires mutual TLS, so the image needs a follow up; see the shared page notes of the log entry.

## Pitfalls

Go cannot build in this sandbox (no module proxy), so CI is the build loop. Tests that move the clock set `Options.HeartbeatInterval` and `HeartbeatTimeout` to hours, so the background sweep never races the test.

## Verify the current state

```bash
make lint-ctl test-ctl
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08 | claude-ctl27-a1 | claim | | claimed issue 251 |
| 2026-11-09 | claude-ctl27-a1 | checkpoint | 8f31914 | steps 1 to 4, ctl workflow green, PR 257 opened |
| 2026-11-09 | claude-ctl27-a1 | finish | 1965c1f | CI green, ready for review |
