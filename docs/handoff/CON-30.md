# Handoff CON-30 Module manifest rules that match the registry and the agent

| Field | Value |
|---|---|
| Issue | #511 |
| Branch | `wp/CON-30` |
| Pull request | #0 |
| Status | in progress |
| Progress | 80% (3 of 4 plan steps) |
| Current holder | claude-1a1d5e, lease until 2026-11-12T17:55:25Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Correct the contract text and schema descriptions that MOD-26 found wrong, decide and state the `extension.wasm` path, and make `tools/contracts/validate.py` check every rule the registry enforces, with tests. The owner approved the change on 2026-11-12 (comment on #511).

## Plan

- [x] 1. Decide the wasm path, write `contracts/changes/CON-30.md`, correct `contracts/README.md` and the schema descriptions, copy the schema to `controller/internal/registry/schema/`.
- [x] 2. Add the validator checks (reserved names, license, file names, symlinks, expected_bytes keys) with tests.
- [x] 3. Log entry with the exact Module Development replacement text, run the checks, open the PR.
- [ ] 4. CI green, review.

## Done and verified

`python3 -m pytest -q tools/contracts` 350 passed; `validate.py` clean on examples, the 15 first party modules and the template; plan.py check, render --check, lint ok.

## Remaining

PR, CI, review. Out of scope fixes the coordinator must authorize: Go test `dns.wasm` and `ui/src/api/schema.d.ts` (see contracts/changes/CON-30.md).

## Next action

Open the PR and poll CI.

## Blockers and open questions

none

## Decisions taken in this package

The wasm path is the constant `extension.wasm` in the schema (the agent cannot honor a path without ENG/AGT changes). The `expected_bytes` key check is validator only, since the registry has none. Manifest v0.4 to v0.5.

## Pitfalls

The registry does not check that a key of `expected_bytes` names a scenario, although the package text lists it with the registry rules.

## Verify the current state

```bash
python3 -m pytest -q tools/contracts
python3 tools/contracts/validate.py
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-12T16:25Z | claude-1a1d5e | claim | none | Claimed #511 |
