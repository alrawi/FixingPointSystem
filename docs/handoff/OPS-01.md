# Handoff OPS-01 Repository scaffolding for Phase 1

| Field | Value |
|---|---|
| Issue | #10 |
| Branch | `wp/OPS-01` |
| Pull request | #63 |
| Status | in progress |
| Progress | 60% (3 of 5 plan steps) |
| Current holder | claude-ops01-8248, lease until 2026-11-02T18:57:20Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

We prepare the shared root files so that later packages add crates, Go modules, and CI jobs without editing the same lines. The deliverables are a root `Cargo.toml` with glob members, a root `Makefile` with `lint`, `test`, and `test-<workstream>` targets, a `go.work` file, a `CODEOWNERS` file, and a documented pattern of one CI workflow per workstream.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Claim the package, create the branch and this handoff file, and open the pull request
- [x] 2. Switch the root `Cargo.toml` to glob members and prove that a new empty crate under `engine/` builds
- [x] 3. Add the root `Makefile` with `lint`, `test`, and `test-<workstream>` targets that delegate to the existing commands
- [ ] 4. Add `go.work`, `CODEOWNERS`, and the documented CI workflow pattern
- [ ] 5. Ingest the result into the wiki, pass `make lint test` and CI, and finish

## Done and verified

1. Claim, branch, handoff file, and pull request #63 with the `wip` label.
2. The root `Cargo.toml` uses a glob member. `make test-ops-workspace` builds a new empty crate `engine/fps-probe` in a temporary copy of the workspace root and passes. With the old explicit member list the same target fails with `package ID specification fps-probe did not match any packages`.
3. The root `Makefile` provides `lint`, `test`, `lint-rust`, `lint-shell`, `lint-deny`, `lint-ops`, `test-con`, `test-ops`, and `test-spk`. `make lint test` passes locally in about 40 seconds.

## Remaining

Step 4 is written but not yet committed with its documentation. Step 5 is open.

## Next action

Document the one workflow per workstream pattern in `docs/wiki/Testing.md` and add `.github/workflows/ops.yml` as its first instance.

## Blockers and open questions

None.

## Decisions taken in this package

- **Character class glob.** Cargo treats a member glob that matches nothing as a literal path and fails to load it. `engine/` does not exist yet, so the plain glob `engine/*` breaks every build today. We use `[es][np][gi][ik][ne][es]/*`, which matches `spikes/*` and `engine/*` position by position, so it always matches the spike crates and picks up engine crates as they appear. A later package may split it into `spikes/*` and `engine/*` once `engine/` holds a crate. The alternative, a placeholder file under `engine/`, lies outside the paths of OPS-01.
- **`modules/*/native` deferred.** No path of depth three named `native` exists, and no glob trick can match one, so this glob would break the build until the first native module crate exists. That package adds the glob on its own line. The owner may instead approve a placeholder outside the OPS-01 paths.
- **Non Rust spikes excluded.** Every directory that a glob matches must hold a `Cargo.toml`, so `spikes/s3-afxdp` and `spikes/s4-trex-astf` sit in `exclude`.
- **Makefile registration.** Each workstream block appends to `LINT_TARGETS` and `TEST_TARGETS` on its own lines, and the aggregate targets sit at the bottom because make expands prerequisites when it reads them.
- **Empty go.work.** Go accepts a `go.work` with only a `go` line. A module created later fails with "directory prefix . does not contain modules listed in go.work" until its package adds `use ./controller` or `use ./cli`, which is the intended single line edit.
- **`lint-deny` and `test-ops-gowork` skip with a notice** when cargo-deny or Go is missing, because CI runs `cargo deny` in its own job.

## Pitfalls

- Cargo fails on a member glob that matches nothing, as cargo 1.97 shows. Test any members change with `make test-ops-workspace`.
- `exclude` does not remove the literal fallback path of an unmatched glob, because Cargo treats a path under an explicit member as included.

## Verify the current state

```bash
make lint test   # after step 3; expected to pass
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-02T17:27:20Z | claude-ops01-8248 | claim | 096ecd5 | Claimed from `status:ready`, no earlier cards |
| 2026-11-02T17:36:00Z | claude-ops01-8248 | checkpoint | pending | Steps 2 and 3 done, `make lint test` passes |
