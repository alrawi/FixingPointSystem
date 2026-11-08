# Handoff CTL-15 Mutual TLS between controller and agents

| Field | Value |
|---|---|
| Issue | #167 |
| Branch | `wp/CTL-15` |
| Pull request | none yet |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-c15a7e, lease until 2026-11-08T22:22:13Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Add the package `controller/internal/pki/` with a controller CA, single use enrollment tokens, short lived agent certificates from a CSR, rotation, revocation, and a `tls.Config` builder for the gRPC server that requires and verifies client certificates. An agent without a valid certificate cannot register. The package is not wired into the gRPC server or the agent.

## Plan

- [x] 1. CA, state file, tokens, issuing, renewal, revocation
- [x] 2. TLS configs and the HTTP enrollment handler
- [x] 3. Tests over real TLS on loopback
- [x] 4. `mk/ctl.mk` targets, Controller wiki page, log entry
- [ ] 5. Pull request, CI, review

## Done and verified

- Steps 1 to 3: `cd controller && GOWORK=off GOPROXY=off go test -race -cover ./internal/pki/` passes. The package uses only the standard library, so it builds without the module proxy.

## Remaining

Step 5 (pull request and CI).

## Next action

Open the pull request, wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

- The authority keeps an allow list of issued serial numbers per identity. Revocation deletes them, and a certificate that the CA signed but the state does not know fails closed.
- The token has the form `id.secret`. The state holds the SHA-256 digest of the secret, and the lookup by id is followed by a constant time digest comparison.
- Enrollment needs a listener without client authentication, so the package serves `POST /enroll` and `POST /renew` through `Handler` on a second listener, and no contract change is needed for the first wiring. See the pull request for the alternative of Enroll and Renew RPCs.

## Pitfalls

- TLS 1.3 reports a rejected client certificate on the first read of the client, not at the dial. Tests exchange one byte.

## Verify the current state

```bash
make test-ctl-pki lint-ctl-pki
```

## Session log

| Time (UTC) | Agent | Entry |
|---|---|---|
| 2026-11-08 | claude-c15a7e | Claimed the package, wrote the package and its tests |
