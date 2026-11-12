# Handoff MOD-26 A module development guide for third party authors

| Field | Value |
|---|---|
| Issue | #502 |
| Branch | `wp/MOD-26` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 5 plan steps) |
| Current holder | claude-a0f343, lease until 2026-11-12T15:15:44Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Third parties are to build their own modules, so `docs/wiki/Module-Development.md` becomes the public guide for module authors outside the project, and `modules/_template/` becomes a complete starting point that passes `tools/contracts/validate.py`. Every statement of the guide names the file it rests on, and the guide says plainly what a third party can do today and what is first party only. Paths: `modules/_template/`, plus (owner and coordinator authorized) `docs/wiki/Module-Development.md`, one link in `docs/wiki/Modules.md`, and the new file `mk/mod-template.mk`.

## Plan

- [ ] 1. Read the sources and claim the package, then push the claim commit.
- [ ] 2. Make `modules/_template/` complete and add `mk/mod-template.mk` with `lint-mod-template`.
- [ ] 3. Write the guide, grounded in the sources.
- [ ] 4. Prove the walkthrough from an empty directory in the scratchpad and validate the result.
- [ ] 5. Run the checks, open the PR, wait for CI, run the review step.

## Done and verified

Nothing yet.

## Remaining

Everything after the claim.

## Next action

Write `modules/_template/manifest.yaml` and its scenario.

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
| 2026-11-12T14:00Z | claude-a0f343 | claim | none | claimed #502 |
