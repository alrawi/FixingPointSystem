# Handoff CTL-31 CA in the enrollment chain and trust store flags

| Field | Value |
|---|---|
| Issue | #275 |
| Branch | `wp/CTL-31` |
| Pull request | none yet |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-c31a7e, lease until 2026-11-09T14:06:59Z (renewed at each checkpoint) |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Both controller TLS listeners send the CA certificate after the leaf, so an agent that pins the CA fingerprint can authenticate the enrollment endpoint before it sends its token. The trust store settings of CTL-28 get the flags `-trust-store-file` and `-allow-unsigned-modules`.

## Plan

- [x] 1. Send `[leaf, CA]` from `serverCertificate`, with a handshake test for both listeners.
- [x] 2. Add the two flags with a precedence test.
- [x] 3. Update `docs/wiki/Controller.md` and write the log entry.
- [ ] 4. Open the PR and get CI green, including `controller-e2e` of the Rust agent.
- [ ] 5. Hand off for review.

## Done and verified

Steps 1 to 3 are written. Go cannot build locally, so only `gofmt -l` ran. CI is the build and test loop.

## Remaining

CI results, in particular `engine/fps-agent/tests/controller_e2e.sh`.

## Next action

Open the pull request and poll the check runs.

## Blockers and open questions

none

## Decisions taken in this package

- The chain is built in `serverCertificate`, so the gRPC listener and the enrollment listener share it and `tls.Certificate.Leaf` stays the leaf.
- The flag test extends the layering pattern of `main_test.go` with a bool flag, using `-allow-unsigned-modules=false` to show a flag over a file.

## Pitfalls

Go modules cannot be downloaded in the sandbox, so nothing compiled locally.

## Verify the current state

```bash
cd controller && GOWORK=off go test ./internal/pki/ ./cmd/fps-controller/
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09 | claude-c31a7e | claim and implementation | none | Claimed #275 and implemented steps 1 to 3 |
