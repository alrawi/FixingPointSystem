# Handoff CTL-28 Verify module signatures at install and in plans

| Field | Value |
|---|---|
| Issue | #262 |
| Branch | `wp/CTL-28` |
| Pull request | not yet opened |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-c28a91, lease until 2026-11-09T05:22:59Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`Registry.Install` verifies the archive signature against a configured trust store before it unpacks, refuses any failure, accepts an unsigned module only with an explicit development setting, and stores the signature and key id with the module. The compiler fills the module signature and key id of the plan from the stored signature.

## Plan

- [x] 1. Registry trust policy, signature options, and verification in Install and InstallDir
- [x] 2. Store the signature in the index entry; HTTP headers and error mapping
- [x] 3. Compiler: ModuleRef and PlanModule carry the signature
- [x] 4. Settings in `controller/internal/app` config, the wiring that uses them, and tests
- [ ] 5. Wiki (Controller page, log entry), make targets, PR

## Done and verified

- `gofmt -l` on registry, compiler, app, and wiring is clean.
- `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass.
- Go tests have not run locally (modules cannot be downloaded); the PR CI is the build and test loop.

## Remaining

Open the PR, wait for CI, fix failures.

## Next action

Open the PR and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

- No trust store means signatures are not required, and the registry logs a warning. A signature supplied without a trust store is ignored and not stored, because a stored signature means verified.
- The development setting `allow_unsigned_modules` only matters with a trust store. A bad signature is refused even with it.
- The trust store file is re-read at every install, so a revocation applies without a restart. A broken file fails the install closed.
- The signature travels in the headers `X-Module-Signature` and `X-Module-Signature-Key-Id` (the contract does not name them yet) and as `registry.WithSignature` in Go.
- The archive is spooled to a staging file so the verified bytes are the unpacked bytes.
- `compiler.ModuleRef.Signature` is a base64 string so the struct stays comparable; the plan JSON then matches proto3 JSON for bytes.
- Edits outside the catalog paths: `app/config.go` (two keys), `wiring/controller.go` and `wiring/adapters.go` (one line each). Without them the settings and the plan fields would be inert.

## Pitfalls

Go modules cannot be downloaded in the sandbox, so only `gofmt` runs locally and CI is the build loop.

## Verify the current state

```bash
cd controller && gofmt -l internal/registry internal/compiler internal/app internal/wiring
cd controller && go test ./internal/registry/... ./internal/compiler/... ./internal/app/... ./internal/wiring/...
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08T23:55Z | claude-c28a91 | claim | none | claimed issue 262 |
