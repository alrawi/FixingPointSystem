# Handoff CTL-25 Scenario contract v0.3 in the controller

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #241 |
| Branch | `wp/CTL-25` |
| Pull request | not yet opened |
| Status | in progress |
| Progress | 80% (2 of 3 plan steps) |
| Current holder | claude-c25a7e |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

The controller's module registry validates scenarios with the v0.3 schema, so a module whose scenario declares profiles and calls dist installs, and an undeclared profile is rejected. Finding: the registry validated no scenario content before, only the `scenario` name, so this package adds the schema check and the profile and dist template rules.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Embedded v0.3 schema copy, `scenario.go` checks, registry integration, tests, make targets, wiki
- [ ] 2. CI green on the PR
- [ ] 3. Review handoff

## Done and verified

Step 1 written without a local build (module proxy is unreachable). `gofmt -l controller/internal/registry` is clean. CI is the build and test loop.

## Remaining

CI result. The error text of the JSON Schema library is not asserted in the tests, only the finding path.

## Next action

Open the PR and read the CI result of the controller workflow.

## Blockers and open questions

Write "none" or name each blocker with a link to its issue.

## Decisions taken in this package

- Scope is checks the controller can make without manifests: schema, `profiles.<name>`, `dist.*`. Module calls stay with the contract validator.
- `scenarios-pending/*.yaml` are checked like listed scenarios and not indexed, as `tools/contracts/validate.py` does.

## Pitfalls

No Go build here; keep edits small.

## Verify the current state

```bash
make test-ctl-registry   # expected: ok
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08T21:55Z | claude-c25a7e | claim | - | claimed #241 |
| 2026-11-08T22:15Z | claude-c25a7e | checkpoint | - | code, tests, wiki written |
