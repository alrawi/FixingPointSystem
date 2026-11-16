# Handoff CON-33 Example tests pair with the example modules

| Field | Value |
|---|---|
| Issue | #577 |
| Branch | `wp/CON-33` |
| Pull request | #581 |
| Status | blocked |
| Progress | 90% (CI red on two test files outside the paths) |
| Current holder | claude-c33a7e, lease until 2026-11-17T00:23:07Z |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

Issue #482: `contracts/examples/tests/http_smoke.yaml` names `http1/web_get`, which belongs to the example module under `contracts/examples/modules/http1` and not to the first party `http1` module. The validator, the plan example, the UI mock, the CLI end to end test, and S4 rely on it, so it keeps its name, and the examples say which modules a test pairs with. No schema or version change (vetted under D13).

## Plan

- [x] 1. Comment line and README sentence, `http1_first_party.yaml`, registration in `FIRST_PARTY_TESTS`, the wrong scenario test, and `contracts/changes/CON-33.md`.
- [x] 2. Run the contract checks and the controller, UI, CLI, and fps-flow tests that read the examples.
- [ ] 3. Open the pull request, wait for CI, and hand over for review.

## Done and verified

- Step 1: `python3 tools/contracts/validate.py` reports 0 errors, 0 warnings.
- Step 2: `make test-con` (354 passed, 1 skipped), `python3 -m pytest spikes/s4-trex-astf/tests` (8 passed), `cargo test -p fps-hosts --test contract_examples` (3 passed), `plan.py check`, `render --check`, `lint` ok. The Go controller tests and the UI tests cannot run here (no Go modules, no node_modules).

## Remaining

The controller store tests and one UI test read every file of `examples/tests/` and need an edit outside the paths of CON-33 (see Pitfalls). The coordinator decides who makes it.

## Next action

Apply the two test edits named in Pitfalls (or have the vetting agent widen the paths), push, and wait for CI.

## Blockers and open questions

none

## Decisions taken in this package

`http1/web_get` is not renamed. The example test pairs with the example modules, and a new test pairs with the first party module.

## Pitfalls

The new example is read by `controller/internal/store` tests through `examples()`, which validate every example against the example modules. `exampleModuleScenarios` in `controller/internal/store/helpers_test.go` maps `dns/lookup_a` to `dns/resolve` for `dns_zone.yaml`, and needs `http1/single_fetch` mapped to `http1/web_get`. `ui/src/pages/ConfigEditor.test.tsx` line 82 expects 7 table rows (header and 6 configurations) and sees 8, and line 298 expects 6 stored configurations and sees 7. CI on 39bb260 confirmed all three: `ctl` / `test` (TestValidateEveryExample, TestEveryExampleStoresAndValidates: no installed module http1 exports scenario http1/single_fetch) and `ui` / `check` (the two length assertions). Both files are outside the paths `contracts/` and `tools/contracts/`, so this package did not edit them.

## Verify the current state

```bash
python3 tools/contracts/validate.py
python3 -m pytest tools/contracts
```

## Session log

| Time (UTC) | Agent | Event | Commit | Note |
|---|---|---|---|---|
| 2026-11-16T23:53Z | claude-c33a7e | claim | | Claimed #577, branch wp/CON-33 from origin/main |
| 2026-11-17T00:05Z | claude-c33a7e | checkpoint | 39bb260 | Deliverables written, contract checks pass, PR #581 open |
| 2026-11-17T00:25Z | claude-c33a7e | block | | CI red as predicted on three assertions in two test files outside the paths of CON-33 |
