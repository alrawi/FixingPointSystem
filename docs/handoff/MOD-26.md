# Handoff MOD-26 A module development guide for third party authors

| Field | Value |
|---|---|
| Issue | #502 |
| Branch | `wp/MOD-26` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (4 of 5 plan steps) |
| Current holder | claude-a0f343, lease until 2026-11-12T15:15:44Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Third parties are to build their own modules, so `docs/wiki/Module-Development.md` becomes the public guide for module authors outside the project, and `modules/_template/` becomes a complete starting point that passes `tools/contracts/validate.py`. Every statement of the guide names the file it rests on, and the guide says plainly what a third party can do today and what is first party only. Paths: `modules/_template/`, plus (owner and coordinator authorized) `docs/wiki/Module-Development.md`, one link in `docs/wiki/Modules.md`, and the new file `mk/mod-template.mk`.

## Plan

- [x] 1. Read the sources and claim the package, then push the claim commit.
- [x] 2. Make `modules/_template/` complete and add `mk/mod-template.mk` with `lint-mod-template`.
- [x] 3. Write the guide, grounded in the sources.
- [x] 4. Prove the walkthrough from an empty directory in the scratchpad and validate the result.
- [ ] 5. Run the checks, open the PR, wait for CI, run the review step.

## Done and verified

- The template validates (`make lint-mod-template` prints 0 errors, 0 warnings). A copy made by the guide commands in an empty scratch directory validates, and an unknown function or a wrong directory name fails with exit 1.
- The tree digest from `check_plans.py --print-digests` equals an independent Python computation. An Ed25519 signature made with Python `cryptography` verifies in a scratch Go module built from the stdlib-only files of `controller/internal/registry/signing/`, the key id matches, and a flipped byte fails.
- `plan.py check`, `render --check`, and `lint` pass. `make lint-rust` could not run, because the disk filled during the build (linker bus error), and the rust job of CI covers it. No Rust file changed.

## Remaining

Open the pull request, wait for CI, and run the review step.

## Next action

Push, open the pull request with `gh api`, and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

none yet

## Pitfalls

- The worktree guard refuses `source`, `export`, and heredocs in a command that runs git. Pass variables inline.

## Verify the current state

```bash
python3 tools/contracts/validate.py --modules <parent directory with a link named template> --no-openapi
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T13:50Z | claude-a0f343 | claim | none | claimed #502 |
