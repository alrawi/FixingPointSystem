# Handoff MOD-03 Content generators

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #170 |
| Branch | `wp/MOD-03` |
| Pull request | #0 |
| Status | in progress |
| Progress | 90% (5 of 6 plan steps, PR pending) |
| Current holder | claude-264413, lease until 2026-11-06T23:01:12Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Ship synthetic content generators for HTML, JSON APIs, multilingual text, email bodies, and binary file types as native template functions of the module `content`, with size parameters and seeded variety, and a provenance note that states every generator is synthetic. Acceptance: the duplicate payload ratio across 10,000 generated bodies is at most 1%.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Crate skeleton, catalog, argument binding, manifest, word lists
- [x] 2. HTML and JSON generators
- [x] 3. Text and email generators
- [x] 4. Binary file generators
- [x] 5. Tests: determinism, size, duplicate ratio over 10,000 bodies per generator, catalog equals manifest
- [ ] 6. README provenance note, mk file, Modules page, log entry, PR

## Done and verified

- Steps 1 to 5: `cargo test -p fps-module-content` passes (15 unit tests, 3 catalog tests, 18 generator tests, 45 s in debug, 8 s in release). `cargo clippy -p fps-module-content --all-targets -- -D warnings` and `cargo fmt` are clean. `make lint-mod-content` reports 0 errors.
- Step 6 partly: README with the provenance section, `mk/mod-content.mk`, the Modules page section, and the log entry are written.

## Remaining

Open the pull request, wait for CI, and fix failures inside the paths of the package. The JPEG kind has no image scan, which the README states.

## Next action

Run `plan.py check`, `render --check`, and `lint`, push, and open the PR.

## Blockers and open questions

none

## Decisions taken in this package

- Output reaches `size` and ends at most about 1 KiB above it, because a generator finishes its current element. Exact sizes would cut HTML and JSON open. The `bin` kind is exact.
- A negative `seed` (default -1) draws from the session generator, and a seed of zero or more fixes the bytes. This gives determinism per seed and variety per session.
- Formats avoid compression: stored deflate blocks, a GIF LZW stream with a clear code every 200 pixels, and JPEG comment segments.
- No CI workflow was created, as the assignment says. The log entry describes the option.

## Pitfalls

- At 512 bytes a PDF repeated 70% of its bodies, because one or two sentences carry little entropy, so the PDF content starts with a title line that has a random identifier.
- The generic `fit` helper probes 33 units to estimate the bytes per unit. A probe of one unit overshot the GIF size by 9 KB.

## Verify the current state

```bash
make test-mod-content   # 4 test binaries pass
make lint-mod-content   # contracts: 0 error(s), 0 warning(s)
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-06T22:05Z | claude-2e55b9 | claim | | Claimed #170 with 6 plan steps |
| 2026-11-06T22:40Z | claude-264413 | takeover of the stopped worker | | Resumed from the claim commit, claimed with a new card, built steps 1 to 5 and the documents |
