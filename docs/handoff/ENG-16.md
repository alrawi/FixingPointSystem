# Handoff ENG-16 Profiles and distributions in the template engine

<!-- Copy this file to docs/handoff/<ID>.md when you claim a work package.
     Update it at every checkpoint. A successor reads only this file, the
     issue, and the branch, so write for an agent that knows nothing else. -->

| Field | Value |
|---|---|
| Issue | #230 |
| Branch | `wp/ENG-16` |
| Pull request | none yet |
| Status | in progress |
| Progress | 90% (4 of 4 plan steps, CI pending) |
| Current holder | claude-eng16a, lease until 2026-11-07T18:51:39Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

fps-flow's template parser and evaluator accept `profiles.<name>` and `dist.pareto|lognormal|exponential|uniform` with the argument rules of CON-10, so `web_profile.yaml` parses.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Parser, checker, evaluator for dist and profiles
- [x] 2. Loader folds scenario profiles into templates
- [x] 3. Unit and example tests
- [x] 4. Engine wiki page, log entry, PR (CI pending)

## Done and verified

- Steps 1 to 3: `cargo test -p fps-flow --offline` passes (56 unit, 6 profiles, grammar, templates, contract examples), `cargo clippy -p fps-flow --all-targets -- -D warnings` and `cargo fmt -p fps-flow --check` are clean, `cargo check -p fps-interp` builds, `pytest engine/fps-flow/tests` passes.
- Step 4: `plan.py check`, `render --check`, and `lint` pass.

## Remaining

CI result on the pull request, then `fps_swarm.py review`.

## Next action

Wait for CI on the PR, then run `fps_swarm.py review 230 --pr <PR>`.

## Blockers and open questions

none

## Decisions taken in this package

- The loader folds `profiles.<name>` into text at compile time (`Template::fold_profiles`), so `RenderCtx` and `fps-interp` stay unchanged.
- `dist.uniform` has no cap parameter, so its draw is not limited to the default cap.
- Every `dist` call draws a fixed number of generator words (lognormal 2, the others 1).
- `CheckScope` gained a `profiles` field. It is only built inside `fps-flow`.

## Pitfalls

The template grammar has no exponent notation, so write `1000000000000`, not `1e12`.

## Verify the current state

```bash
# Commands that a successor runs first, with the expected result
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-07T18:41Z | claude-eng16a | claim | - | claimed #230, branch wp/ENG-16 from main |
| 2026-11-07T19:10Z | claude-eng16a | checkpoint | - | implementation, tests, wiki; PR next |
