# Handoff OPS-01 Repository scaffolding for Phase 1

| Field | Value |
|---|---|
| Issue | #10 |
| Branch | `wp/OPS-01` |
| Pull request | pending |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-ops01-8248, lease until 2026-11-02T18:57:20Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We prepare the shared root files so that later packages add crates, Go modules, and CI jobs without editing the same lines. The deliverables are a root `Cargo.toml` with glob members, a root `Makefile` with `lint`, `test`, and `test-<workstream>` targets, a `go.work` file, a `CODEOWNERS` file, and a documented pattern of one CI workflow per workstream.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [ ] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [ ] 2. Switch the root `Cargo.toml` to glob members and prove that a new empty crate under `engine/` builds
- [ ] 3. Add the root `Makefile` with `lint`, `test`, and `test-<workstream>` targets that delegate to the existing commands
- [ ] 4. Add `go.work`, `CODEOWNERS`, and the documented CI workflow pattern
- [ ] 5. Ingest the result into the wiki, pass `make lint test` and CI, and finish

## Done and verified

Nothing yet.

## Remaining

Every plan step.

## Next action

Edit the root `Cargo.toml` so that `members` uses the globs `spikes/*`, `engine/*`, and `modules/*/native`.

## Blockers and open questions

None.

## Decisions taken in this package

None yet.

## Pitfalls

None yet.

## Verify the current state

```bash
make lint test   # after step 3; expected to pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:27:20Z | claude-ops01-8248 | claim | 096ecd5 | Claimed from `status:ready`, no earlier cards |
