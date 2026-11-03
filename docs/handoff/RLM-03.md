# Handoff RLM-03 Module realism CI job

| Field | Value |
|---|---|
| Issue | #53 |
| Branch | `wp/RLM-03` |
| Pull request | #127 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-rlm-03-ae11, lease until 2026-11-03T17:38:24Z |
| Last verified commit | 7d646de |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Run every changed module in the namespace harness, capture its traffic, and fail the pull request when a realism check fails. The workflow detects the changed module directories, scores each fresh capture with nDPI, tshark, and Zeek, and uploads the realism reports as a workflow artifact. A pull request that breaks the http1 request line fails the job.

## Plan

Each step takes at most 30 minutes of work and ends in a state that tests can check.

- [x] 1. Module selection from changed paths in `realism/modules.py` with unit tests
- [x] 2. Per module capture and score script under `realism/ci/`
- [x] 3. Workflow `.github/workflows/realism.yml` with the detect and module jobs and the report artifact
- [x] 4. Negative control that a broken http1 request line fails the score
- [x] 5. Component page, log entry, CI green, and review

## Done and verified

1. `realism/modules.py` discovers native modules (a directory under `modules/` with `native/Cargo.toml`, `fixtures/capture.sh`, and `fixtures/expected.yaml`) and selects them from changed paths. `python3 -m realism modules` prints the matrix or the names. Verified with `realism/tests/test_modules.py`.
2. `realism/ci/module-realism.sh` runs the capture script of one module and scores the capture with `--parsers tshark,zeek`, then `realism/ci/summary.py` writes the job summary rows. Verified locally as root for dns and http1 with `FPS_REALISM_PARSERS=tshark`, because Zeek is not installed here.
3. `.github/workflows/realism.yml` has the jobs `detect`, `module` (matrix), and `negative-control`, and uploads `realism-<module>` artifacts. `realism/ci/install-tools.sh` installs the tools. Run 41672489089 passed every job, with tshark 4.2.2 and Zeek 9.0.1 and zero parser errors for both modules.
4. `realism/ci/negative_control.py` removes the space after the method in `modules/http1/native/src/message.rs`. In run 41672489089 the run exited with 1, with 0 of 120 sessions conforming and 120 Zeek parser errors. Locally nDPI also gave 0 of 120.
5. `make lint-rlm test-rlm` passes with 139 tests and 3 skips. `plan.py check`, `render --check`, and `lint` pass. Realism.md section, realism README section, and the log entry are added.

## Remaining

The job does not run WASM modules, which do not exist yet. The coordinator may add `e2e-rlm-modules` to Testing.md, as the log entry notes.

## Next action

Review pull request #127.

## Blockers and open questions

none

## Decisions taken in this package

- The job lives in `realism.yml`, as the plan entry names, and not in `rlm.yml`, because its triggers include `modules/**` and it is much slower than the unit tests.
- A change to `realism/`, `engine/`, `tests/harness/`, the root Cargo files, `mk/rlm.mk`, or the workflow selects every module. A test keeps `SHARED_PATHS` equal to the workflow triggers.
- The job scores with an explicit `tshark,zeek` list, so a missing parser is an error and not a silent pass.
- The negative control runs in CI and not only once, so the job keeps proving that it can fail. It fails with exit code 2 when `message.rs` no longer has the edited lines.
- Zeek comes from the openSUSE Build Service repository of the Zeek project, the same source as `rlm.yml`.

## Pitfalls

- Parser error application entries have no `passed` field, so the summary derives the verdict from `parser_errors`.
- `sudo` resets the environment, so the workflow passes `GITHUB_STEP_SUMMARY` and `FPS_FIXTURE_PEER` through `sudo env`.

## Verify the current state

```bash
make lint-rlm test-rlm          # 139 passed, 3 skipped
sudo make e2e-rlm-modules       # needs ndpiReader, tshark, and Zeek; exits 0
```

## Session log

Append one row per session start, checkpoint, release, block, takeover, and finish. Never edit earlier rows.

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-03T16:10Z | claude-rlm-03-ae11 | claim | | Branch created from origin/main 426fcb7 |
| 2026-11-03T16:50Z | claude-rlm-03-ae11 | finish | 7d646de | realism run 41672489089 green, ready for review |
