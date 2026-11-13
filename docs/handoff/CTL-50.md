# Handoff CTL-50 Registry rules as strict as the validator

| Field | Value |
|---|---|
| Issue | #528 |
| Branch | `wp/CTL-50` |
| Pull request | #529 |
| Status | blocked |
| Progress | 95% (3 of 3 plan steps) |
| Current holder | see the status card on #528 |
| Last verified commit | none |

Status takes one of five values, which are in progress, paused, blocked, ready for review, and done.

## Goal

`reservedModuleNames` holds all eight reserved names, and `checkTree` refuses an `expected_bytes` key that names no scenario of the manifest. The Gaps section and Names section of Module Development change through the Shared page notes of the log entry.

## Plan

- [x] 1. Code: eight reserved names, `unknownEstimateKeys` in `checkTree`, tests.
- [x] 2. Docs: Controller page, log entry with the Shared page notes, this handoff.
- [x] 3. PR and CI. Review waits for the CON edit.

## Done and verified

- `gofmt -l controller/internal/registry` is clean. Go cannot build here, so CI runs the Go tests.
- `python3 -m pytest tools/contracts/test_contracts.py -k reserved`: `test_reserved_names_equal_the_registry_list` fails as expected, because the validator constant has five names. Needs a CON change, see the log entry.

## Remaining

The CON change of the log entry, then CI green and review.

## Next action

Coordinator applies the CON edit of the log entry to `tools/contracts/`, then the `contracts` job turns green and the review step runs.

## Blockers and open questions

The contract test needs a CON edit (`tools/contracts/validate.py`, `REGISTRY_RESERVED_NAMES`, and the test). The coordinator applies it.

## Decisions taken in this package

- The key check waits when a scenario file gave no name, so a failed file does not make its scenario's key look wrong.
- The finding path is `/expected_bytes/<key>` with JSON Pointer escaping.

## Pitfalls

The Go map must stay one literal that the contract test's regular expression can read (`reservedModuleNames = map[string]bool{...}` with no `}` inside).

## Verify the current state

```bash
python3 -m pytest tools/contracts/test_contracts.py -q -k reserved
```

## Session log

- 2026-11-12: claimed, code and docs written, PR #529 opened. CI: Go tests, controller-e2e, ops, licenses, proto, scripts green. `contracts` fails only on `test_reserved_names_equal_the_registry_list` (needs the CON edit). `rust` fails on a rustfmt diff in `modules/ldap/native/src/catalog.rs`, which main has too.
