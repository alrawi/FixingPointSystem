# Handoff CTL-18 Compiler port pinning, host share rates, seeds, and module settings

| Field | Value |
|---|---|
| Issue | #135 |
| Branch | `wp/CTL-18` |
| Pull request | none yet |
| Status | in progress |
| Progress | 0% (0 of 4 plan steps) |
| Current holder | claude-dd3be7, lease until 2026-11-06T16:50:37Z |
| Last verified commit | none |

## Goal

Apply decision D12 items 7, 8, 9, and 22 in the test compiler `controller/internal/compiler/`. Pinned port locations compile to one agent, rates follow host shares, a configuration without a seed gets a random nonzero seed, and module settings are validated against the manifest settings schema and carried in the plan.

## Plan

- [ ] 1. Port pinning and per agent ports and pool slices.
- [ ] 2. Rates that follow host shares.
- [ ] 3. Random nonzero seed and module settings.
- [ ] 4. Golden plans, tests, and the Controller wiki page.

## Done and verified

Nothing yet.

## Remaining

Everything.

## Next action

Implement step 1 in `compile.go` and `split.go`.

## Blockers and open questions

none

## Decisions taken in this package

None yet.

## Pitfalls

The Go module proxy does not resolve in the worker sandbox, so the package cannot build locally. CI is the build and test loop.

## Verify the current state

```bash
make lint-ctl test-ctl
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T15:20Z | claude-dd3be7 | claim | none | Claimed #135 |
