# Handoff CTL-28 Verify module signatures at install and in plans

| Field | Value |
|---|---|
| Issue | #262 |
| Branch | `wp/CTL-28` |
| Pull request | not yet opened |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-c28a91, lease until 2026-11-09T05:22:59Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`Registry.Install` verifies the archive signature against a configured trust store before it unpacks, refuses any failure, accepts an unsigned module only with an explicit development setting, and stores the signature and key id with the module. The compiler fills the module signature and key id of the plan from the stored signature.

## Plan

- [ ] 1. Registry trust policy, signature options, and verification in Install and InstallDir
- [ ] 2. Store the signature in the index entry; HTTP headers and error mapping
- [ ] 3. Compiler: ModuleRef and PlanModule carry the signature
- [ ] 4. Settings in `controller/internal/app` config, the wiring that uses them, and tests
- [ ] 5. Wiki (Controller page, log entry), make targets, PR

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Write the trust policy in `controller/internal/registry/trust.go`.

## Blockers and open questions

none

## Decisions taken in this package

To be filled as the work proceeds.

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
