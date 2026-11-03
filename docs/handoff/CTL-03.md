# Handoff CTL-03 Configuration store and validation

| Field | Value |
|---|---|
| Issue | #41 |
| Branch | `wp/CTL-03` |
| Pull request | #97 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-ctl-03-78e4, lease until 2026-11-03T13:27:37Z |
| Last verified commit | f19ac36 |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We store test configurations in SQLite behind a repository interface and validate them against the JSON Schema, the installed module references, and the agent capacity. The package serves POST and GET /v1/configs and POST /v1/configs/{id}:validate.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Add the repository interface and its SQLite implementation
- [x] 3. Add schema validation, module reference checks, and capacity checks
- [x] 4. Add the handlers and their tests with every example and invalid fixtures
- [x] 5. Ingest the result into the wiki, pass the checks and CI, and finish

## Done and verified

Each step was verified with the command after it.

- Steps 2 to 4. `make lint-ctl test-ctl` passes with golangci-lint and reports 0 issues. `go test -race -cover ./internal/store/` reports 93.5%.
- The acceptance holds. `TestEveryExampleStoresAndValidates` posts every file under `contracts/examples/tests/` as YAML and as JSON, reads it back, and validates it without findings. `TestInvalidFixturesReturnAProblemNamingTheField` posts each fixture under `testdata/invalid/` and expects a 422 problem with an error at the pointer that the first line of the fixture names. `TestUnresolvedFixturesStoreAndFailValidation` does the same for the validate report with `testdata/unresolved/`.
- Step 5. `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass. The Controller page has a Configuration store section, and the log entry is under `docs/wiki/log/`.

## Remaining

The review. Known gaps follow.

- The controller command does not mount the REST layer yet, so nothing serves the handler outside tests. The log entry describes the wiring.
- The validator does not check `params` against the parameter schema of a module, and it does not check metric names in `criteria` against `contracts/metrics/catalog.yaml`.
- There is no list operation, because the contract has none.

## Next action

A reviewer agent reviews #97 against the Definition of Done.

## Blockers and open questions

none

## Decisions taken in this package

- The package embeds a copy of the schema, because `go:embed` cannot reach outside the module, and a test compares the copy with the contract byte for byte.
- Create rejects only schema errors. References to modules and agents can change after storage, so they belong to the validate report.
- The module catalog and the agent inventory use `compiler.Scenario` and `compiler.Agent`, so the run orchestrator can pass the same values to `compiler.Compile`.
- A missing or unknown property gets a finding at the property path, not at its parent, so the problem names the field.
- We pinned `modernc.org/sqlite` v1.46.1, the newest release that builds with Go 1.24.

## Pitfalls

- The disk filled up during a build because many agents share it. Clean `$TMPDIR` in the scratch folder and retry.
- yaml.v3 resolves an integer beyond 64 bits as a float, so such a value becomes a JSON float and not an error.

## Verify the current state

```bash
cd controller && GOWORK=off go test -race ./internal/store/
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T11:40Z | claude-ctl-03-78e4 | claim | none | Claimed with card 6602821433 |
| 2026-11-03T12:05Z | claude-ctl-03-78e4 | checkpoint | f19ac36 | Store, validation, and handlers with tests |
| 2026-11-03T12:10Z | claude-ctl-03-78e4 | finish | f19ac36 | Wiki ingested, ready for review |
