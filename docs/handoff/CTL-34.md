# Handoff CTL-34 Byte weights in the compiler

| Field | Value |
|---|---|
| Issue | #299 |
| Branch | `wp/CTL-34` |
| Pull request | not opened yet |
| Status | in progress |
| Progress | 75% (3 of 4 plan steps) |
| Current holder | claude-c34a7e, lease until 2026-11-09T17:41:45Z |
| Last verified commit | none |

## Goal

For `profile.basis: bytes` the compiler turns byte weights into session shares with each scenario's `expected_bytes` (share proportional to weight over bytes per session), `throughput_bps` becomes session rates from the byte weighted mean bytes per session, and `newByteMix` in `controller/internal/runs/bytemix.go` starts mixcontrol from the compiled session weights while keeping the byte weights as its target. The sessions basis stays byte identical.

## Plan

- [x] 1. Compiler conversion in `compile.go` (`bytesToSessions`), with `ErrNoByteEstimate` for a missing estimate; `newByteMix` rebuilds the byte weight as session weight times estimate
- [x] 2. Golden of `enterprise_mix_10g` (basis bytes) regenerated, unit tests in `bytes_basis_test.go`, run tests adjusted, new exact estimates test
- [x] 3. `docs/wiki/Controller.md` and the log entry
- [ ] 4. PR, CI green, review

## Done and verified

Go modules cannot be downloaded in this sandbox, so nothing compiles locally. `gofmt -l` is clean. The golden was computed by a script that repeats the Go arithmetic in the same order; CI is the check.

## Remaining

CI result, and the golden if its last digits differ.

## Next action

Open the PR and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- A missing or non positive `expected_bytes` fails the compilation with `ErrNoByteEstimate` for the bytes basis, whatever the objective. A fallback to session shares would silently run another mix than the test asks for, and mixcontrol needs the estimate anyway.
- `mix[].weight` of a plan is always the session share. The proto comment says "normalized weight within the basis"; the engine only passes the field through. See the log entry for the contract note.
- `newByteMix` needs no configuration: byte weight = session weight times estimate gives back the configured byte weights.

## Pitfalls

The existing run tests assumed that the compiled session ratio equals the byte weight ratio for the bytes basis. They now use the weight over estimate ratio.

## Verify the current state

```bash
make lint-ctl test-ctl-bytemix
cd controller && go test ./internal/compiler/ ./internal/runs/
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-09T16:15Z | claude-c34a7e | claim | none | claimed issue 299 |
