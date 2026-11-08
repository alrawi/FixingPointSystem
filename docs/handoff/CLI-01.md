# Handoff CLI-01 Python SDK

| Field | Value |
|---|---|
| Issue | #168 |
| Branch | `wp/CLI-01` |
| Pull request | #245 |
| Status | ready for review |
| Progress | 100% (5 of 5 plan steps) |
| Current holder | claude-c1a0e1, lease until 2026-11-08T22:22:01Z |
| Last verified commit | 8e0261f |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Generate a Python client from the OpenAPI contract and add helpers for start, wait, and report, with a currency check and tests against a stub server built from the OpenAPI examples. The package has no runtime dependency.

## Plan

- [x] 1. Generator `sdk/python/scripts/generate_client.py` and the committed `_generated.py`
- [x] 2. `Client` transport, bearer token, errors, and run helpers
- [x] 3. Stub server and tests (client and currency check)
- [x] 4. README, `docs/wiki/SDK.md`, log entry
- [x] 5. Open the pull request, wait for CI, request review

## Done and verified

- `cd sdk/python && python3 -m pytest -q`: 30 passed.
- `ruff format --check`, `ruff check`, and `mypy --config-file pyproject.toml fixingpoint scripts tests`: clean.
- `python3 scripts/generate_client.py --check`: exit 0.
- `plan.py check`, `render --check`, and `lint`: ok.

## Remaining

Nothing for this package. No workflow covers `sdk/python/`, so CI runs nothing for it until the coordinator adds `.github/workflows/cli-sdk.yml` and `mk/cli.mk` (described in the log entry).

## Next action

Review pull request #245.

## Blockers and open questions

none

## Decisions taken in this package

- The generator needs PyYAML and the client does not, so the SDK stays dependency free at runtime.
- The generated file is excluded from ruff and mypy errors, because the generator controls its style.
- The OpenAPI document has no per operation examples, so the stub builds bodies from the response schemas and uses the documented `examples` properties.
- `wait_for_run` returns a failed run by default and raises `RunFailedError` on request, so callers can read the reason either way.
- The client refuses redirects so the bearer token cannot leak to another host.

## Pitfalls

The worktree guard refuses heredocs and `source` in shell commands, so write files with the editor tools.

## Verify the current state

```bash
cd sdk/python && python3 -m pytest -q    # 30 passed
python3 scripts/generate_client.py --check
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-08T20:52Z | claude-c1a0e1 | claim | none | claimed #168 |
| 2026-11-08T20:58Z | claude-c1a0e1 | checkpoint | 8e0261f | code, tests, docs written, PR #245 opened |
| 2026-11-08T21:05Z | claude-c1a0e1 | finish | 8e0261f | CI green, ready for review |
