# Handoff CON-33 Example tests pair with the example modules

| Field | Value |
|---|---|
| Issue | #577 |
| Branch | `wp/CON-33` |
| Pull request | #0 |
| Status | in progress |
| Progress | 0% (0 of 3 plan steps) |
| Current holder | claude-c33a7e, lease until 2026-11-17T00:23:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Issue #482: `contracts/examples/tests/http_smoke.yaml` names `http1/web_get`, which belongs to the example module under `contracts/examples/modules/http1` and not to the first party `http1` module. The validator, the plan example, the UI mock, the CLI end to end test, and S4 rely on it, so it keeps its name, and the examples say which modules a test pairs with. No schema or version change (vetted under D13).

## Plan

- [ ] 1. Comment line and README sentence, `http1_first_party.yaml`, registration in `FIRST_PARTY_TESTS`, the wrong scenario test, and `contracts/changes/CON-33.md`.
- [ ] 2. Run the contract checks and the controller, UI, CLI, and fps-flow tests that read the examples.
- [ ] 3. Open the pull request, wait for CI, and hand over for review.

## Done and verified

Nothing yet.

## Remaining

Everything above.

## Next action

Edit `contracts/examples/tests/http_smoke.yaml` and `contracts/README.md`.

## Blockers and open questions

none

## Decisions taken in this package

`http1/web_get` is not renamed. The example test pairs with the example modules, and a new test pairs with the first party module.

## Pitfalls

none yet

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16T23:53Z | claude-c33a7e | claim | | Claimed #577, branch wp/CON-33 from origin/main |
